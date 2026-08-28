//! Read model for the CapabilityStatement page (`/ui/capability-statement`,
//! #653).
//!
//! `/metadata` composes the statement fresh on every request from live server
//! state — registered search parameters, backend capabilities, enabled
//! features — so there is nothing stored to edit and the page is genuinely
//! read-only. The fetch rides the same loopback self-call as the other
//! conformance viewers ([`crate::conformance`]); a failed fetch degrades to a
//! warning, never to fabricated capabilities.

use helios_fhir::FhirVersion;
use helios_fhir_validator::{SchemaResolver, editor, packs};
use serde_json::Value;
use std::collections::HashSet;

/// Why the Resources workspace cannot create its current resource type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreateTargetBlock {
    InvalidType,
    CreateNotAdvertised,
    SchemaUnavailable,
    MetadataUnavailable,
}

/// The three exact sets that make a Resources create target safe.
///
/// Search and the type rail deliberately use the broader `resource_types`
/// catalog. Creation is narrower: the type must also have a live `create`
/// interaction and an editor resource schema for the effective FHIR version.
pub(crate) struct CreateTargets {
    resource_types: HashSet<String>,
    advertised_create: HashSet<String>,
    schema_resources: HashSet<String>,
}

impl CreateTargets {
    pub(crate) fn from_statement(
        resource_types: &[String],
        statement: &Value,
        version: FhirVersion,
    ) -> Result<Self, &'static str> {
        if str_at(statement, &["resourceType"]) != "CapabilityStatement" {
            return Err("metadata is not a CapabilityStatement");
        }
        if str_at(statement, &["fhirVersion"]) != version.full_version() {
            return Err("metadata describes a different FHIR version");
        }

        let server_rest: Vec<&Value> = arr(statement, "rest")
            .iter()
            .filter(|rest| str_at(rest, &["mode"]) == "server")
            .collect();
        if server_rest.is_empty() {
            return Err("metadata has no server REST component");
        }

        let advertised_create = server_rest
            .into_iter()
            .flat_map(|rest| arr(rest, "resource"))
            .filter(|resource| {
                arr(resource, "interaction")
                    .iter()
                    .any(|interaction| str_at(interaction, &["code"]) == "create")
            })
            .filter_map(|resource| resource.get("type").and_then(Value::as_str))
            .map(str::to_string)
            .collect();

        let registry = packs::core_registry(version);
        let schema_resources = resource_types
            .iter()
            .filter(|resource_type| {
                registry
                    .resolve(resource_type)
                    .is_some_and(|schema| editor::is_resource(&schema))
            })
            .cloned()
            .collect();

        Ok(Self {
            resource_types: resource_types.iter().cloned().collect(),
            advertised_create,
            schema_resources,
        })
    }

    pub(crate) fn classify(&self, resource_type: &str) -> Result<(), CreateTargetBlock> {
        if !self.resource_types.contains(resource_type) {
            return Err(CreateTargetBlock::InvalidType);
        }
        if !self.advertised_create.contains(resource_type) {
            return Err(CreateTargetBlock::CreateNotAdvertised);
        }
        if !self.schema_resources.contains(resource_type) {
            return Err(CreateTargetBlock::SchemaUnavailable);
        }
        Ok(())
    }

    pub(crate) fn resource_types_csv(&self) -> String {
        sorted_csv(&self.resource_types)
    }

    pub(crate) fn advertised_create_csv(&self) -> String {
        sorted_csv(&self.advertised_create)
    }

    pub(crate) fn schema_resources_csv(&self) -> String {
        sorted_csv(&self.schema_resources)
    }
}

fn sorted_csv(values: &HashSet<String>) -> String {
    let mut values: Vec<&str> = values.iter().map(String::as_str).collect();
    values.sort_unstable();
    values.join(",")
}

/// The server-summary block: identity fields lifted off the statement root.
pub(crate) struct CapabilitySummary {
    pub description: String,
    pub implementation_url: String,
    pub fhir_version: String,
    pub status: String,
    pub kind: String,
    pub date: String,
    pub formats: Vec<String>,
}

/// One system-level interaction (`rest[0].interaction`). `transaction` is
/// advertised only when the backend supports atomicity while `batch` is
/// unconditional — the flag lets the template say so instead of rendering an
/// undifferentiated list.
pub(crate) struct SystemInteraction {
    pub code: String,
    pub conditional: bool,
}

/// One server operation (`rest[0].operation`), with its OperationDefinition
/// link when the definition points at this server.
pub(crate) struct OperationRow {
    pub name: String,
    /// A same-server `/OperationDefinition/{id}` path, when the canonical
    /// resolves locally; external canonicals render as plain text.
    pub definition_path: String,
    pub definition: String,
}

/// One per-resource row (`rest[0].resource[]`).
pub(crate) struct ResourceRow {
    pub resource_type: String,
    pub interactions: Vec<String>,
    pub search_param_count: usize,
    pub include_count: usize,
    pub revinclude_count: usize,
}

pub(crate) struct CapabilityView {
    pub summary: CapabilitySummary,
    pub interactions: Vec<SystemInteraction>,
    pub operations: Vec<OperationRow>,
    pub resources: Vec<ResourceRow>,
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    let mut cur = value;
    for p in path {
        match cur.get(p) {
            Some(v) => cur = v,
            None => return "",
        }
    }
    cur.as_str().unwrap_or("")
}

fn arr<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Projects the raw CapabilityStatement into the page's view. Defensive over
/// the JSON: absent fields render empty, never panic — the statement shape
/// varies with enabled features and FHIR version.
pub(crate) fn build_view(statement: &Value) -> CapabilityView {
    let rest = arr(statement, "rest")
        .first()
        .cloned()
        .unwrap_or(Value::Null);

    let summary = CapabilitySummary {
        description: str_at(statement, &["implementation", "description"]).to_string(),
        implementation_url: str_at(statement, &["implementation", "url"]).to_string(),
        fhir_version: str_at(statement, &["fhirVersion"]).to_string(),
        status: str_at(statement, &["status"]).to_string(),
        kind: str_at(statement, &["kind"]).to_string(),
        date: str_at(statement, &["date"]).to_string(),
        formats: arr(statement, "format")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    };

    let interactions = arr(&rest, "interaction")
        .iter()
        .map(|i| {
            let code = str_at(i, &["code"]).to_string();
            SystemInteraction {
                // `transaction` only appears when the backend is atomic;
                // `batch` always does. Mark the conditional one.
                conditional: code == "transaction",
                code,
            }
        })
        .collect();

    let operations = arr(&rest, "operation")
        .iter()
        .map(|o| {
            let definition = str_at(o, &["definition"]).to_string();
            // A canonical of this server's own OperationDefinition gets a
            // relative, clickable path; foreign canonicals stay text.
            let definition_path = definition
                .find("/OperationDefinition/")
                .map(|i| definition[i..].to_string())
                .unwrap_or_default();
            OperationRow {
                name: str_at(o, &["name"]).to_string(),
                definition_path,
                definition,
            }
        })
        .collect();

    let resources = arr(&rest, "resource")
        .iter()
        .map(|r| ResourceRow {
            resource_type: str_at(r, &["type"]).to_string(),
            interactions: arr(r, "interaction")
                .iter()
                .map(|i| str_at(i, &["code"]).to_string())
                .collect(),
            search_param_count: arr(r, "searchParam").len(),
            include_count: arr(r, "searchInclude").len(),
            revinclude_count: arr(r, "searchRevInclude").len(),
        })
        .collect();

    CapabilityView {
        summary,
        interactions,
        operations,
        resources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_the_statement_defensively() {
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "status": "active", "kind": "instance", "date": "2026-08-24",
            "fhirVersion": "4.0.1",
            "format": ["application/fhir+json"],
            "implementation": {"description": "Helios FHIR Server", "url": "http://x/"},
            "rest": [{
                "mode": "server",
                "interaction": [{"code": "batch"}, {"code": "transaction"}],
                "operation": [
                    {"name": "export", "definition": "http://x/OperationDefinition/export"},
                    {"name": "viewdef", "definition": "http://sql-on-fhir.org/OperationDefinition/run"}
                ],
                "resource": [{
                    "type": "Patient",
                    "interaction": [{"code": "read"}, {"code": "search-type"}],
                    "searchParam": [{"name": "name"}, {"name": "birthdate"}],
                    "searchInclude": ["Patient:organization"]
                }]
            }]
        });
        let view = build_view(&statement);
        assert_eq!(view.summary.fhir_version, "4.0.1");
        assert_eq!(view.summary.formats.len(), 1);
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "transaction" && i.conditional)
        );
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "batch" && !i.conditional)
        );
        // Any /OperationDefinition/{id} canonical links relatively: the
        // statement this server emits advertises its own definitions, and
        // GET /OperationDefinition/{id} is routed.
        assert_eq!(
            view.operations[0].definition_path,
            "/OperationDefinition/export"
        );
        assert_eq!(
            view.operations[1].definition_path,
            "/OperationDefinition/run"
        );
        assert_eq!(view.resources[0].search_param_count, 2);
        assert_eq!(view.resources[0].include_count, 1);
        assert_eq!(view.resources[0].revinclude_count, 0);

        // An empty statement renders empty, never panics.
        let empty = build_view(&json!({}));
        assert!(empty.resources.is_empty());
        assert!(empty.summary.fhir_version.is_empty());
    }

    #[test]
    #[cfg(feature = "R4")]
    fn create_targets_require_catalog_capability_and_resource_schema() {
        let types = vec!["Patient".to_string(), "Observation".to_string()];
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "fhirVersion": "4.0.1",
            "rest": [{"mode": "server", "resource": [
                {"type": "Patient", "interaction": [{"code": "create"}]},
                {"type": "Observation", "interaction": [{"code": "read"}]},
                {"type": "HumanName", "interaction": [{"code": "create"}]}
            ]}]
        });
        let targets = CreateTargets::from_statement(&types, &statement, FhirVersion::R4).unwrap();

        assert_eq!(targets.classify("Patient"), Ok(()));
        assert_eq!(
            targets.classify("Observation"),
            Err(CreateTargetBlock::CreateNotAdvertised)
        );
        assert_eq!(
            targets.classify("patient"),
            Err(CreateTargetBlock::InvalidType)
        );
        assert_eq!(
            targets.classify("HumanName"),
            Err(CreateTargetBlock::InvalidType)
        );
    }

    #[test]
    fn classifies_an_advertised_type_without_an_editor_schema() {
        let targets = CreateTargets {
            resource_types: HashSet::from(["CustomResource".to_string()]),
            advertised_create: HashSet::from(["CustomResource".to_string()]),
            schema_resources: HashSet::new(),
        };
        assert_eq!(
            targets.classify("CustomResource"),
            Err(CreateTargetBlock::SchemaUnavailable)
        );
    }

    fn assert_core_target(version: FhirVersion, resource_type: &str) {
        let types = vec![resource_type.to_string()];
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "fhirVersion": version.full_version(),
            "rest": [{"mode": "server", "resource": [{
                "type": resource_type,
                "interaction": [{"code": "create"}]
            }]}]
        });
        assert_eq!(
            CreateTargets::from_statement(&types, &statement, version)
                .unwrap()
                .classify(resource_type),
            Ok(())
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn rejects_malformed_wrong_version_and_non_server_metadata() {
        let types = vec!["Patient".to_string()];
        for statement in [
            json!({}),
            json!({
                "resourceType": "Bundle",
                "fhirVersion": "4.0.1",
                "rest": [{"mode": "server"}]
            }),
            json!({
                "resourceType": "CapabilityStatement",
                "rest": [{"mode": "server"}]
            }),
            json!({
                "resourceType": "CapabilityStatement",
                "fhirVersion": "5.0.0",
                "rest": [{"mode": "server"}]
            }),
            json!({
                "resourceType": "CapabilityStatement",
                "fhirVersion": "4.0.1",
                "rest": [{"resource": []}]
            }),
            json!({
                "resourceType": "CapabilityStatement",
                "fhirVersion": "4.0.1",
                "rest": [{"mode": "", "resource": []}]
            }),
            json!({
                "resourceType": "CapabilityStatement",
                "fhirVersion": "4.0.1",
                "rest": [{"mode": "client", "resource": [{
                    "type": "Patient", "interaction": [{"code": "create"}]
                }]}]
            }),
        ] {
            assert!(
                CreateTargets::from_statement(&types, &statement, FhirVersion::R4).is_err(),
                "metadata should be rejected: {statement}"
            );
        }
    }

    #[test]
    #[cfg(feature = "R4")]
    fn ignores_create_interactions_from_client_rest_components() {
        let types = vec!["Patient".to_string(), "Observation".to_string()];
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "fhirVersion": "4.0.1",
            "rest": [
                {"mode": "client", "resource": [{
                    "type": "Observation", "interaction": [{"code": "create"}]
                }]},
                {"mode": "server", "resource": [{
                    "type": "Patient", "interaction": [{"code": "create"}]},
                    {"type": "Observation", "interaction": [{"code": "read"}]}
                ]}
            ]
        });
        let targets = CreateTargets::from_statement(&types, &statement, FhirVersion::R4).unwrap();
        assert_eq!(targets.classify("Patient"), Ok(()));
        assert_eq!(
            targets.classify("Observation"),
            Err(CreateTargetBlock::CreateNotAdvertised)
        );
    }

    #[test]
    fn representative_version_specific_resources_resolve_in_their_pack() {
        #[cfg(feature = "R4")]
        assert_core_target(FhirVersion::R4, "Media");
        #[cfg(feature = "R4B")]
        assert_core_target(FhirVersion::R4B, "Media");
        #[cfg(feature = "R5")]
        assert_core_target(FhirVersion::R5, "ActorDefinition");
        #[cfg(feature = "R6")]
        assert_core_target(FhirVersion::R6, "ActorDefinition");
    }
}
