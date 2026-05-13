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
//! disabled, `POST /$sqlquery-run` returns `501 Not Implemented`.

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::raw_sql::{BoundValue, RawSqlError};
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

/// `POST /$sqlquery-run`
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

    // 3. Parse the FHIR Parameters body — extract SQL plus any named params.
    let (sql, named_params) = match extract_query_and_params(&body) {
        Ok(p) => p,
        Err(msg) => return bad_request(&msg),
    };

    // 4. Validate: only SELECT / CTE allowed.
    if let Err(msg) = validate_select_only(&sql) {
        return bad_request(&msg);
    }

    // 5. Execute via the read-only runner. Parameters are bound by the
    //    driver, never interpolated (spec MUST).
    let rows = match runner
        .run_query(
            tenant.context().tenant_id().as_str(),
            &sql,
            &named_params,
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
        Err(RawSqlError::Parameter(msg)) => {
            return bad_request(&msg);
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
        "json" => format_json_array(&rows),
        "fhir" => format_fhir_parameters(&rows),
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

/// Extracts the SQL string plus any `:name` bound-parameter values from a
/// FHIR `Parameters` body. Also accepts a bare `{"query": "..."}` shape for
/// convenience (no named params).
fn extract_query_and_params(body: &[u8]) -> Result<(String, Vec<(String, BoundValue)>), String> {
    if body.is_empty() {
        return Err("request body is empty; expected a FHIR Parameters resource".to_string());
    }

    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))?;

    if value.get("resourceType").and_then(|v| v.as_str()) == Some("Parameters") {
        let entries = value
            .get("parameter")
            .and_then(|p| p.as_array())
            .ok_or_else(|| "Parameters.parameter must be an array".to_string())?;

        // 1. Find the `query` parameter.
        let sql = entries
            .iter()
            .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("query"))
            .and_then(|e| e.get("valueString").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| {
                "missing 'query' parameter; provide a Parameters resource with name='query'"
                    .to_string()
            })?;

        // 2. Find the nested `parameters` Parameters resource, if present, and
        //    pull every {name, value[x]} pair out of its inner `parameter[]`.
        let mut named: Vec<(String, BoundValue)> = Vec::new();
        if let Some(inner) = entries
            .iter()
            .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("parameters"))
            .and_then(|e| e.get("resource"))
        {
            if inner.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
                return Err("'parameters' must wrap a Parameters resource".to_string());
            }
            let inner_params = inner.get("parameter").and_then(|p| p.as_array());
            if let Some(arr) = inner_params {
                for p in arr {
                    let name = p
                        .get("name")
                        .and_then(|n| n.as_str())
                        .ok_or_else(|| "parameter entry missing 'name'".to_string())?
                        .to_string();
                    let value = bound_value_from_parameter(p)?;
                    named.push((name, value));
                }
            }
        }

        Ok((sql, named))
    } else {
        // Bare shape — no named params.
        let sql = value
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                "missing 'query' parameter; provide a Parameters resource with name='query'"
                    .to_string()
            })?;
        Ok((sql, Vec::new()))
    }
}

/// Reads a single `Parameters.parameter[]` entry and returns the corresponding
/// [`BoundValue`]. Returns `Err` if the entry uses a complex value type that
/// can't safely be bound (e.g. `valueReference`, `valueCoding`).
fn bound_value_from_parameter(p: &Value) -> Result<BoundValue, String> {
    let obj = p
        .as_object()
        .ok_or_else(|| "parameter entry must be an object".to_string())?;

    for (key, value) in obj {
        if !key.starts_with("value") {
            continue;
        }
        return match key.as_str() {
            "valueBoolean" => value
                .as_bool()
                .map(BoundValue::Bool)
                .ok_or_else(|| "valueBoolean must be a JSON boolean".to_string()),
            "valueInteger" | "valuePositiveInt" | "valueUnsignedInt" => value
                .as_i64()
                .map(BoundValue::Int)
                .ok_or_else(|| format!("{key} must be a JSON integer")),
            "valueDecimal" => value
                .as_f64()
                .map(BoundValue::Decimal)
                .or_else(|| value.as_i64().map(|i| BoundValue::Decimal(i as f64)))
                .ok_or_else(|| "valueDecimal must be a JSON number".to_string()),
            "valueString" | "valueCode" | "valueId" | "valueUri" | "valueOid"
            | "valueCanonical" | "valueUrl" | "valueMarkdown" => value
                .as_str()
                .map(|s| BoundValue::Text(s.to_string()))
                .ok_or_else(|| format!("{key} must be a JSON string")),
            "valueDate" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| "valueDate must be a JSON string".to_string())?;
                let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .map_err(|e| format!("invalid valueDate '{s}': {e}"))?;
                Ok(BoundValue::Date(d))
            }
            "valueDateTime" | "valueInstant" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("{key} must be a JSON string"))?;
                let dt = chrono::DateTime::parse_from_rfc3339(s)
                    .map_err(|e| format!("invalid {key} '{s}': {e}"))?;
                Ok(BoundValue::DateTime(dt.with_timezone(&chrono::Utc)))
            }
            other => Err(format!(
                "parameter type '{other}' is not supported for SQL binding"
            )),
        };
    }
    // No value[x] field at all — treat as SQL NULL.
    Ok(BoundValue::Null)
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

/// Serialises the result set as a single JSON array (`_format=json`).
fn format_json_array(rows: &[Value]) -> Response {
    let body = serde_json::to_vec(rows).unwrap_or_else(|_| b"[]".to_vec());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// Serialises the result set as a FHIR `Parameters` resource per the SoF v2
/// spec's SQL-type-to-FHIR-value mapping (`_format=fhir`).
///
/// Each row becomes a top-level `parameter` named `row` with one `part` per
/// non-NULL column. Column types are inferred from the JSON values we already
/// have. NULL values are represented by omitting the corresponding `part`,
/// per the spec.
fn format_fhir_parameters(rows: &[Value]) -> Response {
    let mut parts: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let cols = match row.as_object() {
            Some(o) => o,
            None => continue,
        };
        let mut row_parts: Vec<Value> = Vec::with_capacity(cols.len());
        for (name, value) in cols {
            let part = match value_to_fhir_part(name, value) {
                Ok(opt) => opt,
                Err(msg) => {
                    return operation_outcome(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "not-supported",
                        &msg,
                    );
                }
            };
            if let Some(p) = part {
                row_parts.push(p);
            }
        }
        parts.push(json!({"name": "row", "part": row_parts}));
    }

    let body = json!({
        "resourceType": "Parameters",
        "parameter": parts,
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/fhir+json")],
        serde_json::to_vec(&body).unwrap_or_default(),
    )
        .into_response()
}

/// Maps a single column value to a FHIR `Parameters.parameter.part` entry.
/// Returns `Ok(None)` for SQL NULL (the spec says omit the part).
/// Returns `Err` for unsupported types (caller must respond 422).
fn value_to_fhir_part(name: &str, v: &Value) -> Result<Option<Value>, String> {
    match v {
        // NULL: omit the part entirely.
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(json!({"name": name, "valueBoolean": b}))),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Some(json!({"name": name, "valueInteger": i})))
            } else if let Some(f) = n.as_f64() {
                Ok(Some(json!({"name": name, "valueDecimal": f})))
            } else {
                Err(format!("column '{name}' has a numeric value out of range"))
            }
        }
        Value::String(s) => {
            // RFC 3339 timestamps → valueInstant (rounded to ms by chrono::DateTime).
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                let ms = dt
                    .with_timezone(&chrono::Utc)
                    .format("%Y-%m-%dT%H:%M:%S%.3fZ");
                Ok(Some(json!({"name": name, "valueInstant": ms.to_string()})))
            } else if chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
                Ok(Some(json!({"name": name, "valueDate": s})))
            } else {
                Ok(Some(json!({"name": name, "valueString": s})))
            }
        }
        Value::Object(_) | Value::Array(_) => Err(format!(
            "column '{name}' has a composite SQL type which is not representable as a FHIR scalar value; \
             _format=fhir is not supported for this result"
        )),
    }
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
