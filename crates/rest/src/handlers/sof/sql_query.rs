//! `$sql-query-run` operation handler.
//!
//! Executes a raw SQL `SELECT` query against the FHIR resource store.
//!
//! ## Security model
//!
//! - Only `SELECT` statements (optionally prefixed with CTEs) are accepted.
//!   Any other SQL (DDL, DML, stored-procedure calls) returns `400`.
//! - The query is wrapped in a tenant-boundary CTE before execution so the
//!   caller can only see rows belonging to their tenant.
//! - Execution happens over a **read-only** connection configured via
//!   `HFS_SOF_READONLY_URL`.
//! - A row cap (`HFS_SOF_SQL_QUERY_MAX_ROWS`) and a timeout
//!   (`HFS_SOF_SQL_QUERY_TIMEOUT_SECS`) are enforced server-side.
//!
//! ## Enabling the endpoint
//!
//! The endpoint is disabled by default. Set `HFS_SOF_SQL_QUERY_ENABLED=true`
//! **and** provide `HFS_SOF_READONLY_URL` at startup to activate it.  When
//! disabled, `POST /$sql-query-run` returns `501 Not Implemented`.

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::raw_sql::RawSqlError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Query parameters for `$sql-query-run`.
#[derive(Debug, Deserialize)]
pub struct SqlQueryParams {
    /// Output format: `ndjson` (default) or `csv`.
    #[serde(rename = "_format")]
    pub format: Option<String>,
}

// ============================================================================
// Handler
// ============================================================================

/// `POST /$sql-query-run`
///
/// Accepts a FHIR `Parameters` body with a `query` parameter containing the
/// SQL `SELECT` statement to execute.
///
/// ```text
/// {
///   "resourceType": "Parameters",
///   "parameter": [
///     { "name": "query", "valueString": "SELECT id FROM resources WHERE resource_type = 'Patient' LIMIT 10" }
///   ]
/// }
/// ```
///
/// Returns the result as NDJSON (one row per line) or CSV.
pub async fn sql_query_run_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Query(params): Query<SqlQueryParams>,
    body: axum::body::Bytes,
) -> Response
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let config = state.config();

    // 1. Feature gate ─ disabled by default.
    if !config.sof_sql_query_enabled {
        return not_implemented("$sql-query-run is disabled; set HFS_SOF_SQL_QUERY_ENABLED=true");
    }

    // 2. Runner must be configured (implicitly checks backend capability:
    //    the runner is only wired at startup when HFS_SOF_READONLY_URL is
    //    provided for a backend that supports raw SQL queries).
    let runner = match state.raw_sql_runner() {
        Some(r) => r.clone(),
        None => {
            return not_implemented(
                "$sql-query-run has no read-only runner; set HFS_SOF_READONLY_URL",
            );
        }
    };

    // 3. Parse the FHIR Parameters body.
    let sql = match extract_query_string(&body) {
        Ok(s) => s,
        Err(msg) => return bad_request(&msg),
    };

    // 4. Validate: only SELECT / CTE allowed.
    if let Err(msg) = validate_select_only(&sql) {
        return bad_request(&msg);
    }

    // 5. Execute via the read-only runner.
    let rows = match runner
        .run_query(
            tenant.context().tenant_id().as_str(),
            &sql,
            config.sof_sql_query_max_rows,
            config.sof_sql_query_timeout_secs,
        )
        .await
    {
        Ok(r) => r,
        Err(RawSqlError::Timeout { secs }) => {
            return operation_outcome(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                &format!("Query exceeded {secs}s timeout"),
            );
        }
        Err(RawSqlError::RowLimitExceeded { max_rows }) => {
            return operation_outcome(
                StatusCode::UNPROCESSABLE_ENTITY,
                "too-costly",
                &format!("Result exceeds {max_rows}-row limit; add a WHERE or LIMIT clause"),
            );
        }
        Err(e) => {
            return operation_outcome(
                StatusCode::INTERNAL_SERVER_ERROR,
                "exception",
                &e.to_string(),
            );
        }
    };

    // 6. Serialise.
    let format = params.format.as_deref().unwrap_or("ndjson").to_lowercase();

    match format.as_str() {
        "csv" => format_csv(&rows),
        _ => format_ndjson(&rows),
    }
}

// ============================================================================
// SQL validation
// ============================================================================

/// Returns `Ok(())` if `sql` is a single `SELECT`/`VALUES`/CTE statement,
/// otherwise an error message suitable for returning to the caller.
fn validate_select_only(sql: &str) -> Result<(), String> {
    use sqlparser::ast::Statement;
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser;

    let dialect = GenericDialect {};
    let stmts = Parser::parse_sql(&dialect, sql).map_err(|e| format!("SQL parse error: {e}"))?;

    if stmts.len() != 1 {
        return Err(format!(
            "exactly one statement is required, got {}",
            stmts.len()
        ));
    }

    match &stmts[0] {
        Statement::Query(_) => Ok(()),
        other => {
            let keyword = other
                .to_string()
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_uppercase();
            Err(format!(
                "only SELECT queries are allowed; {keyword} statements are not permitted"
            ))
        }
    }
}

// ============================================================================
// Parameter extraction
// ============================================================================

fn extract_query_string(body: &[u8]) -> Result<String, String> {
    if body.is_empty() {
        return Err("request body is empty; expected a FHIR Parameters resource".to_string());
    }

    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))?;

    // Accept both Parameters resource and bare {"query": "..."}
    let query = if value.get("resourceType").and_then(|v| v.as_str()) == Some("Parameters") {
        value
            .get("parameter")
            .and_then(|p| p.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|entry| entry.get("name").and_then(|n| n.as_str()) == Some("query"))
            })
            .and_then(|entry| entry.get("valueString").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    } else {
        value
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    query.ok_or_else(|| {
        "missing 'query' parameter; provide a Parameters resource with name='query'".to_string()
    })
}

// ============================================================================
// Output formatters
// ============================================================================

fn format_ndjson(rows: &[Value]) -> Response {
    let mut buf = Vec::new();
    for row in rows {
        if let Ok(line) = serde_json::to_vec(row) {
            buf.extend_from_slice(&line);
            buf.push(b'\n');
        }
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        buf,
    )
        .into_response()
}

fn format_csv(rows: &[Value]) -> Response {
    if rows.is_empty() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/csv")],
            Vec::<u8>::new(),
        )
            .into_response();
    }

    let cols: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let mut buf = Vec::new();
    buf.extend_from_slice(cols.join(",").as_bytes());
    buf.push(b'\n');

    for row in rows {
        if let Some(obj) = row.as_object() {
            let values: Vec<String> = cols
                .iter()
                .map(|c| csv_cell(obj.get(c).unwrap_or(&Value::Null)))
                .collect();
            buf.extend_from_slice(values.join(",").as_bytes());
            buf.push(b'\n');
        }
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv")], buf).into_response()
}

fn csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        other => {
            let s = other.to_string();
            format!("\"{}\"", s.replace('"', "\"\""))
        }
    }
}

// ============================================================================
// Error helpers
// ============================================================================

fn not_implemented(detail: &str) -> Response {
    operation_outcome(StatusCode::NOT_IMPLEMENTED, "not-supported", detail)
}

fn bad_request(detail: &str) -> Response {
    operation_outcome(StatusCode::BAD_REQUEST, "invalid", detail)
}

fn operation_outcome(status: StatusCode, issue_code: &str, detail: &str) -> Response {
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": issue_code,
            "diagnostics": detail
        }]
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/fhir+json")],
        serde_json::to_vec(&body).unwrap_or_default(),
    )
        .into_response()
}
