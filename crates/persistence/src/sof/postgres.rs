//! PostgreSQL in-DB SQL-on-FHIR runner.
//!
//! [`PgInDbRunner`] compiles a ViewDefinition to a parameterised PostgreSQL
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are fetched lazily via `tokio_postgres::Client::query_raw` and sent
//! in batches through a bounded [`row_batch_channel`] so the HTTP layer can
//! begin flushing before the full result set has been transferred. A batch
//! holds the rows already received from the server, up to
//! [`ROW_BATCH_MAX_ROWS`]; the loop never waits for more rows to fill one.
//! The async fetch loop runs in a `tokio::spawn` task that holds the pooled
//! connection until the statement has ended. That task's `JoinHandle` is
//! watched by [`watch_row_producer`] so a panic or cancellation reaches the
//! consumer as an `Err` item instead of a silent end of stream.
//!
//! ## Cancellation
//!
//! `tokio-postgres` keeps reading an abandoned result after its row stream
//! is dropped, and the pool recycles connections without a reset, so simply
//! dropping the stream would leave the statement running server-side and
//! delay the connection's next borrower. Instead, when the consumer goes
//! away (watched with [`Sender::closed`](tokio::sync::mpsc::Sender::closed),
//! also while the server is still computing the first row) or the
//! client-side row cap is reached, and the statement's end is not already
//! among the rows received, the loop sends a cancel request for the
//! connection's backend while it still holds the pooled connection, then
//! drains the statement to its end. Only a statement that ends with
//! `query_canceled` (SQLSTATE 57014) — proof that the cancel request was
//! consumed by that statement — returns its connection to the pool; on any
//! other outcome, or after [`CANCEL_DRAIN_TIMEOUT`], the connection is
//! detached from the pool and closed, so a cancel request still in flight
//! can never reach another borrower's statement. A statement that
//! completes normally is never cancelled.

use std::pin::Pin;
use std::time::Duration;

use deadpool_postgres::Pool;
use futures::{FutureExt as _, StreamExt as _};
use helios_fhir::FhirVersion;
use serde_json::{Map, Value};
use tokio_postgres::error::SqlState;
use tracing::{debug, trace, warn};

use crate::core::sof_runner::{
    ROW_BATCH_MAX_ROWS, RowBatch, RowBatchSender, RowStream, SofError, SofRunner, ViewFilters,
    ViewRow, row_batch_channel, watch_row_producer,
};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, SqlViewPlan, append_output_limit};
use super::decode::{ColumnDecode, decode_text};
use super::emit::ResourcePredicates;

/// Bound on cancelling an abandoned statement and draining it to its end;
/// past it, the connection is closed instead of returned to the pool.
const CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// SQL-on-FHIR runner that compiles ViewDefinitions to PostgreSQL SQL.
pub struct PgInDbRunner {
    pool: Pool,
    fhir_version: FhirVersion,
}

impl PgInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool) -> Self {
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
impl SofRunner for PgInDbRunner {
    fn runner_name(&self) -> &'static str {
        "postgres-indb"
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        mut filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        // Build the plan synchronously (cheap, no I/O) so uncompilable views
        // fail before any database access.
        let view_plan =
            SqlViewPlan::build(&view_definition, SqlDialect::Postgres, self.fhir_version)?;

        let tenant_id = tenant.tenant_id().to_string();
        let resource_type = view_definition
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Spec-correct `group` handling: resolve each Group/{id} to its
        // `member.entity` Patient references and fold them into the patient
        // filter. Same pattern as the SQLite runner.
        if !filters.group.is_empty() {
            let resolved =
                resolve_group_refs_to_patient_refs(&self.pool, &tenant_id, &filters.group).await?;
            for p in resolved {
                if !filters.patient.iter().any(|existing| existing == &p) {
                    filters.patient.push(p);
                }
            }
            filters.group.clear();
        }

        // Lower runtime filters into every resource scan and collect typed
        // params. Constants occupy `$3..`; runtime filters allocate once
        // from the next free slot.
        let (sql, columns, decodes, params) = build_pg_statement(
            &view_plan,
            tenant_id,
            resource_type,
            &filters,
            self.fhir_version,
        )?;

        debug!(
            runner = "postgres-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );
        trace!(
            runner = "postgres-indb",
            sql = %sql,
            columns = ?columns,
            constants = view_plan.constants().len(),
            "compiled ViewDefinition SQL"
        );

        let limit = filters.limit;
        let pool = self.pool.clone();

        let (tx, rows) = row_batch_channel();
        let guard_tx = tx.clone();

        let producer = tokio::spawn(async move {
            stream_pg_rows(pool, sql, params, columns, decodes, limit, tx).await;
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);

        Ok(rows)
    }
}

/// Loads each `Group/{id}` from the `resources` table and extracts its
/// `member.entity` Patient references via the shared
/// [`helios_sof::resolve_group_members_to_patient_refs`]. Returns the
/// union of those Patient refs across all supplied group refs. Unknown
/// groups are silently skipped (matches the inline path; absent-target
/// warning is audit item #5).
async fn resolve_group_refs_to_patient_refs(
    pool: &Pool,
    tenant_id: &str,
    group_refs: &[String],
) -> Result<Vec<String>, SofError> {
    if group_refs.is_empty() {
        return Ok(Vec::new());
    }
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to get pg connection: {e}")))?;
    let stmt = client
        .prepare(
            "SELECT data FROM resources \
             WHERE tenant_id = $1 \
               AND resource_type = 'Group' \
               AND id = $2 \
               AND is_deleted = false",
        )
        .await
        .map_err(|e| SofError::Storage(format!("prepare failed: {e}")))?;

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
                return Err(SofError::Storage(format!(
                    "group lookup failed for {r}: {e}"
                )));
            }
        }
    }

    let set = helios_sof::resolve_group_members_to_patient_refs(group_refs, &groups);
    Ok(set.into_iter().collect())
}

// ============================================================================
// SQL runtime-filter lowering
// ============================================================================

/// Renders the final SQL (runtime filters lowered into every resource scan,
/// plus the output limit) and returns it with the visible columns and typed
/// params.
///
/// Params: `$1 = tenant_id`, `$2 = resource_type`, `$3..` constants, then
/// runtime filter values allocated once from
/// [`SqlViewPlan::first_runtime_param`].
fn build_pg_statement(
    view_plan: &SqlViewPlan,
    tenant_id: String,
    resource_type: String,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> Result<(String, Vec<String>, Vec<ColumnDecode>, Vec<PgParam>), SofError> {
    let (predicates, runtime_params) = pg_resource_predicates(
        view_plan.first_runtime_param(),
        &resource_type,
        filters,
        fhir_version,
    );
    let compiled = view_plan.emit(&predicates)?;
    let mut sql = compiled.sql;

    // Every shape's final ORDER BY is total, so one output LIMIT after it
    // returns exactly the unlimited statement's prefix, for flat views,
    // expansions, unions and recursion alike. Oversized public usize limits
    // keep the client-side cap in the fetch loop as their only bound.
    append_output_limit(&mut sql, filters.limit);

    let mut all_params = vec![PgParam::Text(tenant_id), PgParam::Text(resource_type)];
    all_params.extend(compiled.constants.iter().map(PgParam::from_lit));
    all_params.extend(runtime_params);

    Ok((sql, compiled.columns, compiled.column_decodes, all_params))
}

/// Allocates the runtime filter slots once, from `first_param`, and builds
/// the resource predicates (`_since`, Patient/Group compartment) the emitter
/// attaches to every resource scan. Returns them with their bound values, in
/// slot order.
fn pg_resource_predicates(
    first_param: usize,
    resource_type: &str,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> (ResourcePredicates, Vec<PgParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<PgParam> = Vec::new();
    let mut next_param = first_param;

    if let Some(since) = filters.since {
        conditions.push(format!("r.last_updated >= ${next_param}"));
        params.push(PgParam::Timestamp(since));
        next_param += 1;
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Patient",
        resource_type,
        &filters.patient,
        &mut next_param,
        &mut params,
    ) {
        conditions.push(c);
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Group",
        resource_type,
        &filters.group,
        &mut next_param,
        &mut params,
    ) {
        conditions.push(c);
    }

    (
        ResourcePredicates::new(first_param, next_param - first_param, conditions),
        params,
    )
}

/// Builds a PostgreSQL `WHERE` fragment that filters `r` to resources in
/// the named compartment of any of `compartment_refs`. Drives the lookup
/// off the spec's `CompartmentDefinition` via
/// [`helios_fhir::compartment_params`] and queries the pre-populated
/// `search_index` table — no FHIRPath evaluation at query time.
///
/// See the matching SQLite implementation for algorithm details; the only
/// difference here is `$N` parameter syntax instead of `?N`.
fn compartment_filter_sql(
    fhir_version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
    compartment_refs: &[String],
    next_param: &mut usize,
    extra_params: &mut Vec<PgParam>,
) -> Option<String> {
    if compartment_refs.is_empty() {
        return None;
    }

    let canonical_prefix = format!("{}/", compartment_type);

    // Case 1: the view's resource is the compartment owner itself.
    if resource_type == compartment_type {
        let mut ors: Vec<String> = Vec::with_capacity(compartment_refs.len());
        for r in compartment_refs {
            let id = r.strip_prefix(canonical_prefix.as_str()).unwrap_or(r);
            let p = *next_param;
            ors.push(format!("r.id = ${p}"));
            extra_params.push(PgParam::Text(id.to_string()));
            *next_param += 1;
        }
        return Some(format!("({})", ors.join(" OR ")));
    }

    // Case 2: look up the search-param names that link `resource_type`
    // to the compartment.
    let names = helios_fhir::compartment_params(fhir_version, compartment_type, resource_type);
    if names.is_empty() {
        return Some("1=0".to_string());
    }

    let mut name_placeholders = Vec::with_capacity(names.len());
    for n in names {
        let p = *next_param;
        name_placeholders.push(format!("${p}"));
        extra_params.push(PgParam::Text((*n).to_string()));
        *next_param += 1;
    }

    let mut ref_placeholders = Vec::with_capacity(compartment_refs.len());
    for r in compartment_refs {
        let canonical = if r.starts_with(canonical_prefix.as_str()) {
            r.clone()
        } else {
            format!("{}{}", canonical_prefix, r)
        };
        let p = *next_param;
        ref_placeholders.push(format!("${p}"));
        extra_params.push(PgParam::Text(canonical));
        *next_param += 1;
    }

    // `$1` and `$2` are tenant_id and resource_type (bound by the outer
    // query); we reuse them inside the EXISTS subquery so the search_index
    // join stays tenant-isolated and resource-typed.
    Some(format!(
        "EXISTS (SELECT 1 FROM search_index si \
         WHERE si.tenant_id = $1 \
           AND si.resource_type = $2 \
           AND si.resource_id = r.id \
           AND si.param_name IN ({}) \
           AND si.value_reference IN ({}))",
        name_placeholders.join(","),
        ref_placeholders.join(",")
    ))
}

// ============================================================================
// Typed parameter enum — avoids the self-referential borrow issues with
// `Vec<Box<dyn ToSql>>` + `Vec<&dyn ToSql>` that arise in async tasks.
// ============================================================================

#[derive(Clone)]
enum PgParam {
    Text(String),
    Bool(bool),
    Int(i64),
    Decimal(String),
    Null,
    Timestamp(chrono::DateTime<chrono::Utc>),
}

impl PgParam {
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
// Async fetch loop
// ============================================================================

/// Whether a connection whose statement has ended may return to the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionFate {
    /// No cancel request can still reach the connection: recycle it.
    Reuse,
    /// A cancel request may still be in flight, or the statement could not be
    /// confirmed ended: detach the connection from the pool and close it.
    Close,
}

/// Why [`fetch_batches`] stopped reading a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchEnd {
    /// The statement ended: its last row was read, or it failed server-side.
    Ended,
    /// The statement is still running but no longer wanted: the consumer is
    /// gone, the client-side row cap was reached, or a row failed to decode.
    Abandoned,
}

async fn stream_pg_rows(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    tx: RowBatchSender,
) {
    // Nothing runs before a connection is acquired, so a consumer that
    // leaves while the pool is exhausted just ends the task.
    let client = tokio::select! {
        biased;
        () = tx.closed() => return,
        client = pool.get() => client,
    };
    let client = match client {
        Ok(client) => client,
        Err(e) => {
            let _ = tx
                .send(Err(SofError::Storage(format!(
                    "failed to acquire Postgres connection: {e}"
                ))))
                .await;
            return;
        }
    };
    let (result, fate) =
        run_pg_statement(&client, &sql, params, &columns, &decodes, limit, &tx).await;
    if fate == ConnectionFate::Close {
        // Dropping the detached client aborts its connection task, which
        // closes the socket; the pool opens a fresh connection when needed.
        drop(deadpool_postgres::Object::take(client));
    }
    if let Err(e) = result {
        let _ = tx.send(Err(e)).await;
    }
    // tx dropped here, closing the row stream
}

/// Prepares and executes `sql` on `client`, sending its rows to `tx`.
///
/// Returns the error to report on the stream — failures before the first
/// row; row errors are sent in place — and whether `client` may be reused.
async fn run_pg_statement(
    client: &tokio_postgres::Client,
    sql: &str,
    params: Vec<PgParam>,
    columns: &[String],
    decodes: &[ColumnDecode],
    limit: Option<usize>,
    tx: &RowBatchSender,
) -> (Result<(), SofError>, ConnectionFate) {
    if std::env::var("PG_SOF_DEBUG_ALL").is_ok() {
        eprintln!("[PG_SOF_DEBUG_ALL] preparing\n--- SQL ---\n{sql}\n---");
    }
    // Parse + Describe is usually quick, but waits on any lock that blocks
    // reading `resources` (DDL). If the consumer leaves meanwhile, cancel it
    // and still await it, so its named statement is never leaked on the
    // pooled session.
    let prepare = client.prepare(sql);
    futures::pin_mut!(prepare);
    let stmt = tokio::select! {
        biased;
        result = &mut prepare => match result {
            Ok(stmt) => stmt,
            Err(e) => {
                if std::env::var("PG_SOF_DEBUG").is_ok() {
                    eprintln!("[PG_SOF_DEBUG] prepare failed: {e}\n--- SQL ---\n{sql}\n---");
                }
                let error = SofError::Backend(format!("failed to prepare SQL: {e}"));
                return (Err(error), ConnectionFate::Reuse);
            }
        },
        () = tx.closed() => {
            debug!(runner = "postgres-indb", "consumer left while preparing; cancelling");
            let fate = cancel_and_drain(client, async move {
                match prepare.await {
                    // Prepared before the cancel landed: it may still come.
                    Ok(_) => ConnectionFate::Close,
                    Err(e) => fate_after_cancel(&e),
                }
            })
            .await;
            return (Ok(()), fate);
        }
    };
    if tx.is_closed() {
        return (Ok(()), ConnectionFate::Reuse);
    }

    // Build boxed params for query_raw; these are 'static + Send
    let boxed: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = params
        .into_iter()
        .map(|p| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
            match p {
                PgParam::Text(s) => Box::new(s),
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
        .collect();

    // query_raw needs a slice of &dyn ToSql + Sync. Build references that borrow
    // from `boxed` — both live in this async block's stack frame, so no lifetime
    // issue (the future holds them until the stream is exhausted).
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let query = client.query_raw(&stmt, param_refs.iter().copied());
    futures::pin_mut!(query);
    let raw = tokio::select! {
        biased;
        result = &mut query => match result {
            Ok(raw) => raw,
            Err(e) => {
                if std::env::var("PG_SOF_DEBUG").is_ok() {
                    eprintln!("[PG_SOF_DEBUG] query failed: {e}\n--- SQL ---\n{sql}\n---");
                }
                let error = SofError::Backend(format!("query execution failed: {e}"));
                return (Err(error), ConnectionFate::Reuse);
            }
        },
        // Bind and Execute are already on the wire, so the statement runs
        // until it is cancelled; then finish the call and drain what it
        // returns.
        () = tx.closed() => {
            debug!(runner = "postgres-indb", "consumer left while the statement was being bound; cancelling");
            let fate = cancel_and_drain(client, async move {
                match query.await {
                    Ok(raw) => {
                        futures::pin_mut!(raw);
                        drain_cancelled(raw).await
                    }
                    Err(e) => fate_after_cancel(&e),
                }
            })
            .await;
            return (Ok(()), fate);
        }
    };

    futures::pin_mut!(raw);
    let mut rows = 0usize;
    let end = fetch_batches(raw.as_mut(), columns, decodes, limit, tx, &mut rows).await;
    debug!(
        runner = "postgres-indb",
        rows,
        ?end,
        "in-DB view run complete"
    );
    match end {
        FetchEnd::Ended => (Ok(()), ConnectionFate::Reuse),
        // A small result often arrived in full before the consumer left:
        // then the statement has ended and there is nothing to cancel.
        FetchEnd::Abandoned if ended_within_received(raw.as_mut()) => {
            (Ok(()), ConnectionFate::Reuse)
        }
        FetchEnd::Abandoned => (Ok(()), cancel_and_drain(client, drain_cancelled(raw)).await),
    }
}

/// Discards the rows already received from the server, without waiting for
/// more and at most [`ROW_BATCH_MAX_ROWS`] of them; returns whether the
/// statement's end (or its server error) was among them.
fn ended_within_received(mut raw: Pin<&mut tokio_postgres::RowStream>) -> bool {
    for _ in 0..ROW_BATCH_MAX_ROWS {
        match raw.next().now_or_never() {
            None => return false,
            Some(None | Some(Err(_))) => return true,
            Some(Some(Ok(_))) => {}
        }
    }
    false
}

/// Reads `raw` and sends its rows to `tx` in batches until the statement
/// ends or is abandoned, counting the rows read into `rows`.
///
/// A batch takes the rows already received from the server, up to
/// [`ROW_BATCH_MAX_ROWS`], and is sent as soon as no further row is ready,
/// so batching never holds a row back while the server computes the next
/// one, and a short (preview) result is delivered as soon as it ends. A row
/// or server error is sent after the rows that preceded it.
async fn fetch_batches(
    mut raw: Pin<&mut tokio_postgres::RowStream>,
    columns: &[String],
    decodes: &[ColumnDecode],
    limit: Option<usize>,
    tx: &RowBatchSender,
    rows: &mut usize,
) -> FetchEnd {
    let mut batch = RowBatch::new();
    loop {
        // Wait for the next row or the end — or for the consumer to leave,
        // which also catches it leaving while PostgreSQL is still computing
        // the first row (sorting, waiting on a lock, ...).
        let mut next = tokio::select! {
            biased;
            () = tx.closed() => return FetchEnd::Abandoned,
            next = raw.next() => Some(next),
        };
        let mut end = None;
        let mut error = None;
        while let Some(item) = next.take() {
            match item {
                None => end = Some(FetchEnd::Ended),
                Some(Ok(pg_row)) => {
                    if limit.is_some_and(|cap| *rows >= cap) {
                        end = Some(FetchEnd::Abandoned);
                        break;
                    }
                    *rows += 1;
                    match row_to_json(&pg_row, columns, decodes) {
                        Ok(row) => batch.push(row),
                        Err(e) => {
                            error = Some(e);
                            end = Some(FetchEnd::Abandoned);
                            break;
                        }
                    }
                    if batch.len() < ROW_BATCH_MAX_ROWS {
                        // Only a row (or the end) that has already arrived.
                        next = raw.next().now_or_never();
                    }
                }
                Some(Err(e)) => {
                    if std::env::var("PG_SOF_DEBUG").is_ok() {
                        eprintln!("[PG_SOF_DEBUG] row error: {e}");
                    }
                    error = Some(SofError::Backend(format!("row error: {e}")));
                    end = Some(FetchEnd::Ended);
                }
            }
        }

        let mut consumer_gone =
            !batch.is_empty() && tx.send(Ok(std::mem::take(&mut batch))).await.is_err();
        if let Some(e) = error.filter(|_| !consumer_gone) {
            consumer_gone = tx.send(Err(e)).await.is_err();
        }
        match end {
            Some(end) => return end,
            None if consumer_gone => return FetchEnd::Abandoned,
            None => {}
        }
    }
}

/// Cancels the statement running on `client`, then awaits `finish`, which
/// drives that statement to its end and judges the connection; returns
/// whether `client` may be reused.
///
/// Must run while the caller still holds `client`'s pool object: a cancel
/// request names a backend process, not a statement, so one sent after the
/// connection went back to the pool could cancel another borrower's
/// statement. Bounded by [`CANCEL_DRAIN_TIMEOUT`].
async fn cancel_and_drain(
    client: &tokio_postgres::Client,
    finish: impl Future<Output = ConnectionFate>,
) -> ConnectionFate {
    let fate = tokio::time::timeout(CANCEL_DRAIN_TIMEOUT, async {
        // The pool connects without TLS, and so does the cancel request.
        if let Err(e) = client
            .cancel_token()
            .cancel_query(tokio_postgres::NoTls)
            .await
        {
            warn!(
                runner = "postgres-indb",
                error = %e,
                "could not cancel an abandoned SoF statement; closing its connection"
            );
            return ConnectionFate::Close;
        }
        finish.await
    })
    .await;
    let fate = fate.unwrap_or_else(|_| {
        warn!(
            runner = "postgres-indb",
            timeout = ?CANCEL_DRAIN_TIMEOUT,
            "abandoned SoF statement did not end after its cancel request; closing its connection"
        );
        ConnectionFate::Close
    });
    debug!(
        runner = "postgres-indb",
        ?fate,
        "abandoned statement stopped"
    );
    fate
}

/// Reads and discards a cancelled statement's remaining rows.
async fn drain_cancelled(mut raw: Pin<&mut tokio_postgres::RowStream>) -> ConnectionFate {
    while let Some(item) = raw.next().await {
        if let Err(e) = item {
            return fate_after_cancel(&e);
        }
    }
    // The statement completed before the cancel request reached the server,
    // which may still deliver it — to whatever this backend runs next.
    ConnectionFate::Close
}

/// Only `query_canceled` proves the cancel request was consumed by this
/// statement; after any other error it may still be pending.
fn fate_after_cancel(error: &tokio_postgres::Error) -> ConnectionFate {
    if error.code() == Some(&SqlState::QUERY_CANCELED) {
        ConnectionFate::Reuse
    } else {
        ConnectionFate::Close
    }
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
            .map_err(|e| SofError::Backend(format!("failed to read column '{name}': {e}")))?;

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
    use super::*;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let view_plan =
            SqlViewPlan::build(view, SqlDialect::Postgres, FhirVersion::default_enabled())
                .expect("compile test view");
        let resource_type = view["resource"].as_str().unwrap_or_default().to_string();
        let (sql, _, _, params) = build_pg_statement(
            &view_plan,
            "tenant".into(),
            resource_type,
            filters,
            FhirVersion::default_enabled(),
        )
        .expect("emit test view");
        let bindings = params
            .iter()
            .map(|param| match param {
                PgParam::Text(v) => format!("text:{v}"),
                PgParam::Bool(v) => format!("bool:{v}"),
                PgParam::Int(v) => format!("int:{v}"),
                PgParam::Decimal(v) => format!("decimal:{v}"),
                PgParam::Null => "null".into(),
                PgParam::Timestamp(v) => format!("timestamp:{v}"),
            })
            .collect();
        (sql, bindings)
    }

    fn flat_view() -> Value {
        json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]}]})
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
        assert!(unlimited.contains("r.id = $5"), "{unlimited}");
        assert_eq!(bindings[2], "text:male");
        assert_eq!(bindings.last().unwrap(), "text:p-eligible");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    /// Union, repeat, union-with-repeat-branch and multi-path repeat views.
    fn complex_limit_views() -> [Value; 4] {
        [
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"unionAll":[
                {"column":[{"path":"id","name":"id"}]},
                {"column":[{"path":"gender","name":"id"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"unionAll":[
                {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
                {"column":[{"path":"id","name":"v"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item","answer.item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
        ]
    }

    #[test]
    fn test_pg_union_and_recursive_limits_append_one_final_limit() {
        for view in complex_limit_views() {
            let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
            assert!(!unlimited.contains("\nLIMIT "), "{unlimited}");
            let (limited, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(50),
                    ..Default::default()
                },
            );
            // One LIMIT, after the final (outer, for unions) ORDER BY.
            assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
            assert_eq!(limited.matches("\nLIMIT ").count(), 1, "{limited}");
            let tail = &unlimited[unlimited.rfind("ORDER BY ").expect("final ORDER BY")..];
            assert_eq!(
                tail.matches('(').count(),
                tail.matches(')').count(),
                "the final ORDER BY is top-level: {unlimited}"
            );
            if unlimited.contains("\nUNION ALL\n") {
                assert!(
                    union_operands(&limited)
                        .iter()
                        .all(|operand| !operand.contains("\nLIMIT ")),
                    "never per branch: {limited}"
                );
            }
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_pg_unrepresentable_limit_keeps_existing_sql() {
        let mut views = vec![
            flat_view(),
            json!({"resourceType":"ViewDefinition","resource":"Patient",
            "select":[{"forEach":"name","column":[{"name":"family","path":"family"}]}]}),
        ];
        views.extend(complex_limit_views());
        for view in views {
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
    fn test_pg_expansion_final_limit_preserves_filtered_sql_and_bindings() {
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
        assert!(unlimited.contains("r.id = $5"));
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            filters.limit = Some(limit);
            let (limited, limited_bindings) = runtime_sql(&view, &filters);
            assert_eq!(limited, format!("{unlimited}\nLIMIT {limit}"));
            assert_eq!(limited_bindings, bindings);
        }
        // A constant-filtered union with a repeat branch keeps its constant
        // and runtime-filter slots, and takes the same single final LIMIT.
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
        "constant":[{"name":"s","valueString":"completed"}],
        "where":[{"path":"status = %s"}],
        "select":[{"unionAll":[
            {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
            {"column":[{"path":"id","name":"v"}]}
        ]}]});
        filters.limit = None;
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("r.last_updated >= $4"), "{unlimited}");
        assert_eq!(bindings[2], "text:completed");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    /// Splits a runtime statement into its top-level `UNION ALL` operands,
    /// dropping the outer visible projection and the final ordering.
    fn union_operands(sql: &str) -> Vec<&str> {
        let (_, inner) = sql
            .split_once("\nFROM (\n")
            .expect("union is wrapped in an outer SELECT");
        inner
            .split_once("\n) AS u\nORDER BY ")
            .expect("outer ORDER BY over the wrapped union")
            .0
            .split("\nUNION ALL\n")
            .collect()
    }

    /// Returns the `WITH RECURSIVE` CTE body and the outer SELECT of one
    /// recursive statement or union operand.
    fn recursive_parts(sql: &str) -> (&str, &str) {
        let start = sql.find("AS (\n").expect("recursive CTE body") + "AS (\n".len();
        let end = sql.find("\n)\nSELECT").expect("recursive CTE end");
        (&sql[start..end], &sql[end..])
    }

    fn since_and_patient() -> ViewFilters {
        ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        }
    }

    #[test]
    fn test_pg_runtime_filters_reuse_slots_in_every_union_branch() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
        "constant":[{"name":"g","valueString":"male"}],
        "where":[{"path":"gender = %g"}],
        "select":[{"unionAll":[
            {"column":[{"path":"id","name":"v"}]},
            {"forEach":"name","column":[{"path":"family","name":"v"}]},
            {"column":[{"path":"gender","name":"v"}]}
        ]}]});
        let (sql, bindings) = runtime_sql(&view, &since_and_patient());
        let operands = union_operands(&sql);
        assert_eq!(operands.len(), 3, "{sql}");
        for operand in &operands {
            assert!(operand.contains("$3"), "constant in {operand}");
            assert_eq!(
                operand.matches("r.last_updated >= $4").count(),
                1,
                "{operand}"
            );
            assert_eq!(operand.matches("(r.id = $5)").count(), 1, "{operand}");
        }
        assert!(
            !sql.contains("$6"),
            "runtime slots must be allocated once: {sql}"
        );
        assert_eq!(
            bindings,
            [
                "text:tenant",
                "text:Patient",
                "text:male",
                "timestamp:2024-01-01 00:00:00 UTC",
                "text:p-eligible"
            ]
        );
    }

    #[test]
    fn test_pg_runtime_filters_lower_into_every_recursive_seed() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "constant":[{"name":"s","valueString":"completed"}],
            "where":[{"path":"status = %s"}],
            "select":[{"repeat":["item","answer.item"],
                "column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, bindings) = runtime_sql(&view, &since_and_patient());
        let (cte, outer) = recursive_parts(&sql);
        // Two seeds (one per repeat path) each carry the resource predicates.
        assert_eq!(cte.matches("FROM resources r").count(), 2, "{sql}");
        assert_eq!(cte.matches("r.last_updated >= $4").count(), 2, "{sql}");
        assert_eq!(cte.matches("FROM search_index si").count(), 2, "{sql}");
        assert!(!outer.contains("r.last_updated"), "{sql}");
        // #1623 2C: the first-column primary key gains explicit NULL
        // placement and the resource-key/traversal-identity tie-breaks.
        assert!(
            sql.ends_with(
                "FROM rec_0\nORDER BY 1 ASC NULLS LAST, rec_0.last_updated, rec_0.rid, rec_0.ident"
            ),
            "{sql}"
        );
        assert_eq!(bindings[2], "text:completed");
        assert_eq!(bindings[3], "timestamp:2024-01-01 00:00:00 UTC");
        assert_eq!(bindings.last().unwrap(), "text:Patient/p-eligible");
        let slots = bindings.len();
        assert!(sql.contains(&format!("${slots}")), "{sql}");
        assert!(!sql.contains(&format!("${}", slots + 1)), "{sql}");
    }

    #[test]
    fn test_pg_runtime_filters_lower_into_recursive_union_branch_and_rejoin() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
        "select":[{"unionAll":[
            {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
            {"column":[{"path":"id","name":"v"}]}
        ]}]});
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let (sql, bindings) = runtime_sql(&view, &filters);
        let operands = union_operands(&sql);
        assert_eq!(operands.len(), 2, "{sql}");
        let (cte, outer) = recursive_parts(operands[0]);
        assert_eq!(cte.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert!(outer.ends_with("FROM rec_0) AS _recurse_0"), "{sql}");
        assert_eq!(
            operands[1].matches("r.last_updated >= $3").count(),
            1,
            "{sql}"
        );
        assert_eq!(bindings.len(), 3);

        // A sibling resource column rejoins `resources r`; that scan carries
        // the predicates too.
        let rejoin = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"repeat":["item"],"column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, _) = runtime_sql(&rejoin, &filters);
        let (cte, outer) = recursive_parts(&sql);
        assert_eq!(cte.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert_eq!(outer.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert!(
            outer.contains("JOIN resources r ON r.id = rec_0.rid"),
            "{sql}"
        );
    }

    // ------------------------------------------------------------------
    // Streaming: row batches and cancelling an abandoned statement
    // (PostgreSQL 16 testcontainer per test; requires Docker)
    // ------------------------------------------------------------------

    use std::time::{Duration, Instant};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    /// `application_name` of the test pools' sessions.
    const STREAM_TEST_APP: &str = "sof_stream_test";

    struct StreamPg {
        pool: Pool,
        /// A session outside the pool, for `pg_stat_activity`.
        observer: tokio_postgres::Client,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    /// A fresh PostgreSQL with a `pool_size` runner pool (no TLS, like the
    /// backend's pool) and an observer session.
    async fn stream_pg(pool_size: usize) -> StreamPg {
        let container = Postgres::default()
            .with_tag("16-alpine")
            .with_label(
                "github.run_id",
                std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            )
            .start()
            .await
            .expect("start PostgreSQL 16 testcontainer");
        let host = container.get_host().await.expect("host").to_string();
        let port = container.get_host_port_ipv4(5432).await.expect("port");

        let mut cfg = deadpool_postgres::Config::new();
        cfg.host = Some(host.clone());
        cfg.port = Some(port);
        cfg.dbname = Some("postgres".into());
        cfg.user = Some("postgres".into());
        cfg.password = Some("postgres".into());
        cfg.application_name = Some(STREAM_TEST_APP.into());
        let pool = cfg
            .builder(tokio_postgres::NoTls)
            .expect("pool builder")
            .max_size(pool_size)
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .expect("pool");

        let mut config = tokio_postgres::Config::new();
        config
            .host(&host)
            .port(port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (observer, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("observer session");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        StreamPg {
            pool,
            observer,
            _container: container,
        }
    }

    /// Streams `sql` (one text column `n`, decoded as an integer) through the
    /// runner's producer. Returns the row stream and the producer's handle.
    fn stream_raw(
        pool: &Pool,
        sql: String,
        limit: Option<usize>,
    ) -> (RowStream, tokio::task::JoinHandle<()>) {
        let (tx, rows) = row_batch_channel();
        let producer = tokio::spawn(stream_pg_rows(
            pool.clone(),
            sql,
            Vec::new(),
            vec!["n".into()],
            vec![ColumnDecode::Integer],
            limit,
            tx,
        ));
        (rows, producer)
    }

    /// A unique comment tag that finds a statement in `pg_stat_activity`.
    fn statement_marker() -> String {
        format!("sof-stream-{}", uuid::Uuid::new_v4().simple())
    }

    async fn collect_pg_items(mut stream: RowStream) -> Vec<Result<Value, String>> {
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut items = Vec::new();
            while let Some(item) = stream.next().await {
                items.push(item.map_err(|e| e.to_string()));
            }
            items
        })
        .await
        .expect("stream consumption must not hang")
    }

    /// The pid of the pool session running the statement tagged `marker`,
    /// once it is active.
    async fn active_pid(observer: &tokio_postgres::Client, marker: &str) -> i32 {
        let pattern = format!("%{marker}%");
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let row = observer
                    .query_opt(
                        "SELECT pid FROM pg_stat_activity \
                         WHERE application_name = $1 AND state = 'active' AND query LIKE $2",
                        &[&STREAM_TEST_APP, &pattern],
                    )
                    .await
                    .expect("read pg_stat_activity");
                if let Some(row) = row {
                    return row.get(0);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the tagged statement never became active")
    }

    /// The session's `pg_stat_activity` state; `None` once it disconnected.
    async fn session_state(observer: &tokio_postgres::Client, pid: i32) -> Option<String> {
        observer
            .query_opt("SELECT state FROM pg_stat_activity WHERE pid = $1", &[&pid])
            .await
            .expect("read pg_stat_activity")
            .and_then(|row| row.get(0))
    }

    /// Waits (bounded) until the session is no longer running a statement,
    /// and returns its state then.
    async fn state_once_stopped(observer: &tokio_postgres::Client, pid: i32) -> Option<String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = session_state(observer, pid).await;
                if state.as_deref() != Some("active") {
                    return state;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the abandoned statement is still active")
    }

    /// The backend pids of every connection the pool hands out right now.
    async fn pool_pids(pool: &Pool, connections: usize) -> Vec<i32> {
        let mut clients = Vec::new();
        for _ in 0..connections {
            clients.push(pool.get().await.expect("pooled connection"));
        }
        let mut pids = Vec::new();
        for client in &clients {
            let row = client
                .query_one("SELECT pg_backend_pid()", &[])
                .await
                .expect("pooled connection is usable");
            pids.push(row.get(0));
        }
        pids
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_abandoned_statement_is_cancelled_before_its_first_row() {
        let pg = stream_pg(2).await;
        let marker = statement_marker();
        // No row for a minute, like a statement still sorting.
        let (stream, producer) = stream_raw(
            &pg.pool,
            format!("SELECT pg_sleep(60)::text AS n /* {marker} */"),
            None,
        );
        let pid = active_pid(&pg.observer, &marker).await;

        // An unrelated statement on the pool's other connection, running
        // across the cancel.
        let other = pg.pool.get().await.expect("second pooled connection");
        let unrelated = tokio::spawn(async move {
            let row = other.query_one("SELECT pg_backend_pid(), pg_sleep(1.5)::text", &[]);
            row.await.map(|row| row.get::<_, i32>(0))
        });
        tokio::time::sleep(Duration::from_millis(200)).await;

        let dropped = Instant::now();
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), producer)
            .await
            .expect("the producer must stop promptly")
            .expect("producer must not panic");
        // Cancelled (query_canceled), drained, and recycled: still connected.
        assert_eq!(
            state_once_stopped(&pg.observer, pid).await.as_deref(),
            Some("idle")
        );
        println!(
            "[pg-cancel] statement stopped {:?} after the drop",
            dropped.elapsed()
        );

        let unrelated_pid = unrelated
            .await
            .unwrap()
            .expect("the cancel must not reach another session's statement");
        assert_ne!(unrelated_pid, pid);
        let pids = pool_pids(&pg.pool, 2).await;
        assert!(
            pids.contains(&pid),
            "the cancelled connection is reused: {pids:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_abandoned_statement_is_cancelled_while_planning() {
        let pg = stream_pg(1).await;
        // Constant-folded while the statement is planned (at Bind), so the
        // runner is still waiting for `query_raw` when the consumer leaves.
        pg.observer
            .batch_execute(
                "CREATE FUNCTION slow_to_plan() RETURNS text IMMUTABLE LANGUAGE plpgsql \
                 AS $$ BEGIN PERFORM pg_sleep(60); RETURN '1'; END $$",
            )
            .await
            .unwrap();
        let marker = statement_marker();
        let (stream, producer) = stream_raw(
            &pg.pool,
            format!("SELECT slow_to_plan() AS n /* {marker} */"),
            None,
        );
        let pid = active_pid(&pg.observer, &marker).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), producer)
            .await
            .expect("the producer must stop promptly")
            .expect("producer must not panic");
        assert_eq!(
            state_once_stopped(&pg.observer, pid).await.as_deref(),
            Some("idle")
        );
        assert_eq!(pool_pids(&pg.pool, 1).await, [pid]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_abandoned_statement_is_cancelled_mid_stream_and_at_the_row_cap() {
        let pg = stream_pg(1).await;
        // 50M rows streamed from the select list: minutes to finish in full.
        let long_sql =
            |marker: &str| format!("SELECT generate_series(1, 50000000)::text AS n /* {marker} */");

        let marker = statement_marker();
        let (mut stream, producer) = stream_raw(&pg.pool, long_sql(&marker), None);
        let first = stream.next().await.expect("first row").expect("first row");
        assert_eq!(first, json!({"n": 1}));
        let pid = active_pid(&pg.observer, &marker).await;
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), producer)
            .await
            .expect("the producer must stop promptly")
            .expect("producer must not panic");
        assert_eq!(
            state_once_stopped(&pg.observer, pid).await.as_deref(),
            Some("idle")
        );

        // A client-side row cap (no SQL LIMIT) ends the stream and cancels
        // the rest, on the same — reused — connection.
        let marker = statement_marker();
        let (stream, producer) = stream_raw(&pg.pool, long_sql(&marker), Some(10));
        let items = collect_pg_items(stream).await;
        let expected: Vec<Result<Value, String>> = (1..=10).map(|n| Ok(json!({"n": n}))).collect();
        assert_eq!(items, expected);
        tokio::time::timeout(Duration::from_secs(5), producer)
            .await
            .expect("the producer must stop promptly")
            .expect("producer must not panic");
        assert_eq!(
            session_state(&pg.observer, pid).await.as_deref(),
            Some("idle")
        );
        assert_eq!(pool_pids(&pg.pool, 1).await, [pid]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_row_batches_keep_order_multiplicity_and_error_position() {
        let pg = stream_pg(1).await;
        let [pid] = pool_pids(&pg.pool, 1).await[..] else {
            unreachable!()
        };

        // Every n twice, across several full batches and a partial last one.
        let (stream, producer) = stream_raw(
            &pg.pool,
            "SELECT g::text AS n FROM generate_series(1, 1000) g, (VALUES (1), (2)) AS d(k) \
             ORDER BY g, k"
                .into(),
            None,
        );
        let items = collect_pg_items(stream).await;
        producer.await.unwrap();
        let expected: Vec<Result<Value, String>> = (1..=1000)
            .flat_map(|n| [Ok(json!({"n": n})), Ok(json!({"n": n}))])
            .collect();
        assert_eq!(items, expected);

        // Row 700 fails server-side: rows 1..=699, then the error, then the
        // end.
        let (stream, producer) = stream_raw(
            &pg.pool,
            "SELECT CASE WHEN g = 700 THEN (1 / (g - 700))::text ELSE g::text END AS n \
             FROM generate_series(1, 1000) g"
                .into(),
            None,
        );
        let items = collect_pg_items(stream).await;
        producer.await.unwrap();
        assert_eq!(items.len(), 700, "{:?}", items.last());
        for (index, item) in items[..699].iter().enumerate() {
            assert_eq!(item, &Ok(json!({"n": index + 1})));
        }
        let error = items[699].as_ref().expect_err("row 700 is the error");
        assert!(error.contains("row error"), "{error}");

        // Statements that ended on their own are never cancelled: the same
        // connection stays in the pool and runs its next statement.
        assert_eq!(pool_pids(&pg.pool, 1).await, [pid]);
        let client = pg.pool.get().await.unwrap();
        client
            .query_one("SELECT pg_sleep(0.3)::text", &[])
            .await
            .expect("no cancel may be pending on a completed statement's connection");
    }

    /// Consumers that leave before, during and after small results: a cancel
    /// request is only ever sent while the runner holds the connection, and a
    /// connection that might still receive one is closed, so the next
    /// borrower's statement is never cancelled.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_cancel_never_reaches_the_next_borrower() {
        let pg = stream_pg(1).await;
        let mut reused = 0;
        let mut previous_pid = None;
        for round in 0..30usize {
            let (mut stream, producer) = stream_raw(
                &pg.pool,
                format!("SELECT generate_series(1, {})::text AS n", 300 + round * 50),
                None,
            );
            for _ in 0..(round % 4) * 100 {
                if stream.next().await.is_none() {
                    break;
                }
            }
            drop(stream);
            tokio::time::timeout(Duration::from_secs(15), producer)
                .await
                .expect("the producer must stop")
                .expect("producer must not panic");
            let client = pg.pool.get().await.expect("pooled connection");
            let row = client
                .query_one("SELECT pg_backend_pid(), pg_sleep(0.05)::text", &[])
                .await
                .unwrap_or_else(|e| panic!("round {round}: next borrower's statement failed: {e}"));
            let pid: i32 = row.get(0);
            reused += usize::from(previous_pid == Some(pid));
            previous_pid = Some(pid);
        }
        println!("[pg-cancel] connection reused in {reused} of 29 rounds");
    }
}
