//! This server's own `$sql-run` / `$sql-export` OperationDefinitions.
//!
//! A server need not support every parameter of an operation, but which subset
//! it does support has to be discoverable. Base FHIR already provides the
//! mechanism, and
//! [operations-capability](http://hl7.org/fhir/uv/sql-on-fhir/operations-capability.html#partial-operation-support)
//! adopts it:
//!
//! - Citing an OperationDefinition published by the guide asserts support for
//!   the **full** capabilities of that operation, including every parameter it
//!   declares.
//! - A server supporting only a subset SHALL publish its own OperationDefinition
//!   whose `base` is the guide's canonical URL, declaring only the parameters it
//!   supports, and SHALL point `CapabilityStatement.rest.operation.definition`
//!   at its own definition.
//!
//! HFS supports a subset, so it publishes these. What is omitted, and therefore
//! rejected on request:
//!
//! | Parameter | Why |
//! |-----------|-----|
//! | `context` | Supporting artifacts for a subject's transitive dependency graph are not yet resolved |
//! | `source`  | External data sources are out of scope for a storage-backed server; use the stateless `sof-server` |
//!
//! This replaces the pre-ballot `GET /$sql-on-fhir-capabilities` endpoint and
//! its `Parameters` block of `supportsX` booleans. That endpoint was a
//! continuous-build invention and is absent from 3.0.0-ballot, which routes the
//! same information through machine-readable OperationDefinitions instead.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use helios_persistence::core::ResourceStorage;
use serde_json::{Value, json};

use crate::error::RestError;
use crate::state::AppState;

/// Path id for the `$sql-run` definition this server publishes.
pub const SQL_RUN_DEFINITION_ID: &str = "hfs-sql-run";
/// Path id for the `$sql-export` definition this server publishes.
pub const SQL_EXPORT_DEFINITION_ID: &str = "hfs-sql-export";

/// `GET [base]/OperationDefinition/{id}`
///
/// Serves the two definitions above. Any other id is a 404 — this route does
/// not read arbitrary OperationDefinitions out of storage.
pub async fn sof_operation_definition_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let export_available = state.export_controller().is_some();
    let definition = match id.as_str() {
        SQL_RUN_DEFINITION_ID => sql_run_definition(),
        SQL_EXPORT_DEFINITION_ID if export_available => sql_export_definition(),
        _ => {
            return Err(RestError::NotFound {
                resource_type: "OperationDefinition".to_string(),
                id,
            });
        }
    };
    Ok((StatusCode::OK, axum::Json(definition)))
}

/// The `$sql-run` subset this server supports.
///
/// All three subject-naming parameters are supported: canonical URLs and
/// literal references both resolve against storage, and inline resources are
/// accepted on POST.
pub(crate) fn sql_run_definition() -> Value {
    json!({
        "resourceType": "OperationDefinition",
        "id": SQL_RUN_DEFINITION_ID,
        "url": format!("/OperationDefinition/{SQL_RUN_DEFINITION_ID}"),
        "name": "SQLRunSupported",
        "title": "SQL Run (subset supported by this server)",
        "status": "active",
        "kind": "operation",
        "code": "sql-run",
        "base": helios_sof::canonical::SQL_RUN_OPERATION_DEFINITION,
        "system": true,
        "type": false,
        "instance": false,
        "parameter": [
            {
                "name": "subjectCanonical", "use": "in", "min": 0, "max": "1", "type": "canonical",
                "documentation": "Canonical URL of the subject, optionally with a |version suffix."
            },
            {
                "name": "subjectReference", "use": "in", "min": 0, "max": "1", "type": "Reference",
                "documentation": "Literal location of the subject: a relative URL on this server, or an absolute URL."
            },
            {
                "name": "subjectResource", "use": "in", "min": 0, "max": "1", "type": "CanonicalResource",
                "documentation": "Inline ViewDefinition, SQLQuery Library or SQLView Library. Requires POST."
            },
            {
                "name": "parameters", "use": "in", "min": 0, "max": "1", "type": "Parameters",
                "documentation": "Bindings for the parameters a Library declares. Rejected for a ViewDefinition subject."
            },
            {
                "name": "resource", "use": "in", "min": 0, "max": "*", "type": "Resource",
                "documentation": "FHIR resources to transform instead of using server data. Requires a ViewDefinition subject; a Bundle is unwrapped one level."
            },
            {"name": "_format", "use": "in", "min": 0, "max": "1", "type": "code",
             "binding": {"strength": "extensible", "valueSet": helios_sof::canonical::OUTPUT_FORMAT_VALUE_SET}},
            {"name": "header", "use": "in", "min": 0, "max": "1", "type": "boolean"},
            {"name": "patient", "use": "in", "min": 0, "max": "*", "type": "Reference"},
            {"name": "group", "use": "in", "min": 0, "max": "*", "type": "Reference"},
            {"name": "_since", "use": "in", "min": 0, "max": "1", "type": "instant"},
            {"name": "_limit", "use": "in", "min": 0, "max": "1", "type": "integer"},
            {"name": "return", "use": "out", "min": 1, "max": "1", "type": "Binary"}
        ]
    })
}

/// The `$sql-export` subset this server supports.
pub(crate) fn sql_export_definition() -> Value {
    json!({
        "resourceType": "OperationDefinition",
        "id": SQL_EXPORT_DEFINITION_ID,
        "url": format!("/OperationDefinition/{SQL_EXPORT_DEFINITION_ID}"),
        "name": "SQLExportSupported",
        "title": "SQL Export (subset supported by this server)",
        "status": "active",
        "kind": "operation",
        "code": "sql-export",
        "base": helios_sof::canonical::SQL_EXPORT_OPERATION_DEFINITION,
        "system": true,
        "type": false,
        "instance": false,
        "parameter": [
            {
                "name": "subject", "use": "in", "min": 1, "max": "*",
                "documentation": "One or more artifacts to export, in any mixture of ViewDefinitions, SQLQuery Libraries and SQLView Libraries. Each repetition produces exactly one output entry.",
                "part": [
                    {"name": "name", "use": "in", "min": 0, "max": "1", "type": "string"},
                    {"name": "subjectCanonical", "use": "in", "min": 0, "max": "1", "type": "canonical"},
                    {"name": "subjectReference", "use": "in", "min": 0, "max": "1", "type": "Reference"},
                    {"name": "subjectResource", "use": "in", "min": 0, "max": "1", "type": "CanonicalResource"},
                    {"name": "parameters", "use": "in", "min": 0, "max": "1", "type": "Parameters"}
                ]
            },
            {"name": "clientTrackingId", "use": "in", "min": 0, "max": "1", "type": "string"},
            {"name": "_format", "use": "in", "min": 0, "max": "1", "type": "code",
             "binding": {"strength": "extensible", "valueSet": helios_sof::canonical::EXPORT_OUTPUT_FORMAT_VALUE_SET}},
            {"name": "header", "use": "in", "min": 0, "max": "1", "type": "boolean"},
            {"name": "patient", "use": "in", "min": 0, "max": "*", "type": "Reference"},
            {"name": "group", "use": "in", "min": 0, "max": "*", "type": "Reference"},
            {"name": "_since", "use": "in", "min": 0, "max": "1", "type": "instant"},
            {"name": "exportId", "use": "out", "min": 1, "max": "1", "type": "string"},
            {"name": "clientTrackingId", "use": "out", "min": 0, "max": "1", "type": "string"},
            {"name": "status", "use": "out", "min": 1, "max": "1", "type": "code",
             "binding": {"strength": "required", "valueSet": helios_sof::canonical::EXPORT_STATUS_VALUE_SET}},
            {"name": "location", "use": "out", "min": 1, "max": "1", "type": "uri"},
            {"name": "cancelUrl", "use": "out", "min": 0, "max": "1", "type": "uri"},
            {"name": "_format", "use": "out", "min": 0, "max": "1", "type": "code"},
            {"name": "exportStartTime", "use": "out", "min": 0, "max": "1", "type": "instant"},
            {"name": "exportEndTime", "use": "out", "min": 0, "max": "1", "type": "instant"},
            {"name": "exportDuration", "use": "out", "min": 0, "max": "1", "type": "integer"},
            {"name": "estimatedTimeRemaining", "use": "out", "min": 0, "max": "1", "type": "integer"},
            {
                "name": "output", "use": "out", "min": 0, "max": "*",
                "part": [
                    {"name": "name", "use": "out", "min": 1, "max": "1", "type": "string"},
                    {"name": "location", "use": "out", "min": 1, "max": "*", "type": "uri"}
                ]
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param_names(definition: &Value) -> Vec<&str> {
        definition["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn run_definition_bases_on_the_guides_and_omits_what_we_reject() {
        let d = sql_run_definition();
        assert_eq!(d["code"], "sql-run");
        assert_eq!(
            d["base"],
            helios_sof::canonical::SQL_RUN_OPERATION_DEFINITION,
            "base must name the guide's definition, so a client knows what this subsets"
        );
        // System level only.
        assert_eq!(d["system"], true);
        assert_eq!(d["type"], false);
        assert_eq!(d["instance"], false);

        let names = param_names(&d);
        for supported in ["subjectCanonical", "subjectReference", "subjectResource"] {
            assert!(names.contains(&supported), "{supported} missing: {names:?}");
        }
        for unsupported in ["context", "source"] {
            assert!(
                !names.contains(&unsupported),
                "{unsupported} is rejected and must not be declared: {names:?}"
            );
        }
    }

    #[test]
    fn export_definition_takes_a_repeating_subject_with_parts() {
        let d = sql_export_definition();
        assert_eq!(d["code"], "sql-export");
        assert_eq!(
            d["base"],
            helios_sof::canonical::SQL_EXPORT_OPERATION_DEFINITION
        );

        let subject = d["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "subject" && p["use"] == "in")
            .expect("an in `subject` parameter");
        assert_eq!(subject["min"], 1, "subject is 1..*");
        assert_eq!(subject["max"], "*");

        let parts: Vec<&str> = subject["part"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            parts,
            vec![
                "name",
                "subjectCanonical",
                "subjectReference",
                "subjectResource",
                "parameters"
            ]
        );
    }

    #[test]
    fn export_does_not_offer_limit() {
        // `_limit` caps rows in a response; an export delivers files, so there
        // is nothing for it to cap.
        assert!(!param_names(&sql_export_definition()).contains(&"_limit"));
    }

    #[test]
    fn run_offers_limit() {
        assert!(param_names(&sql_run_definition()).contains(&"_limit"));
    }
}
