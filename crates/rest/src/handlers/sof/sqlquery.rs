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
//! and serializes the result in the requested `_format`.
//!
//! ## Deviation: raw-bytes output for flat formats
//!
//! The spec declares the operation's `return` parameter as `Binary | Parameters`
//! (1..1) — strictly, flat formats (csv/json/ndjson/parquet) should be wrapped
//! in a `Binary` resource with the encoded payload in `Binary.data`. HFS
//! instead returns the raw payload bytes with the format's `Content-Type`,
//! matching the convention used by `$viewdefinition-run` and by every other
//! SoF reference implementation. `_format=fhir` still returns a `Parameters`
//! resource as specified.

use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::Stream;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};
use helios_sof::sqlquery::SqlQueryError;
use helios_sof::{
    ColumnFhirType, ContentType, InMemorySqlEngine, QueryResult, TableSchema, bind_supplied_params,
    extract_sqlquery_params_from_json, format_fhir_parameters, parse_sqlquery_library,
};
use serde_json::Value;
use std::time::Duration;
use tracing::warn;

use crate::error::RestError;
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// `POST /$sqlquery-run` and `POST /Library/$sqlquery-run`.
pub async fn sqlquery_run_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    body: axum::extract::Json<Value>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    run_sqlquery(state, tenant, body.0, None).await
}

/// `POST /Library/{id}/$sqlquery-run`.
pub async fn sqlquery_run_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    tenant: TenantExtractor,
    body: axum::extract::Json<Value>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    run_sqlquery(state, tenant, body.0, Some(id)).await
}

async fn run_sqlquery<S>(
    state: AppState<S>,
    tenant: TenantExtractor,
    body: Value,
    path_id: Option<String>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let params = extract_sqlquery_params_from_json(&body);

    // _format is required (spec 1..1).
    let format = params
        .format
        .clone()
        .ok_or_else(|| RestError::BadRequest {
            message: "_format is required; supported values: csv, json, ndjson, parquet, fhir"
                .to_string(),
        })?
        .to_lowercase();

    // Out of scope v1.
    if params.source.is_some() {
        return Err(RestError::UnprocessableEntity {
            message: "the 'source' parameter (external data source) is not supported".to_string(),
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

    // Format output.
    let include_header = params.header.unwrap_or(true);
    let (content_type, body) = render_output(&format, include_header, &result)?;
    Ok(build_response(content_type, body))
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

/// Resolves a canonical-or-relative reference for the given resource type.
///
/// Accepts both:
/// - `Type/{id}` — relative reference, served by `ResourceStorage::read`.
/// - absolute canonical URL (`http(s)://…`, optionally `…|version`) — resolved
///   via `SearchProvider::search` with `url=` (and `version=` when supplied),
///   picking the newest match by `meta.lastUpdated`.
///
/// The spec pins `relatedArtifact.resource` to `canonical([Resource])`, but
/// relative references are widely used in practice and FHIR treats them as
/// valid canonical references in a server-local context.
async fn resolve_resource_canonical_or_relative<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let trimmed = reference.trim();
    let prefix = format!("{resource_type}/");
    if let Some(rest) = trimmed.strip_prefix(prefix.as_str()) {
        let id = rest.split('/').next().unwrap_or("");
        if id.is_empty() {
            return Err(RestError::BadRequest {
                message: format!("'{reference}' has an empty id after '{resource_type}/'"),
            });
        }
        let stored = state
            .storage()
            .read(tenant, resource_type, id)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("failed to read {resource_type}: {e}"),
            })?
            .ok_or_else(|| RestError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })?;
        return Ok(stored.content().clone());
    }
    resolve_by_canonical_url(state, tenant, resource_type, trimmed).await
}

async fn resolve_by_canonical_url<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    url: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // Accept `url|version` syntax per the FHIR canonical convention.
    let (canonical, version) = match url.split_once('|') {
        Some((u, v)) => (u.to_string(), Some(v.to_string())),
        None => (url.to_string(), None),
    };

    let mut query = SearchQuery::new(resource_type);
    query.parameters.push(SearchParameter {
        name: "url".to_string(),
        param_type: SearchParamType::Uri,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, canonical)],
        chain: Vec::new(),
        components: Vec::new(),
    });
    if let Some(v) = version {
        query.parameters.push(SearchParameter {
            name: "version".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, v)],
            chain: Vec::new(),
            components: Vec::new(),
        });
    }

    let result =
        state
            .storage()
            .search(tenant, &query)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("canonical lookup failed for {resource_type} url={url}: {e}"),
            })?;

    let candidates: Vec<_> = result.resources.items.into_iter().collect();
    if candidates.is_empty() {
        return Err(RestError::UnprocessableEntity {
            message: format!("could not resolve canonical {resource_type} '{url}'"),
        });
    }
    // Pick newest by last_modified.
    let chosen = candidates
        .into_iter()
        .max_by_key(|r| r.last_modified())
        .ok_or_else(|| RestError::InternalError {
            message: "unreachable: candidates was non-empty".into(),
        })?;
    Ok(chosen.content().clone())
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
            Ok((content_type_for(ct), body))
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
        SqlQueryError::UnknownCanonical(s) => RestError::UnprocessableEntity {
            message: format!("could not resolve canonical URL: {s}"),
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
