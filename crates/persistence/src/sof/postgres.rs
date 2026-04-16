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
//! connection open until the consumer drops the receiver.

use deadpool_postgres::Pool;
use futures::StreamExt as _;
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, compile_view_definition_dialect};

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// SQL-on-FHIR runner that compiles ViewDefinitions to PostgreSQL SQL.
pub struct PgInDbRunner {
    pool: Pool,
}

impl PgInDbRunner {
    /// Creates a new runner backed by the given connection pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl SofRunner for PgInDbRunner {
    fn runner_name(&self) -> &'static str {
        "postgres-indb"
    }

    async fn run_view<'a>(
        &'a self,
        tenant: &'a TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream<'a>, SofError> {
        // Compile synchronously (cheap, no I/O)
        let compiled = compile_view_definition_dialect(&view_definition, SqlDialect::Postgres)?;

        debug!(
            runner = "postgres-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );

        let tenant_id = tenant.tenant_id().to_string();
        let resource_type = view_definition
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let limit = filters.limit;
        let columns = compiled.columns.clone();
        let pool = self.pool.clone();

        // Build SQL with runtime filters and collect typed params
        let (sql, params) =
            build_pg_sql_and_params(&compiled.sql, tenant_id, resource_type, &filters);

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);

        tokio::spawn(async move {
            stream_pg_rows(pool, sql, params, columns, limit, tx).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

// ============================================================================
// SQL runtime-filter injection
// ============================================================================

/// Builds the final SQL and typed params list for a PG query.
///
/// The base SQL uses `$1 = tenant_id` and `$2 = resource_type`.
/// Extra filter conditions inject `$3`, `$4`, … as needed.
fn build_pg_sql_and_params(
    base_sql: &str,
    tenant_id: String,
    resource_type: String,
    filters: &ViewFilters,
) -> (String, Vec<PgParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut extra: Vec<PgParam> = Vec::new();
    let mut next_param = 3usize;

    if let Some(since) = filters.since {
        conditions.push(format!("r.last_updated >= ${next_param}"));
        extra.push(PgParam::Timestamp(since));
        next_param += 1;
    }

    if let Some(patient) = &filters.patient {
        let p = next_param;
        // PostgreSQL JSONB path: '{subject,reference}' → r.data#>>'subject.reference'
        conditions.push(format!(
            "(r.data#>>'{{subject,reference}}' = ${p} \
             OR r.data#>>'{{patient,reference}}' = ${p})"
        ));
        extra.push(PgParam::Text(patient.clone()));
        next_param += 1;
    }

    if let Some(group) = &filters.group {
        let p = next_param;
        conditions.push(format!("r.data#>>'{{group,reference}}' = ${p}"));
        extra.push(PgParam::Text(group.clone()));
    }

    let sql = if conditions.is_empty() {
        base_sql.to_string()
    } else {
        let joined = conditions.join(" AND ");
        inject_before_order_by(base_sql, &format!(" AND {joined}"))
    };

    let mut all_params = vec![PgParam::Text(tenant_id), PgParam::Text(resource_type)];
    all_params.extend(extra);

    (sql, all_params)
}

/// Inserts `extra` before the trailing `ORDER BY` in `sql`, or appends it.
///
/// The compiler emits `\nORDER BY …` (newline-prefixed), so we search for
/// that pattern first; the space-prefixed variant is a fallback for hand-crafted SQL.
fn inject_before_order_by(sql: &str, extra: &str) -> String {
    let search = ["\nORDER BY", " ORDER BY"];
    for pat in search {
        if let Some(pos) = sql.rfind(pat) {
            let mut s = sql.to_string();
            s.insert_str(pos, extra);
            return s;
        }
    }
    format!("{sql}{extra}")
}

// ============================================================================
// Typed parameter enum — avoids the self-referential borrow issues with
// `Vec<Box<dyn ToSql>>` + `Vec<&dyn ToSql>` that arise in async tasks.
// ============================================================================

#[derive(Clone)]
enum PgParam {
    Text(String),
    Timestamp(chrono::DateTime<chrono::Utc>),
}

// ============================================================================
// Async fetch loop
// ============================================================================

async fn stream_pg_rows(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    limit: Option<usize>,
    tx: tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) {
    if let Err(e) = stream_pg_rows_inner(pool, sql, params, columns, limit, &tx).await {
        let _ = tx.send(Err(e)).await;
    }
}

async fn stream_pg_rows_inner(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    limit: Option<usize>,
    tx: &tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) -> Result<(), SofError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to acquire Postgres connection: {e}")))?;

    let stmt = client
        .prepare(&sql)
        .await
        .map_err(|e| SofError::Backend(format!("failed to prepare SQL: {e}")))?;

    // Build boxed params for query_raw; these are 'static + Send
    let boxed: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = params
        .into_iter()
        .map(|p| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
            match p {
                PgParam::Text(s) => Box::new(s),
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

    let raw = client
        .query_raw(&stmt, param_refs.iter().copied())
        .await
        .map_err(|e| SofError::Backend(format!("query execution failed: {e}")))?;

    // params no longer needed after query_raw returns (data sent to DB)
    drop(param_refs);
    drop(boxed);

    futures::pin_mut!(raw);

    let mut count = 0usize;
    while let Some(row_result) = raw.next().await {
        match row_result {
            Ok(pg_row) => {
                if let Some(cap) = limit {
                    if count >= cap {
                        break;
                    }
                }
                count += 1;
                match row_to_json(&pg_row, &columns) {
                    Ok(row) => {
                        if tx.send(Ok(row)).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(Err(SofError::Backend(format!("row error: {e}"))))
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

// ============================================================================
// Row → JSON conversion
// ============================================================================

/// Converts a `tokio_postgres::Row` into a `serde_json::Value` object.
///
/// The compiled SQL projects all columns as text via `->>`/`#>>` operators.
fn row_to_json(pg_row: &tokio_postgres::Row, columns: &[String]) -> Result<ViewRow, SofError> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val: Option<String> = pg_row
            .try_get(i)
            .map_err(|e| SofError::Backend(format!("failed to read column '{name}': {e}")))?;

        if let Some(s) = val {
            let json_val = serde_json::from_str(&s).unwrap_or(Value::String(s));
            map.insert(name.clone(), json_val);
        }
    }
    Ok(Value::Object(map))
}
