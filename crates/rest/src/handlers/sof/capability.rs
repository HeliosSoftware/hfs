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
//!     { "name": "supportsViewDefinitionRun",    "valueBoolean": true  },
//!     { "name": "supportsViewDefinitionExport", "valueBoolean": false },
//!     { "name": "supportsSqlQueryRun",          "valueBoolean": true  },
//!     { "name": "supportsSqlQueryExport",       "valueBoolean": false },
//!     { "name": "supportsInDbRunner",           "valueBoolean": false },
//!     { "name": "supportsRelativeReference",    "valueBoolean": true  },
//!     { "name": "supportsCanonicalReference",   "valueBoolean": true  },
//!     { "name": "supportsAbsoluteReference",    "valueBoolean": true  },
//!     { "name": "supportedFormat",              "valueCode": "ndjson"  },
//!     { "name": "supportedFormat",              "valueCode": "json"    },
//!     { "name": "supportedFormat",              "valueCode": "csv"     },
//!     { "name": "supportedFormat",              "valueCode": "parquet" },
//!     { "name": "supportedFormat",              "valueCode": "fhir"    }
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
/// - `$viewdefinition-export` — runtime-gated on whether an export controller is wired.
/// - `$sqlquery-run` — always enabled (runs against an in-memory SQLite engine
///   that materializes the SQLQuery Library's depends-on ViewDefinitions).
/// - `supportsInDbRunner` — true when the wired `SofRunner` is not the in-process
///   fallback (i.e. the backend has compiled an in-DB runner).
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

    let mut params: Vec<serde_json::Value> = vec![
        bool_param("supportsViewDefinitionRun", true),
        bool_param("supportsViewDefinitionExport", supports_export),
        bool_param("supportsSqlQueryRun", true),
        bool_param("supportsSqlQueryExport", supports_export),
        bool_param("supportsInDbRunner", supports_indb),
        // Spec SHALL: document which ViewDefinition reference formats are
        // supported. We support:
        // - relative `ViewDefinition/{id}` via storage read
        // - canonical URLs via SearchProvider `url=` lookup
        // - absolute URLs — the canonical resolver treats any non-relative
        //   reference as a canonical-URL lookup, so absolute http(s) URLs
        //   that match a registered canonical are resolved successfully.
        bool_param("supportsRelativeReference", true),
        bool_param("supportsCanonicalReference", true),
        bool_param("supportsAbsoluteReference", true),
    ];

    // Supported output formats (G2: includes parquet). `fhir` is offered by
    // the synchronous run operations only ($viewdefinition-run,
    // $sqlquery-run); per spec PR #365 the export operations bind `_format`
    // to ExportOutputFormatCodes (csv/ndjson/parquet/json) and reject `fhir`.
    // The capability operation's `supportedFormat` is a single flat list, so
    // `fhir` appears here because the server genuinely supports it.
    for fmt in ["ndjson", "json", "csv", "parquet", "fhir"] {
        params.push(json!({
            "name": "supportedFormat",
            "valueCode": fmt
        }));
    }

    // Audit item #13: explicit declaration of the spec's value-set bindings
    // (extensible). The run operations bind `_format` to OutputFormatCodes
    // (which includes `fhir`); the export operations bind to
    // ExportOutputFormatCodes (csv/ndjson/parquet/json, no `fhir`) per spec
    // PR #365. These entries let audit tools discover the bindings without
    // following the CapabilityStatement → OperationDefinition link.
    params.push(json!({
        "name": "formatBinding",
        "part": [
            {
                "name": "valueSet",
                "valueUri": "https://sql-on-fhir.org/ig/ValueSet/OutputFormatCodes"
            },
            {"name": "strength", "valueCode": "extensible"},
            {"name": "operationScope", "valueString": "run"}
        ]
    }));
    params.push(json!({
        "name": "formatBinding",
        "part": [
            {
                "name": "valueSet",
                "valueUri": "https://sql-on-fhir.org/ig/ValueSet/ExportOutputFormatCodes"
            },
            {"name": "strength", "valueCode": "extensible"},
            {"name": "operationScope", "valueString": "export"}
        ]
    }));

    json!({
        "resourceType": "Parameters",
        "parameter": params
    })
}

/// Creates a `{ "name": ..., "valueBoolean": ... }` parameter entry.
fn bool_param(name: &str, value: bool) -> serde_json::Value {
    json!({ "name": name, "valueBoolean": value })
}
