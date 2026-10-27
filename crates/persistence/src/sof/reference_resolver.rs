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
//!   A batch over the cap fails the run rather than leaving references unresolved.
//!   Resolution is one level deep — references discovered *inside* fetched
//!   resources are not chased recursively.
//! - **Fallback preserved:** a not-found / version-mismatched reference simply
//!   contributes nothing, leaving the engine's existing typed-stub / empty
//!   semantics intact. A storage read that fails twice fails the run instead.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde_json::Value;
use tracing::warn;

use crate::core::ResourceStorage;
use crate::core::sof_runner::SofError;
use crate::tenant::TenantContext;

/// Resolves relative `Type/id` references to stored resources, tenant-scoped.
///
/// Implementations MUST scope every read to `tenant` and MUST only return
/// resources whose FHIR version equals `fhir_version`. References that are absent
/// or version-mismatched are omitted from the result (the caller falls back to the
/// engine's stub/empty semantics). A reference that cannot be read at all is an
/// error, never a silent omission: the view's rows would otherwise be wrong.
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
    ) -> Result<Vec<Value>, SofError>;
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
    /// `max_fanout` distinct references per call. A call with more fails with
    /// [`SofError::ResolutionLimit`] before any read.
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
    ) -> Result<Vec<Value>, SofError> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        if refs.len() > self.max_fanout {
            return Err(SofError::ResolutionLimit(format!(
                "a batch of resources references {} distinct resources through resolve(), \
                 more than the limit of {}; narrow the view with a where filter or a smaller _since window",
                refs.len(),
                self.max_fanout
            )));
        }

        let mut out = Vec::new();
        for (resource_type, ids) in group_by_type(refs) {
            let found = match self.storage.read_batch(tenant, resource_type, &ids).await {
                Ok(found) => found,
                Err(e) => {
                    warn!(
                        resource_type,
                        error = %e,
                        "storage resolve(): batch read failed; retrying once"
                    );
                    self.storage
                        .read_batch(tenant, resource_type, &ids)
                        .await
                        .map_err(|e| {
                            let message = format!(
                                "could not read {} referenced {resource_type} resources for resolve(): {e}",
                                ids.len()
                            );
                            // Keep a transient failure transient, so the run
                            // answers 503 rather than 500 (#2048).
                            match SofError::from_storage(e) {
                                SofError::Unavailable(_) => SofError::Unavailable(message),
                                _ => SofError::Storage(message),
                            }
                        })?
                }
            };
            out.extend(
                found
                    .into_iter()
                    .filter(|stored| stored.fhir_version() == fhir_version)
                    .map(|stored| stored.content().clone()),
            );
        }
        Ok(out)
    }
}

/// Groups `refs` by resource type, so each type is read with a single
/// tenant-scoped `read_batch`.
fn group_by_type(refs: &[(String, String)]) -> BTreeMap<&str, Vec<&str>> {
    let mut by_type: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (resource_type, id) in refs {
        by_type
            .entry(resource_type.as_str())
            .or_default()
            .push(id.as_str());
    }
    by_type
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
    use crate::error::{BackendError, StorageError, StorageResult};
    use crate::tenant::{TenantId, TenantPermissions};
    use crate::types::StoredResource;
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
    fn group_by_type_keeps_every_reference() {
        let refs = vec![
            ("Observation".to_string(), "1".to_string()),
            ("Observation".to_string(), "2".to_string()),
            ("Patient".to_string(), "p".to_string()),
        ];
        let grouped = group_by_type(&refs);
        assert_eq!(grouped.get("Observation"), Some(&vec!["1", "2"]));
        assert_eq!(grouped.get("Patient"), Some(&vec!["p"]));
    }

    #[test]
    fn versioned_reference_resolves_to_current_type_id() {
        assert_eq!(
            parse_relative_type_id("Patient/123/_history/2"),
            Some(("Patient".to_string(), "123".to_string()))
        );
    }

    /// A storage whose every `read` fails, either transiently (`Unavailable`,
    /// as an S3 object read does once its retries are spent) or not.
    struct FailingReads {
        transient: bool,
    }

    #[async_trait]
    impl ResourceStorage for FailingReads {
        fn backend_name(&self) -> &'static str {
            "failing-reads"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            Err(StorageError::Backend(if self.transient {
                BackendError::Unavailable {
                    backend_name: "s3".to_string(),
                    message: "failed to read object body after 3 attempts".to_string(),
                }
            } else {
                BackendError::Internal {
                    backend_name: "s3".to_string(),
                    message: "boom".to_string(),
                    source: None,
                }
            }))
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    async fn resolve_over_failing_reads(transient: bool) -> SofError {
        let resolver = StorageBackedResolver::new(Arc::new(FailingReads { transient }), 10);
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        resolver
            .resolve(
                &tenant,
                FhirVersion::default(),
                &[("Patient".to_string(), "p1".to_string())],
            )
            .await
            .expect_err("the reads fail on both attempts")
    }

    /// #2048: a referenced-resource read that runs out of attempts on a
    /// transient failure stays `Unavailable`, so the run answers 503.
    #[tokio::test]
    async fn a_resolve_read_that_stays_unavailable_is_unavailable() {
        match resolve_over_failing_reads(true).await {
            SofError::Unavailable(message) => {
                assert!(
                    message.starts_with(
                        "could not read 1 referenced Patient resources for resolve(): "
                    ),
                    "{message}"
                );
                assert!(message.contains("after 3 attempts"), "{message}");
            }
            other => panic!("expected SofError::Unavailable, got {other:?}"),
        }
    }

    /// Any other read failure is still a storage error (a server fault).
    #[tokio::test]
    async fn a_resolve_read_that_fails_internally_stays_a_storage_error() {
        match resolve_over_failing_reads(false).await {
            SofError::Storage(message) => assert!(message.contains("boom"), "{message}"),
            other => panic!("expected SofError::Storage, got {other:?}"),
        }
    }
}
