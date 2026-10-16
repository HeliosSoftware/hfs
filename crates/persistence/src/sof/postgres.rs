//! PostgreSQL in-DB SQL-on-FHIR runner.
//!
//! [`PgInDbRunner`] compiles a ViewDefinition to a parameterised PostgreSQL
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are fetched lazily via `tokio_postgres::Client::query_raw` (or one
//! keyset page at a time for an export) and sent in batches through the
//! producer core's channel ([`row_batch_channel`]), so the HTTP layer can
//! begin flushing before the full result set has been transferred. The fetch
//! loop runs in a supervised `tokio::spawn` task that holds the pooled
//! connection until the run ends; a panic or cancellation of that task reaches
//! the consumer as an `Err` item, never as a silent end of stream.
//!
//! The columnar path ([`SofRunner::run_view_columnar`]) is the same run with
//! another sink: each row's cells go straight into Arrow builders and whole
//! record batches go through the channel ([`record_batch_channel`]), so
//! everything below (transactions, deadlines, stopping) applies to it too.
//!
//! ## Transactions and deadlines
//!
//! Every statement of a run executes inside `BEGIN READ ONLY` (one
//! transaction for an unpaged run, one per page for an export). When the run
//! has a deadline, the transaction starts with `SET LOCAL statement_timeout`
//! set to the time left, rounded up to whole milliseconds and never `0`
//! ([`statement_timeout_ms`]). The deadline is absolute: each page gets what
//! is left, not a fresh budget, and a page whose deadline already passed ends
//! the run as a timeout without running. Without a deadline no `SET LOCAL` is
//! sent and the pool session's own `statement_timeout` applies.
//!
//! ## Stopping
//!
//! A run stops when its stream is dropped, its cancel token fires or its
//! deadline passes ([`RunStop`]). Every step is raced against that signal:
//! the connection checkout, each statement, every row and, for an export, the
//! page bound, the page and the delivery of its rows. While a statement runs,
//! the deadline itself is enforced by the server (`SET LOCAL`); the client
//! only acts on it [`DEADLINE_GRACE`] later, as a backstop that cancels and
//! closes the connection.
//!
//! What happens to the statement and the connection on a stop:
//!
//! - **ended within received**: if the statement's end is already in the
//!   received buffer when the stop arrives, no cancel is sent (it could land
//!   on the next statement of the session); the runner rolls back and reuses
//!   the connection;
//! - otherwise the runner sends a cancel request while it still holds the
//!   connection ([`cancel_and_drain`]), keeps reading until the statement
//!   reports its end (PostgreSQL may not act on a cancel while it is blocked
//!   writing to a client that stopped reading), then rolls back. The
//!   connection is reused only when the statement ended with exactly the
//!   cancel request's own error ([`fate_after_cancel`]); otherwise it is
//!   closed, because a cancel might still be in flight;
//! - **the cleanup envelope**, measured from the stop: the cancel request
//!   (connect and send) within [`CANCEL_REQUEST_TIMEOUT`], the drain within
//!   [`CANCEL_DRAIN_TIMEOUT`], the `ROLLBACK` within [`ROLLBACK_TIMEOUT`],
//!   all within [`CLEANUP_ENVELOPE`]. A phase that stalls or fails ends the
//!   cleanup: the connection is taken out of the pool and dropped without
//!   awaiting anything else, so the producer always ends and the next
//!   borrower never sees that session. The stop bound is on server work and
//!   holds when the cancel request reaches the server; the drain timeout only
//!   decides between reuse and close. If the cancel request cannot reach the
//!   server, the statement is bounded by its `SET LOCAL statement_timeout`
//!   (the run's deadline) instead;
//! - a session is never returned to the pool inside an open transaction or
//!   with a request in flight ([`Session`]): unless it settled, it is closed.
//!
//! A stop's error kind comes from its [`StopReason`], never from the
//! `57014 query_canceled` the server reports for it. A `57014` without a
//! local stop is classified by the deadline ([`classify_failure`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use deadpool_postgres::Pool;
use futures::{FutureExt as _, StreamExt as _};
use helios_fhir::FhirVersion;
use helios_sof::columnar::{COLUMNAR_BATCH_ROWS, ColumnKind, ColumnarBatchBuilder};
use helios_sof::sqlquery::engine::declared_columns;
use serde_json::{Map, Value};
use tokio::time::Instant;
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;
use tracing::{debug, trace, warn};

use crate::backends::postgres::PostgresBackend;
use crate::core::sof_producer::{
    BatchSender, RecordBatchSender, RunStop, record_batch_channel, row_batch_channel,
    spawn_supervised, statement_timeout_ms,
};
use crate::core::sof_runner::{
    BatchStream, RowStream, RunCapabilities, RunContext, RunOutput, RunRequest, SofError,
    SofRunner, StopReason, ViewRow,
};
use crate::tenant::TenantContext;

use super::decode::{ColumnDecode, decode_text};

use super::compiler::SqlDialect;
use super::runtime::{RuntimeParam, SqlPrepared, prepare_sql};

/// The runner name in logs.
const RUNNER: &str = "postgres-indb";

/// Resources read per page by the export reader.
const EXPORT_PAGE_RESOURCES: usize = 10_000;

/// After a stop, the cancel request (connect and send) must complete within
/// this long, or the connection is closed.
pub(crate) const CANCEL_REQUEST_TIMEOUT: Duration = Duration::from_millis(300);

/// After the cancel request, the statement must report its end within this
/// long, or the connection is closed. Decides reuse versus close only.
pub(crate) const CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// The `ROLLBACK` that settles a stopped or failed run's transaction must
/// complete within this long, or the connection is closed.
pub(crate) const ROLLBACK_TIMEOUT: Duration = Duration::from_millis(200);

/// The whole cleanup after a stop, measured from the stop.
pub(crate) const CLEANUP_ENVELOPE: Duration = Duration::from_secs(1);

/// While a statement runs under `SET LOCAL statement_timeout`, the client
/// acts on the deadline only this long after it, as a backstop (cancel and
/// close) for a server that did not enforce it.
pub(crate) const DEADLINE_GRACE: Duration = Duration::from_secs(1);

/// A server-side `57014` that arrives this close to the deadline counts as
/// the run's own `SET LOCAL statement_timeout`.
const DEADLINE_TOLERANCE: Duration = Duration::from_millis(50);

/// The text of the error a statement ends with when a cancel request stopped
/// it, as the server writes it with `lc_messages` in English. A localized
/// server writes another text, and the connection is then closed rather than
/// reused ([`fate_after_cancel`]).
pub(crate) const USER_CANCEL_MESSAGE: &str = "canceling statement due to user request";

/// SQL-on-FHIR runner that compiles ViewDefinitions to PostgreSQL SQL.
pub struct PgInDbRunner {
    pool: Pool,
    fhir_version: FhirVersion,
    /// `Some(n)`: read the view in keyset pages of `n` resources over
    /// `id COLLATE "C"` (export mode). `None`: one statement (preview, run).
    export_page_size: Option<usize>,
}

impl PgInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            fhir_version: FhirVersion::default_enabled(),
            export_page_size: None,
        }
    }

    /// Returns a runner that consults the given FHIR version's field-type
    /// table when validating `collection: false` columns.
    pub fn with_fhir_version(mut self, version: FhirVersion) -> Self {
        self.fhir_version = version;
        self
    }

    /// Returns a runner that reads each view in keyset pages of `resources`
    /// resources (at least 1) instead of one statement. Rows come out in
    /// bytewise resource-id order; see [`SofRunner::for_export_job`].
    pub fn with_export_page_size(mut self, resources: usize) -> Self {
        self.export_page_size = Some(resources.max(1));
        self
    }
}

#[async_trait::async_trait]
impl SofRunner for PgInDbRunner {
    fn runner_name(&self) -> &'static str {
        RUNNER
    }

    /// An export reads whole types, so its views are read in id-keyset pages:
    /// every statement is short and runs in its own transaction under what is
    /// left of the run's deadline, where one `ORDER BY last_updated, id` over
    /// millions of rows would sort the whole type before its first row.
    fn for_export_job(&self) -> Option<Arc<dyn SofRunner>> {
        Some(Arc::new(PgInDbRunner {
            pool: self.pool.clone(),
            fhir_version: self.fhir_version,
            export_page_size: Some(self.export_page_size.unwrap_or(EXPORT_PAGE_RESOURCES)),
        }))
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
        let group_stop = RunStop::new(&ctx);
        let load_groups = move |refs: Vec<String>| async move {
            load_group_documents(&group_pool, &group_tenant, &refs, &group_stop).await
        };
        let prepare = prepare_sql(
            &view_definition,
            SqlDialect::Postgres,
            self.fhir_version,
            &tenant_id,
            filters,
            self.export_page_size.is_some(),
            load_groups,
        );
        let prepared = match race_setup(&ctx, prepare).await? {
            SqlPrepared::Run(prepared) => prepared,
            SqlPrepared::Empty {
                schema,
                total_order,
            } => {
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
        let run = PgRun {
            pool: self.pool.clone(),
            sql: compiled.sql,
            params: prepared
                .params
                .into_iter()
                .map(PgParam::from_runtime)
                .collect(),
            columns: compiled.columns,
            decodes: compiled.column_decodes,
            limit: prepared.client_limit,
            page_size: self.export_page_size,
            bound_sql: export_bound_sql().to_string(),
            #[cfg(test)]
            recorder: None,
        };
        Ok(RunOutput::new(
            schema,
            capabilities(total_order),
            permutation.normalize(spawn_pg_run(run, &ctx)),
        ))
    }

    /// Columnar sibling of [`run_view`](Self::run_view): every column is text
    /// on the wire, so each cell is read as `&str` and appended straight to
    /// its Arrow builder. Columns whose type is only known per value are
    /// decoded to JSON as on the row path.
    ///
    /// The run is the row path's run in every other respect (#1864): the same
    /// setup race, transactions, `SET LOCAL statement_timeout`, keyset pages
    /// for an export, stop and cleanup ([`produce`]), and typed errors. Each
    /// batch's columns are moved into schema order.
    ///
    /// Returns `Ok(None)` when two output columns share a name: the row path
    /// folds those into one JSON key, positional builders would keep both.
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
        let group_stop = RunStop::new(&ctx);
        let load_groups = move |refs: Vec<String>| async move {
            load_group_documents(&group_pool, &group_tenant, &refs, &group_stop).await
        };
        let prepare = prepare_sql(
            &view_definition,
            SqlDialect::Postgres,
            self.fhir_version,
            &tenant_id,
            filters,
            self.export_page_size.is_some(),
            load_groups,
        );
        let prepared = match race_setup(&ctx, prepare).await? {
            SqlPrepared::Run(prepared) => prepared,
            SqlPrepared::Empty { .. } => return Ok(Some(Box::pin(futures::stream::empty()))),
        };
        let compiled = prepared.query;
        let mut seen = std::collections::HashSet::new();
        if !compiled.columns.iter().all(|c| seen.insert(c.as_str())) {
            return Ok(None);
        }
        // Once per run: the compiled projection's order into document order.
        let permutation = prepared.schema.permutation_from(&compiled.columns)?;
        let kinds = (0..compiled.columns.len())
            .map(|i| {
                compiled
                    .column_decodes
                    .get(i)
                    .copied()
                    .unwrap_or_default()
                    .column_kind()
            })
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
        let run = PgRun {
            pool: self.pool.clone(),
            sql: compiled.sql,
            params: prepared
                .params
                .into_iter()
                .map(PgParam::from_runtime)
                .collect(),
            columns: compiled.columns,
            decodes: compiled.column_decodes,
            limit: prepared.client_limit,
            page_size: self.export_page_size,
            bound_sql: export_bound_sql().to_string(),
            #[cfg(test)]
            recorder: None,
        };
        Ok(Some(permutation.normalize_batches(spawn_pg_columnar_run(
            run, builder, &ctx,
        ))))
    }
}

/// What a PostgreSQL run declares: its schema always, a total order for flat
/// and `forEach` views (by `(last_updated, id)`, or by id for export pages),
/// and a server-side stop (cancel request, `SET LOCAL statement_timeout`).
fn capabilities(total_order: bool) -> RunCapabilities {
    RunCapabilities {
        total_order,
        declared_schema: true,
        cancels_server_side: true,
    }
}

/// Runs the setup of a run (compilation and the Group load) unless the run is
/// cancelled or its deadline passes first. A run whose deadline has already
/// passed ends here, before any SQL.
async fn race_setup<T>(
    ctx: &RunContext,
    setup: impl Future<Output = Result<T, SofError>>,
) -> Result<T, SofError> {
    let stop = RunStop::new(ctx);
    tokio::select! {
        biased;
        reason = stop.stopped() => Err(stop.error_for(reason).unwrap_or(SofError::Cancelled)),
        prepared = setup => prepared,
    }
}

/// Backend-only tenant-scoped Group document loading. Shared runtime
/// preparation interprets and merges the membership of these documents.
///
/// The load runs like one of the run's statements: inside `BEGIN READ ONLY`
/// with `SET LOCAL statement_timeout` from what is left of the run's deadline
/// (a lookup blocked on a lock ends at the deadline on the server too), and
/// under the [`Session`] guard, so a connection whose load did not settle is
/// closed, never re-pooled busy. [`race_setup`] ends the setup as soon as the
/// run is stopped; if that abandons a request still in flight, a cancel
/// request for it is sent before the connection closes (closing the socket
/// alone does not stop a statement that is waiting or executing), within
/// [`CANCEL_REQUEST_TIMEOUT`]. If the cancel request cannot reach the server,
/// the lookup is bounded by its `SET LOCAL` (the run's deadline), as for the
/// run's own statements.
async fn load_group_documents(
    pool: &Pool,
    tenant_id: &str,
    group_refs: &[String],
    stop: &RunStop,
) -> Result<Vec<Value>, SofError> {
    if group_refs.is_empty() {
        return Ok(Vec::new());
    }
    let session = Session::new(
        PostgresBackend::checkout(pool)
            .await
            .map_err(SofError::from_storage)?,
    );
    let timeout_ms = timeout_for_next_transaction(stop)
        .map_err(|reason| stop.error_for(reason).unwrap_or(SofError::Cancelled))?;
    let cancel_on_abandon = CancelOnAbandon(Some(session.client().cancel_token()));
    let loaded = read_group_documents(&session, tenant_id, group_refs, timeout_ms).await;
    cancel_on_abandon.disarm();
    loaded
}

/// The requests of [`load_group_documents`] on its session; the session
/// settles only after the `COMMIT`.
async fn read_group_documents(
    session: &Session,
    tenant_id: &str,
    group_refs: &[String],
    timeout_ms: Option<i32>,
) -> Result<Vec<Value>, SofError> {
    let client = session.client();
    session.unsettle();
    client
        .batch_execute(&begin_sql(timeout_ms))
        .await
        .map_err(|e| pg_sof_error("failed to start the Group lookup's transaction", &e))?;
    let stmt = client
        .prepare(
            "SELECT data FROM resources \
             WHERE tenant_id = $1 \
               AND resource_type = 'Group' \
               AND id = $2 \
               AND is_deleted = false",
        )
        .await
        .map_err(|e| pg_sof_error("prepare failed", &e))?;

    let mut groups = Vec::with_capacity(group_refs.len());
    for r in group_refs {
        let id = r.strip_prefix("Group/").unwrap_or(r);
        match client.query_opt(&stmt, &[&tenant_id, &id]).await {
            Ok(Some(row)) => {
                let data: Value = row.get(0);
                groups.push(data);
            }
            Ok(None) => continue,
            Err(e) => {
                return Err(pg_sof_error(&format!("group lookup failed for {r}"), &e));
            }
        }
    }
    client
        .batch_execute("COMMIT")
        .await
        .map_err(|e| pg_sof_error("failed to end the Group lookup's transaction", &e))?;
    session.settle();
    Ok(groups)
}

/// Sends a cancel request for the session's request in flight when dropped
/// while armed: the future that awaited the request was abandoned (the run
/// stopped during its setup), and the connection is about to close without
/// the request having ended. Disarmed once the requests returned.
struct CancelOnAbandon(Option<tokio_postgres::CancelToken>);

impl CancelOnAbandon {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for CancelOnAbandon {
    fn drop(&mut self) {
        let Some(token) = self.0.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            match tokio::time::timeout(CANCEL_REQUEST_TIMEOUT, token.cancel_query(NoTls)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    debug!(runner = RUNNER, error = %e, "cancel request for an abandoned Group lookup failed");
                }
                Err(_) => {
                    debug!(
                        runner = RUNNER,
                        "cancel request for an abandoned Group lookup stalled"
                    );
                }
            }
        });
    }
}

/// `t` rounded up to the next whole microsecond (unchanged when it already is
/// one), the precision of a PostgreSQL `timestamptz`. Saturates at `t` itself
/// on overflow, which no stored instant reaches.
fn ceil_to_micros(t: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    let sub_micro = t.timestamp_subsec_nanos() % 1_000;
    if sub_micro == 0 {
        return t;
    }
    t.checked_add_signed(chrono::Duration::nanoseconds(i64::from(1_000 - sub_micro)))
        .unwrap_or(t)
}

// ============================================================================
// Typed parameter enum — avoids the self-referential borrow issues with
// `Vec<Box<dyn ToSql>>` + `Vec<&dyn ToSql>` that arise in async tasks.
// ============================================================================

#[derive(Clone)]
enum PgParam {
    Text(String),
    /// One whole reference list, bound as `text[]`.
    TextArray(Vec<String>),
    Bool(bool),
    Int(i64),
    Decimal(String),
    Null,
    Timestamp(chrono::DateTime<chrono::Utc>),
}

impl PgParam {
    /// The bound value of one runtime parameter.
    ///
    /// A [`RuntimeParam::Timestamp`] (the `_since` cutoff, compared as
    /// `r.last_updated >= $n`) is rounded **up** to the microsecond here
    /// (#1864): `timestamptz` stores microseconds, and the driver truncates a
    /// finer instant when it binds it, so a cutoff a fraction of a microsecond
    /// after a resource's stored instant would keep that resource although it
    /// was stored before the cutoff. Rounding up keeps `_since` inclusive at
    /// the stored precision: a cutoff equal to the stored instant keeps it, any
    /// later one drops it.
    ///
    /// The rounding is applied only to the bound value. `prepared.params` (and
    /// so the SQL goldens, which serialize it) keep the caller's instant, e.g.
    /// `…56.123456789Z` in `since.postgres.sql`.
    fn from_runtime(value: RuntimeParam) -> Self {
        match value {
            RuntimeParam::Text(value) => Self::Text(value),
            RuntimeParam::Literal(value) => Self::from_lit(&value),
            RuntimeParam::TextList(value) => Self::TextArray(value),
            RuntimeParam::Timestamp(value) => Self::Timestamp(ceil_to_micros(value)),
        }
    }

    /// Lifts a [`super::ir::LitValue`] (used by `ViewDefinition.constant[]`)
    /// into the runtime parameter representation. Decimals bind as text and
    /// rely on PG's implicit cast to `numeric` at the call site.
    fn from_lit(v: &super::ir::LitValue) -> Self {
        match v {
            super::ir::LitValue::Null => PgParam::Null,
            super::ir::LitValue::Bool(b) => PgParam::Bool(*b),
            super::ir::LitValue::Int(n) => PgParam::Int(*n),
            super::ir::LitValue::Decimal(s) => PgParam::Decimal(s.clone()),
            super::ir::LitValue::Str(s) => PgParam::Text(s.clone()),
        }
    }
}

// ============================================================================
// Producer: session, steps, stop and cleanup
// ============================================================================

/// One run's statement and how to read it.
pub(crate) struct PgRun {
    pub(crate) pool: Pool,
    pub(crate) sql: String,
    params: Vec<PgParam>,
    pub(crate) columns: Vec<String>,
    pub(crate) decodes: Vec<ColumnDecode>,
    /// Rows the client stops at (the SQL has no `LIMIT` for them).
    pub(crate) limit: Option<usize>,
    /// `Some(n)`: keyset pages of `n` resources (export).
    pub(crate) page_size: Option<usize>,
    /// The statement that finds a page's last id ([`export_bound_sql`]).
    pub(crate) bound_sql: String,
    /// What the run sent, in order (tests only).
    #[cfg(test)]
    pub(crate) recorder: Option<Recorder>,
}

impl PgRun {
    #[cfg(test)]
    fn record(&self, event: Recorded) {
        if let Some(recorder) = &self.recorder {
            recorder.0.lock().unwrap().push(event);
        }
    }

    #[cfg(not(test))]
    fn record(&self, _event: Recorded) {}
}

/// What a run sent to the server, in order, with when (tests only; the
/// release build records nothing).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Recorded {
    /// `BEGIN READ ONLY`, with the `SET LOCAL statement_timeout` value sent
    /// with it, if any.
    Begin {
        at: Instant,
        timeout_ms: Option<i32>,
    },
    /// A statement of the run (`prepare`, `bound`, `page`, `query`).
    Statement(&'static str),
    Commit,
    Rollback,
    CancelRequest,
    /// How the statement behind a cancel request ended.
    Drained(DrainOutcome),
    /// The connection's fate when the run ended.
    Fate(ConnectionFate),
}

/// The test recorder of [`Recorded`] events.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct Recorder(pub(crate) Arc<std::sync::Mutex<Vec<Recorded>>>);

#[cfg(test)]
impl Recorder {
    pub(crate) fn events(&self) -> Vec<Recorded> {
        self.0.lock().unwrap().clone()
    }
}

/// What happens to a run's connection when the run ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionFate {
    /// The session settled (no transaction open, nothing in flight): it goes
    /// back to the pool.
    Reuse,
    /// The session might still be busy or receive a cancel: it is taken out
    /// of the pool and dropped, which closes it.
    Close,
}

/// A checked-out connection that goes back to the pool only once settled.
///
/// The transaction guard: before a request is sent the session is marked
/// unsettled, and only a completed `COMMIT` or `ROLLBACK` settles it. A
/// session dropped while unsettled (a stop, an error, a panic, an aborted
/// task) is taken out of the pool and closed, so a session inside an open
/// `READ ONLY` transaction, or with a statement still running, is never
/// handed to the next borrower.
pub(crate) struct Session {
    client: Option<deadpool_postgres::Client>,
    settled: AtomicBool,
}

impl Session {
    pub(crate) fn new(client: deadpool_postgres::Client) -> Self {
        Self {
            client: Some(client),
            settled: AtomicBool::new(true),
        }
    }

    pub(crate) fn client(&self) -> &deadpool_postgres::Client {
        self.client
            .as_ref()
            .expect("a session holds its client until dropped")
    }

    /// A request is about to be sent.
    pub(crate) fn unsettle(&self) {
        self.settled.store(false, Ordering::SeqCst);
    }

    /// The session's transaction ended and nothing is in flight.
    pub(crate) fn settle(&self) {
        self.settled.store(true, Ordering::SeqCst);
    }

    /// Ends the session: [`ConnectionFate::Close`] closes it even if it
    /// settled.
    pub(crate) fn finish(self, fate: ConnectionFate) {
        if fate == ConnectionFate::Close {
            self.settled.store(false, Ordering::SeqCst);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::SeqCst) {
            if let Some(client) = self.client.take() {
                // Out of the pool, then dropped: the connection closes.
                drop(deadpool_postgres::Object::take(client));
            }
        }
    }
}

/// How a run ended before all its rows were sent.
enum RunEnd {
    Stopped(StopReason),
    Failed(SofError),
}

/// Starts a run's producer and returns its row stream.
pub(crate) fn spawn_pg_run(run: PgRun, ctx: &RunContext) -> RowStream {
    let (out, stop, rows) = row_batch_channel(RUNNER, ctx);
    let reporter = out.reporter();
    spawn_supervised(RUNNER, reporter, produce(run, stop, PgSink::Rows(out)));
    rows
}

/// Starts a columnar run's producer and returns its record-batch stream. The
/// producer is the row path's ([`produce`]); only where the rows go differs.
pub(crate) fn spawn_pg_columnar_run(
    run: PgRun,
    builder: ColumnarBatchBuilder,
    ctx: &RunContext,
) -> BatchStream {
    let (out, stop, batches) = record_batch_channel(RUNNER, ctx);
    let reporter = out.reporter();
    let sink = PgSink::Columnar {
        out,
        builder: Box::new(builder),
    };
    spawn_supervised(RUNNER, reporter, produce(run, stop, sink));
    batches
}

/// Where a run's rows go: JSON rows through the batch channel
/// ([`run_view`](SofRunner::run_view)), or cells appended to Arrow builders
/// and sent as record batches
/// ([`run_view_columnar`](SofRunner::run_view_columnar)).
enum PgSink {
    Rows(BatchSender),
    Columnar {
        out: RecordBatchSender,
        builder: Box<ColumnarBatchBuilder>,
    },
}

impl PgSink {
    /// Adds one server row. `Ok(true)` when a batch is ready to be flushed.
    fn push_row(&mut self, pg_row: &tokio_postgres::Row, run: &PgRun) -> Result<bool, SofError> {
        match self {
            Self::Rows(out) => {
                let row = row_to_json(pg_row, &run.columns, &run.decodes)?;
                Ok(out.push(Ok(row)))
            }
            Self::Columnar { out, builder } => {
                append_pg_row(builder, pg_row, &run.columns, &run.decodes)?;
                if !builder.end_row() {
                    return Ok(false);
                }
                match builder.flush() {
                    Ok(Some(batch)) => Ok(out.push(Ok(batch))),
                    Ok(None) => Ok(false),
                    Err(e) => Err(SofError::backend(format!("columnar flush: {e}"))),
                }
            }
        }
    }

    /// Sends what is ready, raced against the stop ([`ItemSender::flush`]).
    /// A columnar run's partial batch stays in its builder until the end.
    ///
    /// [`ItemSender::flush`]: crate::core::sof_producer::ItemSender::flush
    async fn flush(&mut self, stop: &RunStop) -> Result<(), StopReason> {
        match self {
            Self::Rows(out) => out.flush(stop).await,
            Self::Columnar { out, .. } => out.flush(stop).await,
        }
    }

    /// Ends the stream normally. A columnar run sends its last, partial
    /// batch first.
    async fn finish(self, stop: &RunStop) {
        match self {
            Self::Rows(out) => out.finish(stop).await,
            Self::Columnar {
                mut out,
                mut builder,
            } => match builder.flush() {
                Ok(Some(batch)) => {
                    out.push(Ok(batch));
                    out.finish(stop).await;
                }
                Ok(None) => out.finish(stop).await,
                Err(e) => {
                    out.fail(stop, SofError::backend(format!("columnar flush: {e}")))
                        .await
                }
            },
        }
    }

    /// Ends the stream with `error`. A columnar run's partial batch is
    /// dropped, never sent as if complete.
    async fn fail(self, stop: &RunStop, error: SofError) {
        match self {
            Self::Rows(out) => out.fail(stop, error).await,
            Self::Columnar { out, .. } => out.fail(stop, error).await,
        }
    }

    /// Ends the stream after a stop ([`ItemSender::end_after_stop`]).
    ///
    /// [`ItemSender::end_after_stop`]: crate::core::sof_producer::ItemSender::end_after_stop
    async fn end_after_stop(self, stop: &RunStop, reason: StopReason) {
        match self {
            Self::Rows(out) => out.end_after_stop(stop, reason).await,
            Self::Columnar { out, .. } => out.end_after_stop(stop, reason).await,
        }
    }
}

async fn produce(run: PgRun, stop: RunStop, mut out: PgSink) {
    if let Some(reason) = stop.check() {
        return out.end_after_stop(&stop, reason).await;
    }
    let session = tokio::select! {
        biased;
        reason = stop.stopped() => return out.end_after_stop(&stop, reason).await,
        client = PostgresBackend::checkout(&run.pool) => match client {
            Ok(client) => Session::new(client),
            Err(e) => return end_run(out, &stop, RunEnd::Failed(SofError::from_storage(e))).await,
        },
    };
    let (result, fate) = match run.page_size {
        None => read_unpaged(&run, &session, &stop, &mut out).await,
        Some(page_size) => read_pages(&run, page_size, &session, &stop, &mut out).await,
    };
    run.record(Recorded::Fate(fate));
    // The connection goes back (or is closed) before the stream ends, so a
    // consumer that sees the end can rely on the pool.
    session.finish(fate);
    match result {
        Ok((rows, pages)) => {
            debug!(runner = RUNNER, rows, pages, "in-DB view run complete");
            out.finish(&stop).await;
        }
        Err(end) => end_run(out, &stop, end).await,
    }
}

async fn end_run(out: PgSink, stop: &RunStop, end: RunEnd) {
    match end {
        RunEnd::Stopped(reason) => {
            debug!(runner = RUNNER, ?reason, "in-DB view run stopped");
            out.end_after_stop(stop, reason).await;
        }
        RunEnd::Failed(error) => {
            log_run_failure(&error);
            out.fail(stop, error).await;
        }
    }
}

/// Logs a failed run; a server fault keeps its SQLSTATE in the log.
fn log_run_failure(error: &SofError) {
    match error {
        SofError::Backend { sqlstate, message } => warn!(
            runner = RUNNER,
            sqlstate = sqlstate.as_deref().unwrap_or(""),
            error = %message,
            "SoF PostgreSQL run failed"
        ),
        other => debug!(runner = RUNNER, error = %other, "SoF PostgreSQL run ended with an error"),
    }
}

/// The result of one request raced against the stop.
enum Step<T> {
    Done(Result<T, tokio_postgres::Error>),
    /// The run stopped while the request was in flight; the session was
    /// cleaned up and has this fate.
    Stopped(StopReason, ConnectionFate),
}

/// Sends one request of the session (`request` is its future) and awaits it,
/// raced against the stop. The server bounds the request by the deadline, so
/// the client acts on the deadline only after [`DEADLINE_GRACE`]; that
/// backstop always closes the connection.
async fn step<T>(
    session: &Session,
    run: &PgRun,
    stop: &RunStop,
    request: impl Future<Output = Result<T, tokio_postgres::Error>>,
) -> Step<T> {
    session.unsettle();
    tokio::pin!(request);
    let reason = tokio::select! {
        biased;
        done = &mut request => return Step::Done(done),
        reason = stop.stopped_with_grace(DEADLINE_GRACE) => reason,
    };
    let stopped_at = Instant::now();
    let fate = match request.as_mut().now_or_never() {
        // Its end was already received: nothing runs that a cancel could
        // stop, but a cancel could reach the next statement.
        Some(_) => rollback(session, run, stopped_at + CLEANUP_ENVELOPE).await,
        None => cancel_and_drain(session, run, stopped_at, request.map(|r| r.map(|_| ()))).await,
    };
    let fate = backstop_fate(reason, fate);
    Step::Stopped(reason, fate)
}

/// The deadline reaches a step the server bounds only as the backstop, which
/// means the server did not enforce it: close the connection.
fn backstop_fate(reason: StopReason, fate: ConnectionFate) -> ConnectionFate {
    if reason == StopReason::DeadlineExceeded {
        ConnectionFate::Close
    } else {
        fate
    }
}

/// How the statement behind a cancel request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DrainOutcome {
    /// It ended normally: the cancel request may still be on its way.
    Completed,
    /// It failed with this SQLSTATE and server message.
    Failed {
        sqlstate: Option<String>,
        message: Option<String>,
    },
    /// It did not report its end within the drain timeout.
    TimedOut,
}

impl DrainOutcome {
    fn from_error(e: &tokio_postgres::Error) -> Self {
        Self::Failed {
            sqlstate: e.code().map(|c| c.code().to_string()),
            message: e.as_db_error().map(|db| db.message().to_string()),
        }
    }
}

/// Whether a connection whose statement was cancelled may be reused: only
/// when the statement ended with exactly the cancel request's error
/// (`57014`, [`USER_CANCEL_MESSAGE`]), which proves the request was consumed
/// by that statement. A statement that completed, timed out, failed
/// otherwise, or reported the cancel in another language (a localized
/// server) leaves a cancel that might still land on the next statement, so
/// the connection is closed.
pub(crate) fn fate_after_cancel(outcome: &DrainOutcome) -> ConnectionFate {
    match outcome {
        DrainOutcome::Failed {
            sqlstate: Some(code),
            message: Some(message),
        } if code == SqlState::QUERY_CANCELED.code() && message == USER_CANCEL_MESSAGE => {
            ConnectionFate::Reuse
        }
        _ => ConnectionFate::Close,
    }
}

/// Cancels the session's running statement while still holding the
/// connection, drains the statement (`drain` awaits its end), then rolls
/// back, all inside the cleanup envelope that started at `stopped_at`.
/// Returns whether the connection can be reused.
pub(crate) async fn cancel_and_drain(
    session: &Session,
    run: &PgRun,
    stopped_at: Instant,
    drain: impl Future<Output = Result<(), tokio_postgres::Error>>,
) -> ConnectionFate {
    let envelope_end = stopped_at + CLEANUP_ENVELOPE;
    run.record(Recorded::CancelRequest);
    let token = session.client().cancel_token();
    match tokio::time::timeout(CANCEL_REQUEST_TIMEOUT, token.cancel_query(NoTls)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            debug!(runner = RUNNER, error = %e, "cancel request failed; closing the connection");
            return ConnectionFate::Close;
        }
        Err(_) => {
            debug!(
                runner = RUNNER,
                "cancel request stalled; closing the connection"
            );
            return ConnectionFate::Close;
        }
    }
    let budget = CANCEL_DRAIN_TIMEOUT.min(envelope_end.saturating_duration_since(Instant::now()));
    let outcome = match tokio::time::timeout(budget, drain).await {
        Ok(Ok(())) => DrainOutcome::Completed,
        Ok(Err(e)) => DrainOutcome::from_error(&e),
        Err(_) => DrainOutcome::TimedOut,
    };
    run.record(Recorded::Drained(outcome.clone()));
    match fate_after_cancel(&outcome) {
        ConnectionFate::Reuse => rollback(session, run, envelope_end).await,
        ConnectionFate::Close => {
            debug!(
                runner = RUNNER,
                ?outcome,
                "cancelled statement did not confirm the cancel; closing the connection"
            );
            ConnectionFate::Close
        }
    }
}

/// Rolls the session's transaction back within [`ROLLBACK_TIMEOUT`] (and
/// before `envelope_end`). Settles the session on success.
async fn rollback(session: &Session, run: &PgRun, envelope_end: Instant) -> ConnectionFate {
    run.record(Recorded::Rollback);
    let budget = ROLLBACK_TIMEOUT.min(envelope_end.saturating_duration_since(Instant::now()));
    match tokio::time::timeout(budget, session.client().batch_execute("ROLLBACK")).await {
        Ok(Ok(())) => {
            session.settle();
            ConnectionFate::Reuse
        }
        _ => {
            debug!(
                runner = RUNNER,
                "ROLLBACK failed or stalled; closing the connection"
            );
            ConnectionFate::Close
        }
    }
}

/// Which `statement_timeout` bounded a statement that the server stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementLimit {
    /// No `SET LOCAL` was sent (the run has no deadline): the pool session's
    /// own `statement_timeout` applies.
    Session,
    /// The run's `SET LOCAL statement_timeout`: `passed` says whether the
    /// deadline had passed when the error arrived.
    RunDeadline {
        passed: bool,
        budget: Option<Duration>,
    },
}

impl StatementLimit {
    fn for_run(stop: &RunStop, set_local: bool) -> Self {
        if !set_local {
            return Self::Session;
        }
        let passed = stop
            .deadline()
            .is_some_and(|d| Instant::now() + DEADLINE_TOLERANCE >= d);
        Self::RunDeadline {
            passed,
            budget: stop.budget(),
        }
    }
}

/// Classifies a statement error that no local stop explains (council C6),
/// and says what to do with the connection.
///
/// - `57014 query_canceled` under the pool session's timeout (no `SET
///   LOCAL`), or under the run's `SET LOCAL` once the deadline passed:
///   [`SofError::Timeout`], and the connection is reused after `ROLLBACK`
///   (no cancel request is in flight);
/// - any other `57014` (another session cancelled it, an operator):
///   [`SofError::Backend`] with its SQLSTATE, and the connection is closed;
/// - a closed connection: closed;
/// - anything else: [`sof_error_for_sqlstate`], reused after `ROLLBACK`.
pub(crate) fn classify_failure(
    code: Option<&SqlState>,
    message: String,
    closed: bool,
    limit: StatementLimit,
) -> (SofError, ConnectionFate) {
    if code == Some(&SqlState::QUERY_CANCELED) {
        return match limit {
            StatementLimit::Session => (
                SofError::Timeout {
                    budget: None,
                    message,
                },
                ConnectionFate::Reuse,
            ),
            StatementLimit::RunDeadline {
                passed: true,
                budget,
            } => (SofError::Timeout { budget, message }, ConnectionFate::Reuse),
            StatementLimit::RunDeadline { passed: false, .. } => (
                SofError::backend_with_sqlstate(SqlState::QUERY_CANCELED.code(), message),
                ConnectionFate::Close,
            ),
        };
    }
    let fate = if closed {
        ConnectionFate::Close
    } else {
        ConnectionFate::Reuse
    };
    (sof_error_for_sqlstate(code, message), fate)
}

/// Ends a run whose statement failed: classify, then roll back when the
/// connection may be reused.
async fn fail_statement<T>(
    session: &Session,
    run: &PgRun,
    stop: &RunStop,
    context: &str,
    e: &tokio_postgres::Error,
    set_local: bool,
) -> (Result<T, RunEnd>, ConnectionFate) {
    if std::env::var("PG_SOF_DEBUG").is_ok() {
        eprintln!(
            "[PG_SOF_DEBUG] {context}: {e}\n--- SQL ---\n{}\n---",
            run.sql
        );
    }
    let (error, fate) = classify_failure(
        e.code(),
        format!("{context}: {}", pg_error_text(e)),
        e.is_closed(),
        StatementLimit::for_run(stop, set_local),
    );
    let fate = match fate {
        ConnectionFate::Reuse => rollback(session, run, Instant::now() + CLEANUP_ENVELOPE).await,
        ConnectionFate::Close => ConnectionFate::Close,
    };
    (Err(RunEnd::Failed(error)), fate)
}

/// The `BEGIN` of a run's transaction, with its `SET LOCAL statement_timeout`
/// when the run has a deadline.
pub(crate) fn begin_sql(timeout_ms: Option<i32>) -> String {
    match timeout_ms {
        Some(ms) => format!("BEGIN READ ONLY; SET LOCAL statement_timeout = {ms}"),
        None => "BEGIN READ ONLY".to_string(),
    }
}

/// The `statement_timeout` for a transaction that starts now: the time left,
/// or `None` without a deadline. `Err` when the deadline already passed: the
/// statement must not run.
fn timeout_for_next_transaction(stop: &RunStop) -> Result<Option<i32>, StopReason> {
    match stop.remaining() {
        None => Ok(None),
        Some(left) if left.is_zero() => Err(StopReason::DeadlineExceeded),
        Some(left) => Ok(Some(statement_timeout_ms(left))),
    }
}

/// Unwraps a [`Step`] inside a reader: a failure or a stop returns from the
/// reader with the result and the connection's fate.
macro_rules! step_or_return {
    ($session:expr, $run:expr, $stop:expr, $set_local:expr, $context:expr, $request:expr) => {
        match step($session, $run, $stop, $request).await {
            Step::Done(Ok(value)) => value,
            Step::Done(Err(e)) => {
                return fail_statement($session, $run, $stop, $context, &e, $set_local).await;
            }
            Step::Stopped(reason, fate) => return (Err(RunEnd::Stopped(reason)), fate),
        }
    };
}

/// Starts a run's transaction, or ends the run as a timeout if its deadline
/// already passed. Returns whether a `SET LOCAL` was sent.
macro_rules! begin_or_return {
    ($session:expr, $run:expr, $stop:expr) => {{
        let timeout_ms = match timeout_for_next_transaction($stop) {
            Ok(timeout_ms) => timeout_ms,
            Err(reason) => return (Err(RunEnd::Stopped(reason)), ConnectionFate::Reuse),
        };
        $run.record(Recorded::Begin {
            at: Instant::now(),
            timeout_ms,
        });
        let client = $session.client();
        step_or_return!(
            $session,
            $run,
            $stop,
            timeout_ms.is_some(),
            "failed to start the run's transaction",
            client.batch_execute(&begin_sql(timeout_ms))
        );
        timeout_ms.is_some()
    }};
}

/// Starts an unpaged statement's row stream (`query_raw`), raced against the
/// stop like [`step`]. Unlike other requests, `query_raw` resolves only once
/// the server flushes its `BindComplete`, which PostgreSQL holds back while it
/// executes the `Execute` that followed in the same packet: the request can
/// stay pending for the whole statement, and resolve `Ok` after a cancel. A
/// stop here therefore drains the row stream as well, so the cancel's own
/// error decides the connection's fate.
async fn start_rows(
    session: &Session,
    run: &PgRun,
    stop: &RunStop,
    request: impl Future<Output = Result<tokio_postgres::RowStream, tokio_postgres::Error>>,
) -> Step<tokio_postgres::RowStream> {
    session.unsettle();
    tokio::pin!(request);
    let reason = tokio::select! {
        biased;
        done = &mut request => return Step::Done(done),
        reason = stop.stopped_with_grace(DEADLINE_GRACE) => reason,
    };
    let stopped_at = Instant::now();
    let fate = match request.as_mut().now_or_never() {
        Some(Ok(rows)) => {
            let rows = std::pin::pin!(rows);
            stop_rows(session, run, rows, false).await
        }
        Some(Err(_)) => rollback(session, run, stopped_at + CLEANUP_ENVELOPE).await,
        None => {
            let drain = async move {
                let rows = request.await?;
                let mut rows = std::pin::pin!(rows);
                while let Some(row) = rows.next().await {
                    row?;
                }
                Ok(())
            };
            cancel_and_drain(session, run, stopped_at, drain).await
        }
    };
    Step::Stopped(reason, backstop_fate(reason, fate))
}

/// Pins a row stream for the readers.
type PinnedRows<'a> = Pin<&'a mut tokio_postgres::RowStream>;

/// The rows of a statement that already arrived, without waiting: `true` when
/// the statement's end was among them (its end is in the received buffer).
/// Looks at a bounded number of items so a fast server cannot keep it busy.
fn ended_within_received(rows: &mut PinnedRows<'_>) -> bool {
    for _ in 0..4096 {
        match rows.as_mut().next().now_or_never() {
            Some(Some(Ok(_))) => continue,
            Some(Some(Err(_))) | Some(None) => return true,
            None => return false,
        }
    }
    false
}

/// Stops an unpaged run's statement while its rows stream: rolls back if its
/// end already arrived, otherwise cancels and drains it. `backstop` closes
/// the connection whatever the cleanup's outcome.
async fn stop_rows(
    session: &Session,
    run: &PgRun,
    mut rows: PinnedRows<'_>,
    backstop: bool,
) -> ConnectionFate {
    let stopped_at = Instant::now();
    let fate = if ended_within_received(&mut rows) {
        rollback(session, run, stopped_at + CLEANUP_ENVELOPE).await
    } else {
        let drain = async move {
            while let Some(row) = rows.next().await {
                row?;
            }
            Ok(())
        };
        cancel_and_drain(session, run, stopped_at, drain).await
    };
    if backstop {
        ConnectionFate::Close
    } else {
        fate
    }
}

/// Reads an unpaged run: one transaction, one statement, rows streamed as the
/// server sends them. Returns `(rows, pages)` and the connection's fate.
async fn read_unpaged(
    run: &PgRun,
    session: &Session,
    stop: &RunStop,
    out: &mut PgSink,
) -> (Result<(usize, usize), RunEnd>, ConnectionFate) {
    let set_local = begin_or_return!(session, run, stop);
    let client = session.client();
    if std::env::var("PG_SOF_DEBUG_ALL").is_ok() {
        eprintln!(
            "[PG_SOF_DEBUG_ALL] preparing\n--- SQL ---\n{}\n---",
            run.sql
        );
    }
    run.record(Recorded::Statement("prepare"));
    let stmt = step_or_return!(
        session,
        run,
        stop,
        set_local,
        "failed to prepare SQL",
        client.prepare(&run.sql)
    );
    let boxed = box_params(run.params.clone());
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    run.record(Recorded::Statement("query"));
    let rows = match start_rows(
        session,
        run,
        stop,
        client.query_raw(&stmt, param_refs.iter().copied()),
    )
    .await
    {
        Step::Done(Ok(rows)) => rows,
        Step::Done(Err(e)) => {
            return fail_statement(session, run, stop, "query execution failed", &e, set_local)
                .await;
        }
        Step::Stopped(reason, fate) => return (Err(RunEnd::Stopped(reason)), fate),
    };
    let mut rows = std::pin::pin!(rows);

    let mut count = 0usize;
    loop {
        if run.limit.is_some_and(|cap| count >= cap) {
            // The cap is the client's (the SQL has no LIMIT for it): the
            // statement may still be producing rows nobody reads.
            let fate = stop_rows(session, run, rows.as_mut(), false).await;
            return (Ok((count, 0)), fate);
        }
        let next = match rows.as_mut().next().now_or_never() {
            Some(next) => next,
            None => {
                // About to wait on the server: send what is pending first, so
                // sparse rows are not held back by batching.
                if let Err(reason) = out.flush(stop).await {
                    let fate = stop_rows(session, run, rows.as_mut(), false).await;
                    return (Err(RunEnd::Stopped(reason)), fate);
                }
                tokio::select! {
                    biased;
                    reason = stop.stopped_with_grace(DEADLINE_GRACE) => {
                        let backstop = reason == StopReason::DeadlineExceeded;
                        let fate = stop_rows(session, run, rows.as_mut(), backstop).await;
                        return (Err(RunEnd::Stopped(reason)), fate);
                    }
                    next = rows.next() => next,
                }
            }
        };
        match next {
            None => break,
            Some(Err(e)) => {
                return fail_statement(session, run, stop, "row error", &e, set_local).await;
            }
            Some(Ok(pg_row)) => {
                count += 1;
                match out.push_row(&pg_row, run) {
                    Ok(full) => {
                        if full {
                            if let Err(reason) = out.flush(stop).await {
                                let fate = stop_rows(session, run, rows.as_mut(), false).await;
                                return (Err(RunEnd::Stopped(reason)), fate);
                            }
                        }
                    }
                    Err(e) => {
                        let fate = stop_rows(session, run, rows.as_mut(), false).await;
                        return (Err(RunEnd::Failed(e)), fate);
                    }
                }
            }
        }
    }
    drop(param_refs);
    drop(boxed);
    run.record(Recorded::Commit);
    step_or_return!(
        session,
        run,
        stop,
        set_local,
        "failed to commit the run's transaction",
        client.batch_execute("COMMIT")
    );
    session.settle();
    (Ok((count, 1)), ConnectionFate::Reuse)
}

/// Reads an export run page by page. A page is `(after, through]` in
/// `id COLLATE "C"` order, whole resources only, so a `forEach` never splits
/// a resource across pages. Each page runs in its own transaction
/// (`BEGIN READ ONLY; SET LOCAL statement_timeout`, bound, page, `COMMIT`)
/// under what is left of the run's deadline, and is collected before it is
/// delivered: the statement ends before the consumer's back-pressure can
/// hold it open. Returns `(rows, pages)` and the connection's fate.
async fn read_pages(
    run: &PgRun,
    page_size: usize,
    session: &Session,
    stop: &RunStop,
    out: &mut PgSink,
) -> (Result<(usize, usize), RunEnd>, ConnectionFate) {
    let client = session.client();
    let mut params = run.params.clone();
    // `$1`/`$2` are tenant and resource type; the range slots are the last two.
    let (Some(PgParam::Text(tenant)), Some(PgParam::Text(resource_type))) =
        (params.first().cloned(), params.get(1).cloned())
    else {
        return (
            Err(RunEnd::Failed(SofError::backend(
                "export page has no tenant binding",
            ))),
            ConnectionFate::Reuse,
        );
    };
    let Some(range) = params.len().checked_sub(2).filter(|i| *i >= 2) else {
        return (
            Err(RunEnd::Failed(SofError::backend(
                "export page has no id range binding",
            ))),
            ConnectionFate::Reuse,
        );
    };
    if run.limit == Some(0) {
        return (Ok((0, 0)), ConnectionFate::Reuse);
    }
    let page_limit = i64::try_from(page_size).unwrap_or(i64::MAX);

    let mut statements: Option<(tokio_postgres::Statement, tokio_postgres::Statement)> = None;
    let mut after = String::new();
    let mut count = 0usize;
    let mut pages = 0usize;
    loop {
        // Between pages nothing runs and no transaction is open.
        if let Some(reason) = stop.check() {
            return (Err(RunEnd::Stopped(reason)), ConnectionFate::Reuse);
        }
        let set_local = begin_or_return!(session, run, stop);
        let (bound_stmt, page_stmt) = match &statements {
            Some(statements) => statements.clone(),
            None => {
                run.record(Recorded::Statement("prepare"));
                let bound_stmt = step_or_return!(
                    session,
                    run,
                    stop,
                    set_local,
                    "failed to prepare SQL",
                    client.prepare(&run.bound_sql)
                );
                let page_stmt = step_or_return!(
                    session,
                    run,
                    stop,
                    set_local,
                    "failed to prepare SQL",
                    client.prepare(&run.sql)
                );
                statements = Some((bound_stmt.clone(), page_stmt.clone()));
                (bound_stmt, page_stmt)
            }
        };

        run.record(Recorded::Statement("bound"));
        let bound_params: [&(dyn tokio_postgres::types::ToSql + Sync); 4] =
            [&tenant, &resource_type, &after, &page_limit];
        let bound = step_or_return!(
            session,
            run,
            stop,
            set_local,
            "page bound error",
            client.query_opt(&bound_stmt, &bound_params)
        );
        let Some(bound) = bound else {
            run.record(Recorded::Commit);
            step_or_return!(
                session,
                run,
                stop,
                set_local,
                "failed to commit the page",
                client.batch_execute("COMMIT")
            );
            session.settle();
            break;
        };
        let through: String = match bound.try_get(0) {
            Ok(through) => through,
            Err(e) => {
                let fate = rollback(session, run, Instant::now() + CLEANUP_ENVELOPE).await;
                return (
                    Err(RunEnd::Failed(SofError::backend(format!(
                        "failed to read page bound: {e}"
                    )))),
                    fate,
                );
            }
        };

        params[range] = PgParam::Text(after);
        params[range + 1] = PgParam::Text(through.clone());
        let boxed = box_params(params.clone());
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
            .iter()
            .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        run.record(Recorded::Statement("page"));
        let rows = step_or_return!(
            session,
            run,
            stop,
            set_local,
            "row error",
            client.query(&page_stmt, &param_refs)
        );
        drop(param_refs);
        drop(boxed);
        run.record(Recorded::Commit);
        step_or_return!(
            session,
            run,
            stop,
            set_local,
            "failed to commit the page",
            client.batch_execute("COMMIT")
        );
        session.settle();

        // Delivery: the page's transaction is closed, so a stop here leaves
        // nothing to cancel.
        pages += 1;
        for pg_row in &rows {
            if run.limit.is_some_and(|cap| count >= cap) {
                return (Ok((count, pages)), ConnectionFate::Reuse);
            }
            count += 1;
            let full = match out.push_row(pg_row, run) {
                Ok(full) => full,
                Err(e) => return (Err(RunEnd::Failed(e)), ConnectionFate::Reuse),
            };
            if full {
                if let Err(reason) = out.flush(stop).await {
                    return (Err(RunEnd::Stopped(reason)), ConnectionFate::Reuse);
                }
            }
        }
        // The next page waits on the server: deliver this one's tail first.
        if let Err(reason) = out.flush(stop).await {
            return (Err(RunEnd::Stopped(reason)), ConnectionFate::Reuse);
        }
        if run.limit.is_some_and(|cap| count >= cap) {
            break;
        }
        after = through;
    }
    (Ok((count, pages)), ConnectionFate::Reuse)
}

/// Boxes runtime parameters for `tokio_postgres`.
fn box_params(params: Vec<PgParam>) -> Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> {
    params
        .into_iter()
        .map(|p| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
            match p {
                PgParam::Text(s) => Box::new(s),
                PgParam::TextArray(v) => Box::new(v),
                // Bind Bool/Int/Decimal constants as text so they compare
                // cleanly against `->>`/`#>>` JSON-text projections without
                // a per-call PG type-mismatch. Numeric contexts apply
                // explicit `::numeric` casts via `lower_binop_dialect`;
                // boolean contexts compare against `'true'`/`'false'`.
                PgParam::Bool(b) => Box::new(if b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }),
                PgParam::Int(n) => Box::new(n.to_string()),
                PgParam::Decimal(s) => Box::new(s),
                PgParam::Null => Box::new(None::<String>),
                PgParam::Timestamp(dt) => Box::new(dt),
            }
        })
        .collect()
}

/// The run error for a PostgreSQL driver error, typed by SQLSTATE while the
/// code is still in hand: PostgreSQL localizes messages (`lc_messages`), so a
/// kind is never taken from the text. `context` says what the runner was
/// doing; the message keeps the SQLSTATE and the server's text
/// ([`pg_error_text`]).
fn pg_sof_error(context: &str, e: &tokio_postgres::Error) -> SofError {
    sof_error_for_sqlstate(e.code(), format!("{context}: {}", pg_error_text(e)))
}

/// Classifies a PostgreSQL error by its SQLSTATE, with the same policy as
/// [`crate::error::classify_postgres_error`] for the storage layer. A run's
/// own statements go through [`classify_failure`] first, which decides a
/// `57014` by the run's deadline; this is the policy without that context:
///
/// - `57014 query_canceled` → [`SofError::Timeout`] with no run budget (a
///   limit outside the run, the pool session's `statement_timeout`);
/// - `53300 too_many_connections`, `53400 configuration_limit_exceeded`,
///   `57P01 admin_shutdown`, `57P02 crash_shutdown`, `57P03 cannot_connect_now`
///   → [`SofError::Unavailable`] (a retry may succeed);
/// - any other SQLSTATE → [`SofError::Backend`] carrying it, so it is logged;
/// - no SQLSTATE (a closed connection, an I/O or decoding error) →
///   [`SofError::Backend`] without one.
fn sof_error_for_sqlstate(code: Option<&SqlState>, message: String) -> SofError {
    let Some(code) = code else {
        return SofError::backend(message);
    };
    // `SqlState` is compared with `==`, never matched as a pattern (it is not
    // structural-match).
    if *code == SqlState::QUERY_CANCELED {
        return SofError::Timeout {
            budget: None,
            message,
        };
    }
    if *code == SqlState::TOO_MANY_CONNECTIONS
        || *code == SqlState::CONFIGURATION_LIMIT_EXCEEDED
        || *code == SqlState::ADMIN_SHUTDOWN
        || *code == SqlState::CRASH_SHUTDOWN
        || *code == SqlState::CANNOT_CONNECT_NOW
    {
        return SofError::Unavailable { message };
    }
    SofError::backend_with_sqlstate(code.code(), message)
}

/// Error text that keeps what `Display` drops for a server error: it prints
/// only `db error`, hiding the SQLSTATE and message (`57014`, "canceling
/// statement due to statement timeout").
fn pg_error_text(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!(
            "{} (SQLSTATE {}): {}",
            db.severity(),
            db.code().code(),
            db.message()
        ),
        None => e.to_string(),
    }
}

// ============================================================================
// Keyset-paged export: the page bound
// ============================================================================

/// The id that closes the next page: the last of the next `$4` live ids of the
/// tenant's type after `$3`, in the order `idx_resources_reindex_id` stores
/// them. No row means the type is exhausted. The scan stays on the index (no
/// heap access, no sort), so it costs one page of index entries.
fn export_bound_sql() -> &'static str {
    "SELECT id FROM (\n  SELECT id FROM resources\n  WHERE tenant_id = $1 AND resource_type = $2\n    AND is_deleted = FALSE AND id COLLATE \"C\" > $3\n  ORDER BY id COLLATE \"C\"\n  LIMIT $4\n) page\nORDER BY id COLLATE \"C\" DESC\nLIMIT 1"
}

// ============================================================================
// Columnar rows
// ============================================================================

/// Appends one `tokio_postgres::Row` to the builder.
///
/// Every projected column is text, so each cell is borrowed as `&str`. Typed
/// columns parse it in the builder; `Deferred` ones decode it to JSON exactly
/// as [`row_to_json`] does.
fn append_pg_row(
    builder: &mut ColumnarBatchBuilder,
    pg_row: &tokio_postgres::Row,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> Result<(), SofError> {
    for (i, name) in columns.iter().enumerate() {
        let val: Option<&str> = pg_row
            .try_get(i)
            .map_err(|e| SofError::backend(format!("failed to read column '{name}': {e}")))?;
        let decode = decodes.get(i).copied().unwrap_or_default();
        match val {
            None => builder.append_null(i),
            Some(s) => match decode.column_kind() {
                ColumnKind::Deferred => {
                    builder.append_deferred(i, decode_text(decode, s.to_owned()))
                }
                _ => builder.append_str(i, s),
            },
        }
    }
    Ok(())
}

// ============================================================================
// Row → JSON conversion
// ============================================================================

/// Converts a `tokio_postgres::Row` into a `serde_json::Value` object.
///
/// The compiled SQL projects all columns as text via `->>`/`#>>` operators, so
/// each text value is decoded according to its column's [`ColumnDecode`]. A
/// SQL `NULL` is written as an explicit JSON `null` so the key is never lost.
fn row_to_json(
    pg_row: &tokio_postgres::Row,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> Result<ViewRow, SofError> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val: Option<String> = pg_row
            .try_get(i)
            .map_err(|e| SofError::backend(format!("failed to read column '{name}': {e}")))?;

        let json_val = match val {
            Some(s) => decode_text(decodes.get(i).copied().unwrap_or_default(), s),
            None => Value::Null,
        };
        map.insert(name.clone(), json_val);
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::super::compiler::compile_view_definition_dialect;
    use super::super::runtime::SqlRunPlan;
    use super::*;
    use crate::core::sof_runner::ViewFilters;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let run = SqlRunPlan::compile(view, SqlDialect::Postgres, FhirVersion::default_enabled())
            .expect("compile test view")
            .finish("tenant", filters)
            .expect("runtime sql");
        let sql = run.query.sql;
        let params: Vec<_> = run.params.into_iter().map(PgParam::from_runtime).collect();
        let bindings = params
            .iter()
            .map(|param| match param {
                PgParam::Text(v) => format!("text:{v}"),
                PgParam::TextArray(v) => format!("text[]:{}", v.join(",")),
                PgParam::Bool(v) => format!("bool:{v}"),
                PgParam::Int(v) => format!("int:{v}"),
                PgParam::Decimal(v) => format!("decimal:{v}"),
                PgParam::Null => "null".into(),
                PgParam::Timestamp(v) => format!("timestamp:{v}"),
            })
            .collect();
        (sql, bindings)
    }

    /// #1864 (E-1864-1, A18): the `_since` cutoff is rounded up to the
    /// microsecond where it becomes a bound value, and only there:
    /// `prepared.params` keeps the caller's instant.
    #[test]
    fn since_binds_rounded_up_to_the_microsecond() {
        let bound = |since: &str| {
            let since: chrono::DateTime<chrono::Utc> = since.parse().unwrap();
            let run = SqlRunPlan::compile(
                &flat_view(),
                SqlDialect::Postgres,
                FhirVersion::default_enabled(),
            )
            .expect("compile test view")
            .finish(
                "tenant",
                &ViewFilters {
                    since: Some(since),
                    ..Default::default()
                },
            )
            .expect("runtime sql");
            let caller = run
                .params
                .iter()
                .find_map(|p| match p {
                    RuntimeParam::Timestamp(t) => Some(*t),
                    _ => None,
                })
                .expect("a since parameter");
            assert_eq!(caller, since, "prepared.params keeps the caller's instant");
            let bound = run
                .params
                .into_iter()
                .map(PgParam::from_runtime)
                .find_map(|p| match p {
                    PgParam::Timestamp(t) => Some(t),
                    _ => None,
                })
                .expect("a bound since");
            bound.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        };
        for (since, expected) in [
            // Sub-microsecond: up to the next microsecond.
            (
                "2024-05-01T12:00:00.123456789Z",
                "2024-05-01T12:00:00.123457000Z",
            ),
            (
                "2024-05-01T12:00:00.000000001Z",
                "2024-05-01T12:00:00.000001000Z",
            ),
            // Across a second boundary.
            (
                "2024-05-01T12:00:59.999999500Z",
                "2024-05-01T12:01:00.000000000Z",
            ),
            // Already whole microseconds (and whole seconds): unchanged.
            (
                "2024-05-01T12:00:00.123456000Z",
                "2024-05-01T12:00:00.123456000Z",
            ),
            ("2024-05-01T12:00:00Z", "2024-05-01T12:00:00.000000000Z"),
        ] {
            assert_eq!(bound(since), expected, "{since}");
        }
    }

    fn flat_view() -> Value {
        json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]}]})
    }

    #[test]
    fn test_pg_export_page_sql_is_ranged_and_id_ordered() {
        let view = flat_view();
        let plain = runtime_sql(&view, &ViewFilters::default()).0;
        assert!(plain.contains("ORDER BY r.last_updated, r.id COLLATE \"C\""));
        assert!(!plain.contains("r.id COLLATE \"C\" >"));

        let paged =
            SqlRunPlan::compile(&view, SqlDialect::Postgres, FhirVersion::default_enabled())
                .unwrap()
                .finish_paged("tenant", &ViewFilters::default())
                .unwrap();
        assert!(paged.query.sql.contains("r.id COLLATE \"C\" > $3"));
        assert!(paged.query.sql.contains("r.id COLLATE \"C\" <= $4"));
        assert!(paged.query.sql.ends_with("ORDER BY r.id COLLATE \"C\""));
        // tenant, type, then the two range slots last.
        assert_eq!(paged.params.len(), 4);
    }

    #[test]
    fn test_pg_export_bound_sql_walks_the_reindex_id_index() {
        let sql = export_bound_sql();
        for needle in [
            "tenant_id = $1",
            "resource_type = $2",
            "is_deleted = FALSE",
            "id COLLATE \"C\" > $3",
            "ORDER BY id COLLATE \"C\"\n  LIMIT $4",
            "ORDER BY id COLLATE \"C\" DESC\nLIMIT 1",
        ] {
            assert!(sql.contains(needle), "missing {needle:?} in {sql}");
        }
        assert!(!sql.contains("last_updated"));
    }

    #[test]
    fn test_pg_export_page_size_is_at_least_one() {
        let mut config = deadpool_postgres::Config::new();
        config.dbname = Some("unused".into());
        let pool = config.create_pool(None, tokio_postgres::NoTls).unwrap();
        let runner = PgInDbRunner::new(pool.clone()).with_export_page_size(0);
        assert_eq!(runner.export_page_size, Some(1));
        assert_eq!(PgInDbRunner::new(pool).export_page_size, None);
    }

    #[test]
    fn test_pg_runtime_sql_appends_final_output_limit() {
        let view = flat_view();
        let compiled = compile_view_definition_dialect(
            &view,
            SqlDialect::Postgres,
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
    fn test_pg_limit_preserves_constant_and_runtime_bindings() {
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
        assert!(unlimited.contains("$3"), "{unlimited}");
        assert!(unlimited.contains("r.last_updated >= $4"), "{unlimited}");
        assert!(unlimited.contains("r.id = ANY($5::text[])"), "{unlimited}");
        assert_eq!(bindings[2], "text:male");
        assert_eq!(bindings.last().unwrap(), "text[]:p-eligible");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    #[test]
    fn test_pg_union_and_recursive_limits_keep_existing_sql() {
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
            assert_eq!(limited, unlimited);
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_pg_unrepresentable_limit_keeps_existing_sql() {
        for view in [
            flat_view(),
            json!({"resourceType":"ViewDefinition","resource":"Patient",
            "select":[{"forEach":"name","column":[{"name":"family","path":"family"}]}]}),
        ] {
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
    }

    #[test]
    fn test_pg_foreach_limit_preserves_filtered_sql_and_bindings() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],
            "select":[{"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        let mut filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        };
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("r.last_updated >= $4"));
        assert!(unlimited.contains("r.id = ANY($5::text[])"));
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            filters.limit = Some(limit);
            let (limited, limited_bindings) = runtime_sql(&view, &filters);
            assert_eq!(limited, format!("{unlimited}\nLIMIT {limit}"));
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    fn test_pg_runtime_filters_reach_every_resources_scan() {
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
            assert_eq!(sql.matches("r.last_updated >= $3").count(), scans, "{sql}");
            let membership = if view["resource"] == "Patient" {
                "r.id = ANY($4::text[])"
            } else {
                "si.value_reference = ANY("
            };
            assert_eq!(sql.matches(membership).count(), scans, "{sql}");
        }
    }

    #[test]
    fn test_pg_compartment_filter_binds_one_parameter_for_any_number_of_refs() {
        let version = FhirVersion::default_enabled();
        for resource in ["Patient", "Observation"] {
            let view = json!({"resourceType":"ViewDefinition", "resource":resource,
                "select":[{"column":[{"path":"id","name":"id"}]}]});
            let filters = ViewFilters {
                patient: (0..70_000).map(|i| format!("Patient/p{i}")).collect(),
                ..Default::default()
            };
            let run = SqlRunPlan::compile(&view, SqlDialect::Postgres, version)
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            let sql = run.query.sql;
            let params: Vec<_> = run.params.into_iter().map(PgParam::from_runtime).collect();
            assert!(!sql.contains(" OR "), "{sql}");
            let (expected_params, first) = if resource == "Patient" {
                assert!(sql.contains("r.id = ANY($3::text[])"), "{sql}");
                (3, "p0")
            } else {
                assert!(sql.contains("si.value_reference = ANY($"), "{sql}");
                (
                    2 + helios_fhir::compartment_params(version, "Patient", resource).len() + 1,
                    "Patient/p0",
                )
            };
            assert_eq!(params.len(), expected_params, "{sql}");
            assert!(params.len() < 65_535);
            let Some(PgParam::TextArray(refs)) = params.last() else {
                panic!("last param must be the text[] list");
            };
            assert_eq!(refs.len(), 70_000);
            assert_eq!(refs[0], first);
        }
    }

    /// #1864 (A4, A5): a PostgreSQL error is typed by its SQLSTATE, never by
    /// its (localizable) text, and a server fault keeps the SQLSTATE so the
    /// REST layer can log it.
    #[test]
    fn pg_errors_are_typed_by_sqlstate() {
        let typed = |code: &str| {
            sof_error_for_sqlstate(
                Some(&SqlState::from_code(code)),
                format!("row error: ERROR (SQLSTATE {code}): text"),
            )
        };
        assert!(
            matches!(typed("57014"), SofError::Timeout { budget: None, ref message } if message.contains("57014")),
            "a server-side statement_timeout is a timeout"
        );
        for code in ["53300", "53400", "57P01", "57P02", "57P03"] {
            assert!(
                matches!(typed(code), SofError::Unavailable { .. }),
                "{code} is transient"
            );
        }
        for code in ["42P01", "22P02", "XX000", "40001"] {
            match typed(code) {
                SofError::Backend {
                    sqlstate: Some(kept),
                    message,
                } => {
                    assert_eq!(kept, code);
                    assert!(message.contains(code), "{message}");
                }
                other => panic!("{code}: expected Backend with its SQLSTATE, got {other:?}"),
            }
        }
        assert!(
            matches!(
                sof_error_for_sqlstate(None, "connection closed".into()),
                SofError::Backend { sqlstate: None, .. }
            ),
            "an error without a SQLSTATE is a backend fault without one"
        );
    }
}

#[cfg(test)]
#[path = "postgres_stop_tests.rs"]
mod stop_tests;

#[cfg(all(test, feature = "postgres"))]
#[path = "postgres_docker_tests.rs"]
mod docker_tests;
