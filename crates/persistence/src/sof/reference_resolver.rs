//! Storage-backed `resolve()` prefetch for server-side FHIRPath evaluation.
//!
//! The FHIRPath `resolve()` function dereferences a `Reference` to the resource
//! it points at. In-scope resolution (against `contained` resources and the
//! resources already in the evaluation context) is handled by the `helios-fhirpath`
//! engine; this module adds the **server-side** piece: dereferencing a relative
//! `Type/id` reference to a resource that lives in the storage backend.
//!
//! It follows the same *eager pre-hydration* strategy as the trusted-server
//! remote-`resolve()` prefetch (`helios_sof::remote_fetch`), but sources resources
//! from a tenant-scoped [`ResourceStorage`] instead of HTTP. Before evaluation, the
//! caller scans the resources under evaluation for relative references
//! ([`collect_missing_references`]), batch-reads the referenced resources from
//! storage ([`StorageReferenceResolver::resolve`]), and folds them into the
//! evaluation's resolution pool. The evaluator itself stays synchronous — all I/O
//! happens here, in async code, before the engine runs.
//!
//! ## Safety properties
//!
//! - **Tenant isolation:** every read goes through [`ResourceStorage`] with the
//!   caller's [`TenantContext`]; resolution never crosses tenant boundaries.
//! - **FHIR version match:** only resources whose stored version equals the
//!   evaluation's version are returned, so the engine never mixes versions.
//! - **No network / no SSRF:** only relative `Type/id` references are resolved.
//!   Absolute URLs (`http(s)://…`), `urn:` references, bare ids, and `#fragment`
//!   references are ignored here — absolute-URL resolution is the separate,
//!   allowlisted concern of `helios_sof::remote_resolver`.
//! - **Bounded fan-out:** a single evaluation can reference many resources, so the
//!   reference set is de-duplicated and capped ([`StorageBackedResolver::new`]).
//!   Resolution is one level deep — references discovered *inside* fetched
//!   resources are not chased recursively.
//! - **Fallback preserved:** a not-found / errored / version-mismatched reference
//!   simply contributes nothing, leaving the engine's existing typed-stub / empty
//!   semantics intact.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde_json::Value;
use tracing::warn;

use crate::core::ResourceStorage;
use crate::tenant::TenantContext;

/// Resolves relative `Type/id` references to stored resources, tenant-scoped.
///
/// Implementations MUST scope every read to `tenant` and MUST only return
/// resources whose FHIR version equals `fhir_version`. Resolution is best-effort:
/// references that are absent, errored, or version-mismatched are simply omitted
/// from the result (the caller falls back to the engine's stub/empty semantics).
#[async_trait]
pub trait StorageReferenceResolver: Send + Sync {
    /// Batch-resolves `refs` (each `(resource_type, id)`) for `tenant`, returning
    /// the raw FHIR JSON of those that exist and match `fhir_version`. The result
    /// holds at most `refs.len()` items, in unspecified order.
    async fn resolve(
        &self,
        tenant: &TenantContext,
        fhir_version: FhirVersion,
        refs: &[(String, String)],
    ) -> Vec<Value>;
}

/// A [`StorageReferenceResolver`] backed by any [`ResourceStorage`] backend.
pub struct StorageBackedResolver {
    storage: Arc<dyn ResourceStorage>,
    max_fanout: usize,
}

impl StorageBackedResolver {
    /// Default cap on the number of distinct references resolved per evaluation.
    /// Bounds the storage reads a single `resolve()`-bearing view can trigger.
    pub const DEFAULT_MAX_FANOUT: usize = 1000;

    /// Creates a resolver that reads from `storage`, resolving at most
    /// `max_fanout` distinct references per call (excess references are dropped
    /// with a warning rather than read).
    pub fn new(storage: Arc<dyn ResourceStorage>, max_fanout: usize) -> Self {
        Self {
            storage,
            max_fanout,
        }
    }
}

#[async_trait]
impl StorageReferenceResolver for StorageBackedResolver {
    async fn resolve(
        &self,
        tenant: &TenantContext,
        fhir_version: FhirVersion,
        refs: &[(String, String)],
    ) -> Vec<Value> {
        if refs.is_empty() {
            return Vec::new();
        }

        // Cap fan-out before doing any I/O.
        let capped = &refs[..refs.len().min(self.max_fanout)];
        if capped.len() < refs.len() {
            warn!(
                requested = refs.len(),
                cap = self.max_fanout,
                "storage resolve(): reference fan-out exceeded cap; extra references left unresolved"
            );
        }

        // Group ids by resource type so each type is read with a single
        // tenant-scoped `read_batch`.
        let mut ids_by_type: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (resource_type, id) in capped {
            ids_by_type
                .entry(resource_type.as_str())
                .or_default()
                .push(id.as_str());
        }

        let mut out = Vec::new();
        for (resource_type, ids) in ids_by_type {
            match self.storage.read_batch(tenant, resource_type, &ids).await {
                Ok(found) => {
                    for stored in found {
                        // Only hand the engine resources of the version it is
                        // evaluating against; never mix FHIR versions.
                        if stored.fhir_version() == fhir_version {
                            out.push(stored.content().clone());
                        }
                    }
                }
                // Best-effort: a backend error for one type must not fail the whole
                // evaluation — the affected references fall back to stub/empty.
                Err(e) => {
                    warn!(
                        resource_type,
                        error = %e,
                        "storage resolve(): batch read failed; references of this type left unresolved"
                    );
                }
            }
        }
        out
    }
}

/// Collects the distinct relative `Type/id` references found anywhere in
/// `resources` that are **not already present** among `resources` themselves.
///
/// "Present" references are skipped because the engine resolves those in-scope;
/// only genuinely external references need a storage read. References that are not
/// resolvable from storage — `#fragment`, bare ids, absolute URLs, and `urn:`
/// references — are excluded (see the module docs on the no-network property).
///
/// This function is pure (no I/O) and order-stable.
pub fn collect_missing_references(resources: &[Value]) -> Vec<(String, String)> {
    let present: HashSet<(String, String)> = resources.iter().filter_map(resource_key).collect();

    let mut reference_strings = BTreeSet::new();
    for resource in resources {
        collect_reference_strings(resource, &mut reference_strings);
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for reference in &reference_strings {
        if let Some(key) = parse_relative_type_id(reference)
            && !present.contains(&key)
            && seen.insert(key.clone())
        {
            out.push(key);
        }
    }
    out
}

/// Returns a resource JSON object's `(resourceType, id)` key, if both are present.
fn resource_key(resource: &Value) -> Option<(String, String)> {
    let resource_type = resource.get("resourceType")?.as_str()?;
    let id = resource.get("id")?.as_str()?;
    Some((resource_type.to_string(), id.to_string()))
}

/// Recursively collects every `"reference"` string field's value.
///
/// Walks the JSON structurally so it captures every `Reference` element
/// regardless of the path it appears at; non-`Reference` `"reference"` keys are
/// harmless because [`parse_relative_type_id`] only accepts `Type/id` shapes.
fn collect_reference_strings(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "reference"
                    && let Value::String(s) = child
                {
                    out.insert(s.clone());
                }
                collect_reference_strings(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_reference_strings(item, out);
            }
        }
        _ => {}
    }
}

/// Parses a relative FHIR reference into `(ResourceType, id)`.
///
/// Returns `None` for anything that is not a storage-resolvable relative
/// reference: `#fragment` / bare ids, absolute URLs (`scheme://…`), and `urn:`
/// references. A versioned reference (`Type/id/_history/vid`) resolves to its
/// current `(Type, id)`.
fn parse_relative_type_id(reference: &str) -> Option<(String, String)> {
    if reference.starts_with('#') || reference.contains("://") || reference.starts_with("urn:") {
        return None;
    }
    let mut segments = reference.split('/');
    let resource_type = segments.next()?;
    let id = segments.next()?;
    if id.is_empty() {
        return None;
    }
    // A FHIR resource type is a capitalized, all-alphabetic token.
    let mut chars = resource_type.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    if !resource_type.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((resource_type.to_string(), id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_relative_type_id_references() {
        let obs = json!({
            "resourceType": "Observation",
            "id": "obs-1",
            "subject": {"reference": "Patient/123"},
            "performer": [{"reference": "Practitioner/p1"}]
        });
        let refs = collect_missing_references(&[obs]);
        assert!(refs.contains(&("Patient".to_string(), "123".to_string())));
        assert!(refs.contains(&("Practitioner".to_string(), "p1".to_string())));
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn excludes_non_storage_references() {
        let obs = json!({
            "resourceType": "Observation",
            "id": "obs-1",
            "a": {"reference": "#contained"},          // fragment
            "b": {"reference": "123"},                  // bare id
            "c": {"reference": "http://ex.org/fhir/Patient/9"}, // absolute URL
            "d": {"reference": "urn:uuid:abcd"},        // urn
            "e": {"reference": "Patient/keep"}          // the only resolvable one
        });
        let refs = collect_missing_references(&[obs]);
        assert_eq!(refs, vec![("Patient".to_string(), "keep".to_string())]);
    }

    #[test]
    fn excludes_references_already_present_in_scope() {
        // The Patient is in scope alongside the Observation, so it must not be
        // queued for a (redundant) storage read.
        let obs = json!({
            "resourceType": "Observation", "id": "o1",
            "subject": {"reference": "Patient/in-scope"}
        });
        let patient = json!({"resourceType": "Patient", "id": "in-scope"});
        let refs = collect_missing_references(&[obs, patient]);
        assert!(
            refs.is_empty(),
            "in-scope reference must not be queued: {refs:?}"
        );
    }

    #[test]
    fn dedups_repeated_references() {
        let bundle = vec![
            json!({"resourceType": "Observation", "id": "o1", "subject": {"reference": "Patient/123"}}),
            json!({"resourceType": "Observation", "id": "o2", "subject": {"reference": "Patient/123"}}),
        ];
        let refs = collect_missing_references(&bundle);
        assert_eq!(refs, vec![("Patient".to_string(), "123".to_string())]);
    }

    #[test]
    fn versioned_reference_resolves_to_current_type_id() {
        assert_eq!(
            parse_relative_type_id("Patient/123/_history/2"),
            Some(("Patient".to_string(), "123".to_string()))
        );
    }
}
