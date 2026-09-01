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

/// One system-level interaction (`rest[0].interaction`).
pub(crate) struct SystemInteraction {
    pub code: String,
    pub href: String,
    pub tag_class: &'static str,
}

/// One server operation (`rest[0].operation`).
pub(crate) struct OperationRow {
    pub name: String,
    /// An absolute HTTP(S) canonical, without its optional `|version` suffix.
    /// Empty when the advertised definition is not safe to navigate to.
    pub definition_href: String,
    pub definition: String,
}

/// An interaction code and the semantic tag style used across UI tables.
pub(crate) struct InteractionTag {
    pub code: String,
    pub tag_class: &'static str,
}

/// One per-resource row (`rest[0].resource[]`).
pub(crate) struct ResourceRow {
    pub resource_type: String,
    /// A safe advertised profile, or the version's official core resource
    /// page. Empty only when neither is safe to construct.
    pub resource_href: String,
    pub interactions: Vec<InteractionTag>,
    pub search_param_count: usize,
    pub include_count: usize,
    pub revinclude_count: usize,
}

pub(crate) struct CapabilityView {
    pub summary: CapabilitySummary,
    pub interactions: Vec<SystemInteraction>,
    pub operations: Vec<OperationRow>,
    pub resources: Vec<ResourceRow>,
    /// The explanatory backend-role note is rendered once, even if malformed
    /// metadata advertises the transaction interaction more than once.
    pub has_conditional_transaction: bool,
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
pub(crate) fn build_view(statement: &Value, version: FhirVersion) -> CapabilityView {
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

    let interactions: Vec<SystemInteraction> = arr(&rest, "interaction")
        .iter()
        .map(|i| {
            let code = str_at(i, &["code"]).to_string();
            SystemInteraction {
                href: system_interaction_href(version, &code).unwrap_or_default(),
                tag_class: interaction_tag_class(&code),
                code,
            }
        })
        .collect();
    let has_conditional_transaction = interactions.iter().any(|i| i.code == "transaction");

    let operations = arr(&rest, "operation")
        .iter()
        .map(|o| {
            let definition = str_at(o, &["definition"]).to_string();
            OperationRow {
                name: str_at(o, &["name"]).to_string(),
                definition_href: safe_canonical_href(&definition).unwrap_or_default(),
                definition,
            }
        })
        .collect();

    let registry = packs::core_registry(version);
    let resources = arr(&rest, "resource")
        .iter()
        .map(|r| {
            let resource_type = str_at(r, &["type"]).to_string();
            let advertised_profile = str_at(r, &["profile"]);
            let is_core_resource = registry
                .resolve(&resource_type)
                .is_some_and(|schema| editor::is_resource(&schema));
            let safe_profile = safe_canonical_href(advertised_profile);
            let resource_href = if is_core_resource
                && (safe_profile.is_none()
                    || is_core_resource_canonical(advertised_profile, &resource_type))
            {
                resource_definition_href(version, &resource_type)
            } else {
                safe_profile.unwrap_or_default()
            };
            ResourceRow {
                resource_type,
                resource_href,
                interactions: arr(r, "interaction")
                    .iter()
                    .map(|i| {
                        let code = str_at(i, &["code"]).to_string();
                        InteractionTag {
                            tag_class: interaction_tag_class(&code),
                            code,
                        }
                    })
                    .collect(),
                search_param_count: arr(r, "searchParam").len(),
                include_count: arr(r, "searchInclude").len(),
                revinclude_count: arr(r, "searchRevInclude").len(),
            }
        })
        .collect();

    CapabilityView {
        summary,
        interactions,
        operations,
        resources,
        has_conditional_transaction,
    }
}

fn fhir_docs_root(version: FhirVersion) -> &'static str {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => "https://hl7.org/fhir/R4/",
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => "https://hl7.org/fhir/R4B/",
        #[cfg(feature = "R5")]
        FhirVersion::R5 => "https://hl7.org/fhir/R5/",
        #[cfg(feature = "R6")]
        FhirVersion::R6 => "https://hl7.org/fhir/6.0.0-ballot4/",
    }
}

fn system_interaction_href(version: FhirVersion, code: &str) -> Option<String> {
    let fragment = match code {
        "transaction" => "http.html#transaction",
        "batch" => "http.html#batch",
        "search-system" => "http.html#search",
        "history-system" => "http.html#history",
        _ => return None,
    };
    Some(format!("{}{fragment}", fhir_docs_root(version)))
}

fn resource_definition_href(version: FhirVersion, resource_type: &str) -> String {
    // Callers validate the type against the version's schema registry before
    // deriving a path. Core resource pages use lowercase ASCII slugs.
    format!(
        "{}{resource_slug}.html",
        fhir_docs_root(version),
        resource_slug = resource_type.to_ascii_lowercase()
    )
}

/// Whether an advertised profile is the exact HL7 core canonical for the
/// advertised resource type. A FHIR canonical may carry one `|version`
/// qualifier, which does not change the profile identity.
fn is_core_resource_canonical(canonical: &str, resource_type: &str) -> bool {
    let (base, version) = canonical
        .split_once('|')
        .map_or((canonical, None), |(base, version)| (base, Some(version)));
    if version.is_some_and(|version| {
        version.is_empty()
            || version.contains('|')
            || version.contains('?')
            || version.contains('#')
    }) {
        return false;
    }

    base == format!("http://hl7.org/fhir/StructureDefinition/{resource_type}")
        || base == format!("https://hl7.org/fhir/StructureDefinition/{resource_type}")
}

/// Returns an absolute HTTP(S) canonical stripped of its optional FHIR
/// `|version` qualifier. Everything else stays visible as plain text.
fn safe_canonical_href(canonical: &str) -> Option<String> {
    let fragment_start = canonical.find('#').unwrap_or(canonical.len());
    let before_fragment = &canonical[..fragment_start];
    let fragment = &canonical[fragment_start..];
    let base = before_fragment
        .split_once('|')
        .map_or(before_fragment, |(url, _)| url);
    let raw = format!("{base}{fragment}");
    let parsed = reqwest::Url::parse(&raw).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.has_host())
        .filter(|has_host| *has_host)
        .map(|_| raw)
}

fn interaction_tag_class(code: &str) -> &'static str {
    match code {
        "read" | "vread" | "search-type" | "search-system" | "history-instance"
        | "history-type" | "history-system" => "tag--member",
        "create" | "update" | "patch" | "batch" | "transaction" => "tag--config",
        "delete" => "tag--excluded",
        _ => "tag--muted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[cfg(feature = "R4")]
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
        let view = build_view(&statement, FhirVersion::R4);
        assert_eq!(view.summary.fhir_version, "4.0.1");
        assert_eq!(view.summary.formats.len(), 1);
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "transaction" && i.tag_class == "tag--config")
        );
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "batch" && i.tag_class == "tag--config")
        );
        assert_eq!(
            view.operations[0].definition_href,
            "http://x/OperationDefinition/export"
        );
        assert_eq!(
            view.operations[1].definition_href,
            "http://sql-on-fhir.org/OperationDefinition/run"
        );
        assert!(view.has_conditional_transaction);
        assert_eq!(
            view.resources[0].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(view.resources[0].search_param_count, 2);
        assert_eq!(view.resources[0].include_count, 1);
        assert_eq!(view.resources[0].revinclude_count, 0);

        // An empty statement renders empty, never panics.
        let empty = build_view(&json!({}), FhirVersion::R4);
        assert!(empty.resources.is_empty());
        assert!(empty.summary.fhir_version.is_empty());
    }

    #[test]
    fn fhir_documentation_roots_are_version_aware() {
        #[cfg(feature = "R4")]
        assert_eq!(fhir_docs_root(FhirVersion::R4), "https://hl7.org/fhir/R4/");
        #[cfg(feature = "R4B")]
        assert_eq!(
            fhir_docs_root(FhirVersion::R4B),
            "https://hl7.org/fhir/R4B/"
        );
        #[cfg(feature = "R5")]
        assert_eq!(fhir_docs_root(FhirVersion::R5), "https://hl7.org/fhir/R5/");
        #[cfg(feature = "R6")]
        assert_eq!(
            fhir_docs_root(FhirVersion::R6),
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
    fn system_interactions_have_exact_links_and_semantic_classes() {
        let statement = json!({"rest": [{"interaction": [
            {"code": "transaction"}, {"code": "batch"},
            {"code": "search-system"}, {"code": "history-system"},
            {"code": "read"}, {"code": "delete"}, {"code": "future-code"}
        ]}]});
        let view = build_view(&statement, FhirVersion::R4);

        let link = |code: &str| {
            view.interactions
                .iter()
                .find(|interaction| interaction.code == code)
                .map(|interaction| interaction.href.as_str())
                .unwrap()
        };
        assert_eq!(
            link("transaction"),
            "https://hl7.org/fhir/R4/http.html#transaction"
        );
        assert_eq!(link("batch"), "https://hl7.org/fhir/R4/http.html#batch");
        assert_eq!(
            link("search-system"),
            "https://hl7.org/fhir/R4/http.html#search"
        );
        assert_eq!(
            link("history-system"),
            "https://hl7.org/fhir/R4/http.html#history"
        );
        assert_eq!(link("read"), "");
        assert_eq!(link("future-code"), "");

        let class = |code: &str| {
            view.interactions
                .iter()
                .find(|interaction| interaction.code == code)
                .map(|interaction| interaction.tag_class)
                .unwrap()
        };
        assert_eq!(class("transaction"), "tag--config");
        assert_eq!(class("read"), "tag--member");
        assert_eq!(class("delete"), "tag--excluded");
        assert_eq!(class("future-code"), "tag--muted");
    }

    #[test]
    fn system_interaction_links_follow_each_enabled_version_root() {
        fn assert_links(version: FhirVersion, root: &str) {
            for (code, fragment) in [
                ("transaction", "http.html#transaction"),
                ("batch", "http.html#batch"),
                ("search-system", "http.html#search"),
                ("history-system", "http.html#history"),
            ] {
                assert_eq!(
                    system_interaction_href(version, code),
                    Some(format!("{root}{fragment}"))
                );
            }
            assert_eq!(system_interaction_href(version, "unknown"), None);
        }

        #[cfg(feature = "R4")]
        assert_links(FhirVersion::R4, "https://hl7.org/fhir/R4/");
        #[cfg(feature = "R4B")]
        assert_links(FhirVersion::R4B, "https://hl7.org/fhir/R4B/");
        #[cfg(feature = "R5")]
        assert_links(FhirVersion::R5, "https://hl7.org/fhir/R5/");
        #[cfg(feature = "R6")]
        assert_links(FhirVersion::R6, "https://hl7.org/fhir/6.0.0-ballot4/");
    }

    #[test]
    fn interaction_codes_use_the_semantic_tag_palette() {
        for code in [
            "read",
            "vread",
            "search-type",
            "search-system",
            "history-instance",
            "history-type",
            "history-system",
        ] {
            assert_eq!(interaction_tag_class(code), "tag--member", "{code}");
        }
        for code in ["create", "update", "patch", "batch", "transaction"] {
            assert_eq!(interaction_tag_class(code), "tag--config", "{code}");
        }
        assert_eq!(interaction_tag_class("delete"), "tag--excluded");
        assert_eq!(interaction_tag_class("future-code"), "tag--muted");
    }

    #[test]
    fn canonicals_are_safe_and_strip_the_fhir_version_qualifier() {
        assert_eq!(
            safe_canonical_href("https://example.org/OperationDefinition/export|1.2.3"),
            Some("https://example.org/OperationDefinition/export".to_string())
        );
        assert_eq!(
            safe_canonical_href("http://example.org/StructureDefinition/Patient"),
            Some("http://example.org/StructureDefinition/Patient".to_string())
        );
        assert_eq!(
            safe_canonical_href("https://example.org/Profile|1.0#details"),
            Some("https://example.org/Profile#details".to_string())
        );
        assert_eq!(
            safe_canonical_href("https://example.org/Profile#details|part"),
            Some("https://example.org/Profile#details|part".to_string())
        );
        for unsafe_value in [
            "OperationDefinition/export",
            "/OperationDefinition/export",
            "javascript:alert(1)",
            "urn:oid:1.2.3",
            "https://",
            "not a URL",
        ] {
            assert_eq!(safe_canonical_href(unsafe_value), None, "{unsafe_value}");
        }
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
    #[cfg(feature = "R4")]
    fn core_profile_near_misses_are_not_rewritten() {
        let profiles = [
            "https://hl7.org.example.com/fhir/StructureDefinition/Patient",
            "https://hl7.org/fhir/StructureDefinition/Observation",
            "https://hl7.org/fhir/StructureDefinition/Patient?mode=custom",
            "https://hl7.org/fhir/StructureDefinition/Patient#custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1#custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1?mode=custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1|extra",
        ];
        let statement = json!({"rest": [{"resource": profiles.map(|profile| json!({
            "type": "Patient",
            "profile": profile
        }))}]});
        let view = build_view(&statement, FhirVersion::R4);

        for (row, profile) in view.resources.iter().zip(profiles) {
            assert_eq!(
                row.resource_href,
                safe_canonical_href(profile).unwrap(),
                "{profile}"
            );
        }
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
    fn duplicate_transactions_produce_one_note_state() {
        let statement = json!({"rest": [{"interaction": [
            {"code": "transaction"}, {"code": "transaction"}
        ]}]});
        let view = build_view(&statement, FhirVersion::R4);
        assert!(view.has_conditional_transaction);
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
