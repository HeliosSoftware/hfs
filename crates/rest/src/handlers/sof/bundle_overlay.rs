//! The transaction-scoped dependency resolver for terminology
//! materialization (#1824, §6.14): a FHIR transaction's entries are not
//! persisted until the executor commits, yet a Library entry's dependencies
//! may be ValueSets the same bundle creates, replaces, patches or deletes.
//! The pre-pass in `handlers::batch` records every entry into a
//! [`BundleOverlay`] in execution order, and [`OverlayThenStorage`] resolves
//! a reference against that overlay first and the tenant's storage second.
//!
//! Rules (reviewed in rounds 3 to 6 of the plan):
//!
//! - **Identity, not URL, is the key.** Two in-bundle ValueSets with the same
//!   `url` and different `version` are two candidates; a pinned reference
//!   resolves to its version, an unpinned one to the candidate latest in
//!   execution order, mirroring storage's "newest wins".
//! - **Supersession is recorded by previous identity and applied before
//!   selection.** A DELETE or PUT by id suppresses the stored resource's
//!   identity; the canonical lookup removes suppressed identities from the
//!   full storage candidate list and only then chooses, so a deleted or
//!   replaced artifact is never resurrected and a surviving older version is
//!   never hidden.
//! - **Undetermined targets are refused, not guessed.** A conditional PUT,
//!   DELETE or POST (`ifNoneExist`) on a ValueSet, ConceptMap or Library is
//!   decided only inside the executor, so when such an entry exists any
//!   materialization resolution is refused up front in strict mode.
//! - **No storage reads inside the executor.** The executor prepares PATCH
//!   candidates while its storage transaction is open; asking the pool for a
//!   second connection there is the hazard behind #787. So the pre-pass
//!   snapshots every storage lookup it makes, and inside the executor
//!   [`OverlayThenStorage`] answers from the overlay and that snapshot only.
//!   A reference the pre-pass never saw is "not found" there, which the
//!   materializer reports as a needed side write and strict mode rolls back
//!   (`[R5-3]`).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::error::StorageError;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use serde_json::Value;

use super::graph::{DEPENDENCY_RESOURCE_TYPES, Identity};
use super::references::{canonical_candidates, canonical_matches, split_canonical_version};
use super::terminology_materialize::{DependencySource, Lookup};
use crate::error::RestError;
use crate::state::AppState;

/// The resource types whose in-bundle state the overlay tracks.
pub(crate) const OVERLAY_RESOURCE_TYPES: [&str; 3] = ["ValueSet", "ConceptMap", "Library"];

/// Whether `resource_type` is one the overlay tracks.
pub(crate) fn is_overlay_type(resource_type: &str) -> bool {
    OVERLAY_RESOURCE_TYPES.contains(&resource_type)
}

#[derive(Debug, Clone)]
struct CanonicalEntry {
    version: Option<String>,
    seq: usize,
    candidate: Value,
}

#[derive(Debug, Clone)]
enum OverlayEntry {
    Candidate(Value),
    Deleted,
}

#[derive(Default)]
struct Inner {
    /// `Type/id` → the entry's in-bundle state.
    by_id: HashMap<String, OverlayEntry>,
    /// `url` → every in-bundle candidate for it, in execution order.
    by_canonical: HashMap<String, Vec<CanonicalEntry>>,
    /// Stored identities deleted or replaced earlier in the bundle.
    suppressed: HashSet<Identity>,
    /// Conditional entries on overlay types whose outcome only the executor
    /// decides, described for the refusal message.
    undetermined: Vec<String>,
    /// In-bundle SQL Library candidates (POST/PUT bodies and provisional
    /// PATCH results), for T2's "is it depended on" question.
    libraries: Vec<Value>,
    /// Stored SQL Libraries, captured once by the pre-pass for T2 inside the
    /// executor.
    stored_libraries: Option<Vec<Value>>,
    /// Every storage lookup the pre-pass made, by reference, for the
    /// executor's use.
    snapshot: HashMap<String, Option<StoredResource>>,
    seq: usize,
}

/// The transaction-scoped view of a bundle's terminology artifacts.
#[derive(Default)]
pub(crate) struct BundleOverlay {
    inner: Mutex<Inner>,
}

fn type_and_id(candidate: &Value) -> Option<String> {
    let rt = candidate.get("resourceType").and_then(Value::as_str)?;
    let id = candidate.get("id").and_then(Value::as_str)?;
    Some(format!("{rt}/{id}"))
}

impl BundleOverlay {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records a POST or PUT candidate (or a prepared PATCH candidate) under
    /// its `Type/id` when it has one and under its `url`. A later record for
    /// the same `Type/id` replaces the earlier one; the canonical list keeps
    /// every candidate, in order.
    pub(crate) fn record_candidate(&self, candidate: &Value) {
        let mut inner = self.inner.lock().expect("overlay lock");
        inner.seq += 1;
        let seq = inner.seq;
        if let Some(key) = type_and_id(candidate) {
            inner
                .by_id
                .insert(key, OverlayEntry::Candidate(candidate.clone()));
        }
        if let Some(url) = candidate.get("url").and_then(Value::as_str) {
            let version = candidate
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string);
            let entries = inner.by_canonical.entry(url.to_string()).or_default();
            // A re-record of the same identity (the pre-pass transforming a
            // candidate it indexed earlier) replaces, not duplicates.
            if let Some(same) = entries.iter_mut().find(|e| {
                e.version == version
                    && type_and_id(&e.candidate).is_some()
                    && type_and_id(&e.candidate) == type_and_id(candidate)
            }) {
                same.candidate = candidate.clone();
                same.seq = seq;
            } else {
                entries.push(CanonicalEntry {
                    version,
                    seq,
                    candidate: candidate.clone(),
                });
            }
        }
    }

    /// Records a DELETE by id: the `Type/id` is gone for the rest of the
    /// bundle, and the stored resource's identity (when it existed) is
    /// suppressed from canonical lookups.
    pub(crate) fn record_deleted(&self, resource_type: &str, id: &str, stored: Option<&Value>) {
        let mut inner = self.inner.lock().expect("overlay lock");
        inner
            .by_id
            .insert(format!("{resource_type}/{id}"), OverlayEntry::Deleted);
        if let Some(stored) = stored {
            inner.suppressed.insert(Identity::of(stored));
        }
    }

    /// Records that a PUT by id replaces `stored`: its previous identity is
    /// suppressed from canonical lookups (the candidate itself is recorded
    /// with [`Self::record_candidate`]).
    pub(crate) fn record_replaced(&self, stored: &Value) {
        let mut inner = self.inner.lock().expect("overlay lock");
        inner.suppressed.insert(Identity::of(stored));
    }

    /// Records a conditional entry whose outcome only the executor decides.
    pub(crate) fn record_undetermined(&self, description: String) {
        self.inner
            .lock()
            .expect("overlay lock")
            .undetermined
            .push(description);
    }

    /// The undetermined entries recorded so far.
    pub(crate) fn undetermined(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("overlay lock")
            .undetermined
            .clone()
    }

    /// Records an in-bundle SQL Library candidate for T2.
    pub(crate) fn record_library(&self, library: &Value) {
        self.inner
            .lock()
            .expect("overlay lock")
            .libraries
            .push(library.clone());
    }

    /// Every in-bundle Library candidate recorded so far.
    pub(crate) fn libraries(&self) -> Vec<Value> {
        self.inner.lock().expect("overlay lock").libraries.clone()
    }

    /// Captures the tenant's stored SQL Libraries for T2 inside the executor.
    pub(crate) fn set_stored_libraries(&self, libraries: Vec<Value>) {
        self.inner.lock().expect("overlay lock").stored_libraries = Some(libraries);
    }

    /// The stored SQL Libraries the pre-pass captured, if it did.
    pub(crate) fn stored_libraries(&self) -> Option<Vec<Value>> {
        self.inner
            .lock()
            .expect("overlay lock")
            .stored_libraries
            .clone()
    }

    fn by_id(&self, key: &str) -> Option<OverlayEntry> {
        self.inner
            .lock()
            .expect("overlay lock")
            .by_id
            .get(key)
            .cloned()
    }

    /// The in-bundle candidates `reference` selects, in execution order.
    fn canonical(&self, reference: &str) -> Vec<CanonicalEntry> {
        let (url, _) = split_canonical_version(reference);
        let inner = self.inner.lock().expect("overlay lock");
        inner
            .by_canonical
            .get(&url)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| canonical_matches(&e.candidate, reference))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_suppressed(&self, identity: &Identity) -> bool {
        self.inner
            .lock()
            .expect("overlay lock")
            .suppressed
            .contains(identity)
    }

    fn snapshot_get(&self, reference: &str) -> Option<Option<StoredResource>> {
        self.inner
            .lock()
            .expect("overlay lock")
            .snapshot
            .get(reference)
            .cloned()
    }

    fn snapshot_put(&self, reference: &str, stored: Option<StoredResource>) {
        self.inner
            .lock()
            .expect("overlay lock")
            .snapshot
            .insert(reference.to_string(), stored);
    }
}

/// [`DependencySource`] over a bundle overlay first and the tenant's storage
/// second. `live` is `true` in the pre-pass (storage is read and snapshotted)
/// and `false` inside the executor (only the snapshot answers).
pub(crate) struct OverlayThenStorage<'a, S> {
    pub state: &'a AppState<S>,
    pub tenant: &'a TenantContext,
    pub overlay: &'a BundleOverlay,
    pub live: bool,
}

impl<S> OverlayThenStorage<'_, S>
where
    S: SearchProvider + Send + Sync + 'static,
{
    /// The storage half of a lookup: suppressed identities removed from the
    /// full candidate list **before** selection (`[R5-2]`).
    async fn storage_lookup(&self, reference: &str) -> Result<Option<StoredResource>, RestError> {
        if !self.live {
            return Ok(self.overlay.snapshot_get(reference).flatten());
        }
        let found = self.storage_lookup_live(reference).await?;
        self.overlay.snapshot_put(reference, found.clone());
        Ok(found)
    }

    async fn storage_lookup_live(
        &self,
        reference: &str,
    ) -> Result<Option<StoredResource>, RestError> {
        let reference = reference.trim();
        for resource_type in DEPENDENCY_RESOURCE_TYPES {
            let prefix = format!("{resource_type}/");
            if let Some(id) = reference.strip_prefix(prefix.as_str()) {
                let id = id.split('/').next().unwrap_or("");
                return match self
                    .state
                    .storage()
                    .read(self.tenant, resource_type, id)
                    .await
                {
                    Ok(Some(stored)) => {
                        if self.overlay.is_suppressed(&Identity::of(stored.content())) {
                            Ok(None)
                        } else {
                            Ok(Some(stored))
                        }
                    }
                    Ok(None) | Err(StorageError::Resource(_)) => Ok(None),
                    Err(e) => Err(e.into()),
                };
            }
        }
        let mut best: Option<(&'static str, super::references::CanonicalCandidate)> = None;
        for resource_type in DEPENDENCY_RESOURCE_TYPES {
            let candidates =
                canonical_candidates(self.state, self.tenant, resource_type, reference)
                    .await?
                    .into_iter()
                    .filter(|c| {
                        let identity = Identity {
                            resource_type: resource_type.to_string(),
                            id: Some(c.id.clone()),
                            url: c
                                .resource
                                .get("url")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            version: c
                                .resource
                                .get("version")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        };
                        !self.overlay.is_suppressed(&identity)
                    });
            for candidate in candidates {
                if best
                    .as_ref()
                    .is_none_or(|(_, seen)| candidate.last_modified >= seen.last_modified)
                {
                    best = Some((resource_type, candidate));
                }
            }
        }
        let Some((resource_type, chosen)) = best else {
            return Ok(None);
        };
        match self
            .state
            .storage()
            .read(self.tenant, resource_type, &chosen.id)
            .await
        {
            Ok(stored) => Ok(stored),
            Err(StorageError::Resource(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl<S> DependencySource for OverlayThenStorage<'_, S>
where
    S: SearchProvider + Send + Sync + 'static,
{
    async fn lookup(&self, reference: &str) -> Result<Lookup, RestError> {
        let reference = reference.trim();
        // `Type/id`: the overlay's entry for that id, else storage.
        if DEPENDENCY_RESOURCE_TYPES
            .iter()
            .any(|t| reference.starts_with(&format!("{t}/")))
        {
            return match self.overlay.by_id(reference) {
                Some(OverlayEntry::Candidate(candidate)) => Ok(Lookup::Candidate(candidate)),
                Some(OverlayEntry::Deleted) => Ok(Lookup::NotFound),
                None => Ok(self
                    .storage_lookup(reference)
                    .await?
                    .map_or(Lookup::NotFound, Lookup::Stored)),
            };
        }
        // Canonical: an in-bundle candidate wins (the latest in execution
        // order among those the reference selects), else the newest
        // surviving stored one.
        let mut overlay = self.overlay.canonical(reference);
        if let Some(latest) = overlay.iter_mut().max_by_key(|e| e.seq) {
            return Ok(Lookup::Candidate(std::mem::take(&mut latest.candidate)));
        }
        Ok(self
            .storage_lookup(reference)
            .await?
            .map_or(Lookup::NotFound, Lookup::Stored))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vs(id: &str, url: &str, version: Option<&str>) -> Value {
        let mut v = json!({"resourceType": "ValueSet", "id": id, "url": url, "status": "active"});
        if let Some(version) = version {
            v["version"] = json!(version);
        }
        v
    }

    #[test]
    fn two_versions_of_one_url_are_two_candidates_and_a_pin_selects_its_own() {
        let overlay = BundleOverlay::new();
        overlay.record_candidate(&vs("a", "http://example.org/vs", Some("1")));
        overlay.record_candidate(&vs("b", "http://example.org/vs", Some("2")));
        let pinned1 = overlay.canonical("http://example.org/vs|1");
        assert_eq!(pinned1.len(), 1);
        assert_eq!(pinned1[0].candidate["id"], "a");
        let pinned2 = overlay.canonical("http://example.org/vs|2");
        assert_eq!(pinned2[0].candidate["id"], "b");
        let unpinned = overlay.canonical("http://example.org/vs");
        assert_eq!(unpinned.len(), 2);
        assert_eq!(
            unpinned.iter().max_by_key(|e| e.seq).unwrap().candidate["id"],
            "b",
            "unpinned resolves to the latest in execution order"
        );
        assert!(overlay.canonical("http://example.org/vs|3").is_empty());
    }

    #[test]
    fn re_recording_the_same_identity_replaces_the_candidate() {
        let overlay = BundleOverlay::new();
        overlay.record_candidate(&vs("a", "http://example.org/vs", None));
        let mut expanded = vs("a", "http://example.org/vs", None);
        expanded["expansion"] = json!({"contains": []});
        overlay.record_candidate(&expanded);
        let entries = overlay.canonical("http://example.org/vs");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].candidate.get("expansion").is_some());
        match overlay.by_id("ValueSet/a") {
            Some(OverlayEntry::Candidate(c)) => assert!(c.get("expansion").is_some()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn delete_and_replace_suppress_the_stored_identity() {
        let overlay = BundleOverlay::new();
        let stored = vs("x", "http://example.org/vs", Some("1"));
        overlay.record_deleted("ValueSet", "x", Some(&stored));
        assert!(overlay.is_suppressed(&Identity::of(&stored)));
        assert!(matches!(
            overlay.by_id("ValueSet/x"),
            Some(OverlayEntry::Deleted)
        ));
        let other = vs("y", "http://example.org/vs", Some("2"));
        assert!(!overlay.is_suppressed(&Identity::of(&other)));
        overlay.record_replaced(&other);
        assert!(overlay.is_suppressed(&Identity::of(&other)));
    }

    #[test]
    fn undetermined_and_libraries_are_recorded() {
        let overlay = BundleOverlay::new();
        assert!(overlay.undetermined().is_empty());
        overlay.record_undetermined("POST ValueSet?url=x (ifNoneExist)".into());
        assert_eq!(overlay.undetermined().len(), 1);
        overlay.record_library(&json!({"resourceType": "Library", "id": "l"}));
        assert_eq!(overlay.libraries().len(), 1);
        assert!(overlay.stored_libraries().is_none());
        overlay.set_stored_libraries(vec![]);
        assert_eq!(overlay.stored_libraries(), Some(vec![]));
    }

    #[test]
    fn overlay_types_are_the_three_the_pre_pass_tracks() {
        assert!(
            is_overlay_type("ValueSet")
                && is_overlay_type("ConceptMap")
                && is_overlay_type("Library")
        );
        assert!(!is_overlay_type("ViewDefinition") && !is_overlay_type("Patient"));
    }
}
