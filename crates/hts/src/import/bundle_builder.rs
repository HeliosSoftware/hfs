//! Shared FHIR CodeSystem → Bundle JSON builder for non-`hl7-npm` importers.
//!
//! Each format-specific importer (SNOMED RF2, LOINC CSV, ICD-10-CM, …) parses
//! its source file into a flat list of [`BuilderConcept`]s and then funnels the
//! data through [`build_code_system_bundle`] to produce a FHIR Bundle JSON
//! payload.  The resulting bytes are consumed by
//! [`BundleImportBackend::import_bundle`](super::BundleImportBackend::import_bundle),
//! so the importer is free to run against either the SQLite or PostgreSQL
//! backend without touching a connection pool directly.
//!
//! Hierarchy edges are encoded on each concept as
//! `property: [{code: "parent", valueCode: "<parent-code>"}]`.  The shared
//! bundle parser (`super::bundle_parser`) recognises that convention and
//! populates the `concept_hierarchy` rows on both backends.

use serde_json::{Value, json};

/// Metadata for a single CodeSystem emitted by an importer.
#[derive(Debug, Clone)]
pub(crate) struct CodeSystemMeta<'a> {
    pub id: &'a str,
    pub url: &'a str,
    pub version: Option<&'a str>,
    pub name: Option<&'a str>,
    pub title: Option<&'a str>,
    pub status: &'a str,
    pub content: &'a str,
}

/// One concept row ready to be serialised into `CodeSystem.concept[]`.
#[derive(Debug, Clone, Default)]
pub(crate) struct BuilderConcept<'a> {
    pub code: &'a str,
    pub display: Option<&'a str>,
    pub definition: Option<&'a str>,
    /// Encoded as `property: [{code: "parent", valueCode: "<code>"}]` on the
    /// emitted FHIR concept.
    pub parent_code: Option<&'a str>,
    pub designations: &'a [BuilderDesignation<'a>],
    pub extra_properties: &'a [BuilderProperty<'a>],
}

/// Free-form property to attach to a concept (for importers that carry
/// metadata beyond display/hierarchy — e.g. RxNorm TTY, LOINC COMPONENT).
#[derive(Debug, Clone)]
pub(crate) struct BuilderProperty<'a> {
    pub code: &'a str,
    /// FHIR value[x] element — `"valueCode"`, `"valueString"`, etc.
    pub value_key: &'a str,
    pub value: &'a str,
}

/// Alternate designation / translation for a concept.
#[derive(Debug, Clone)]
pub(crate) struct BuilderDesignation<'a> {
    pub language: Option<&'a str>,
    pub use_system: Option<&'a str>,
    pub use_code: Option<&'a str>,
    pub value: &'a str,
}

/// Build a single-entry FHIR Bundle containing a CodeSystem resource with the
/// given metadata and concept batch.  Returns the serialised JSON bytes.
pub(crate) fn build_code_system_bundle(
    meta: &CodeSystemMeta<'_>,
    concepts: &[BuilderConcept<'_>],
) -> Vec<u8> {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{
            "resource": build_code_system_resource(meta, concepts)
        }]
    });
    serde_json::to_vec(&bundle).expect("serialise Bundle")
}

/// Build a FHIR CodeSystem resource JSON value.  Exposed for tests and for
/// importers that want to embed the resource in a larger payload.
pub(crate) fn build_code_system_resource(
    meta: &CodeSystemMeta<'_>,
    concepts: &[BuilderConcept<'_>],
) -> Value {
    let mut cs = json!({
        "resourceType": "CodeSystem",
        "id": meta.id,
        "url": meta.url,
        "status": meta.status,
        "content": meta.content,
    });
    let obj = cs.as_object_mut().expect("CodeSystem is object");
    if let Some(v) = meta.version {
        obj.insert("version".into(), json!(v));
    }
    if let Some(v) = meta.name {
        obj.insert("name".into(), json!(v));
    }
    if let Some(v) = meta.title {
        obj.insert("title".into(), json!(v));
    }
    if !concepts.is_empty() {
        obj.insert(
            "concept".into(),
            Value::Array(concepts.iter().map(encode_concept).collect()),
        );
    }
    cs
}

fn encode_concept(c: &BuilderConcept<'_>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".into(), json!(c.code));
    if let Some(d) = c.display {
        obj.insert("display".into(), json!(d));
    }
    if let Some(d) = c.definition {
        obj.insert("definition".into(), json!(d));
    }

    let mut properties: Vec<Value> = Vec::new();
    if let Some(parent) = c.parent_code {
        properties.push(json!({
            "code": "parent",
            "valueCode": parent,
        }));
    }
    for p in c.extra_properties {
        properties.push(json!({
            "code": p.code,
            p.value_key: p.value,
        }));
    }
    if !properties.is_empty() {
        obj.insert("property".into(), Value::Array(properties));
    }

    if !c.designations.is_empty() {
        let desigs: Vec<Value> = c
            .designations
            .iter()
            .map(|d| {
                let mut m = serde_json::Map::new();
                if let Some(lang) = d.language {
                    m.insert("language".into(), json!(lang));
                }
                if d.use_system.is_some() || d.use_code.is_some() {
                    let mut u = serde_json::Map::new();
                    if let Some(sys) = d.use_system {
                        u.insert("system".into(), json!(sys));
                    }
                    if let Some(code) = d.use_code {
                        u.insert("code".into(), json!(code));
                    }
                    m.insert("use".into(), Value::Object(u));
                }
                m.insert("value".into(), json!(d.value));
                Value::Object(m)
            })
            .collect();
        obj.insert("designation".into(), Value::Array(desigs));
    }

    Value::Object(obj)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_bundle_has_one_codesystem_entry() {
        let meta = CodeSystemMeta {
            id: "cs-1",
            url: "http://example.org/cs",
            version: Some("1.0"),
            name: Some("Example"),
            title: Some("Example CS"),
            status: "active",
            content: "complete",
        };
        let concepts = [BuilderConcept {
            code: "A",
            display: Some("Alpha"),
            ..Default::default()
        }];

        let bytes = build_code_system_bundle(&meta, &concepts);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["resourceType"], "Bundle");
        assert_eq!(v["entry"].as_array().unwrap().len(), 1);
        let cs = &v["entry"][0]["resource"];
        assert_eq!(cs["resourceType"], "CodeSystem");
        assert_eq!(cs["url"], "http://example.org/cs");
        assert_eq!(cs["concept"][0]["code"], "A");
        assert_eq!(cs["concept"][0]["display"], "Alpha");
    }

    #[test]
    fn parent_code_becomes_parent_property() {
        let meta = CodeSystemMeta {
            id: "cs",
            url: "http://x",
            version: None,
            name: None,
            title: None,
            status: "active",
            content: "complete",
        };
        let concepts = [BuilderConcept {
            code: "child",
            parent_code: Some("root"),
            ..Default::default()
        }];
        let bytes = build_code_system_bundle(&meta, &concepts);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let prop = &v["entry"][0]["resource"]["concept"][0]["property"][0];
        assert_eq!(prop["code"], "parent");
        assert_eq!(prop["valueCode"], "root");
    }

    #[test]
    fn designation_is_emitted() {
        let meta = CodeSystemMeta {
            id: "cs",
            url: "http://x",
            version: None,
            name: None,
            title: None,
            status: "active",
            content: "complete",
        };
        let desig = [BuilderDesignation {
            language: Some("fr"),
            use_system: Some("http://snomed.info/sct"),
            use_code: Some("900000000000013009"),
            value: "foo",
        }];
        let concepts = [BuilderConcept {
            code: "c",
            designations: &desig,
            ..Default::default()
        }];
        let bytes = build_code_system_bundle(&meta, &concepts);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let d = &v["entry"][0]["resource"]["concept"][0]["designation"][0];
        assert_eq!(d["language"], "fr");
        assert_eq!(d["use"]["system"], "http://snomed.info/sct");
        assert_eq!(d["value"], "foo");
    }
}
