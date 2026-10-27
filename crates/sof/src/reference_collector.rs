//! Reference collection for remote `resolve()` prefetch (Phase 3).
//!
//! Gathers the FHIR `Reference.reference` strings present in an input bundle (and,
//! during chained prefetch, in fetched resources) so the prefetch stage can decide
//! which ones are eligible for remote fetching. This module is pure (no I/O).
//!
//! References are found by structurally walking the JSON for `"reference"` string
//! fields, which captures every `Reference` element regardless of the path it
//! appears at. Candidates are later filtered against the allowlist
//! ([`crate::remote_resolver::RemoteResolveConfig::fetch_decision`]), so occasional
//! non-`Reference` `"reference"` keys are harmless — they simply won't match.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::SofBundle;

/// Collects all unique `reference` strings found anywhere in the bundle.
pub fn collect_reference_strings(bundle: &SofBundle) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(json) = bundle_to_json(bundle) {
        collect_into(&json, &mut out);
    }
    out.into_iter().collect()
}

/// Collects all unique `reference` strings from an arbitrary resource JSON value
/// (used to discover chained references inside fetched resources).
pub fn collect_references_from_json(value: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    collect_into(value, &mut out);
    out.into_iter().collect()
}

/// Collects all unique `reference` strings across a slice of raw resource JSON
/// values (the streaming/NDJSON chunk shape).
pub fn collect_reference_strings_from_resources(resources: &[Value]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for resource in resources {
        collect_into(resource, &mut out);
    }
    out.into_iter().collect()
}

/// Collects the `"ResourceType/id"` keys of a slice of raw resource JSON values.
pub fn collect_resource_keys_from_resources(resources: &[Value]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for resource in resources {
        if let (Some(rt), Some(id)) = (
            resource.get("resourceType").and_then(Value::as_str),
            resource.get("id").and_then(Value::as_str),
        ) {
            keys.insert(format!("{rt}/{id}"));
        }
    }
    keys
}

/// Collects the `"ResourceType/id"` keys of every resource already in the bundle,
/// so the prefetch can skip references that resolve locally.
pub fn collect_resource_keys(bundle: &SofBundle) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let Some(json) = bundle_to_json(bundle) else {
        return keys;
    };
    let Some(entries) = json.get("entry").and_then(Value::as_array) else {
        return keys;
    };
    for entry in entries {
        let resource = entry.get("resource").unwrap_or(entry);
        if let (Some(rt), Some(id)) = (
            resource.get("resourceType").and_then(Value::as_str),
            resource.get("id").and_then(Value::as_str),
        ) {
            keys.insert(format!("{rt}/{id}"));
        }
    }
    keys
}

/// Returns the `"Type/id"` key implied by a reference string (relative or the
/// trailing `Type/id` of an absolute URL), used to test local resolvability.
///
/// Mirrors the (capitalised-type) heuristic of the in-scope resolver. Returns
/// `None` for fragment/bare references and anything without a `Type/id` tail.
pub fn reference_type_id(reference: &str) -> Option<String> {
    // Drop query/fragment before inspecting path segments.
    let without_query = reference.split(['?', '#']).next().unwrap_or(reference);
    let trimmed = without_query.trim_end_matches('/');
    let mut segments = trimmed.rsplitn(3, '/');
    let id = segments.next()?;
    let resource_type = segments.next()?;
    if id.is_empty() || resource_type.is_empty() {
        return None;
    }
    // A FHIR resource type is a capitalised token.
    if !resource_type
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
    {
        return None;
    }
    Some(format!("{resource_type}/{id}"))
}

/// Whether `reference` already resolves against a resource present in the bundle.
pub fn resolves_in_bundle(reference: &str, bundle_keys: &BTreeSet<String>) -> bool {
    match reference_type_id(reference) {
        Some(key) => bundle_keys.contains(&key),
        None => false,
    }
}

fn collect_into(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("reference") {
                out.insert(reference.clone());
            }
            for child in map.values() {
                collect_into(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_into(item, out);
            }
        }
        _ => {}
    }
}

fn bundle_to_json(bundle: &SofBundle) -> Option<Value> {
    match bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(b) => serde_json::to_value(b).ok(),
        #[cfg(feature = "R4B")]
        SofBundle::R4B(b) => serde_json::to_value(b).ok(),
        #[cfg(feature = "R5")]
        SofBundle::R5(b) => serde_json::to_value(b).ok(),
        #[cfg(feature = "R6")]
        SofBundle::R6(b) => serde_json::to_value(b).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_type_id_from_relative_and_absolute() {
        assert_eq!(
            reference_type_id("Patient/123").as_deref(),
            Some("Patient/123")
        );
        assert_eq!(
            reference_type_id("https://example.org/fhir/Patient/123").as_deref(),
            Some("Patient/123")
        );
        assert_eq!(
            reference_type_id("https://example.org/fhir/Patient/123?_format=json").as_deref(),
            Some("Patient/123")
        );
        assert_eq!(reference_type_id("#contained"), None);
        assert_eq!(reference_type_id("urn:uuid:abc"), None);
    }

    #[test]
    fn collects_nested_references() {
        let resource = serde_json::json!({
            "resourceType": "Encounter",
            "subject": { "reference": "Patient/1" },
            "participant": [
                { "individual": { "reference": "Practitioner/2" } },
                { "individual": { "reference": "Practitioner/3" } }
            ]
        });
        let mut refs = collect_references_from_json(&resource);
        refs.sort();
        assert_eq!(refs, vec!["Patient/1", "Practitioner/2", "Practitioner/3"]);
    }
}
