//! PostgreSQL in-DB SQL-on-FHIR runner.
//!
//! [`PgInDbRunner`] compiles a ViewDefinition to a parameterised PostgreSQL
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are fetched lazily via `tokio_postgres::Client::query_raw` and sent
//! through a bounded `tokio::sync::mpsc` channel (buffer: 256) so the HTTP
//! layer can begin flushing before the full result set has been transferred.
//! The async fetch loop runs in a `tokio::spawn` task that holds the pooled
//! connection open until the consumer drops the receiver. That task's
//! `JoinHandle` is watched by [`watch_row_producer`](crate::core::sof_runner::watch_row_producer)
//! so a panic or cancellation reaches the consumer as an `Err` item instead
//! of a silent end of stream.

use std::sync::Arc;

use deadpool_postgres::Pool;
use futures::{FutureExt as _, StreamExt as _};
use helios_fhir::FhirVersion;
use helios_sof::columnar::{COLUMNAR_BATCH_ROWS, ColumnKind, ColumnarBatchBuilder};
use helios_sof::sqlquery::engine::{DeclaredColumn, declared_columns};
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, trace};

use helios_sof::arrow::record_batch::RecordBatch;

use crate::core::sof_runner::{
    BatchSender, BatchStream, RowBatchSender, RowBatcher, RowStream, SofError, SofRunner,
    ViewFilters, ViewRow, row_batch_channel, row_stream, watch_row_producer,
};
use crate::tenant::TenantContext;

use super::decode::{ColumnDecode, decode_text};

use super::compiler::SqlDialect;
use super::runtime::{RuntimeParam, prepare_sql_export_page, prepare_sql_run};

/// Channel buffer depth for the columnar path (batches, not rows).
const BATCH_CHANNEL_BUFFER: usize = 2;

/// Resources read per page by the export reader.
const EXPORT_PAGE_RESOURCES: usize = 10_000;

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
        "postgres-indb"
    }

    /// An export reads whole types, so its views are read in id-keyset pages:
    /// every statement is short and has its own statement timeout, where one
    /// `ORDER BY last_updated, id` over millions of rows would sort the whole
    /// type before its first row.
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
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        let tenant_id = tenant.tenant_id().to_string();
        let group_pool = self.pool.clone();
        let group_tenant = tenant_id.clone();
        let load_groups = move |refs: Vec<String>| async move {
            load_group_documents(&group_pool, &group_tenant, &refs).await
        };
        let prepared = match self.export_page_size {
            Some(_) => {
                prepare_sql_export_page(
                    &view_definition,
                    SqlDialect::Postgres,
                    self.fhir_version,
                    &tenant_id,
                    filters,
                    load_groups,
                )
                .await?
            }
            None => {
                prepare_sql_run(
                    &view_definition,
                    SqlDialect::Postgres,
                    self.fhir_version,
                    &tenant_id,
                    filters,
                    load_groups,
                )
                .await?
            }
        };
        let Some(prepared) = prepared else {
            return Ok(Box::pin(futures::stream::empty()));
        };
        let compiled = prepared.query;
        debug!(runner = "postgres-indb", tenant = %tenant_id, "executing compiled ViewDefinition");
        trace!(
            runner = "postgres-indb", sql = %compiled.sql, columns = ?compiled.columns,
            constants = compiled.constants.len(), "compiled ViewDefinition SQL"
        );
        let params = prepared
            .params
            .into_iter()
            .map(PgParam::from_runtime)
            .collect();
        let pool = self.pool.clone();
        let (tx, rx) = row_batch_channel();
        let guard_tx = tx.clone();
        let page_size = self.export_page_size;
        let producer = tokio::spawn(async move {
            match page_size {
                Some(page_size) => {
                    stream_pg_pages(
                        pool,
                        compiled.sql,
                        params,
                        compiled.columns,
                        compiled.column_decodes,
                        prepared.client_limit,
                        page_size,
                        tx,
                    )
                    .await;
                }
                None => {
                    stream_pg_rows(
                        pool,
                        compiled.sql,
                        params,
                        compiled.columns,
                        compiled.column_decodes,
                        prepared.client_limit,
                        tx,
                    )
                    .await;
                }
            }
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);
        Ok(row_stream(rx))
    }

    /// Columnar sibling of [`run_view`](Self::run_view): every column is text
    /// on the wire, so each cell is read as `&str` and appended straight to
    /// its Arrow builder. Columns whose type is only known per value are
    /// decoded to JSON as on the row path.
    ///
    /// Returns `Ok(None)` when two output columns share a name: the row path
    /// folds those into one JSON key, positional builders would keep both.
    async fn run_view_columnar(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<Option<BatchStream>, SofError> {
        let tenant_id = tenant.tenant_id().to_string();
        let group_pool = self.pool.clone();
        let group_tenant = tenant_id.clone();
        let Some(prepared) = prepare_sql_run(
            &view_definition,
            SqlDialect::Postgres,
            self.fhir_version,
            &tenant_id,
            filters,
            move |refs| async move { load_group_documents(&group_pool, &group_tenant, &refs).await },
        ).await? else {
            return Ok(Some(Box::pin(futures::stream::empty())));
        };
        let compiled = prepared.query;
        let mut seen = std::collections::HashSet::new();
        if !compiled.columns.iter().all(|c| seen.insert(c.as_str())) {
            return Ok(None);
        }
        let declared = declared_columns(&view_definition);
        debug!(runner = "postgres-indb", tenant = %tenant_id, "executing compiled ViewDefinition (columnar)");
        trace!(
            runner = "postgres-indb", sql = %compiled.sql, columns = ?compiled.columns,
            constants = compiled.constants.len(), "compiled ViewDefinition SQL"
        );
        let params = prepared
            .params
            .into_iter()
            .map(PgParam::from_runtime)
            .collect();
        let pool = self.pool.clone();
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<RecordBatch, SofError>>(BATCH_CHANNEL_BUFFER);
        let guard_tx = tx.clone();
        let producer = tokio::spawn(async move {
            stream_pg_batches(
                pool,
                compiled.sql,
                params,
                compiled.columns,
                compiled.column_decodes,
                declared,
                prepared.client_limit,
                tx,
            )
            .await;
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);
        Ok(Some(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Backend-only tenant-scoped Group document loading. Shared runtime
/// preparation interprets and merges the membership of these documents.
async fn load_group_documents(
    pool: &Pool,
    tenant_id: &str,
    group_refs: &[String],
) -> Result<Vec<Value>, SofError> {
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

    Ok(groups)
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
    fn from_runtime(value: RuntimeParam) -> Self {
        match value {
            RuntimeParam::Text(value) => Self::Text(value),
            RuntimeParam::Literal(value) => Self::from_lit(&value),
            RuntimeParam::TextList(value) => Self::TextArray(value),
            RuntimeParam::Timestamp(value) => Self::Timestamp(value),
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
// Async fetch loop
// ============================================================================

async fn stream_pg_rows(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    tx: RowBatchSender,
) {
    let mut batcher = RowBatcher::new(tx);
    match stream_pg_rows_inner(pool, sql, params, columns, decodes, limit, &mut batcher).await {
        Ok(()) => batcher.finish().await,
        Err(e) => batcher.error(e).await,
    }
}

async fn stream_pg_rows_inner(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    batcher: &mut RowBatcher,
) -> Result<(), SofError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to acquire Postgres connection: {e}")))?;

    if std::env::var("PG_SOF_DEBUG_ALL").is_ok() {
        eprintln!("[PG_SOF_DEBUG_ALL] preparing\n--- SQL ---\n{sql}\n---");
    }
    let stmt = client.prepare(&sql).await.map_err(|e| {
        if std::env::var("PG_SOF_DEBUG").is_ok() {
            eprintln!("[PG_SOF_DEBUG] prepare failed: {e}\n--- SQL ---\n{sql}\n---");
        }
        SofError::Backend(format!("failed to prepare SQL: {e}"))
    })?;

    // Build boxed params for query_raw; these are 'static + Send
    let boxed = box_params(params);

    // query_raw needs a slice of &dyn ToSql + Sync. Build references that borrow
    // from `boxed` — both live in this async block's stack frame, so no lifetime
    // issue (the future holds them until the stream is exhausted).
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let raw = client
        .query_raw(&stmt, param_refs.iter().copied())
        .await
        .map_err(|e| {
            if std::env::var("PG_SOF_DEBUG").is_ok() {
                eprintln!("[PG_SOF_DEBUG] query failed: {e}\n--- SQL ---\n{sql}\n---");
            }
            SofError::Backend(format!("query execution failed: {e}"))
        })?;

    // params no longer needed after query_raw returns (data sent to DB)
    drop(param_refs);
    drop(boxed);

    futures::pin_mut!(raw);

    let mut count = 0usize;
    loop {
        // Deliver the pending rows before waiting on the database, so a
        // slow or selective query still shows its first rows promptly.
        let next = match raw.next().now_or_never() {
            Some(next) => next,
            None => {
                if !batcher.flush().await {
                    break; // receiver dropped
                }
                raw.next().await
            }
        };
        let Some(row_result) = next else { break };
        match row_result {
            Ok(pg_row) => {
                if let Some(cap) = limit {
                    if count >= cap {
                        break;
                    }
                }
                count += 1;
                match row_to_json(&pg_row, &columns, &decodes) {
                    Ok(row) => {
                        if !batcher.push(row).await {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        batcher.error(e).await;
                        break;
                    }
                }
            }
            Err(e) => {
                if std::env::var("PG_SOF_DEBUG").is_ok() {
                    eprintln!("[PG_SOF_DEBUG] row error: {e}\n--- SQL ---\n{sql}\n---");
                }
                batcher
                    .error(SofError::Backend(format!(
                        "row error: {}",
                        pg_row_error_text(&e, &columns, &decodes)
                    )))
                    .await;
                break;
            }
        }
    }

    debug!(
        runner = "postgres-indb",
        rows = count,
        "in-DB view run complete"
    );
    Ok(())
    // tx dropped here, closing the ReceiverStream
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

/// [`pg_error_text`] for a failure while reading rows. The declared column
/// type is applied by a SQL cast, so a declared `integer` / `decimal` column
/// over a value that is not a number fails the whole query with a message that
/// quotes the value but not the column (`invalid input syntax for type
/// bigint: "Garcia"`). PostgreSQL does not say which expression failed, so for
/// those data-exception errors the declared numeric columns are listed.
fn pg_row_error_text(
    e: &tokio_postgres::Error,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> String {
    let text = pg_error_text(e);
    let is_cast_failure = e.as_db_error().is_some_and(|db| {
        matches!(
            db.code().code(),
            "22P02" /* invalid_text_representation */ | "22003" /* numeric_value_out_of_range */
        )
    });
    if !is_cast_failure {
        return text;
    }
    let numeric: Vec<&str> = columns
        .iter()
        .zip(decodes)
        .filter(|(_, d)| matches!(d, ColumnDecode::Integer | ColumnDecode::Decimal))
        .map(|(c, _)| c.as_str())
        .collect();
    if numeric.is_empty() {
        text
    } else {
        format!(
            "{text} (a declared integer/decimal column received a value that is not a number or is out of range; declared numeric columns: {})",
            numeric.join(", ")
        )
    }
}

// ============================================================================
// Keyset-paged fetch loop (export)
// ============================================================================

/// The id that closes the next page: the last of the next `$4` live ids of the
/// tenant's type after `$3`, in the order `idx_resources_reindex_id` stores
/// them. No row means the type is exhausted. The scan stays on the index (no
/// heap access, no sort), so it costs one page of index entries.
fn export_bound_sql() -> &'static str {
    "SELECT id FROM (\n  SELECT id FROM resources\n  WHERE tenant_id = $1 AND resource_type = $2\n    AND is_deleted = FALSE AND id COLLATE \"C\" > $3\n  ORDER BY id COLLATE \"C\"\n  LIMIT $4\n) page\nORDER BY id COLLATE \"C\" DESC\nLIMIT 1"
}

#[allow(clippy::too_many_arguments)]
async fn stream_pg_pages(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    page_size: usize,
    tx: RowBatchSender,
) {
    let mut batcher = RowBatcher::new(tx);
    match stream_pg_pages_inner(
        pool,
        sql,
        params,
        columns,
        decodes,
        limit,
        page_size,
        &mut batcher,
    )
    .await
    {
        Ok(()) => batcher.finish().await,
        Err(e) => batcher.error(e).await,
    }
}

/// Reads the view page by page. A page is `(after, through]` in
/// `id COLLATE "C"` order, whole resources only, so a `forEach` never splits a
/// resource across pages. Each page is collected before it is sent: the
/// statement ends before the consumer's back-pressure can hold it open
/// against the statement timeout.
#[allow(clippy::too_many_arguments)]
async fn stream_pg_pages_inner(
    pool: Pool,
    sql: String,
    mut params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    page_size: usize,
    batcher: &mut RowBatcher,
) -> Result<(), SofError> {
    // `$1`/`$2` are tenant and resource type; the range slots are the last two.
    let (Some(PgParam::Text(tenant)), Some(PgParam::Text(resource_type))) =
        (params.first().cloned(), params.get(1).cloned())
    else {
        return Err(SofError::Backend(
            "export page has no tenant binding".into(),
        ));
    };
    let range = params
        .len()
        .checked_sub(2)
        .filter(|i| *i >= 2)
        .ok_or_else(|| SofError::Backend("export page has no id range binding".into()))?;
    if limit == Some(0) {
        return Ok(());
    }

    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to acquire Postgres connection: {e}")))?;
    let bound_stmt = client
        .prepare(export_bound_sql())
        .await
        .map_err(|e| SofError::Backend(format!("failed to prepare SQL: {}", pg_error_text(&e))))?;
    let page_stmt = client
        .prepare(&sql)
        .await
        .map_err(|e| SofError::Backend(format!("failed to prepare SQL: {}", pg_error_text(&e))))?;
    let page_limit = i64::try_from(page_size).unwrap_or(i64::MAX);

    let mut after = String::new();
    let mut count = 0usize;
    let mut pages = 0usize;
    loop {
        if batcher.is_closed() {
            return Ok(());
        }
        let bound_params: [&(dyn tokio_postgres::types::ToSql + Sync); 4] =
            [&tenant, &resource_type, &after, &page_limit];
        let bound = tokio::select! {
            r = client.query_opt(&bound_stmt, &bound_params) => r,
            _ = batcher.closed() => return Ok(()),
        }
        .map_err(|e| SofError::Backend(format!("page bound error: {}", pg_error_text(&e))))?;
        let Some(bound) = bound else { break };
        let through: String = bound
            .try_get(0)
            .map_err(|e| SofError::Backend(format!("failed to read page bound: {e}")))?;

        params[range] = PgParam::Text(after);
        params[range + 1] = PgParam::Text(through.clone());
        let boxed = box_params(params.clone());
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
            .iter()
            .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let rows = tokio::select! {
            r = client.query(&page_stmt, &param_refs) => r,
            _ = batcher.closed() => return Ok(()),
        }
        .map_err(|e| {
            if std::env::var("PG_SOF_DEBUG").is_ok() {
                eprintln!("[PG_SOF_DEBUG] page error: {e}\n--- SQL ---\n{sql}\n---");
            }
            SofError::Backend(format!(
                "row error: {}",
                pg_row_error_text(&e, &columns, &decodes)
            ))
        })?;
        drop(param_refs);
        drop(boxed);

        pages += 1;
        for pg_row in &rows {
            if limit.is_some_and(|cap| count >= cap) {
                return Ok(());
            }
            count += 1;
            let row = row_to_json(pg_row, &columns, &decodes)?;
            if !batcher.push(row).await {
                return Ok(()); // receiver dropped
            }
        }
        // The next page starts a new statement; do not hold these rows for it.
        if !batcher.flush().await {
            return Ok(());
        }
        if limit.is_some_and(|cap| count >= cap) {
            break;
        }
        after = through;
    }

    debug!(
        runner = "postgres-indb",
        rows = count,
        pages,
        "in-DB paged view run complete"
    );
    Ok(())
}

// ============================================================================
// Columnar fetch loop
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn stream_pg_batches(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    declared: Vec<DeclaredColumn>,
    limit: Option<usize>,
    tx: BatchSender,
) {
    if let Err(e) =
        stream_pg_batches_inner(pool, sql, params, columns, decodes, &declared, limit, &tx).await
    {
        let _ = tx.send(Err(e)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_pg_batches_inner(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
    declared: &[DeclaredColumn],
    limit: Option<usize>,
    tx: &BatchSender,
) -> Result<(), SofError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to acquire Postgres connection: {e}")))?;
    let stmt = client
        .prepare(&sql)
        .await
        .map_err(|e| SofError::Backend(format!("failed to prepare SQL: {e}")))?;
    let boxed = box_params(params);
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let raw = client
        .query_raw(&stmt, param_refs.iter().copied())
        .await
        .map_err(|e| SofError::Backend(format!("query execution failed: {e}")))?;
    drop(param_refs);
    drop(boxed);
    futures::pin_mut!(raw);

    let kinds = (0..columns.len())
        .map(|i| decodes.get(i).copied().unwrap_or_default().column_kind())
        .collect();
    let mut builder = ColumnarBatchBuilder::new(columns.clone(), kinds, COLUMNAR_BATCH_ROWS)
        .map_err(|e| SofError::Backend(format!("columnar builder: {e}")))?;
    builder
        .pin_declared_types(declared)
        .map_err(|e| SofError::Backend(format!("columnar builder: {e}")))?;

    let mut count = 0usize;
    // False once the stream ended on an error or a closed receiver: the
    // partial batch is then dropped instead of flushed.
    let mut completed = true;
    while let Some(row_result) = raw.next().await {
        let pg_row = match row_result {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(Err(SofError::Backend(format!(
                        "row error: {}",
                        pg_row_error_text(&e, &columns, &decodes)
                    ))))
                    .await;
                completed = false;
                break;
            }
        };
        if let Some(cap) = limit {
            if count >= cap {
                break;
            }
        }
        count += 1;
        if let Err(e) = append_pg_row(&mut builder, &pg_row, &columns, &decodes) {
            let _ = tx.send(Err(e)).await;
            completed = false;
            break;
        }
        if builder.end_row() {
            match builder.flush() {
                Ok(Some(batch)) => {
                    if tx.send(Ok(batch)).await.is_err() {
                        completed = false;
                        break; // receiver dropped
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = tx
                        .send(Err(SofError::Backend(format!("columnar flush: {e}"))))
                        .await;
                    completed = false;
                    break;
                }
            }
        }
    }

    if completed {
        match builder.flush() {
            Ok(Some(batch)) => {
                let _ = tx.send(Ok(batch)).await;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = tx
                    .send(Err(SofError::Backend(format!("columnar flush: {e}"))))
                    .await;
            }
        }
    }
    debug!(
        runner = "postgres-indb",
        rows = count,
        "in-DB columnar view run complete"
    );
    Ok(())
}

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
            .map_err(|e| SofError::Backend(format!("failed to read column '{name}': {e}")))?;
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
    use super::super::runtime::SqlRunPlan;
    use super::*;
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
}
