//! `$sqlquery-run` operation handler (SQL-on-FHIR v2).
//!
//! Implements three routes per the spec:
//! - `POST /$sqlquery-run` (system)
//! - `POST /Library/$sqlquery-run` (type)
//! - `POST /Library/{id}/$sqlquery-run` (instance — `{id}` binds the Library)
//!
//! Execution model: the SQLQuery Library declares one or more `relatedArtifact`
//! ViewDefinitions (`type=depends-on`, with a `label`). For each, this handler
//! calls the wired `SofRunner` to produce a row stream, materializes the rows
//! into a per-request in-memory SQLite database (one table per `label`), binds
//! the supplied `Library.parameter` values to the SQL, runs the user's query,
//! truncates the result to a caller-supplied `_limit` (if any), and serializes
//! the result in the requested `_format`.
//!
//! ## Output shape for flat formats
//!
//! The spec declares the operation's `return` parameter as `Binary | Parameters`
//! (1..1). By default, flat formats (csv/json/ndjson/parquet) are returned as
//! raw payload bytes with the format's `Content-Type` — matching `$viewdefinition-run`
//! and every other SoF reference implementation. Callers that want a strictly
//! spec-shaped response can ask for the `Binary` wrapper by setting
//! `Accept: application/fhir+json`; in that case the bytes are base64-encoded
//! into `Binary.data` and the response is a FHIR `Binary` resource.
//! `_format=fhir` always returns a `Parameters` resource as specified.
//!
//! ## Type fidelity under `_format=fhir`
//!
//! The spec defines a SQL-type → FHIR-`value[X]` mapping (e.g. DATE →
//! `valueDate`, TIMESTAMP → `valueDateTime`). The engine is in-memory
//! SQLite, which has only five storage classes (INTEGER, REAL, TEXT, BLOB,
//! NULL) and no native DATE/TIMESTAMP types. We recover the spec-mandated
//! FHIR types by looking up each output column name against the
//! depends-on ViewDefinitions' `select.column.type` declarations and
//! preferring that type when one is found.
//!
//! For output columns produced by SQL expressions that do **not** match a
//! VD-declared column (e.g. `SELECT date('now') AS today FROM t` or
//! `SELECT COUNT(*) AS n FROM t`), the column type falls back to whatever
//! SQLite's storage class implies: INTEGER → `valueInteger`, REAL →
//! `valueDecimal`, TEXT → `valueString`, BLOB → `valueBase64Binary`. The
//! spec's `date()` → `valueDate` / `datetime()` → `valueDateTime`
//! mappings are therefore not preserved for synthesised string columns;
//! they surface as `valueString`. Authors who need precise FHIR typing on
//! computed columns should declare the column in a depends-on
//! ViewDefinition projection, or accept the `valueString` fallback.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use futures::Stream;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::tenant::TenantContext;
use helios_sof::sqlquery::SqlQueryError;
use helios_sof::{
    ColumnFhirType, ContentType, InMemorySqlEngine, QueryResult, TableSchema, bind_supplied_params,
    extract_sqlquery_params_from_json, format_fhir_parameters, parse_sqlquery_library,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tracing::warn;

use super::references::resolve_resource_canonical_or_relative;
use crate::error::RestError;
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Query-string parameters accepted by `$sqlquery-run`. The spec ships every
/// `in` parameter on the operation; we only honor the ones that make sense in
/// a URL: `_format`, `header`, and `_limit`. Everything else (Library,
/// parameters, source) is body-only.
#[derive(Debug, Default, Deserialize)]
pub struct SqlQueryRunQuery {
    /// `_format` URL fallback when the body omits it. Body wins on conflict.
    #[serde(rename = "_format")]
    pub format: Option<String>,
    /// CSV `header` toggle from the URL. Body wins on conflict. Anything that
    /// isn't `true`/`false`/`1`/`0` is treated as unspecified.
    pub header: Option<String>,
    /// `_limit` URL fallback when the body omits it. Body wins on conflict.
    /// Per SoF v2 PR #353 this is a soft cap on the final result set; rows
    /// past the limit are dropped silently (not an error).
    #[serde(rename = "_limit")]
    pub limit: Option<u32>,
}

/// `POST /$sqlquery-run` and `POST /Library/$sqlquery-run`.
pub async fn sqlquery_run_handler<S>(
    State(state): State<AppState<S>>,
    Query(query): Query<SqlQueryRunQuery>,
    tenant: TenantExtractor,
    headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    run_sqlquery(state, tenant, body.0, query, &headers, None).await
}

/// `POST /Library/{id}/$sqlquery-run`.
pub async fn sqlquery_run_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Query(query): Query<SqlQueryRunQuery>,
    tenant: TenantExtractor,
    headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    run_sqlquery(state, tenant, body.0, query, &headers, Some(id)).await
}

async fn run_sqlquery<S>(
    state: AppState<S>,
    tenant: TenantExtractor,
    body: Value,
    query: SqlQueryRunQuery,
    headers: &HeaderMap,
    path_id: Option<String>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let params = extract_sqlquery_params_from_json(&body);

    // _format precedence: body (Parameters) > query string > Accept header
    // > `ndjson` default. SoF v2 PR #353 makes `_format` `0..1` defaulting
    // to `ndjson`.
    let format = resolve_format(params.format.as_deref(), query.format.as_deref(), headers);

    // Spec: the `source` parameter is 0..1 and points at an external data
    // source containing the ViewDefinition tables. We don't implement
    // external sources, so a request that supplies one is asking for
    // behavior we can't honor — reject with 400 per the spec's mapping of
    // "unknown parameter / value type mismatch".
    if params.source.is_some() {
        return Err(RestError::BadRequest {
            message: "the 'source' parameter is not supported by this server; \
                      the query will only run against the SQLQuery Library's \
                      depends-on ViewDefinitions"
                .to_string(),
        });
    }

    // Mutual exclusion: queryResource and queryReference cannot both be supplied.
    if params.query_reference.is_some() && params.query_resource.is_some() {
        return Err(RestError::BadRequest {
            message: "supply at most one of queryReference or queryResource".to_string(),
        });
    }

    // Instance route binds the Library by path id; body queryReference / queryResource
    // would conflict.
    if path_id.is_some() && (params.query_reference.is_some() || params.query_resource.is_some()) {
        return Err(RestError::BadRequest {
            message: "the instance route binds Library by path id; \
                      do not also supply queryReference or queryResource in the body"
                .to_string(),
        });
    }

    let library_json = resolve_library(&state, &tenant, &path_id, &params).await?;
    let library = parse_sqlquery_library(&library_json).map_err(sqlquery_err_to_rest)?;

    // Cap depends-on count.
    let max_vds = state.config().sof_sqlquery_max_vds;
    if library.depends_on.len() > max_vds {
        return Err(RestError::UnprocessableEntity {
            message: format!(
                "Library declares {} depends-on ViewDefinitions; max allowed is {}",
                library.depends_on.len(),
                max_vds
            ),
        });
    }

    // SELECT-only validation.
    validate_select_only(&library.sql)?;

    // Materialize each depends-on VD into the engine.
    let runner = state
        .sof_runner()
        .ok_or_else(|| RestError::NotImplemented {
            feature: "$sqlquery-run is not available: the configured storage backend does not \
                      provide a SOF runner"
                .to_string(),
        })?
        .clone();
    let mut engine = InMemorySqlEngine::open().map_err(sqlquery_err_to_rest)?;
    let max_source_rows = state.config().sof_sqlquery_max_source_rows_per_vd;

    // Keep each materialized VD's schema around for output-column type refinement.
    let mut schemas_in_order: Vec<(String, TableSchema)> = Vec::new();
    for dep in &library.depends_on {
        let label = dep.label.clone();
        let vd_json = resolve_canonical_view_definition(&state, tenant.context(), &dep.url).await?;
        let schema = TableSchema::from_view_definition(&vd_json);
        engine
            .create_table(&label, &schema)
            .map_err(sqlquery_err_to_rest)?;

        let row_stream = runner
            .run_view(tenant.context(), vd_json, ViewFilters::default())
            .await
            .map_err(|e| RestError::UnprocessableEntity {
                message: format!("ViewDefinition '{label}' failed to materialize: {e}"),
            })?;
        let row_stream = adapt_row_stream(row_stream);
        engine
            .insert_rows(&label, &schema, Box::pin(row_stream), max_source_rows)
            .await
            .map_err(sqlquery_err_to_rest)?;
        schemas_in_order.push((label, schema));
    }

    // Bind Library.parameter values from the supplied `parameters` Parameters.
    let bindings = bind_supplied_params(&library.parameters, params.parameters.as_ref())
        .map_err(sqlquery_err_to_rest)?;

    // Run user SQL with timeout + row cap.
    let max_rows = state.config().sof_sqlquery_max_rows;
    let timeout_secs = state.config().sof_sqlquery_timeout_secs;
    let interrupt = engine.interrupt_handle();
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
        interrupt.interrupt();
    });
    let sql = library.sql.clone();
    let exec_result =
        tokio::task::spawn_blocking(move || engine.execute_select(&sql, &bindings, max_rows)).await;
    watchdog.abort();
    let mut result = match exec_result {
        Ok(Ok(r)) => r,
        Ok(Err(SqlQueryError::Sqlite(e))) if e.to_string().contains("interrupted") => {
            return Err(RestError::UnprocessableEntity {
                message: format!("query exceeded {timeout_secs}s timeout"),
            });
        }
        Ok(Err(e)) => return Err(sqlquery_err_to_rest(e)),
        Err(join_err) => {
            return Err(RestError::InternalError {
                message: format!("sqlquery worker panicked: {join_err}"),
            });
        }
    };

    // SoF v2 PR #353: apply caller-supplied `_limit` as a soft cap on the
    // final result set, AFTER SQL evaluation (including any in-query LIMIT).
    // Body wins over URL on conflict. Truncating to fewer rows than the cap
    // is not an error per the PR's wording.
    if let Some(user_limit) = params.limit.or(query.limit) {
        let cap = user_limit as usize;
        if result.rows.len() > cap {
            result.rows.truncate(cap);
        }
    }

    // Refine output column types: when a result column name matches a column
    // we materialized from a depends-on ViewDefinition, prefer the VD-declared
    // FHIR type. Walk depends-on in declaration order so the lookup is
    // deterministic when two VDs declare the same column name.
    let mut name_to_type: std::collections::HashMap<String, ColumnFhirType> =
        std::collections::HashMap::new();
    for (_label, schema) in &schemas_in_order {
        for col in &schema.columns {
            name_to_type
                .entry(col.name.clone())
                .or_insert_with(|| col.fhir_type.clone());
        }
    }
    for (i, col) in result.columns.iter().enumerate() {
        if let Some(t) = name_to_type.get(col) {
            result.column_types[i] = t.clone();
        }
    }

    // Format output. `header` precedence mirrors `_format`: body > query.
    let include_header = params
        .header
        .or_else(|| parse_header_str(query.header.as_deref()))
        .unwrap_or(true);
    let wrap_in_binary = wants_fhir_binary(&format, headers);
    let (content_type, body) = render_output(&format, include_header, &result, wrap_in_binary)?;
    Ok(build_response(content_type, body))
}

/// Resolves the output format. Precedence (SoF v2 PR #353): body `_format` >
/// URL `_format` > Accept header > `ndjson` default. `_format` is `0..1`.
///
/// Accept mapping mirrors `$viewdefinition-run`: `application/json` → `json`,
/// `application/x-ndjson`/`application/ndjson` → `ndjson`, `text/csv` → `csv`,
/// `application/octet-stream`/`application/parquet` → `parquet`,
/// `application/fhir+json` → `fhir`. `application/fhir+xml` is **not**
/// mapped — the FHIR formatter only emits JSON, and routing xml-asking
/// clients to a JSON response was misleading. Unknown Accept values fall
/// through to the `ndjson` default.
fn resolve_format(
    body_format: Option<&str>,
    query_format: Option<&str>,
    headers: &HeaderMap,
) -> String {
    if let Some(f) = body_format.or(query_format) {
        return f.to_lowercase();
    }
    if let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_lowercase)
    {
        let mapped = accept
            .split(',')
            .map(|s| s.split(';').next().unwrap_or("").trim())
            .find_map(|mime| match mime {
                "application/json" => Some("json"),
                "application/x-ndjson" | "application/ndjson" => Some("ndjson"),
                "text/csv" => Some("csv"),
                "application/octet-stream" | "application/parquet" => Some("parquet"),
                "application/fhir+json" => Some("fhir"),
                _ => None,
            });
        if let Some(f) = mapped {
            return f.to_string();
        }
    }
    "ndjson".to_string()
}

/// Parses the query-string `header` value into a bool. Anything that isn't
/// "true"/"1"/"false"/"0" (case-insensitive) is treated as unspecified so the
/// body value or default wins.
fn parse_header_str(s: Option<&str>) -> Option<bool> {
    let s = s?.trim();
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// True when the caller asked for a `Binary`-wrapped flat-format response by
/// setting `Accept: application/fhir+json`. `_format=fhir` is never wrapped
/// (the response is already a FHIR resource).
fn wants_fhir_binary(format: &str, headers: &HeaderMap) -> bool {
    if format == "fhir" || format == "application/fhir+json" {
        return false;
    }
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_lowercase)
    else {
        return false;
    };
    accept
        .split(',')
        .map(|s| s.split(';').next().unwrap_or("").trim())
        .any(|m| m == "application/fhir+json")
}

/// Sniff SQL to confirm a single `SELECT`/CTE statement. The spec doesn't
/// strictly require this but every reference impl rejects DDL/DML here.
fn validate_select_only(sql: &str) -> Result<(), RestError> {
    use sqlparser::ast::Statement;
    use sqlparser::dialect::SQLiteDialect;
    use sqlparser::parser::Parser;

    let stmts = Parser::parse_sql(&SQLiteDialect {}, sql).map_err(|e| RestError::BadRequest {
        message: format!("SQL parse error: {e}"),
    })?;
    if stmts.len() != 1 {
        return Err(RestError::BadRequest {
            message: format!("exactly one SQL statement is required, got {}", stmts.len()),
        });
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
            Err(RestError::BadRequest {
                message: format!(
                    "only SELECT queries are allowed; {keyword} statements are not permitted"
                ),
            })
        }
    }
}

async fn resolve_library<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    path_id: &Option<String>,
    params: &helios_sof::SqlQueryRunParams,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // Instance route wins.
    if let Some(id) = path_id {
        let stored = state
            .storage()
            .read(tenant.context(), "Library", id)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("failed to read Library: {e}"),
            })?
            .ok_or_else(|| RestError::NotFound {
                resource_type: "Library".to_string(),
                id: id.clone(),
            })?;
        return Ok(stored.content().clone());
    }
    if let Some(library_json) = &params.query_resource {
        return Ok(library_json.clone());
    }
    if let Some(reference) = &params.query_reference {
        return resolve_library_reference(state, tenant.context(), reference).await;
    }
    Err(RestError::BadRequest {
        message: "supply queryReference, queryResource, or use the instance route".to_string(),
    })
}

async fn resolve_library_reference<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    resolve_resource_canonical_or_relative(state, tenant, "Library", reference).await
}

async fn resolve_canonical_view_definition<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    resolve_resource_canonical_or_relative(state, tenant, "ViewDefinition", url).await
}

/// Convert a `RowStream<Result<Value, SofError>>` into a stream
/// `Result<Value, String>` for the engine (it doesn't know the persistence type).
fn adapt_row_stream(
    stream: helios_persistence::core::sof_runner::RowStream,
) -> impl Stream<Item = Result<Value, String>> + Send {
    use futures::StreamExt;
    stream.map(|r| r.map_err(|e| e.to_string()))
}

fn render_output(
    format: &str,
    include_header: bool,
    result: &QueryResult,
    wrap_in_binary: bool,
) -> Result<(&'static str, Vec<u8>), RestError> {
    match format {
        "fhir" | "application/fhir+json" => {
            let bytes = format_fhir_parameters(result).map_err(sqlquery_err_to_rest)?;
            Ok(("application/fhir+json", bytes))
        }
        _ => {
            // Build a ProcessedResult directly so columns keep their SQL order.
            // (Going through `rows_to_processed_result` discards order because
            // serde_json::Map doesn't preserve insertion order by default.)
            let processed = helios_sof::ProcessedResult {
                columns: result.columns.clone(),
                rows: result
                    .rows
                    .iter()
                    .map(|r| helios_sof::ProcessedRow { values: r.clone() })
                    .collect(),
            };
            let ct = parse_content_type(format, include_header).ok_or_else(|| {
                RestError::BadRequest {
                    message: format!(
                        "unsupported _format value '{format}'; supported: csv, json, ndjson, parquet, fhir"
                    ),
                }
            })?;
            let body = helios_sof::format_output(processed, ct, None).map_err(|e| {
                RestError::InternalError {
                    message: format!("output formatter failed: {e}"),
                }
            })?;
            let inner_ct = content_type_for(ct);
            if wrap_in_binary {
                let binary = json!({
                    "resourceType": "Binary",
                    "contentType": inner_ct,
                    "data": base64::engine::general_purpose::STANDARD.encode(&body),
                });
                let bytes = serde_json::to_vec(&binary).map_err(|e| RestError::InternalError {
                    message: format!("failed to serialize Binary wrapper: {e}"),
                })?;
                Ok(("application/fhir+json", bytes))
            } else {
                Ok((inner_ct, body))
            }
        }
    }
}

fn parse_content_type(format: &str, include_header: bool) -> Option<ContentType> {
    match format {
        "ndjson" | "application/x-ndjson" | "application/ndjson" => Some(ContentType::NdJson),
        "json" | "application/json" => Some(ContentType::Json),
        "csv" | "text/csv" => Some(if include_header {
            ContentType::CsvWithHeader
        } else {
            ContentType::Csv
        }),
        "parquet" | "application/parquet" | "application/octet-stream" => {
            Some(ContentType::Parquet)
        }
        _ => None,
    }
}

fn content_type_for(ct: ContentType) -> &'static str {
    match ct {
        ContentType::Csv | ContentType::CsvWithHeader => "text/csv; charset=utf-8",
        ContentType::Json => "application/json",
        ContentType::NdJson => "application/x-ndjson",
        ContentType::Parquet => "application/octet-stream",
    }
}

fn build_response(ct: &'static str, body: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, ct)], body).into_response()
}

fn sqlquery_err_to_rest(e: SqlQueryError) -> RestError {
    match e {
        SqlQueryError::MalformedLibrary(msg) => RestError::UnprocessableEntity { message: msg },
        SqlQueryError::MissingSql => RestError::UnprocessableEntity {
            message: "SQLQuery Library has no SQL content (application/sql)".to_string(),
        },
        SqlQueryError::MissingDependsOnLabel => RestError::UnprocessableEntity {
            message: "depends-on entry missing label".into(),
        },
        // Spec: "Library or ViewDefinition not found" → 404. (Currently
        // unreached because canonical resolution lives in `references.rs`
        // which raises NotFound directly, but kept consistent for any
        // future engine-level resolver path.)
        SqlQueryError::UnknownCanonical(s) => RestError::NotFound {
            resource_type: "Resource".to_string(),
            id: s,
        },
        SqlQueryError::TooManyDependsOn { count, max } => RestError::UnprocessableEntity {
            message: format!("too many depends-on ViewDefinitions: {count} (max {max})"),
        },
        SqlQueryError::RowCapExceeded { max } => RestError::UnprocessableEntity {
            message: format!("result exceeds {max}-row limit; add a WHERE/LIMIT clause"),
        },
        SqlQueryError::Timeout { secs } => RestError::UnprocessableEntity {
            message: format!("query exceeded {secs}s timeout"),
        },
        SqlQueryError::NotSelect(msg) => RestError::BadRequest { message: msg },
        SqlQueryError::BindParameter(msg) => RestError::BadRequest { message: msg },
        SqlQueryError::InvalidIdentifier(name) => RestError::BadRequest {
            message: format!("invalid identifier '{name}'"),
        },
        SqlQueryError::UnsupportedFhirValue(col) => RestError::UnprocessableEntity {
            message: format!(
                "column '{col}' has a composite value not representable as a FHIR scalar; \
                 _format=fhir cannot be used for this query"
            ),
        },
        SqlQueryError::Sqlite(err) => {
            warn!(error = %err, "sqlite error during $sqlquery-run");
            RestError::UnprocessableEntity {
                message: format!("SQLite error: {err}"),
            }
        }
    }
}
