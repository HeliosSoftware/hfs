//! Read model for the CapabilityStatement page (`/ui/capability-statement`,
//! #653).
//!
//! `/metadata` composes the statement fresh on every request from live server
//! state — registered search parameters, backend capabilities, enabled
//! features — so there is nothing stored to edit and the page is genuinely
//! read-only. The fetch rides the same loopback self-call as the other
//! conformance viewers ([`crate::conformance`]); a failed fetch degrades to a
//! warning, never to fabricated capabilities.
//!
//! The projection and the cards themselves live in
//! [`helios_ui_chrome::capability`] (#808) — HTS renders the same document
//! from the same code, so a fix lands once. What stays here is what only HFS
//! can supply: the [`FhirVersion`] → [`DocsVersion`] mapping, the core schema
//! pack that decides which resource types have an official page in the
//! release being rendered, and [`CreateTargets`], which is not part of the
//! page at all — it is how the Resources workspace decides whether it may
//! offer to create a type.

use helios_fhir::FhirVersion;
use helios_fhir_validator::{SchemaResolver, editor, packs};
use helios_ui_chrome::capability::{
    CapabilityView, CoreResourceCatalog, DocsVersion, build_view as chrome_build_view,
};
use serde_json::Value;
use std::collections::HashSet;

/// Read a string down a fixed path, or `""`. `CreateTargets` reads the raw
/// statement rather than the projected view: it needs `resourceType` and
/// `rest[].mode`, which the page never renders and the shared view therefore
/// does not carry.
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

/// The FHIR release the shared cards should link into.
///
/// A total `match`, not a string round-trip: `FhirVersion`'s variants are
/// feature-gated, so this is the one place that has to know which releases
/// this build actually carries — and adding a release makes it fail to
/// compile rather than silently linking at the wrong specification.
fn docs_version(version: FhirVersion) -> DocsVersion {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => DocsVersion::R4,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => DocsVersion::R4B,
        #[cfg(feature = "R5")]
        FhirVersion::R5 => DocsVersion::R5,
        #[cfg(feature = "R6")]
        FhirVersion::R6 => DocsVersion::R6,
    }
}

/// The release's core schema pack, answering the shared projection's one
/// open question: does this resource type have an official page in *this*
/// release?
///
/// It has to be the pack rather than a hard-coded list, because the answer
/// moves between releases — `DocumentManifest` is R4/R4B only,
/// `ActorDefinition` is R5/R6 only — and #797 was exactly the bug of getting
/// that wrong.
struct CorePack(std::sync::Arc<helios_fhir_validator::SchemaRegistry>);

impl CoreResourceCatalog for CorePack {
    fn is_core_resource(&self, resource_type: &str) -> bool {
        self.0
            .resolve(resource_type)
            .is_some_and(|schema| editor::is_resource(&schema))
    }
}

/// Projects the raw CapabilityStatement into the page's view.
///
/// A thin adapter over [`helios_ui_chrome::capability::build_view`]: this
/// crate contributes the release mapping and the resource catalog, and the
/// projection itself is the one HTS renders too (#808).
pub(crate) fn build_view(statement: &Value, version: FhirVersion) -> CapabilityView {
    chrome_build_view(
        statement,
        docs_version(version),
        &CorePack(packs::core_registry(version)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shared crate proves each [`DocsVersion`] links at the right
    /// documentation root; this proves HFS hands it the right one. Both
    /// halves are needed — #797 was a wrong root, and a wrong *mapping*
    /// would look identical to the operator.
    #[test]
    fn every_enabled_release_maps_onto_its_own_documentation_root() {
        #[cfg(feature = "R4")]
        assert_eq!(
            docs_version(FhirVersion::R4).docs_root(),
            "https://hl7.org/fhir/R4/"
        );
        #[cfg(feature = "R4B")]
        assert_eq!(
            docs_version(FhirVersion::R4B).docs_root(),
            "https://hl7.org/fhir/R4B/"
        );
        #[cfg(feature = "R5")]
        assert_eq!(
            docs_version(FhirVersion::R5).docs_root(),
            "https://hl7.org/fhir/R5/"
        );
        #[cfg(feature = "R6")]
        assert_eq!(
            docs_version(FhirVersion::R6).docs_root(),
            "https://hl7.org/fhir/6.0.0-ballot4/"
        );
    }

    #[test]
    fn hfs_core_profiles_follow_each_enabled_version_root() {
        fn assert_core_profile(version: FhirVersion, expected: &str) {
            let statement = json!({"rest": [{"resource": [{
                "type": "Patient",
                "profile": "http://hl7.org/fhir/StructureDefinition/Patient"
            }]}]});
            let view = build_view(&statement, version);
            assert_eq!(view.resources[0].resource_href, expected);
        }

        #[cfg(feature = "R4")]
        assert_core_profile(FhirVersion::R4, "https://hl7.org/fhir/R4/patient.html");
        #[cfg(feature = "R4B")]
        assert_core_profile(FhirVersion::R4B, "https://hl7.org/fhir/R4B/patient.html");
        #[cfg(feature = "R5")]
        assert_core_profile(FhirVersion::R5, "https://hl7.org/fhir/R5/patient.html");
        #[cfg(feature = "R6")]
        assert_core_profile(
            FhirVersion::R6,
            "https://hl7.org/fhir/6.0.0-ballot4/patient.html",
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn core_profiles_are_versioned_while_custom_profiles_are_preserved() {
        let statement = json!({"rest": [{"resource": [
            {"type": "Patient", "profile": "http://hl7.org/fhir/StructureDefinition/Patient"},
            {"type": "Patient", "profile": "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1"},
            {"type": "Patient", "profile": "https://example.org/Patient|2.0"},
            {"type": "Observation", "profile": "javascript:alert(1)"},
            {"type": "Encounter"},
            {"type": "NotARealResource", "profile": "https://example.org/custom"},
            {"type": "UnknownUnsafe", "profile": "javascript:alert(2)"},
            {"type": "UnknownMissing"},
            {"type": "<script>"}
        ]}]});
        let view = build_view(&statement, FhirVersion::R4);

        assert_eq!(
            view.resources[0].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(
            view.resources[1].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(
            view.resources[2].resource_href,
            "https://example.org/Patient"
        );
        assert_eq!(
            view.resources[3].resource_href,
            "https://hl7.org/fhir/R4/observation.html"
        );
        assert_eq!(
            view.resources[4].resource_href,
            "https://hl7.org/fhir/R4/encounter.html"
        );
        assert_eq!(
            view.resources[5].resource_href,
            "https://example.org/custom"
        );
        assert!(view.resources[6].resource_href.is_empty());
        assert!(view.resources[7].resource_href.is_empty());
        assert!(view.resources[8].resource_href.is_empty());
    }

    #[test]
    fn version_specific_resource_links_require_a_resource_schema() {
        fn assert_link(version: FhirVersion, resource_type: &str, expected: &str) {
            let statement = json!({"rest": [{"resource": [{"type": resource_type}]}]});
            let view = build_view(&statement, version);
            assert_eq!(view.resources[0].resource_href, expected, "{resource_type}");
        }

        #[cfg(feature = "R4")]
        {
            assert_link(
                FhirVersion::R4,
                "DocumentManifest",
                "https://hl7.org/fhir/R4/documentmanifest.html",
            );
            assert_link(
                FhirVersion::R4,
                "Media",
                "https://hl7.org/fhir/R4/media.html",
            );
            assert_link(FhirVersion::R4, "SubscriptionTopic", "");
            assert_link(FhirVersion::R4, "ActorDefinition", "");
        }
        #[cfg(feature = "R4B")]
        {
            assert_link(
                FhirVersion::R4B,
                "DocumentManifest",
                "https://hl7.org/fhir/R4B/documentmanifest.html",
            );
            assert_link(
                FhirVersion::R4B,
                "SubscriptionTopic",
                "https://hl7.org/fhir/R4B/subscriptiontopic.html",
            );
            assert_link(
                FhirVersion::R4B,
                "Media",
                "https://hl7.org/fhir/R4B/media.html",
            );
            assert_link(FhirVersion::R4B, "ActorDefinition", "");
        }
        #[cfg(feature = "R5")]
        {
            assert_link(
                FhirVersion::R5,
                "SubscriptionTopic",
                "https://hl7.org/fhir/R5/subscriptiontopic.html",
            );
            assert_link(
                FhirVersion::R5,
                "ActorDefinition",
                "https://hl7.org/fhir/R5/actordefinition.html",
            );
            assert_link(FhirVersion::R5, "DocumentManifest", "");
            assert_link(FhirVersion::R5, "Media", "");
        }
        #[cfg(feature = "R6")]
        {
            assert_link(
                FhirVersion::R6,
                "SubscriptionTopic",
                "https://hl7.org/fhir/6.0.0-ballot4/subscriptiontopic.html",
            );
            assert_link(
                FhirVersion::R6,
                "ActorDefinition",
                "https://hl7.org/fhir/6.0.0-ballot4/actordefinition.html",
            );
            assert_link(FhirVersion::R6, "DocumentManifest", "");
            assert_link(FhirVersion::R6, "Media", "");
        }
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
