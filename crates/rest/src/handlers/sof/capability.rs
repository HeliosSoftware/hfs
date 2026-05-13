//! SQL-on-FHIR capabilities handler.
//!
//! Implements `GET /$sql-on-fhir-capabilities`, which returns a FHIR `Parameters`
//! resource describing what SQL-on-FHIR features this server instance supports.
//!
//! The response follows the [operations-capability](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/operations-capability.html)
//! shape from the SQL-on-FHIR v2 specification.
//!
//! ## Response shape
//!
//! ```json
//! {
//!   "resourceType": "Parameters",
//!   "parameter": [
//!     { "name": "supportsViewDefinitionRun",    "valueBoolean": true },
//!     { "name": "supportsViewDefinitionExport", "valueBoolean": false },
//!     { "name": "supportsSqlQueryRun",          "valueBoolean": false },
//!     { "name": "supportsInDbRunner",           "valueBoolean": false },
//!     { "name": "supportedFormat",              "valueCode": "ndjson" },
//!     { "name": "supportedFormat",              "valueCode": "json"   },
//!     { "name": "supportedFormat",              "valueCode": "csv"    }
//!   ]
//! }
//! ```

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use helios_persistence::core::ResourceStorage;
use serde_json::json;

use crate::state::AppState;

/// `GET /$sql-on-fhir-capabilities`
///
/// Returns a `Parameters` resource listing the SQL-on-FHIR features that this
/// server instance currently supports.
///
/// Feature flags used at build time:
/// - `$viewdefinition-run` — always enabled when the `sof` feature is active.
/// - `$viewdefinition-export` — enabled in Phase 4 (not yet).
/// - `$sql-query-run` — enabled in Phase 6 (not yet).
/// - `supportsInDbRunner` — true when the wired `SofRunner` is not the in-process
///   fallback (i.e. the backend has compiled an in-DB runner in Phase 3+).
pub async fn sof_capabilities_handler<S>(State(state): State<AppState<S>>) -> impl IntoResponse
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let caps = build_sof_capabilities(&state);
    (StatusCode::OK, axum::Json(caps))
}

/// Builds the SQL-on-FHIR `Parameters` capabilities response.
pub(crate) fn build_sof_capabilities<S>(state: &AppState<S>) -> serde_json::Value
where
    S: ResourceStorage + Send + Sync + 'static,
{
    // Determine whether the wired runner is in-DB (Phase 3+) or the in-process fallback.
    let supports_indb = state
        .sof_runner()
        .map(|r| r.runner_name() != "inprocess")
        .unwrap_or(false);

    // Determine feature availability at runtime
    let supports_export = state.export_controller().is_some();
    let supports_sql_query =
        state.raw_sql_runner().is_some() && state.config().sof_sql_query_enabled;

    let mut params: Vec<serde_json::Value> = vec![
        bool_param("supportsViewDefinitionRun", true),
        bool_param("supportsViewDefinitionExport", supports_export),
        bool_param("supportsSqlQueryRun", supports_sql_query),
        bool_param("supportsInDbRunner", supports_indb),
        // Spec SHALL: document which ViewDefinition reference formats are
        // supported. We currently support only relative `ViewDefinition/{id}`.
        bool_param("supportsRelativeReference", true),
        bool_param("supportsCanonicalReference", false),
        bool_param("supportsAbsoluteReference", false),
    ];

    // Supported output formats (G2: includes parquet; fhir included for $sqlquery-run).
    for fmt in ["ndjson", "json", "csv", "parquet", "fhir"] {
        params.push(json!({
            "name": "supportedFormat",
            "valueCode": fmt
        }));
    }

    json!({
        "resourceType": "Parameters",
        "parameter": params
    })
}

/// Creates a `{ "name": ..., "valueBoolean": ... }` parameter entry.
fn bool_param(name: &str, value: bool) -> serde_json::Value {
    json!({ "name": name, "valueBoolean": value })
}
