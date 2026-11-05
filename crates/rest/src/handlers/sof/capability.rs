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
/// Path id for the `$reindex` definition this server publishes.
pub const REINDEX_DEFINITION_ID: &str = "hfs-reindex";

/// `GET [base]/OperationDefinition/{id}`
///
/// Serves the definitions above. Any other id is a 404 — this route does not
/// read arbitrary OperationDefinitions out of storage.
pub async fn sof_operation_definition_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let export_available = state.export_controller().is_some();
    let reindex_available = state.reindex().is_some();
    let definition = match id.as_str() {
        SQL_RUN_DEFINITION_ID => sql_run_definition(),
        SQL_EXPORT_DEFINITION_ID if export_available => sql_export_definition(),
        REINDEX_DEFINITION_ID if reindex_available => reindex_definition(),
        _ => {
            return Err(RestError::NotFound {
                resource_type: "OperationDefinition".to_string(),
                id,
            });
        }
    };
    Ok((StatusCode::OK, axum::Json(definition)))
}

/// The `$reindex` operation this server publishes.
///
/// `base` is absent on purpose: `$reindex` is an HFS administrative operation
/// with no HL7 counterpart to derive from, unlike the SQL on FHIR pair below.
/// It is advertised in `CapabilityStatement.rest.operation` only where an index
/// exists to rebuild, which is what makes it discoverable to an operator whose
/// search index has fallen behind its primary (#1021).
pub(crate) fn reindex_definition() -> Value {
    json!({
        "resourceType": "OperationDefinition",
        "id": REINDEX_DEFINITION_ID,
        "url": format!("/OperationDefinition/{REINDEX_DEFINITION_ID}"),
        "name": "Reindex",
        "title": "Rebuild the search index from stored resources",
        "status": "active",
        "kind": "operation",
        "code": "reindex",
        "affectsState": true,
        "system": true,
        "type": true,
        "instance": false,
        "description": "Re-extracts search parameters from every stored resource and rewrites every search index the deployment maintains, including an Elasticsearch secondary. Runs in the background: the kick-off returns 202 with a job id, polled at GET [base]/$reindex-status/{id} and cancelled with DELETE on the same path. Job state is held in memory on the node that accepted the kick-off, so poll the node you kicked off against. Requires the system/reindex operation scope.",
        "parameter": [
            {
                "name": "clearExisting", "use": "in", "min": 0, "max": "1", "type": "boolean",
                "documentation": "Clear each index before rebuilding it. Defaults to false, which overwrites entries in place; a resource deleted from the primary since the last index keeps its stale entry unless this is set."
            },
            {
                "name": "batchSize", "use": "in", "min": 0, "max": "1", "type": "integer",
                "documentation": "Resources read and rewritten per page. Defaults to 100."
            },
            {
                "name": "jobId", "use": "out", "min": 1, "max": "1", "type": "string",
                "documentation": "Identifier to poll at [base]/$reindex-status/{jobId}."
            }
        ]
    })
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

    /// The advertised `definition` URL has to resolve, or the citation is a dead
    /// link — and it must 404 where the operation is not served, matching the
    /// CapabilityStatement gating.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn reindex_definition_is_served_only_where_reindex_is_wired() {
        use crate::config::ServerConfig;
        use crate::state::AppState;
        use helios_persistence::backends::sqlite::SqliteBackend;
        use helios_persistence::search::{ReindexOperation, TenantSearchRegistries};
        use std::sync::Arc;

        let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
        backend.init_schema().expect("init schema");

        let unwired = AppState::new(backend.clone(), ServerConfig::default());
        assert!(
            sof_operation_definition_handler(
                State(unwired),
                Path(REINDEX_DEFINITION_ID.to_string())
            )
            .await
            .is_err(),
            "not served where $reindex answers 501"
        );

        let registries = Arc::new(TenantSearchRegistries::base_only());
        let wired = AppState::new(backend.clone(), ServerConfig::default())
            .with_reindex(Arc::new(ReindexOperation::new(backend, registries)));
        let response =
            sof_operation_definition_handler(State(wired), Path(REINDEX_DEFINITION_ID.to_string()))
                .await
                .expect("served where $reindex is wired")
                .into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// `$reindex` is the documented recovery for a search index that has fallen
    /// behind its primary, and it was invisible: absent from
    /// `CapabilityStatement.rest.operation`, so an operator had no discoverable
    /// way to learn it exists (#1021). This pins the definition it now cites.
    #[test]
    fn reindex_definition_describes_the_operation_the_handler_accepts() {
        let d = reindex_definition();
        assert_eq!(d["code"], "reindex");
        assert_eq!(d["id"], REINDEX_DEFINITION_ID);
        assert_eq!(
            d["affectsState"], true,
            "a reindex rewrites the search index"
        );
        assert_eq!(d["system"], true, "POST [base]/$reindex");
        assert_eq!(d["type"], true, "POST [base]/{{type}}/$reindex");
        assert!(
            d.get("base").is_none(),
            "no HL7 counterpart to subset — unlike the SQL on FHIR pair"
        );

        // The in-parameters are exactly the two the kick-off handler reads; an
        // OperationDefinition that named others would advertise support that
        // does not exist.
        let names = param_names(&d);
        assert!(names.contains(&"clearExisting"));
        assert!(names.contains(&"batchSize"));
        assert!(names.contains(&"jobId"));
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
