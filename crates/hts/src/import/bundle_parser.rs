//! Backend-agnostic FHIR Bundle parser.
//!
//! [`parse_bundle`] deserialises a raw FHIR Bundle JSON payload into a
//! [`ParsedBundle`] intermediate representation.  Both the SQLite and
//! PostgreSQL write layers consume this representation, so all JSON-walking
//! logic lives here exactly once.
//!
//! ## What this module does NOT do
//!
//! It does not touch any database.  All DB writes happen in the backend-specific
//! modules (`import/fhir_bundle.rs` for SQLite,
//! `backend/postgres/mod.rs` for PostgreSQL).

use serde_json::Value;
use uuid::Uuid;

use crate::error::HtsError;

// ── Public intermediate types ─────────────────────────────────────────────────

/// A FHIR Bundle parsed into backend-agnostic Rust structs.
///
/// Resources are partitioned in import-dependency order:
/// CodeSystems first, then ValueSets, then ConceptMaps.
///
/// `parse_errors` accumulates non-fatal parse failures (e.g. a CodeSystem
/// missing its `url`).  Write layers should propagate these into
/// [`super::ImportStats::errors`] so callers can return HTTP 207 Multi-Status.
#[derive(Debug, Default)]
pub struct ParsedBundle {
    pub code_systems: Vec<ParsedCodeSystem>,
    pub value_sets: Vec<ParsedValueSet>,
    pub concept_maps: Vec<ParsedConceptMap>,
    /// Non-fatal parse errors.  Resources with errors are skipped; the rest
    /// of the bundle continues to be imported.
    pub parse_errors: Vec<String>,
}

/// A single FHIR CodeSystem resource extracted from a Bundle entry.
#[derive(Debug)]
pub struct ParsedCodeSystem {
    pub id: String,
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub content: String,
    /// Flat list of all concepts in the system (including nested descendants).
    /// Each entry carries `parent_code` so the hierarchy can be reconstructed.
    pub concepts: Vec<ParsedConcept>,
    /// The full original JSON resource (stored in `resource_json` column).
    pub resource_json: Value,
}

/// One concept row, already flattened from the potentially nested FHIR tree.
///
/// `parent_code` is `Some` when the concept was either nested under another
/// concept **or** carried a `property[{code:"parent", valueCode:"X"}]` entry.
#[derive(Debug)]
pub struct ParsedConcept {
    pub code: String,
    pub display: Option<String>,
    pub definition: Option<String>,
    pub properties: Vec<ParsedProperty>,
    pub designations: Vec<ParsedDesignation>,
    /// The direct parent code, if any.  Used to populate `concept_hierarchy`.
    pub parent_code: Option<String>,
}

/// A single FHIR concept property (e.g. `inactive`, `parent`).
#[derive(Debug)]
pub struct ParsedProperty {
    pub code: String,
    pub value_type: String,
    pub value: String,
    /// Set to `true` when this property encodes a hierarchy edge (`code == "parent"`).
    pub is_parent_edge: bool,
    /// The parent code for the hierarchy edge (only meaningful when `is_parent_edge`).
    pub parent_code_value: Option<String>,
}

/// A single FHIR concept designation (alternate name or translation).
#[derive(Debug)]
pub struct ParsedDesignation {
    pub language: Option<String>,
    pub use_system: Option<String>,
    pub use_code: Option<String>,
    pub value: String,
}

/// A single FHIR ValueSet resource extracted from a Bundle entry.
#[derive(Debug)]
pub struct ParsedValueSet {
    pub id: String,
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: String,
    /// The `compose` element serialised back to JSON, stored verbatim for
    /// lazy expansion.  `None` when the ValueSet has no `compose`.
    pub compose_json: Option<String>,
    pub resource_json: Value,
}

/// A single FHIR ConceptMap resource extracted from a Bundle entry.
#[derive(Debug)]
pub struct ParsedConceptMap {
    pub id: String,
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub source_uri: Option<String>,
    pub target_uri: Option<String>,
    pub status: String,
    /// Flat list of all source→target mappings across all groups.
    pub elements: Vec<ParsedMapElement>,
    pub resource_json: Value,
}

/// One row in `concept_map_elements`.
#[derive(Debug)]
pub struct ParsedMapElement {
    pub source_system: String,
    pub source_code: String,
    pub target_system: String,
    pub target_code: String,
    pub equivalence: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a raw FHIR Bundle JSON payload into a [`ParsedBundle`].
///
/// Validates that the root `resourceType` is `"Bundle"`.  Partitions entries
/// into CodeSystems, ValueSets, and ConceptMaps (in that order, to match
/// import-dependency order).  Unknown resource types are logged at `debug`
/// level and skipped.
///
/// # Errors
///
/// Returns [`HtsError::InvalidRequest`] when the bytes are not valid JSON or
/// the root resource is not a Bundle.
pub fn parse_bundle(data: &[u8]) -> Result<ParsedBundle, HtsError> {
    let root: Value = serde_json::from_slice(data)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid JSON: {e}")))?;

    let resource_type = root["resourceType"].as_str().unwrap_or("");
    if resource_type != "Bundle" {
        return Err(HtsError::InvalidRequest(format!(
            "Expected a FHIR Bundle resource, got '{resource_type}'"
        )));
    }

    let mut bundle = ParsedBundle::default();

    let entries = root["entry"].as_array().cloned().unwrap_or_default();
    for entry in &entries {
        let resource = &entry["resource"];
        if resource.is_null() {
            continue;
        }
        let rid = resource["id"].as_str().unwrap_or("<no-id>");
        match resource["resourceType"].as_str() {
            Some("CodeSystem") => match parse_code_system(resource) {
                Some(cs) => bundle.code_systems.push(cs),
                None => bundle.parse_errors.push(format!(
                    "CodeSystem '{rid}' skipped: missing required field 'url'"
                )),
            },
            Some("ValueSet") => match parse_value_set(resource) {
                Some(vs) => bundle.value_sets.push(vs),
                None => bundle.parse_errors.push(format!(
                    "ValueSet '{rid}' skipped: missing required field 'url'"
                )),
            },
            Some("ConceptMap") => match parse_concept_map(resource) {
                Some(cm) => bundle.concept_maps.push(cm),
                None => bundle.parse_errors.push(format!(
                    "ConceptMap '{rid}' skipped: missing required field 'url'"
                )),
            },
            Some(rt) => {
                tracing::debug!(
                    resource_type = rt,
                    "Skipping unsupported resource type in Bundle"
                );
            }
            None => {}
        }
    }

    Ok(bundle)
}

// ── CodeSystem parsing ────────────────────────────────────────────────────────

/// Parse a single CodeSystem JSON value (e.g. from a CRUD handler).
///
/// Returns `None` when `url` is absent.
pub fn parse_code_system_value(cs: &Value) -> Option<ParsedCodeSystem> {
    parse_code_system(cs)
}

/// Parse a single ValueSet JSON value.
pub fn parse_value_set_value(vs: &Value) -> Option<ParsedValueSet> {
    parse_value_set(vs)
}

/// Parse a single ConceptMap JSON value.
pub fn parse_concept_map_value(cm: &Value) -> Option<ParsedConceptMap> {
    parse_concept_map(cm)
}

fn parse_code_system(cs: &Value) -> Option<ParsedCodeSystem> {
    let url = cs["url"].as_str()?;

    let id = cs["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut concepts: Vec<ParsedConcept> = Vec::new();

    if let Some(top_level) = cs["concept"].as_array() {
        flatten_concepts(top_level, None, &mut concepts);
    }

    Some(ParsedCodeSystem {
        id,
        url: url.to_owned(),
        version: cs["version"].as_str().map(str::to_owned),
        name: cs["name"].as_str().map(str::to_owned),
        title: cs["title"].as_str().map(str::to_owned),
        status: cs["status"].as_str().unwrap_or("active").to_owned(),
        content: cs["content"].as_str().unwrap_or("complete").to_owned(),
        concepts,
        resource_json: cs.clone(),
    })
}

/// Recursively flatten a nested concept tree into a flat `Vec<ParsedConcept>`.
///
/// `parent_code` is propagated to each child via the nesting relationship.
/// A `"parent"` property on a concept also generates a hierarchy edge to the
/// property's `valueCode` (in addition to or instead of the nesting edge).
fn flatten_concepts(
    concepts: &[Value],
    nesting_parent: Option<&str>,
    out: &mut Vec<ParsedConcept>,
) {
    for concept_val in concepts {
        let code = match concept_val["code"].as_str() {
            Some(c) => c.to_owned(),
            None => continue, // skip malformed
        };

        let display = concept_val["display"].as_str().map(str::to_owned);
        let definition = concept_val["definition"].as_str().map(str::to_owned);

        // Parse properties; collect any additional parent-edge codes.
        let mut properties: Vec<ParsedProperty> = Vec::new();
        let mut property_parent: Option<String> = None;

        if let Some(props) = concept_val["property"].as_array() {
            for prop in props {
                if let Some(p) = parse_property(prop) {
                    if p.is_parent_edge {
                        if let Some(ref pv) = p.parent_code_value {
                            property_parent = Some(pv.clone());
                        }
                    }
                    properties.push(p);
                }
            }
        }

        // Parse designations.
        let mut designations: Vec<ParsedDesignation> = Vec::new();
        if let Some(desigs) = concept_val["designation"].as_array() {
            for desig in desigs {
                if let Some(d) = parse_designation(desig) {
                    designations.push(d);
                }
            }
        }

        // Determine effective parent code: nesting takes precedence; property
        // parent is used when there is no nesting parent.
        let parent_code = nesting_parent.map(str::to_owned).or(property_parent);

        out.push(ParsedConcept {
            code: code.clone(),
            display,
            definition,
            properties,
            designations,
            parent_code,
        });

        // Recurse into nested children.
        if let Some(children) = concept_val["concept"].as_array() {
            flatten_concepts(children, Some(&code), out);
        }
    }
}

fn parse_property(prop: &Value) -> Option<ParsedProperty> {
    let code = prop["code"].as_str()?.to_owned();
    let (value_type, value) = extract_property_value(prop);
    if value.is_empty() {
        return None;
    }

    let is_parent_edge = code == "parent" && value_type == "code";
    let parent_code_value = if is_parent_edge {
        Some(value.clone())
    } else {
        None
    };

    Some(ParsedProperty {
        code,
        value_type,
        value,
        is_parent_edge,
        parent_code_value,
    })
}

fn parse_designation(desig: &Value) -> Option<ParsedDesignation> {
    let value = desig["value"].as_str()?.to_owned();
    Some(ParsedDesignation {
        language: desig["language"].as_str().map(str::to_owned),
        use_system: desig["use"]["system"].as_str().map(str::to_owned),
        use_code: desig["use"]["code"].as_str().map(str::to_owned),
        value,
    })
}

/// Detect the FHIR property value type from the `valueXxx` fields present
/// in the JSON object and return `(value_type, serialised_value)`.
///
/// Returns `("string", "")` when no recognised value field is found.
pub fn extract_property_value(prop: &Value) -> (String, String) {
    if let Some(v) = prop["valueCode"].as_str() {
        return ("code".into(), v.into());
    }
    if let Some(v) = prop["valueBoolean"].as_bool() {
        return ("boolean".into(), v.to_string());
    }
    if let Some(v) = prop["valueInteger"].as_i64() {
        return ("integer".into(), v.to_string());
    }
    if let Some(v) = prop["valueDecimal"].as_f64() {
        return ("decimal".into(), v.to_string());
    }
    if let Some(v) = prop["valueString"].as_str() {
        return ("string".into(), v.into());
    }
    if let Some(v) = prop["valueDateTime"].as_str() {
        return ("dateTime".into(), v.into());
    }
    if let Some(v) = prop["value"].as_str() {
        return ("string".into(), v.into());
    }
    ("string".into(), String::new())
}

// ── ValueSet parsing ──────────────────────────────────────────────────────────

fn parse_value_set(vs: &Value) -> Option<ParsedValueSet> {
    let url = vs["url"].as_str()?;

    let id = vs["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let compose_json = vs["compose"]
        .as_object()
        .map(|_| serde_json::to_string(&vs["compose"]).unwrap_or_default());

    Some(ParsedValueSet {
        id,
        url: url.to_owned(),
        version: vs["version"].as_str().map(str::to_owned),
        name: vs["name"].as_str().map(str::to_owned),
        title: vs["title"].as_str().map(str::to_owned),
        status: vs["status"].as_str().unwrap_or("active").to_owned(),
        compose_json,
        resource_json: vs.clone(),
    })
}

// ── ConceptMap parsing ────────────────────────────────────────────────────────

fn parse_concept_map(cm: &Value) -> Option<ParsedConceptMap> {
    let url = cm["url"].as_str()?;

    let id = cm["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let source_uri = cm["sourceUri"]
        .as_str()
        .or_else(|| cm["sourceScope"].as_str())
        .map(str::to_owned);

    let target_uri = cm["targetUri"]
        .as_str()
        .or_else(|| cm["targetScope"].as_str())
        .map(str::to_owned);

    let mut elements: Vec<ParsedMapElement> = Vec::new();
    let empty = vec![];
    let groups = cm["group"].as_array().unwrap_or(&empty);

    for group in groups {
        let source_system = group["source"].as_str().unwrap_or("").to_owned();
        let target_system = group["target"].as_str().unwrap_or("").to_owned();
        let group_elements = group["element"].as_array().unwrap_or(&empty);

        for element in group_elements {
            let raw_source_code = element["code"].as_str().unwrap_or("").to_owned();
            let targets = element["target"].as_array().unwrap_or(&empty);

            // HL7 terminology package sometimes encodes multiple source codes as a
            // comma-separated string (e.g. "unconfirmed, provisional"). Split them
            // so each code gets its own row in concept_map_elements.
            let source_codes: Vec<String> = raw_source_code
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();

            for target in targets {
                let target_code = target["code"].as_str().unwrap_or("").to_owned();
                let equivalence = target["equivalence"]
                    .as_str()
                    .unwrap_or("equivalent")
                    .to_owned();

                if raw_source_code.is_empty() || target_code.is_empty() {
                    continue;
                }

                for source_code in &source_codes {
                    elements.push(ParsedMapElement {
                        source_system: source_system.clone(),
                        source_code: source_code.clone(),
                        target_system: target_system.clone(),
                        target_code: target_code.clone(),
                        equivalence: equivalence.clone(),
                    });
                }
            }
        }
    }

    Some(ParsedConceptMap {
        id,
        url: url.to_owned(),
        version: cm["version"].as_str().map(str::to_owned),
        name: cm["name"].as_str().map(str::to_owned),
        title: cm["title"].as_str().map(str::to_owned),
        source_uri,
        target_uri,
        status: cm["status"].as_str().unwrap_or("active").to_owned(),
        elements,
        resource_json: cm.clone(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_bundle() -> &'static [u8] {
        br#"{
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "CodeSystem",
                        "id": "cs1",
                        "url": "http://example.org/cs",
                        "name": "TestCS",
                        "status": "active",
                        "content": "complete",
                        "concept": [
                            {
                                "code": "A", "display": "Alpha",
                                "concept": [
                                    { "code": "B", "display": "Beta",
                                      "concept": [{ "code": "C", "display": "Gamma" }]
                                    }
                                ]
                            }
                        ]
                    }
                },
                {
                    "resource": {
                        "resourceType": "ValueSet",
                        "id": "vs1",
                        "url": "http://example.org/vs",
                        "status": "active",
                        "compose": { "include": [{ "system": "http://example.org/cs" }] }
                    }
                },
                {
                    "resource": {
                        "resourceType": "ConceptMap",
                        "id": "cm1",
                        "url": "http://example.org/cm",
                        "status": "active",
                        "group": [{
                            "source": "http://example.org/src",
                            "target": "http://example.org/tgt",
                            "element": [{
                                "code": "A",
                                "target": [{ "code": "X", "equivalence": "equivalent" }]
                            }]
                        }]
                    }
                }
            ]
        }"#
    }

    #[test]
    fn parse_bundle_partitions_resources_correctly() {
        let parsed = parse_bundle(minimal_bundle()).unwrap();
        assert_eq!(parsed.code_systems.len(), 1);
        assert_eq!(parsed.value_sets.len(), 1);
        assert_eq!(parsed.concept_maps.len(), 1);
    }

    #[test]
    fn code_system_concepts_flattened_with_parents() {
        let parsed = parse_bundle(minimal_bundle()).unwrap();
        let cs = &parsed.code_systems[0];
        // A, B, C — three concepts total
        assert_eq!(cs.concepts.len(), 3);

        let a = cs.concepts.iter().find(|c| c.code == "A").unwrap();
        let b = cs.concepts.iter().find(|c| c.code == "B").unwrap();
        let c = cs.concepts.iter().find(|c| c.code == "C").unwrap();

        assert!(a.parent_code.is_none(), "A is a root concept");
        assert_eq!(b.parent_code.as_deref(), Some("A"));
        assert_eq!(c.parent_code.as_deref(), Some("B"));
    }

    #[test]
    fn value_set_compose_json_extracted() {
        let parsed = parse_bundle(minimal_bundle()).unwrap();
        let vs = &parsed.value_sets[0];
        assert!(vs.compose_json.is_some());
        let compose: serde_json::Value =
            serde_json::from_str(vs.compose_json.as_ref().unwrap()).unwrap();
        assert!(compose["include"].is_array());
    }

    #[test]
    fn concept_map_elements_flattened() {
        let parsed = parse_bundle(minimal_bundle()).unwrap();
        let cm = &parsed.concept_maps[0];
        assert_eq!(cm.elements.len(), 1);
        assert_eq!(cm.elements[0].source_code, "A");
        assert_eq!(cm.elements[0].target_code, "X");
        assert_eq!(cm.elements[0].equivalence, "equivalent");
    }

    #[test]
    fn non_bundle_resource_returns_error() {
        let data = br#"{"resourceType":"Patient","id":"p1"}"#;
        let err = parse_bundle(data).unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    #[test]
    fn invalid_json_returns_error() {
        let err = parse_bundle(b"not json").unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    #[test]
    fn unknown_resource_types_are_skipped() {
        let data = br#"{
            "resourceType": "Bundle", "type": "collection",
            "entry": [{ "resource": { "resourceType": "Patient", "id": "p1" } }]
        }"#;
        let parsed = parse_bundle(data).unwrap();
        assert!(parsed.code_systems.is_empty());
        assert!(parsed.value_sets.is_empty());
        assert!(parsed.concept_maps.is_empty());
    }

    #[test]
    fn property_parent_edge_detected() {
        let data = br#"{
            "resourceType": "Bundle", "type": "collection",
            "entry": [{
                "resource": {
                    "resourceType": "CodeSystem",
                    "url": "http://example.org/cs",
                    "status": "active",
                    "content": "complete",
                    "concept": [{
                        "code": "CHILD",
                        "display": "Child",
                        "property": [{ "code": "parent", "valueCode": "ROOT" }]
                    }]
                }
            }]
        }"#;
        let parsed = parse_bundle(data).unwrap();
        let cs = &parsed.code_systems[0];
        let child = cs.concepts.iter().find(|c| c.code == "CHILD").unwrap();
        assert_eq!(child.parent_code.as_deref(), Some("ROOT"));
        let parent_prop = child
            .properties
            .iter()
            .find(|p| p.code == "parent")
            .unwrap();
        assert!(parent_prop.is_parent_edge);
    }

    #[test]
    fn extract_property_value_handles_all_types() {
        let cases = [
            (serde_json::json!({"valueCode": "ABC"}), "code", "ABC"),
            (serde_json::json!({"valueBoolean": true}), "boolean", "true"),
            (serde_json::json!({"valueInteger": 42}), "integer", "42"),
            (
                serde_json::json!({"valueString": "hello"}),
                "string",
                "hello",
            ),
            (
                serde_json::json!({"valueDateTime": "2024-01-01"}),
                "dateTime",
                "2024-01-01",
            ),
        ];
        for (prop, expected_type, expected_value) in &cases {
            let (vt, v) = extract_property_value(prop);
            assert_eq!(vt, *expected_type);
            assert_eq!(v, *expected_value);
        }
    }
}
