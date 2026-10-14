//! SQLite in-DB SQL-on-FHIR runner.
//!
//! [`SqliteInDbRunner`] compiles a ViewDefinition to a parameterised SQLite
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are sent in batches through the producer core's channel
//! ([`row_batch_channel`]) so the HTTP layer can begin flushing to the client
//! before the full result set is read. The blocking SQLite iteration runs in a
//! dedicated, supervised `spawn_blocking` thread so it never stalls the async
//! runtime; a panic inside it reaches the consumer as an `Err` item instead of
//! a silent end of stream.
//!
//! ## Stopping
//!
//! A run stops when its stream is dropped, its cancel token fires or its
//! deadline passes. SQLite has no server to cancel, so the runner interrupts
//! the statement on its own connection:
//!
//! - a [`StatementInterrupter`] holds the connection's interrupt handle while,
//!   and only while, the run's statement executes. An async bridge waits for
//!   the stop, records its reason, then interrupts;
//! - a [`BatchFlusher`] progress handler runs every [`PROGRESS_OPS`] virtual
//!   machine instructions. It aborts the statement once a stop reason is
//!   recorded or the deadline has passed (so a deadline needs no async
//!   wake-up), and it sends a partial batch that has waited
//!   [`BATCH_LINGER`], so sparse rows are not held back by batching;
//! - a stop-aware busy handler replaces the connection's busy timeout while
//!   the run's statements wait for a database lock (SQLite's lock wait calls
//!   neither the progress handler nor checks an interrupt). It waits in
//!   [`BUSY_SLICE`] sleeps, still bounded by the connection's own busy
//!   timeout, and gives up at once when a stop is recorded or the deadline
//!   passes, so a lock-blocked statement stops like a stepping one;
//! - all three are removed (the interrupter disarmed under its lock, the
//!   connection's own busy timeout restored) before the connection goes back
//!   to the r2d2 pool, which also serves ordinary writes: an interrupt never
//!   reaches a later statement on that connection.
//!
//! A stop's error kind comes from its [`StopReason`], never from SQLite's
//! "interrupted" error.
//!
//! The columnar path ([`SofRunner::run_view_columnar`]) is the same run with
//! another sink: each row's cells go straight into Arrow builders and whole
//! record batches go through the channel, so all of the above applies to it.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use helios_fhir::FhirVersion;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};

use helios_sof::columnar::{COLUMNAR_BATCH_ROWS, ColumnarBatchBuilder};
use helios_sof::sqlquery::engine::declared_columns;

use crate::core::sof_producer::{
    BATCH_LINGER, BatchSender, RecordBatchSender, RunStop, StopFlag, bridge_stop,
    record_batch_channel, row_batch_channel, supervise,
};
use crate::core::sof_runner::{
    BatchStream, RowStream, RunCapabilities, RunContext, RunOutput, RunRequest, SofError,
    SofRunner, StopReason,
};
use crate::tenant::TenantContext;

use super::compiler::SqlDialect;
use super::decode::{ColumnDecode, decode_text};
use super::runtime::{RuntimeParam, SqlPrepared, prepare_sql};

/// The runner name in logs.
const RUNNER: &str = "sqlite-indb";

/// The progress handler runs every this many SQLite virtual machine
/// instructions (a few microseconds of work).
pub(crate) const PROGRESS_OPS: i32 = 1_000;

/// A stopped run waiting for a pooled connection notices the stop within
/// this long.
const CHECKOUT_SLICE: Duration = Duration::from_millis(50);

/// A stopped run whose statement waits for a database lock notices the stop
/// within this long (one sleep of its busy handler).
const BUSY_SLICE: Duration = Duration::from_millis(10);

/// SQL-on-FHIR runner that compiles ViewDefinitions to SQLite SQL.
pub struct SqliteInDbRunner {
    pool: Pool<SqliteConnectionManager>,
    fhir_version: FhirVersion,
}

impl SqliteInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            fhir_version: FhirVersion::default_enabled(),
        }
    }

    /// Returns a runner that consults the given FHIR version's field-type
    /// table when validating `collection: false` columns.
    pub fn with_fhir_version(mut self, version: FhirVersion) -> Self {
        self.fhir_version = version;
        self
    }
}

#[async_trait::async_trait]
impl SofRunner for SqliteInDbRunner {
    fn runner_name(&self) -> &'static str {
        RUNNER
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        request: RunRequest,
        ctx: RunContext,
    ) -> Result<RunOutput, SofError> {
        let RunRequest {
            view: view_definition,
            filters,
        } = request;
        let tenant_id = tenant.tenant_id().to_string();
        let group_pool = self.pool.clone();
        let group_tenant = tenant_id.clone();
        // The setup (the Group load) runs under the run's stop as well.
        let setup_stop = RunStop::new(&ctx);
        let group_stop = setup_stop.clone();
        let prepared = match prepare_sql(
            &view_definition,
            SqlDialect::Sqlite,
            self.fhir_version,
            &tenant_id,
            filters,
            false,
            move |refs| load_group_documents(group_pool, group_tenant, refs, group_stop),
        )
        .await?
        {
            SqlPrepared::Run(prepared) => prepared,
            SqlPrepared::Empty {
                schema,
                total_order,
            } => {
                // A Group without Patient members selects nothing, but a run
                // that was stopped by now ends with its stop, not with a
                // clean empty result.
                if let Some(reason) = setup_stop.check() {
                    return Err(stopped_error(&setup_stop, reason));
                }
                return Ok(RunOutput::new(
                    schema,
                    capabilities(total_order),
                    Box::pin(futures::stream::empty()),
                ));
            }
        };
        let schema = prepared.schema;
        let total_order = prepared.total_order;
        let compiled = prepared.query;
        // Once per run: the compiled projection's order into document order.
        let permutation = schema.permutation_from(&compiled.columns)?;
        debug!(runner = RUNNER, tenant = %tenant_id, "executing compiled ViewDefinition");
        trace!(
            runner = RUNNER, sql = %compiled.sql, columns = ?compiled.columns,
            constants = compiled.constants.len(), "compiled ViewDefinition SQL"
        );
        let run = SqliteRun {
            pool: self.pool.clone(),
            sql: compiled.sql,
            params: prepared
                .params
                .into_iter()
                .map(SqliteParam::from_runtime)
                .collect(),
            columns: compiled.columns,
            decodes: compiled.column_decodes,
            limit: prepared.client_limit,
        };
        Ok(RunOutput::new(
            schema,
            capabilities(total_order),
            permutation.normalize(spawn_sqlite_run(run, &ctx)),
        ))
    }

    /// Columnar sibling of [`run_view`](Self::run_view): each cell is
    /// appended straight to its Arrow builder, without the JSON step. The run
    /// is the row path's run in every other respect (#1864): the same setup
    /// under the run's stop, interrupter, progress handler, deadline and
    /// stop-aware busy handler ([`produce`]), and typed errors. Each batch's
    /// columns are moved into schema order.
    ///
    /// Returns `Ok(None)` when two output columns share a name: the row path
    /// collapses those into one JSON key, positional builders would keep both.
    async fn run_view_columnar(
        &self,
        tenant: &TenantContext,
        request: RunRequest,
        ctx: RunContext,
    ) -> Result<Option<BatchStream>, SofError> {
        let RunRequest {
            view: view_definition,
            filters,
        } = request;
        let tenant_id = tenant.tenant_id().to_string();
        let group_pool = self.pool.clone();
        let group_tenant = tenant_id.clone();
        // The setup (the Group load) runs under the run's stop as well.
        let setup_stop = RunStop::new(&ctx);
        let group_stop = setup_stop.clone();
        let prepared = match prepare_sql(
            &view_definition,
            SqlDialect::Sqlite,
            self.fhir_version,
            &tenant_id,
            filters,
            false,
            move |refs| load_group_documents(group_pool, group_tenant, refs, group_stop),
        )
        .await?
        {
            SqlPrepared::Run(prepared) => prepared,
            SqlPrepared::Empty { .. } => {
                if let Some(reason) = setup_stop.check() {
                    return Err(stopped_error(&setup_stop, reason));
                }
                return Ok(Some(Box::pin(futures::stream::empty())));
            }
        };
        let compiled = prepared.query;
        // The row path collapses duplicate column names into one JSON key;
        // positional builders would keep both, so leave those views to it.
        let mut seen = std::collections::HashSet::new();
        if !compiled.columns.iter().all(|c| seen.insert(c.as_str())) {
            return Ok(None);
        }
        // Once per run: the compiled projection's order into document order.
        let permutation = prepared.schema.permutation_from(&compiled.columns)?;
        let kinds = compiled
            .column_decodes
            .iter()
            .map(|d| d.column_kind())
            .collect();
        let mut builder =
            ColumnarBatchBuilder::new(compiled.columns.clone(), kinds, COLUMNAR_BATCH_ROWS)
                .map_err(|e| SofError::backend(format!("columnar builder: {e}")))?;
        builder
            .pin_declared_types(&declared_columns(&view_definition))
            .map_err(|e| SofError::backend(format!("columnar builder: {e}")))?;
        debug!(runner = RUNNER, tenant = %tenant_id, "executing compiled ViewDefinition (columnar)");
        trace!(
            runner = RUNNER, sql = %compiled.sql, columns = ?compiled.columns,
            constants = compiled.constants.len(), "compiled ViewDefinition SQL"
        );
        let run = SqliteRun {
            pool: self.pool.clone(),
            sql: compiled.sql,
            params: prepared
                .params
                .into_iter()
                .map(SqliteParam::from_runtime)
                .collect(),
            columns: compiled.columns,
            decodes: compiled.column_decodes,
            limit: prepared.client_limit,
        };
        Ok(Some(permutation.normalize_batches(
            spawn_sqlite_columnar_run(run, builder, &ctx),
        )))
    }
}

/// What a SQLite run declares: its schema always, a total order for flat and
/// `forEach` views, and a stop that ends the statement itself (interrupt and
/// progress handler), not only the client side.
fn capabilities(total_order: bool) -> RunCapabilities {
    RunCapabilities {
        total_order,
        declared_schema: true,
        cancels_server_side: true,
    }
}

/// Backend-only Group document loading, under the run's stop.
///
/// Acquisition, query preparation, reads and JSON decoding all run outside
/// the async runtime, on a blocking worker that waits for the pooled
/// connection in slices ([`checkout`]: a stop is noticed while waiting, an
/// exhausted pool is [`SofError::Unavailable`]) and reads with the run's
/// interrupter, progress handler and stop-aware busy handler installed
/// ([`RunHooks`]), so a cancel, a drop of the run's caller (its token fires)
/// or the deadline stops it like the run's own statement, whether it is
/// stepping or waiting for a database lock. The worker is joined before the
/// load reports its result.
async fn load_group_documents(
    pool: Pool<SqliteConnectionManager>,
    tenant_id: String,
    group_refs: Vec<String>,
    stop: RunStop,
) -> Result<Vec<Value>, SofError> {
    if let Some(reason) = stop.check() {
        return Err(stopped_error(&stop, reason));
    }
    let flag = StopFlag::default();
    let interrupter = StatementInterrupter::default();
    let done = CancellationToken::new();
    let _bridge = {
        let interrupter = interrupter.clone();
        bridge_stop(stop.clone(), flag.clone(), done.clone(), move || {
            interrupter.interrupt()
        })
    };
    let worker_stop = stop.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let _done = done.drop_guard();
        read_group_documents(
            &pool,
            &tenant_id,
            &group_refs,
            &worker_stop,
            &flag,
            &interrupter,
        )
    })
    .await
    .map_err(|e| SofError::Storage(format!("sqlite group lookup task failed: {e}")))?;
    loaded.map_err(|end| match end {
        RunEnd::Stopped(reason) => stopped_error(&stop, reason),
        RunEnd::Failed(error) => error,
    })
}

/// The blocking body of [`load_group_documents`].
fn read_group_documents(
    pool: &Pool<SqliteConnectionManager>,
    tenant_id: &str,
    group_refs: &[String],
    stop: &RunStop,
    flag: &StopFlag,
    interrupter: &StatementInterrupter,
) -> Result<Vec<Value>, RunEnd> {
    let conn = checkout(pool, stop, flag)?;
    let hook_flag = flag.clone();
    let deadline = stop.std_deadline();
    let _hooks = RunHooks::install(&conn, interrupter.clone(), flag, deadline, move || {
        stop_requested(&hook_flag, deadline)
    })
    .map_err(|e| interpret(e, flag, "group lookup setup failed"))?;
    // A stop that came before the interrupter was armed is seen here.
    if let Some(reason) = flag.get().or_else(|| stop.check()) {
        return Err(RunEnd::Stopped(reason));
    }
    let mut stmt = conn
        .prepare(
            "SELECT data FROM resources \
             WHERE tenant_id = ?1 \
               AND resource_type = 'Group' \
               AND id = ?2 \
               AND is_deleted = 0",
        )
        .map_err(|e| interpret(e, flag, "group lookup prepare failed"))?;
    let mut groups = Vec::with_capacity(group_refs.len());
    for reference in group_refs {
        let id = reference.strip_prefix("Group/").unwrap_or(reference);
        let res: rusqlite::Result<Vec<u8>> = stmt.query_row([tenant_id, id], |row| row.get(0));
        match res {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => groups.push(value),
                Err(_) => continue,
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => {
                return Err(interpret(
                    e,
                    flag,
                    &format!("group lookup failed for {reference}"),
                ));
            }
        }
    }
    Ok(groups)
}

/// The error a run (or its setup) stopped for `reason` ends with; a drop,
/// which has no error item, ends as cancelled.
fn stopped_error(stop: &RunStop, reason: StopReason) -> SofError {
    stop.error_for(reason).unwrap_or(SofError::Cancelled)
}

#[derive(Clone, Debug)]
enum SqliteParam {
    Text(String),
    Bool(bool),
    Int(i64),
    /// Decimal preserved as text — SQLite is dynamic-typed and accepts text
    /// for numeric comparisons.
    Decimal(String),
    Null,
}

impl SqliteParam {
    fn from_runtime(value: RuntimeParam) -> Self {
        match value {
            RuntimeParam::Text(value) => Self::Text(value),
            RuntimeParam::Literal(value) => Self::from_lit(&value),
            RuntimeParam::Timestamp(value) => Self::Text(value.to_rfc3339()),
            RuntimeParam::TextList(values) => Self::Text(
                serde_json::to_string(&values).expect("a list of strings always serialises"),
            ),
        }
    }

    fn from_lit(v: &super::ir::LitValue) -> Self {
        match v {
            super::ir::LitValue::Null => SqliteParam::Null,
            super::ir::LitValue::Bool(b) => SqliteParam::Bool(*b),
            super::ir::LitValue::Int(n) => SqliteParam::Int(*n),
            super::ir::LitValue::Decimal(s) => SqliteParam::Decimal(s.clone()),
            super::ir::LitValue::Str(s) => SqliteParam::Text(s.clone()),
        }
    }
}

impl rusqlite::ToSql for SqliteParam {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value};
        Ok(match self {
            SqliteParam::Text(s) => ToSqlOutput::Borrowed(s.as_str().into()),
            SqliteParam::Bool(b) => ToSqlOutput::Owned(Value::Integer(if *b { 1 } else { 0 })),
            SqliteParam::Int(n) => ToSqlOutput::Owned(Value::Integer(*n)),
            // Bind as REAL so SQLite's type-affinity rules let the value
            // compare numerically against `json_extract` results (which are
            // INTEGER/REAL for JSON numbers). Binding as TEXT puts the
            // value in a different storage class and SQLite ranks any TEXT
            // as greater than any numeric value, breaking `<` / `>`.
            SqliteParam::Decimal(s) => match s.parse::<f64>() {
                Ok(n) => ToSqlOutput::Owned(Value::Real(n)),
                Err(_) => ToSqlOutput::Owned(Value::Text(s.clone())),
            },
            SqliteParam::Null => ToSqlOutput::Owned(Value::Null),
        })
    }
}

// ============================================================================
// Blocking producer: interrupter, progress handler, row loop
// ============================================================================

/// One run's statement and how to read it.
pub(crate) struct SqliteRun {
    pub(crate) pool: Pool<SqliteConnectionManager>,
    pub(crate) sql: String,
    params: Vec<SqliteParam>,
    pub(crate) columns: Vec<String>,
    pub(crate) decodes: Vec<ColumnDecode>,
    /// Rows the client stops at.
    pub(crate) limit: Option<usize>,
}

/// Interrupts the statement a run's connection is executing.
///
/// Armed with the connection's interrupt handle only while the run's
/// statement executes, and disarmed under its lock before the connection goes
/// back to the pool: once disarmed, no interrupt can reach the connection, so
/// a stop that races the end of the run never hits the next borrower's
/// statement. (`sqlite3_interrupt` also only affects statements that are
/// running when it is called, and its effect ends when they do.)
#[derive(Clone, Default)]
pub(crate) struct StatementInterrupter(Arc<Mutex<Option<rusqlite::InterruptHandle>>>);

impl StatementInterrupter {
    fn arm(&self, conn: &rusqlite::Connection) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(conn.get_interrupt_handle());
    }

    fn disarm(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Interrupts the armed statement, if any.
    pub(crate) fn interrupt(&self) {
        if let Some(handle) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            handle.interrupt();
        }
    }
}

/// Where a run's rows go: JSON rows through the batch channel
/// ([`run_view`](SofRunner::run_view)), or cells appended to Arrow builders
/// and sent as record batches
/// ([`run_view_columnar`](SofRunner::run_view_columnar)).
enum SqliteSink {
    Rows(BatchSender),
    Columnar {
        out: RecordBatchSender,
        builder: Box<ColumnarBatchBuilder>,
    },
}

impl SqliteSink {
    /// Adds one result row. `Ok(true)` when a batch is ready to be flushed.
    fn push_row(&mut self, row: &rusqlite::Row<'_>, run: &SqliteRun) -> Result<bool, RunEnd> {
        let row_error =
            |e: rusqlite::Error| RunEnd::Failed(SofError::backend(format!("row error: {e}")));
        match self {
            Self::Rows(out) => {
                let row = map_sqlite_row(row, &run.columns, &run.decodes)
                    .map(Value::Object)
                    .map_err(row_error)?;
                Ok(out.push(Ok(row)))
            }
            Self::Columnar { out, builder } => {
                append_sqlite_row(row, &run.decodes, builder).map_err(row_error)?;
                if !builder.end_row() {
                    return Ok(false);
                }
                match builder.flush() {
                    Ok(Some(batch)) => Ok(out.push(Ok(batch))),
                    Ok(None) => Ok(false),
                    Err(e) => Err(RunEnd::Failed(SofError::backend(format!(
                        "failed to build record batch: {e}"
                    )))),
                }
            }
        }
    }

    fn blocking_flush(
        &mut self,
        handle: &tokio::runtime::Handle,
        stop: &RunStop,
    ) -> Result<(), StopReason> {
        match self {
            Self::Rows(out) => out.blocking_flush(handle, stop),
            Self::Columnar { out, .. } => out.blocking_flush(handle, stop),
        }
    }

    fn pending_age(&self) -> Option<Duration> {
        match self {
            Self::Rows(out) => out.pending_age(),
            Self::Columnar { out, .. } => out.pending_age(),
        }
    }

    fn try_flush(&mut self) -> Result<(), StopReason> {
        match self {
            Self::Rows(out) => out.try_flush(),
            Self::Columnar { out, .. } => out.try_flush(),
        }
    }

    /// Ends the stream normally. A columnar run sends its last, partial
    /// batch first.
    fn blocking_finish(self, handle: &tokio::runtime::Handle, stop: &RunStop) {
        match self {
            Self::Rows(out) => out.blocking_finish(handle, stop),
            Self::Columnar {
                mut out,
                mut builder,
            } => match builder.flush() {
                Ok(Some(batch)) => {
                    out.push(Ok(batch));
                    out.blocking_finish(handle, stop);
                }
                Ok(None) => out.blocking_finish(handle, stop),
                Err(e) => out.blocking_fail(
                    handle,
                    stop,
                    SofError::backend(format!("failed to build record batch: {e}")),
                ),
            },
        }
    }

    /// Ends the stream with `error`. A columnar run's partial batch is
    /// dropped, never sent as if complete.
    fn blocking_fail(self, handle: &tokio::runtime::Handle, stop: &RunStop, error: SofError) {
        match self {
            Self::Rows(out) => out.blocking_fail(handle, stop, error),
            Self::Columnar { out, .. } => out.blocking_fail(handle, stop, error),
        }
    }

    fn blocking_end_after_stop(
        self,
        handle: &tokio::runtime::Handle,
        stop: &RunStop,
        reason: StopReason,
    ) {
        match self {
            Self::Rows(out) => out.blocking_end_after_stop(handle, stop, reason),
            Self::Columnar { out, .. } => out.blocking_end_after_stop(handle, stop, reason),
        }
    }
}

/// The run's sink, shared by the row loop and the progress handler (both on
/// the producer thread, never at the same time). `None` once the run ended.
type SharedSender = Arc<Mutex<Option<SqliteSink>>>;

/// The progress handler of a run: aborts the statement on a stop or a passed
/// deadline, and sends a partial batch that waited [`BATCH_LINGER`].
struct BatchFlusher {
    out: SharedSender,
    flag: StopFlag,
    deadline: Option<std::time::Instant>,
}

impl BatchFlusher {
    /// `true` aborts the running statement.
    fn check(&self) -> bool {
        if stop_requested(&self.flag, self.deadline) {
            return true;
        }
        if let Ok(mut out) = self.out.try_lock() {
            if let Some(out) = out.as_mut() {
                if out.pending_age().is_some_and(|age| age >= BATCH_LINGER)
                    && out.try_flush().is_err()
                {
                    self.flag.set(StopReason::Dropped);
                    return true;
                }
            }
        }
        false
    }
}

/// Whether a statement must be aborted now: a stop reason is recorded, or
/// the deadline passed (recorded as the reason).
fn stop_requested(flag: &StopFlag, deadline: Option<std::time::Instant>) -> bool {
    if flag.get().is_some() {
        return true;
    }
    if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
        flag.set(StopReason::DeadlineExceeded);
        return true;
    }
    false
}

/// The interrupter, the progress handler and the stop-aware busy handler,
/// installed on a connection for one run (or its Group load). Dropping it
/// disarms the interrupter, removes the progress handler and restores the
/// connection's own busy timeout; it is declared after the connection and
/// before the statement, so it drops after the statement is finalized and
/// before the connection goes back to the pool, on every path (an early
/// return or a panic included).
struct RunHooks<'c> {
    conn: &'c rusqlite::Connection,
    interrupter: StatementInterrupter,
    /// The connection's own busy timeout, restored on drop.
    busy_timeout: Duration,
    /// This thread's lock-wait state before the hooks were installed.
    previous_wait: Option<LockWait>,
}

impl<'c> RunHooks<'c> {
    /// Arms `interrupter`, installs `check` as the progress handler (`true`
    /// aborts the running statement) and replaces the connection's busy
    /// timeout with [`stop_aware_busy_wait`] for the run's `flag` and
    /// `deadline`, bounded by that same busy timeout.
    ///
    /// The busy handler is a plain function, so its state lives in a
    /// thread-local: SQLite calls it on the thread that runs the statement,
    /// which is the thread that installs (and later drops) the hooks.
    fn install(
        conn: &'c rusqlite::Connection,
        interrupter: StatementInterrupter,
        flag: &StopFlag,
        deadline: Option<std::time::Instant>,
        check: impl FnMut() -> bool + Send + 'static,
    ) -> rusqlite::Result<Self> {
        let busy_ms: u32 = conn.pragma_query_value(None, "busy_timeout", |row| row.get(0))?;
        let busy_timeout = Duration::from_millis(busy_ms.into());
        interrupter.arm(conn);
        conn.progress_handler(PROGRESS_OPS, Some(check));
        let previous_wait = LOCK_WAIT.with(|wait| {
            wait.replace(Some(LockWait {
                flag: flag.clone(),
                deadline,
                timeout: busy_timeout,
                since: std::time::Instant::now(),
            }))
        });
        // Built before the handler is set, so a failure below still undoes
        // everything above.
        let hooks = Self {
            conn,
            interrupter,
            busy_timeout,
            previous_wait,
        };
        conn.busy_handler(Some(stop_aware_busy_wait))?;
        Ok(hooks)
    }
}

impl Drop for RunHooks<'_> {
    fn drop(&mut self) {
        self.interrupter.disarm();
        self.conn.progress_handler(0, None::<fn() -> bool>);
        if let Err(e) = self.conn.busy_timeout(self.busy_timeout) {
            debug!(runner = RUNNER, error = %e, "failed to restore the connection's busy timeout");
        }
        let previous = self.previous_wait.take();
        LOCK_WAIT.with(|wait| *wait.borrow_mut() = previous);
    }
}

thread_local! {
    /// The lock-wait state of the run whose [`RunHooks`] are installed on a
    /// connection used by this thread; `None` when there is none.
    static LOCK_WAIT: RefCell<Option<LockWait>> = const { RefCell::new(None) };
}

/// What [`stop_aware_busy_wait`] needs to know about the run.
struct LockWait {
    flag: StopFlag,
    deadline: Option<std::time::Instant>,
    /// The connection's own busy timeout: no single lock wait lasts longer.
    timeout: Duration,
    /// When the current lock wait began.
    since: std::time::Instant,
}

/// The busy handler while a run's hooks are installed: SQLite calls it each
/// time a statement finds the database locked, with the number of earlier
/// calls for the same lock. It sleeps [`BUSY_SLICE`] (less near the timeout
/// or the deadline) and retries, up to the connection's own busy timeout, but
/// gives up at once (`SQLITE_BUSY`) when a stop is recorded or the deadline
/// passed (recorded as the reason), so [`interpret`] ends the run with its
/// stop. Without hooks on this thread it gives up, like no handler.
fn stop_aware_busy_wait(calls: i32) -> bool {
    LOCK_WAIT.with(|wait| {
        let mut wait = wait.borrow_mut();
        let Some(wait) = wait.as_mut() else {
            return false;
        };
        let now = std::time::Instant::now();
        if calls == 0 {
            wait.since = now;
        }
        if stop_requested(&wait.flag, wait.deadline) {
            return false;
        }
        let left = wait.timeout.saturating_sub(now.duration_since(wait.since));
        if left.is_zero() {
            return false;
        }
        let mut nap = BUSY_SLICE.min(left);
        if let Some(deadline) = wait.deadline {
            nap = nap.min(deadline.saturating_duration_since(now));
        }
        std::thread::sleep(nap);
        true
    })
}

/// How a run ended before all its rows were sent.
enum RunEnd {
    Stopped(StopReason),
    Failed(SofError),
}

/// Starts a run's blocking producer and its stop bridge, and returns the row
/// stream.
pub(crate) fn spawn_sqlite_run(run: SqliteRun, ctx: &RunContext) -> RowStream {
    let (out, stop, rows) = row_batch_channel(RUNNER, ctx);
    let reporter = out.reporter();
    supervise(
        RUNNER,
        reporter,
        start_producer(run, stop, SqliteSink::Rows(out)),
    );
    rows
}

/// Starts a columnar run's blocking producer and its stop bridge, and returns
/// the record-batch stream. The producer is the row path's ([`produce`]);
/// only where the rows go differs.
pub(crate) fn spawn_sqlite_columnar_run(
    run: SqliteRun,
    builder: ColumnarBatchBuilder,
    ctx: &RunContext,
) -> BatchStream {
    let (out, stop, batches) = record_batch_channel(RUNNER, ctx);
    let reporter = out.reporter();
    let sink = SqliteSink::Columnar {
        out,
        builder: Box::new(builder),
    };
    supervise(RUNNER, reporter, start_producer(run, stop, sink));
    batches
}

/// Spawns the blocking producer of a run and its stop bridge; returns both
/// tasks for [`supervise`].
fn start_producer(
    run: SqliteRun,
    stop: RunStop,
    out: SqliteSink,
) -> Vec<tokio::task::JoinHandle<()>> {
    let flag = StopFlag::default();
    let interrupter = StatementInterrupter::default();
    let done = CancellationToken::new();
    let bridge = {
        let interrupter = interrupter.clone();
        bridge_stop(stop.clone(), flag.clone(), done.clone(), move || {
            interrupter.interrupt()
        })
    };
    let handle = tokio::runtime::Handle::current();
    let producer = tokio::task::spawn_blocking(move || {
        let _done = done.drop_guard();
        produce(run, stop, out, flag, interrupter, handle);
    });
    vec![producer, bridge]
}

fn produce(
    run: SqliteRun,
    stop: RunStop,
    out: SqliteSink,
    flag: StopFlag,
    interrupter: StatementInterrupter,
    handle: tokio::runtime::Handle,
) {
    let out: SharedSender = Arc::new(Mutex::new(Some(out)));
    let result = read(&run, &stop, &out, &flag, &interrupter, &handle);
    let Some(out) = out.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    match result {
        Ok(rows) => {
            debug!(runner = RUNNER, rows, "in-DB view run complete");
            out.blocking_finish(&handle, &stop);
        }
        Err(RunEnd::Stopped(reason)) => {
            debug!(runner = RUNNER, ?reason, "in-DB view run stopped");
            out.blocking_end_after_stop(&handle, &stop, reason);
        }
        Err(RunEnd::Failed(error)) => out.blocking_fail(&handle, &stop, error),
    }
}

/// Waits for a pooled connection in short slices, so a stop is noticed while
/// waiting; gives up after the pool's own connection timeout with
/// [`SofError::Unavailable`] (503 with `Retry-After`: the pool stayed
/// exhausted, a retry can succeed).
fn checkout(
    pool: &Pool<SqliteConnectionManager>,
    stop: &RunStop,
    flag: &StopFlag,
) -> Result<PooledConnection<SqliteConnectionManager>, RunEnd> {
    let give_up_at = std::time::Instant::now() + pool.connection_timeout();
    loop {
        if let Some(reason) = flag.get().or_else(|| stop.check()) {
            return Err(RunEnd::Stopped(reason));
        }
        let left = give_up_at.saturating_duration_since(std::time::Instant::now());
        match pool.get_timeout(CHECKOUT_SLICE.min(left)) {
            Ok(conn) => return Ok(conn),
            Err(_) if !left.is_zero() => continue,
            Err(e) => {
                return Err(RunEnd::Failed(SofError::Unavailable {
                    message: format!("failed to acquire SQLite connection: {e}"),
                }));
            }
        }
    }
}

/// A SQLite error of the run: an interrupt, or a lock wait the busy handler
/// gave up ([`stop_aware_busy_wait`]), with a recorded stop reason is that
/// stop, anything else a backend error with `context`.
fn interpret(e: rusqlite::Error, flag: &StopFlag, context: &str) -> RunEnd {
    let stopped = matches!(
        &e,
        rusqlite::Error::SqliteFailure(f, _) if matches!(
            f.code,
            rusqlite::ErrorCode::OperationInterrupted | rusqlite::ErrorCode::DatabaseBusy
        )
    );
    match flag.get() {
        Some(reason) if stopped => RunEnd::Stopped(reason),
        _ => RunEnd::Failed(SofError::backend(format!("{context}: {e}"))),
    }
}

/// The row loop. Returns the number of rows read.
fn read(
    run: &SqliteRun,
    stop: &RunStop,
    out: &SharedSender,
    flag: &StopFlag,
    interrupter: &StatementInterrupter,
    handle: &tokio::runtime::Handle,
) -> Result<usize, RunEnd> {
    if let Some(reason) = stop.check() {
        return Err(RunEnd::Stopped(reason));
    }
    let conn = checkout(&run.pool, stop, flag)?;
    let deadline = stop.std_deadline();
    let flusher = BatchFlusher {
        out: out.clone(),
        flag: flag.clone(),
        deadline,
    };
    let _hooks = RunHooks::install(&conn, interrupter.clone(), flag, deadline, move || {
        flusher.check()
    })
    .map_err(|e| interpret(e, flag, "failed to prepare the connection"))?;
    // A stop that came before the interrupter was armed had nothing to
    // interrupt; its reason was recorded first, so it is seen here.
    if let Some(reason) = flag.get().or_else(|| stop.check()) {
        return Err(RunEnd::Stopped(reason));
    }
    let mut stmt = conn
        .prepare(&run.sql)
        .map_err(|e| interpret(e, flag, "failed to prepare SQL"))?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(run.params.iter()))
        .map_err(|e| interpret(e, flag, "query execution failed"))?;

    let mut count = 0usize;
    loop {
        if run.limit.is_some_and(|cap| count >= cap) {
            break;
        }
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(e) => return Err(interpret(e, flag, "row error")),
        };
        count += 1;
        let mut sender = out.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sender) = sender.as_mut() else {
            return Err(RunEnd::Stopped(StopReason::Dropped));
        };
        if sender.push_row(row, run)? {
            sender.blocking_flush(handle, stop).map_err(|reason| {
                flag.set(reason);
                RunEnd::Stopped(reason)
            })?;
        }
    }
    Ok(count)
}

/// One result row as the flat JSON object every runner emits: every
/// compiled column is present, a SQL NULL as JSON `null`. A row must not
/// drop its NULL columns — the formatters take the column list from the
/// first row, so a first row without `gender` would cut the header and
/// every later row down to its own non-null keys (#1569).
///
/// TEXT and BLOB values are decoded per column through [`decode_text`], so a
/// string column keeps `"44054006"`, `"true"` and `"null"` as strings (#1769).
/// Native INTEGER and REAL values pass through unchanged.
fn map_sqlite_row(
    row: &rusqlite::Row<'_>,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> rusqlite::Result<Map<String, Value>> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            // SQLite has no boolean type: `json_extract` yields INTEGER 1/0
            // for a JSON boolean, which a boolean column must report as one.
            ValueRef::Integer(n @ (0 | 1))
                if decodes.get(i).copied() == Some(ColumnDecode::Boolean) =>
            {
                Value::Bool(n == 1)
            }
            ValueRef::Integer(n) => Value::from(n),
            ValueRef::Real(f) => {
                Value::from(serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)))
            }
            ValueRef::Text(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                decode_text(decodes.get(i).copied().unwrap_or_default(), s)
            }
            ValueRef::Blob(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                decode_text(decodes.get(i).copied().unwrap_or_default(), s)
            }
        };
        map.insert(name.clone(), val);
    }
    Ok(map)
}

/// Appends one result row to the builder, cell by cell, like
/// [`map_sqlite_row`] but without the intermediate JSON object.
fn append_sqlite_row(
    row: &rusqlite::Row<'_>,
    decodes: &[ColumnDecode],
    builder: &mut ColumnarBatchBuilder,
) -> rusqlite::Result<()> {
    for i in 0..builder.column_count() {
        match row.get_ref(i)? {
            ValueRef::Null => builder.append_null(i),
            ValueRef::Integer(n) => builder.append_i64(i, n),
            ValueRef::Real(f) if f.is_finite() => builder.append_f64(i, f),
            // Matches the row path: NaN and +/-Inf are reported as 0.
            ValueRef::Real(_) => builder.append_i64(i, 0),
            ValueRef::Text(b) | ValueRef::Blob(b) => {
                let text = String::from_utf8_lossy(b);
                let decode = decodes.get(i).copied().unwrap_or_default();
                match decode {
                    ColumnDecode::Json | ColumnDecode::Auto => {
                        builder.append_deferred(i, decode_text(decode, text.into_owned()));
                    }
                    _ => builder.append_str(i, &text),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::compiler::compile_view_definition_dialect;
    use super::super::runtime::SqlRunPlan;
    use super::*;
    use crate::core::sof_runner::{RunKind, ViewFilters};
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let run = SqlRunPlan::compile(view, SqlDialect::Sqlite, FhirVersion::default_enabled())
            .expect("compile test view")
            .finish("tenant", filters)
            .expect("runtime sql");
        let sql = run.query.sql;
        let params: Vec<_> = run
            .params
            .into_iter()
            .skip(2)
            .map(SqliteParam::from_runtime)
            .collect();
        let bindings = params
            .iter()
            .map(|param| match param {
                SqliteParam::Text(v) => format!("text:{v}"),
                SqliteParam::Bool(v) => format!("bool:{v}"),
                SqliteParam::Int(v) => format!("int:{v}"),
                SqliteParam::Decimal(v) => format!("decimal:{v}"),
                SqliteParam::Null => "null".into(),
            })
            .collect();
        (sql, bindings)
    }

    fn flat_view() -> Value {
        json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]}]})
    }

    #[test]
    fn test_sqlite_runtime_sql_appends_final_output_limit() {
        let view = flat_view();
        let compiled = compile_view_definition_dialect(
            &view,
            SqlDialect::Sqlite,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
        assert_eq!(unlimited, compiled.sql);
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            let (sql, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(limit),
                    ..Default::default()
                },
            );
            assert_eq!(sql, format!("{unlimited}\nLIMIT {limit}"));
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    fn test_sqlite_limit_preserves_constant_and_runtime_bindings() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],
            "select":[{"column":[{"path":"id","name":"id"}]}]});
        let mut filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        };
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("?3"), "{unlimited}");
        assert!(unlimited.contains("r.last_updated >= ?4"), "{unlimited}");
        assert!(
            unlimited.contains("r.id IN (SELECT value FROM json_each(?5))"),
            "{unlimited}"
        );
        assert_eq!(bindings[0], "text:male");
        assert_eq!(bindings.last().unwrap(), r#"text:["p-eligible"]"#);
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    #[test]
    fn test_sqlite_limit_is_global_for_union_and_recursive_sql() {
        let views = [
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"unionAll":[
                {"column":[{"path":"id","name":"id"}]},
                {"column":[{"path":"gender","name":"id"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
        ];
        for view in views {
            let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
            let (limited, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(50),
                    ..Default::default()
                },
            );
            assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_sqlite_unrepresentable_limit_keeps_existing_sql() {
        let view = flat_view();
        let unlimited = runtime_sql(&view, &ViewFilters::default());
        for limit in [i64::MAX as usize + 1, usize::MAX] {
            assert_eq!(
                runtime_sql(
                    &view,
                    &ViewFilters {
                        limit: Some(limit),
                        ..Default::default()
                    }
                ),
                unlimited
            );
        }
    }

    #[test]
    fn test_sqlite_runtime_filters_reach_every_resources_scan() {
        let qr = |select: Value| {
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "status":"active", "select": select})
        };
        let patient = |select: Value| {
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
                "status":"active", "select": select})
        };
        let id_col = json!({"column":[{"path":"id","name":"value"}]});
        let gender_col = json!({"column":[{"path":"gender","name":"value"}]});
        let repeat_item = json!({"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]});
        let repeat_value = json!({"repeat":["item"], "column":[{"path":"linkId","name":"value"}]});
        let views = vec![
            (
                patient(json!([{"unionAll":[id_col.clone(), gender_col.clone()]}])),
                2,
            ),
            (
                patient(json!([{"unionAll":[id_col.clone(), gender_col.clone(), id_col.clone()]}])),
                3,
            ),
            (qr(json!([repeat_item.clone()])), 1),
            (
                qr(json!([{"column":[{"path":"id","name":"qr"}]}, repeat_item.clone()])),
                2,
            ),
            (
                qr(json!([{"repeat":["item","answer.item"],
                    "column":[{"path":"linkId","name":"link_id"}]}])),
                2,
            ),
            (qr(json!([{"unionAll":[repeat_value, id_col.clone()]}])), 2),
            (qr(json!([{"unionAll":[repeat_item.clone()]}])), 1),
        ];
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".to_string()],
            ..Default::default()
        };
        for (view, scans) in views {
            let (sql, _) = runtime_sql(&view, &filters);
            assert!(!sql.contains("FROM rec_0 AND"), "{sql}");
            assert_eq!(sql.matches("r.last_updated >= ?3").count(), scans, "{sql}");
            let membership = if view["resource"] == "Patient" {
                "r.id IN (SELECT value FROM json_each(?4))"
            } else {
                "si.value_reference IN (SELECT value FROM json_each("
            };
            assert_eq!(sql.matches(membership).count(), scans, "{sql}");
        }
    }

    #[test]
    fn test_sqlite_compartment_filter_binds_one_parameter_for_any_number_of_refs() {
        let version = FhirVersion::default_enabled();
        for resource in ["Patient", "Observation"] {
            let view = json!({"resourceType":"ViewDefinition", "resource":resource,
                "select":[{"column":[{"path":"id","name":"id"}]}]});
            let filters = ViewFilters {
                patient: (0..5_000).map(|i| format!("Patient/p{i}")).collect(),
                ..Default::default()
            };
            let run = SqlRunPlan::compile(&view, SqlDialect::Sqlite, version)
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            let sql = run.query.sql;
            let params: Vec<_> = run
                .params
                .into_iter()
                .skip(2)
                .map(SqliteParam::from_runtime)
                .collect();
            assert!(!sql.contains(" OR "), "{sql}");
            let (expected_params, first) = if resource == "Patient" {
                assert!(
                    sql.contains("r.id IN (SELECT value FROM json_each(?3))"),
                    "{sql}"
                );
                (1, "p0")
            } else {
                assert!(
                    sql.contains("si.value_reference IN (SELECT value FROM json_each(?"),
                    "{sql}"
                );
                (
                    helios_fhir::compartment_params(version, "Patient", resource).len() + 1,
                    "Patient/p0",
                )
            };
            assert_eq!(params.len(), expected_params, "{sql}");
            let Some(SqliteParam::Text(json)) = params.last() else {
                panic!("last param must be the JSON array: {params:?}");
            };
            let refs: Vec<String> = serde_json::from_str(json).unwrap();
            assert_eq!(refs.len(), 5_000);
            assert_eq!(refs[0], first);
        }
    }

    fn group_test_pool() -> Pool<SqliteConnectionManager> {
        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_secs(1))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TABLE resources (
                tenant_id TEXT NOT NULL, resource_type TEXT NOT NULL, id TEXT NOT NULL,
                data BLOB NOT NULL, last_updated TEXT NOT NULL, is_deleted INTEGER NOT NULL
            )",
            )
            .unwrap();
        pool
    }

    #[tokio::test(flavor = "current_thread")]
    async fn group_pool_acquisition_does_not_block_the_async_runtime() {
        use futures::StreamExt;

        let pool = group_test_pool();
        let connection = pool.get().unwrap();
        for resource in [
            json!({"resourceType":"Patient","id":"p1"}),
            json!({"resourceType":"Group","id":"g1","member":[{"entity":{"reference":"Patient/p1"}}]}),
        ] {
            connection
                .execute(
                    "INSERT INTO resources VALUES (?1,?2,?3,?4,?5,0)",
                    rusqlite::params![
                        "tenant",
                        resource["resourceType"].as_str().unwrap(),
                        resource["id"].as_str().unwrap(),
                        serde_json::to_vec(&resource).unwrap(),
                        "2024-01-01T00:00:00+00:00"
                    ],
                )
                .unwrap();
        }
        let runner = SqliteInDbRunner::new(pool);
        let task = tokio::spawn(async move {
            let tenant = TenantContext::new(
                crate::tenant::TenantId::new("tenant"),
                crate::tenant::TenantPermissions::full_access(),
            );
            runner
                .run_view(
                    &tenant,
                    RunRequest::new(
                        flat_view(),
                        ViewFilters {
                            group: vec!["Group/g1".into()],
                            ..Default::default()
                        },
                    ),
                    RunContext::detached(RunKind::Preview),
                )
                .await
                .map(|output| output.rows)
                .unwrap()
                .collect::<Vec<_>>()
                .await
        });
        // Let run_view reach group acquisition while this test owns the pool's
        // only connection. A synchronous pool.get would exhaust its timeout
        // before the current-thread runtime could execute this timer.
        tokio::task::yield_now().await;
        let started = std::time::Instant::now();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(
            !task.is_finished(),
            "group lookup is waiting for our connection"
        );
        drop(connection);
        let rows = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_ref().unwrap(), &json!({"id":"p1"}));
    }

    /// A run of raw SQL whose single column is `n`, on `pool`.
    fn raw_run(pool: Pool<SqliteConnectionManager>, sql: String) -> SqliteRun {
        SqliteRun {
            pool,
            sql,
            params: Vec::new(),
            columns: vec!["n".to_string()],
            decodes: vec![ColumnDecode::default()],
            limit: None,
        }
    }

    /// Counts to `n` with a recursive CTE and returns the counts `picks`
    /// selects: SQLite steps for a long time between two picked rows.
    fn counting_sql(n: u64, picks: &str) -> String {
        format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {n}) \
             SELECT x AS n FROM c WHERE {picks}"
        )
    }

    fn pool_of_one() -> Pool<SqliteConnectionManager> {
        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_secs(30))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        pool.get()
            .unwrap()
            .execute_batch("CREATE TABLE writes (n INTEGER NOT NULL)")
            .unwrap();
        pool
    }

    /// Waits (up to `max`) until the pool's only connection is idle again.
    async fn released_within(
        pool: &Pool<SqliteConnectionManager>,
        max: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + max;
        while std::time::Instant::now() < deadline {
            if pool.state().idle_connections == 1 {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        pool.state().idle_connections == 1
    }

    /// A10 (C15): a row that SQLite finds long before the next one reaches
    /// the consumer while the statement is still stepping, not when a batch
    /// fills or the statement ends: the progress handler sends a partial
    /// batch once it waited `BATCH_LINGER`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sqlite_sparse_rows_are_not_held_back_by_batching() {
        use futures::{FutureExt, StreamExt};

        let pool = pool_of_one();
        let run = raw_run(
            pool.clone(),
            counting_sql(2_000_000_000, "x = 1 OR x = 2000000000"),
        );
        let started = std::time::Instant::now();
        let mut rows = spawn_sqlite_run(run, &RunContext::detached(RunKind::Preview));
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), rows.next())
            .await
            .expect("the first row arrives")
            .expect("a row")
            .expect("a row");
        let waited = started.elapsed();
        assert_eq!(first, json!({"n": 1}));
        assert!(
            waited < std::time::Duration::from_millis(500),
            "the sparse first row waited {waited:?}"
        );
        assert!(
            rows.next().now_or_never().is_none(),
            "the statement is still stepping toward the next row"
        );
        drop(rows);
        assert!(released_within(&pool, std::time::Duration::from_secs(1)).await);
    }

    /// A10 (C15): dropping the stream interrupts a statement that has been
    /// stepping for a while without producing a row; the connection is back
    /// in the pool within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_stream_interrupts_a_long_sqlite_statement() {
        let pool = pool_of_one();
        let run = raw_run(pool.clone(), counting_sql(2_000_000_000, "x = 0"));
        let rows = spawn_sqlite_run(run, &RunContext::detached(RunKind::Preview));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            pool.state().idle_connections,
            0,
            "the statement is stepping"
        );
        drop(rows);
        let dropped = std::time::Instant::now();
        assert!(
            released_within(&pool, std::time::Duration::from_secs(1)).await,
            "the statement was still stepping {:?} after the drop",
            dropped.elapsed()
        );
    }

    /// A9 (C15): the r2d2 pool also serves ordinary writes, so an interrupt
    /// aimed at a stopped run must never reach the next statement on its
    /// connection. 200 runs stopped at different points (by drop and by
    /// cancel), each followed at once by an unrelated write on the pool of
    /// one; every write succeeds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sqlite_interrupt_never_reaches_the_next_statement() {
        use futures::StreamExt;

        let pool = pool_of_one();
        for i in 0..200u64 {
            let run = raw_run(pool.clone(), counting_sql(5_000_000, "x % 50 = 0"));
            let ctx = RunContext::detached(RunKind::Preview);
            let cancel = ctx.cancel.clone();
            let mut rows = spawn_sqlite_run(run, &ctx);
            for _ in 0..(i * 13) % 700 {
                if rows.next().await.is_none() {
                    break;
                }
            }
            if i % 2 == 0 {
                drop(rows);
            } else {
                cancel.cancel();
                // Keep the stream until the stop lands, then let it go.
                tokio::spawn(async move { while rows.next().await.is_some() {} });
            }
            let writer = pool.clone();
            tokio::task::spawn_blocking(move || {
                let conn = writer.get().expect("the pool's connection");
                conn.execute("INSERT INTO writes (n) VALUES (?1)", [i as i64])
                    .unwrap_or_else(|e| panic!("run {i}: the next statement failed: {e}"));
            })
            .await
            .expect("writer");
        }
        let count: i64 = pool
            .get()
            .unwrap()
            .query_row("SELECT count(*) FROM writes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 200);
    }

    #[tokio::test]
    async fn group_documents_are_tenant_scoped_and_deleted_groups_are_ignored() {
        let pool = group_test_pool();
        {
            let connection = pool.get().unwrap();
            for (tenant, id, deleted) in [
                ("tenant", "live", 0),
                ("other", "foreign", 0),
                ("tenant", "deleted", 1),
            ] {
                let group = json!({"resourceType":"Group","id":id,"member":[{"entity":{"reference":"Patient/p1"}}]});
                connection
                    .execute(
                        "INSERT INTO resources VALUES (?1,'Group',?2,?3,'2024-01-01',?4)",
                        rusqlite::params![tenant, id, serde_json::to_vec(&group).unwrap(), deleted],
                    )
                    .unwrap();
            }
        }
        let groups = load_group_documents(
            pool,
            "tenant".into(),
            vec![
                "live".into(),
                "Group/foreign".into(),
                "Group/deleted".into(),
                "Group/missing".into(),
            ],
            RunStop::new(&RunContext::detached(RunKind::Preview)),
        )
        .await
        .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["id"], "live");
    }

    /// #1864 N11 (L1-F5): a pool that stays exhausted for its whole
    /// connection timeout ends the run with the retryable
    /// `SofError::Unavailable` (503 with `Retry-After`), not a storage fault.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_exhausted_pool_ends_the_run_unavailable() {
        use futures::StreamExt;

        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_millis(200))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        let held = pool.get().unwrap();
        let rows = spawn_sqlite_run(
            raw_run(pool.clone(), "SELECT 1 AS n".to_string()),
            &RunContext::detached(RunKind::Preview),
        );
        let items =
            tokio::time::timeout(std::time::Duration::from_secs(5), rows.collect::<Vec<_>>())
                .await
                .expect("the run ends once the pool's timeout passed");
        drop(held);
        assert!(
            matches!(items.as_slice(), [Err(SofError::Unavailable { .. })]),
            "{items:?}"
        );
    }

    /// A run of `flat_view` filtered on `Group/g1`, on a pool whose only
    /// connection the test holds (the pool's own timeout is 30 s).
    fn group_run_on_held_pool() -> (
        SqliteInDbRunner,
        Pool<SqliteConnectionManager>,
        PooledConnection<SqliteConnectionManager>,
        RunRequest,
    ) {
        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_secs(30))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        let held = pool.get().unwrap();
        held.execute_batch(
            "CREATE TABLE resources (
                tenant_id TEXT NOT NULL, resource_type TEXT NOT NULL, id TEXT NOT NULL,
                data BLOB NOT NULL, last_updated TEXT NOT NULL, is_deleted INTEGER NOT NULL
            )",
        )
        .unwrap();
        let request = RunRequest::new(
            flat_view(),
            ViewFilters {
                group: vec!["Group/g1".into()],
                ..Default::default()
            },
        );
        (SqliteInDbRunner::new(pool.clone()), pool, held, request)
    }

    /// #1864 N11 (L1-F2, L2-F2): the Group load of a run's setup runs under
    /// the run's stop. With the pool's only connection held (its own timeout
    /// is 30 s), a cancel and a deadline each end `run_view` within the bound
    /// with their own kind, and the setup's worker never takes the connection
    /// afterwards.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_during_the_group_load_ends_the_run_within_the_bound() {
        let bound = std::time::Duration::from_secs(1);
        let after = std::time::Duration::from_millis(300);
        let tenant = TenantContext::new(
            crate::tenant::TenantId::new("tenant"),
            crate::tenant::TenantPermissions::full_access(),
        );
        for cell in ["cancel", "deadline"] {
            let (runner, pool, held, request) = group_run_on_held_pool();
            let mut ctx = RunContext::detached(RunKind::Preview);
            if cell == "deadline" {
                ctx.deadline = Some(tokio::time::Instant::now() + after);
            }
            let cancel = ctx.cancel.clone();
            let started = std::time::Instant::now();
            let run = {
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    runner
                        .run_view(&tenant, request, ctx)
                        .await
                        .map(|output| output.schema)
                })
            };
            tokio::time::sleep(after).await;
            if cell == "cancel" {
                cancel.cancel();
            }
            let result = tokio::time::timeout(bound, run)
                .await
                .unwrap_or_else(|_| panic!("{cell}: run_view still waits for the pool"))
                .unwrap();
            eprintln!(
                "SOF-CANCEL sqlite group load {cell}: ended {:?} after the start (stop at {after:?})",
                started.elapsed()
            );
            match (cell, result) {
                ("cancel", Err(SofError::Cancelled)) => {}
                ("deadline", Err(SofError::Timeout { .. })) => {}
                (cell, other) => panic!("{cell}: {other:?}"),
            }
            // The stopped load's worker gave up its wait: the connection,
            // released now, stays idle.
            drop(held);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert_eq!(pool.state().idle_connections, 1, "{cell}");
        }
    }

    /// The busy timeout every pooled connection of the lock-wait tests gets,
    /// as the backend's own default (`busy_timeout_ms`).
    const LOCKED_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    /// A file-backed database in rollback-journal mode (no WAL) with the
    /// `resources` table, a pool of one connection whose busy timeout is
    /// [`LOCKED_BUSY_TIMEOUT`], and a second, unpooled connection that holds
    /// an EXCLUSIVE transaction: every read on the pool waits for that lock.
    fn locked_database() -> (
        tempfile::TempDir,
        Pool<SqliteConnectionManager>,
        rusqlite::Connection,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.db");
        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_secs(30))
            .build(
                SqliteConnectionManager::file(&path)
                    .with_init(|conn| conn.busy_timeout(LOCKED_BUSY_TIMEOUT)),
            )
            .unwrap();
        pool.get()
            .unwrap()
            .execute_batch(
                "PRAGMA journal_mode = DELETE;
                 CREATE TABLE resources (
                    tenant_id TEXT NOT NULL, resource_type TEXT NOT NULL, id TEXT NOT NULL,
                    data BLOB NOT NULL, last_updated TEXT NOT NULL, is_deleted INTEGER NOT NULL
                 )",
            )
            .unwrap();
        let locker = rusqlite::Connection::open(&path).unwrap();
        locker.execute_batch("BEGIN EXCLUSIVE").unwrap();
        (dir, pool, locker)
    }

    /// The busy timeout of the pool's only (idle) connection.
    fn pooled_busy_timeout(pool: &Pool<SqliteConnectionManager>) -> std::time::Duration {
        let conn = pool.try_get().expect("the pool's only connection is idle");
        let ms: u64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        std::time::Duration::from_millis(ms)
    }

    /// #1864 N11 re-review (L1-F2, L2-F2): a Group lookup that waits for a
    /// database lock — not for a pooled connection — stops with the run.
    /// Another connection holds an EXCLUSIVE transaction and the pool's busy
    /// timeout is 30 s; a cancel and a deadline each end `run_view` within
    /// the bound with their own kind. `run_view` joins the lookup's worker
    /// before it returns, so the connection is back in the pool at once, with
    /// its own busy timeout restored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_ends_a_group_lookup_waiting_for_a_database_lock() {
        let bound = std::time::Duration::from_secs(1);
        let after = std::time::Duration::from_millis(300);
        let tenant = TenantContext::new(
            crate::tenant::TenantId::new("tenant"),
            crate::tenant::TenantPermissions::full_access(),
        );
        for cell in ["cancel", "deadline"] {
            let (_dir, pool, locker) = locked_database();
            let runner = SqliteInDbRunner::new(pool.clone());
            let request = RunRequest::new(
                flat_view(),
                ViewFilters {
                    group: vec!["Group/g1".into()],
                    ..Default::default()
                },
            );
            let mut ctx = RunContext::detached(RunKind::Preview);
            if cell == "deadline" {
                ctx.deadline = Some(tokio::time::Instant::now() + after);
            }
            let cancel = ctx.cancel.clone();
            let run = {
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    runner
                        .run_view(&tenant, request, ctx)
                        .await
                        .map(|output| output.schema)
                })
            };
            tokio::time::sleep(after - std::time::Duration::from_millis(50)).await;
            assert!(!run.is_finished(), "{cell}: the lookup waits for the lock");
            assert_eq!(pool.state().idle_connections, 0, "{cell}: lookup holds it");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if cell == "cancel" {
                cancel.cancel();
            }
            let stopped = std::time::Instant::now();
            let result = tokio::time::timeout(bound, run)
                .await
                .unwrap_or_else(|_| panic!("{cell}: run_view still waits for the lock"))
                .unwrap();
            eprintln!(
                "SOF-CANCEL sqlite group lock wait {cell}: ended {:?} after the stop",
                stopped.elapsed()
            );
            match (cell, result) {
                ("cancel", Err(SofError::Cancelled)) => {}
                ("deadline", Err(SofError::Timeout { .. })) => {}
                (cell, other) => panic!("{cell}: {other:?}"),
            }
            assert_eq!(
                pooled_busy_timeout(&pool),
                LOCKED_BUSY_TIMEOUT,
                "{cell}: the pooled connection keeps its own busy timeout"
            );
            drop(locker);
        }
    }

    /// #1864 N11 re-review: the run's own statement waits for a database lock
    /// under the same stop-aware busy handler as its Group lookup. A cancel
    /// ends the stream with `Cancelled` within the bound and the connection
    /// goes back to the pool with its own busy timeout. Without a stop, a
    /// lock released before the busy timeout lets the run read normally.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_ends_a_run_statement_waiting_for_a_database_lock() {
        use futures::StreamExt;

        let bound = std::time::Duration::from_secs(1);
        let (_dir, pool, locker) = locked_database();
        let count = "SELECT count(*) AS n FROM resources".to_string();
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let rows = spawn_sqlite_run(raw_run(pool.clone(), count.clone()), &ctx);
        let run = tokio::spawn(rows.collect::<Vec<_>>());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!run.is_finished(), "the statement waits for the lock");
        cancel.cancel();
        let items = tokio::time::timeout(bound, run)
            .await
            .expect("the stream still waits for the lock")
            .unwrap();
        assert!(
            matches!(items.as_slice(), [Err(SofError::Cancelled)]),
            "{items:?}"
        );
        assert!(released_within(&pool, bound).await);
        assert_eq!(pooled_busy_timeout(&pool), LOCKED_BUSY_TIMEOUT);

        // No stop: the wait lasts until the lock goes (well inside the busy
        // timeout), then the run reads normally.
        let rows = spawn_sqlite_run(
            raw_run(pool.clone(), count),
            &RunContext::detached(RunKind::Preview),
        );
        let run = tokio::spawn(rows.collect::<Vec<_>>());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!run.is_finished(), "the statement waits for the lock");
        locker.execute_batch("ROLLBACK").unwrap();
        let items = tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("the run reads once the lock is gone")
            .unwrap();
        assert!(
            matches!(items.as_slice(), [Ok(row)] if row == &json!({"n": 0})),
            "{items:?}"
        );
    }

    /// #1864 N11 (L2-F2): a run stopped before its setup never reports a
    /// clean empty result for a Group filter, even one that would select
    /// nothing.
    #[tokio::test]
    async fn a_stopped_run_with_a_group_filter_ends_with_its_stop() {
        let (runner, _pool, held, request) = group_run_on_held_pool();
        drop(held);
        let ctx = RunContext::detached(RunKind::Preview);
        ctx.cancel.cancel();
        let tenant = TenantContext::new(
            crate::tenant::TenantId::new("tenant"),
            crate::tenant::TenantPermissions::full_access(),
        );
        let result = runner.run_view(&tenant, request, ctx).await;
        assert!(
            matches!(result, Err(SofError::Cancelled)),
            "{:?}",
            result.map(|output| output.schema)
        );
    }
}
