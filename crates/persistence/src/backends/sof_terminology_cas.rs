//! The [`SofTerminologyStore`] algorithm for stores without multi-object
//! transactions: MongoDB (a standalone server has none) and S3 (#2185).
//!
//! SQLite and PostgreSQL rewrite a relation, its references and the orphan
//! sweep in one transaction. Here the same contract rests on two primitives
//! every such store has: a write that is atomic for one object, and a
//! compare-and-swap of one small object.
//!
//! - **Generations.** A relation's rows are written under a fresh, unique
//!   generation id that nothing points at yet. A generation is never written
//!   again once complete, so a reader that finds one finds all of it.
//! - **One registry per tenant.** A single [`TerminologyRegistry`] object maps
//!   each canonical to its current generation and the Libraries referencing
//!   it. It is the only thing a write changes in place, and it changes by
//!   compare-and-swap, so concurrent Library writes serialize on it and none
//!   is lost.
//!
//! [`set_library_dependencies`] writes the new generations first, then swaps
//! the registry, and only then deletes the generations the swap left
//! unreferenced (superseded by a rewrite, or dropped with their last
//! reference). A crash between those steps leaves at worst unreferenced
//! generations, which no reader can reach and a tenant purge removes; it never
//! leaves the registry pointing at a missing or partial relation.
//!
//! [`load`] reads the registry, then the generation it names. A concurrent
//! write can delete that generation in between; the read then comes back
//! missing or short of the registered row count, and [`load`] re-reads the
//! registry. If it keeps coming back that way, [`load`] reports the relation
//! as not stored, so `$sql-run` expands it through the terminology server
//! instead of failing.
//!
//! [`SofTerminologyStore`]: crate::core::sof_terminology::SofTerminologyStore

use std::time::Duration;

use async_trait::async_trait;
use helios_sof::sqlquery::{TerminologyKind, TerminologyRelation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Bound on registry compare-and-swap attempts per write. Contention is two
/// Library writes in one tenant at once, so a handful of rounds is plenty;
/// exhausting it fails the write rather than looping.
const MAX_SWAP_ATTEMPTS: usize = 16;

/// Bound on registry re-reads in [`load`] when a concurrent write removed the
/// generation the registry named.
const MAX_LOAD_ATTEMPTS: usize = 4;

/// Base unit of the jittered backoff between compare-and-swap attempts.
const BACKOFF_BASE: Duration = Duration::from_millis(5);

/// One tenant's registry: which generation holds each canonical's rows, and
/// which Libraries depend on it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct TerminologyRegistry {
    pub(crate) entries: Vec<RegistryEntry>,
}

/// One canonical's relation in a [`TerminologyRegistry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RegistryEntry {
    pub(crate) canonical: String,
    pub(crate) kind: String,
    pub(crate) generation: String,
    pub(crate) row_count: usize,
    pub(crate) libraries: Vec<String>,
}

/// A relation just written under `generation`, not yet in the registry.
#[derive(Debug, Clone)]
pub(crate) struct WrittenGeneration {
    pub(crate) canonical: String,
    pub(crate) kind: TerminologyKind,
    pub(crate) generation: String,
    pub(crate) row_count: usize,
}

impl TerminologyRegistry {
    fn entry(&self, canonical: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.canonical == canonical)
    }

    /// Makes `written` the complete dependency set of `library_id`, returning
    /// the generations no entry points at any more.
    fn apply(&mut self, library_id: &str, written: &[WrittenGeneration]) -> Vec<String> {
        let mut garbage = Vec::new();
        for entry in &mut self.entries {
            entry.libraries.retain(|l| l != library_id);
        }
        for w in written {
            match self.entries.iter_mut().find(|e| e.canonical == w.canonical) {
                Some(entry) => {
                    if entry.generation != w.generation {
                        garbage.push(std::mem::replace(
                            &mut entry.generation,
                            w.generation.clone(),
                        ));
                    }
                    entry.kind = w.kind.as_str().to_string();
                    entry.row_count = w.row_count;
                    entry.libraries.push(library_id.to_string());
                }
                None => self.entries.push(RegistryEntry {
                    canonical: w.canonical.clone(),
                    kind: w.kind.as_str().to_string(),
                    generation: w.generation.clone(),
                    row_count: w.row_count,
                    libraries: vec![library_id.to_string()],
                }),
            }
        }
        self.entries.retain(|e| {
            if e.libraries.is_empty() {
                garbage.push(e.generation.clone());
                false
            } else {
                true
            }
        });
        garbage
    }
}

/// The per-store primitives the algorithm is built on.
#[async_trait]
pub(crate) trait GenerationStore: Send + Sync {
    /// What the store compares on a swap (a version, an ETag).
    type Token: Send + Sync;

    /// The tenant's registry and its swap token; an empty registry when none
    /// is stored.
    async fn read_registry(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<(TerminologyRegistry, Self::Token)>;

    /// Replaces the registry if it is still at `expected`. `Ok(false)` means a
    /// concurrent write got there first.
    async fn swap_registry(
        &self,
        tenant: &TenantContext,
        expected: &Self::Token,
        registry: &TerminologyRegistry,
    ) -> StorageResult<bool>;

    /// Stores `rows` under `generation`, which nothing references yet.
    async fn write_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
        rows: &[Value],
    ) -> StorageResult<()>;

    /// The rows stored under `generation`, `None` when it is gone. May come
    /// back short when the generation is being deleted concurrently.
    async fn read_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
    ) -> StorageResult<Option<Vec<Value>>>;

    /// Deletes `generations`; absent ones are not an error.
    async fn delete_generations(
        &self,
        tenant: &TenantContext,
        generations: &[String],
    ) -> StorageResult<()>;
}

/// Jittered exponential backoff before swap attempt `attempt` (1-based retry).
fn backoff(attempt: usize) -> Duration {
    let ceiling = BACKOFF_BASE * (1u32 << attempt.min(5));
    let span = u64::try_from(ceiling.as_nanos()).unwrap_or(u64::MAX).max(1);
    Duration::from_nanos(Uuid::new_v4().as_u128() as u64 % span)
}

/// `SofTerminologyStore::set_library_dependencies` over a [`GenerationStore`].
pub(crate) async fn set_library_dependencies<S: GenerationStore>(
    store: &S,
    tenant: &TenantContext,
    library_id: &str,
    relations: Vec<(String, TerminologyRelation)>,
) -> StorageResult<()> {
    let mut written: Vec<WrittenGeneration> = Vec::with_capacity(relations.len());
    for (canonical, relation) in &relations {
        let generation = Uuid::new_v4().simple().to_string();
        if let Err(e) = store
            .write_generation(tenant, &generation, &relation.rows)
            .await
        {
            discard(store, tenant, &written, Some(&generation)).await;
            return Err(e);
        }
        written.push(WrittenGeneration {
            canonical: canonical.clone(),
            kind: relation.kind,
            generation,
            row_count: relation.rows.len(),
        });
    }

    for attempt in 0..MAX_SWAP_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(backoff(attempt)).await;
        }
        let (current, token) = match store.read_registry(tenant).await {
            Ok(read) => read,
            Err(e) => {
                discard(store, tenant, &written, None).await;
                return Err(e);
            }
        };
        let mut next = current.clone();
        let garbage = next.apply(library_id, &written);
        if next == current {
            return Ok(());
        }
        match store.swap_registry(tenant, &token, &next).await {
            Ok(true) => {
                if let Err(e) = store.delete_generations(tenant, &garbage).await {
                    // The registry is consistent; the leftovers are
                    // unreachable and a tenant purge removes them.
                    tracing::warn!(
                        error = %e,
                        generations = garbage.len(),
                        "failed to delete superseded SQL query terminology rows"
                    );
                }
                return Ok(());
            }
            Ok(false) => continue,
            // An error does not say the swap failed: a timeout can arrive
            // after the store applied it. Discarding blindly would delete
            // generations the registry now names, and every later `load` of
            // them would fail. Re-read the registry and decide from that.
            Err(e) => return settle_failed_swap(store, tenant, &written, &next, &garbage, e).await,
        }
    }
    discard(store, tenant, &written, None).await;
    Err(StorageError::Concurrency(
        ConcurrencyError::OptimisticLockFailure {
            resource_type: "SofTerminologyRegistry".to_string(),
            id: library_id.to_string(),
            expected_etag: "W/\"*\"".to_string(),
            actual_etag: None,
        },
    ))
}

/// Resolves a swap that came back with `error`, which may or may not have
/// been applied.
///
/// If the registry now reads as `next`, the swap landed: finish the write
/// (delete `garbage`) and succeed. Otherwise discard only the `written`
/// generations the registry does not name, and fail with `error`. If the
/// registry cannot be read, nothing is discarded: an unreferenced generation
/// is harmless (a tenant purge removes it), a deleted referenced one is not.
async fn settle_failed_swap<S: GenerationStore>(
    store: &S,
    tenant: &TenantContext,
    written: &[WrittenGeneration],
    next: &TerminologyRegistry,
    garbage: &[String],
    error: StorageError,
) -> StorageResult<()> {
    let now = match store.read_registry(tenant).await {
        Ok((now, _)) => now,
        Err(read_error) => {
            tracing::warn!(
                error = %error,
                read_error = %read_error,
                generations = written.len(),
                "SQL query terminology registry swap failed and could not be re-read; \
                 keeping the new rows"
            );
            return Err(error);
        }
    };
    if &now == next {
        if let Err(e) = store.delete_generations(tenant, garbage).await {
            tracing::warn!(
                error = %e,
                generations = garbage.len(),
                "failed to delete superseded SQL query terminology rows"
            );
        }
        return Ok(());
    }
    let unreferenced: Vec<WrittenGeneration> = written
        .iter()
        .filter(|w| !now.entries.iter().any(|e| e.generation == w.generation))
        .cloned()
        .collect();
    discard(store, tenant, &unreferenced, None).await;
    Err(error)
}

/// Best-effort removal of generations a failed write left unreferenced.
async fn discard<S: GenerationStore>(
    store: &S,
    tenant: &TenantContext,
    written: &[WrittenGeneration],
    extra: Option<&str>,
) {
    let generations: Vec<String> = written
        .iter()
        .map(|w| w.generation.clone())
        .chain(extra.map(str::to_string))
        .collect();
    if let Err(e) = store.delete_generations(tenant, &generations).await {
        tracing::warn!(error = %e, "failed to discard unregistered SQL query terminology rows");
    }
}

/// `SofTerminologyStore::load` over a [`GenerationStore`].
pub(crate) async fn load<S: GenerationStore>(
    store: &S,
    tenant: &TenantContext,
    canonical: &str,
) -> StorageResult<Option<TerminologyRelation>> {
    for _ in 0..MAX_LOAD_ATTEMPTS {
        let (registry, _) = store.read_registry(tenant).await?;
        let Some(entry) = registry.entry(canonical) else {
            return Ok(None);
        };
        let Some(kind) = TerminologyKind::parse(&entry.kind) else {
            return Ok(None);
        };
        match store.read_generation(tenant, &entry.generation).await? {
            Some(rows) if rows.len() == entry.row_count => {
                return Ok(Some(TerminologyRelation { kind, rows }));
            }
            // Superseded and deleted while we read: the registry has moved on.
            _ => continue,
        }
    }
    // Under sustained rewrites, or with a registry naming rows that are gone.
    // Neither is the caller's precondition failing, so do not answer with a
    // concurrency error: report the relation as not stored, and the read
    // path expands it through the terminology server.
    tracing::warn!(
        canonical,
        attempts = MAX_LOAD_ATTEMPTS,
        "SQL query terminology rows kept changing or are missing; not serving them from storage"
    );
    Ok(None)
}

/// Implements `SofTerminologyStore` for a backend that is a
/// [`GenerationStore`].
macro_rules! impl_sof_terminology_store {
    ($backend:ty) => {
        #[async_trait::async_trait]
        impl crate::core::sof_terminology::SofTerminologyStore for $backend {
            async fn set_library_dependencies(
                &self,
                tenant: &crate::tenant::TenantContext,
                library_id: &str,
                relations: Vec<(String, helios_sof::sqlquery::TerminologyRelation)>,
            ) -> crate::error::StorageResult<()> {
                crate::backends::sof_terminology_cas::set_library_dependencies(
                    self, tenant, library_id, relations,
                )
                .await
            }

            async fn load(
                &self,
                tenant: &crate::tenant::TenantContext,
                canonical: &str,
            ) -> crate::error::StorageResult<Option<helios_sof::sqlquery::TerminologyRelation>>
            {
                crate::backends::sof_terminology_cas::load(self, tenant, canonical).await
            }
        }
    };
}
pub(crate) use impl_sof_terminology_store;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory [`GenerationStore`] whose swap token is a version.
    #[derive(Default)]
    struct Memory {
        registry: Mutex<HashMap<String, (TerminologyRegistry, u64)>>,
        generations: Mutex<HashMap<(String, String), Vec<Value>>>,
        /// Swaps to fail before succeeding, to exercise the retry.
        lose_swaps: Mutex<usize>,
        /// Fails the next swap with an error, after applying it when the flag
        /// is `Some(true)` (a timeout on a write that landed).
        fail_swap: Mutex<Option<bool>>,
    }

    #[async_trait]
    impl GenerationStore for Memory {
        type Token = u64;

        async fn read_registry(
            &self,
            tenant: &TenantContext,
        ) -> StorageResult<(TerminologyRegistry, u64)> {
            Ok(self
                .registry
                .lock()
                .unwrap()
                .get(tenant.tenant_id().as_str())
                .cloned()
                .unwrap_or_default())
        }

        async fn swap_registry(
            &self,
            tenant: &TenantContext,
            expected: &u64,
            registry: &TerminologyRegistry,
        ) -> StorageResult<bool> {
            {
                let mut lose = self.lose_swaps.lock().unwrap();
                if *lose > 0 {
                    *lose -= 1;
                    return Ok(false);
                }
            }
            let fail = self.fail_swap.lock().unwrap().take();
            let error = || {
                Err(StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "memory".to_string(),
                        message: "swap timed out".to_string(),
                        source: None,
                    },
                ))
            };
            if fail == Some(false) {
                return error();
            }
            let mut map = self.registry.lock().unwrap();
            let slot = map
                .entry(tenant.tenant_id().as_str().to_string())
                .or_default();
            if slot.1 != *expected {
                return Ok(false);
            }
            *slot = (registry.clone(), expected + 1);
            if fail == Some(true) {
                return error();
            }
            Ok(true)
        }

        async fn write_generation(
            &self,
            tenant: &TenantContext,
            generation: &str,
            rows: &[Value],
        ) -> StorageResult<()> {
            self.generations.lock().unwrap().insert(
                (tenant.tenant_id().as_str().into(), generation.into()),
                rows.to_vec(),
            );
            Ok(())
        }

        async fn read_generation(
            &self,
            tenant: &TenantContext,
            generation: &str,
        ) -> StorageResult<Option<Vec<Value>>> {
            Ok(self
                .generations
                .lock()
                .unwrap()
                .get(&(tenant.tenant_id().as_str().into(), generation.into()))
                .cloned())
        }

        async fn delete_generations(
            &self,
            tenant: &TenantContext,
            generations: &[String],
        ) -> StorageResult<()> {
            let mut map = self.generations.lock().unwrap();
            for g in generations {
                map.remove(&(tenant.tenant_id().as_str().into(), g.clone()));
            }
            Ok(())
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access())
    }

    fn vs(codes: &[&str]) -> TerminologyRelation {
        TerminologyRelation {
            kind: TerminologyKind::ValueSet,
            rows: codes.iter().map(|c| json!({"code": c})).collect(),
        }
    }

    #[tokio::test]
    async fn rewrites_and_releases_leave_no_unreferenced_generation() {
        let store = Memory::default();
        let t = tenant();
        set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1"]))])
            .await
            .unwrap();
        set_library_dependencies(&store, &t, "b", vec![("u".into(), vs(&["1", "2"]))])
            .await
            .unwrap();
        assert_eq!(store.generations.lock().unwrap().len(), 1);
        assert_eq!(load(&store, &t, "u").await.unwrap(), Some(vs(&["1", "2"])));

        set_library_dependencies(&store, &t, "a", vec![])
            .await
            .unwrap();
        assert!(load(&store, &t, "u").await.unwrap().is_some());
        set_library_dependencies(&store, &t, "b", vec![])
            .await
            .unwrap();
        assert!(load(&store, &t, "u").await.unwrap().is_none());
        assert!(store.generations.lock().unwrap().is_empty());
        assert!(store.registry.lock().unwrap()["acme"].0.entries.is_empty());
    }

    #[tokio::test]
    async fn a_lost_swap_is_retried_against_the_new_registry() {
        let store = Memory::default();
        *store.lose_swaps.lock().unwrap() = 3;
        let t = tenant();
        set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1"]))])
            .await
            .unwrap();
        assert_eq!(load(&store, &t, "u").await.unwrap(), Some(vs(&["1"])));
    }

    #[tokio::test]
    async fn exhausted_swaps_fail_and_discard_the_new_generations() {
        let store = Memory::default();
        *store.lose_swaps.lock().unwrap() = MAX_SWAP_ATTEMPTS;
        let t = tenant();
        let err = set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1"]))])
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Concurrency(_)));
        assert!(store.generations.lock().unwrap().is_empty());
        assert!(load(&store, &t, "u").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_swap_that_errors_after_landing_completes_the_write() {
        let store = Memory::default();
        let t = tenant();
        set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1"]))])
            .await
            .unwrap();
        *store.fail_swap.lock().unwrap() = Some(true);
        set_library_dependencies(&store, &t, "b", vec![("u".into(), vs(&["1", "2"]))])
            .await
            .expect("the swap landed, so the write succeeded");
        // The new generation is kept and the superseded one deleted.
        assert_eq!(load(&store, &t, "u").await.unwrap(), Some(vs(&["1", "2"])));
        assert_eq!(store.generations.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_swap_that_errors_without_landing_keeps_the_old_relation() {
        let store = Memory::default();
        let t = tenant();
        set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1"]))])
            .await
            .unwrap();
        *store.fail_swap.lock().unwrap() = Some(false);
        let err = set_library_dependencies(&store, &t, "b", vec![("u".into(), vs(&["1", "2"]))])
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::Backend(_)));
        // The unregistered generation is discarded; the registered one stays.
        assert_eq!(load(&store, &t, "u").await.unwrap(), Some(vs(&["1"])));
        assert_eq!(store.generations.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn releasing_an_unknown_library_writes_nothing() {
        let store = Memory::default();
        // Every swap would fail; an unchanged registry must not attempt one.
        *store.lose_swaps.lock().unwrap() = usize::MAX;
        set_library_dependencies(&store, &tenant(), "nobody", vec![])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_short_generation_is_not_served() {
        let store = Memory::default();
        let t = tenant();
        set_library_dependencies(&store, &t, "a", vec![("u".into(), vs(&["1", "2"]))])
            .await
            .unwrap();
        // Simulate a concurrent delete caught half way.
        for rows in store.generations.lock().unwrap().values_mut() {
            rows.pop();
        }
        assert_eq!(load(&store, &t, "u").await.unwrap(), None);
    }

    #[test]
    fn apply_keeps_a_canonical_another_library_still_references() {
        let mut registry = TerminologyRegistry::default();
        let w = |g: &str| WrittenGeneration {
            canonical: "u".into(),
            kind: TerminologyKind::ValueSet,
            generation: g.into(),
            row_count: 1,
        };
        assert!(registry.apply("a", &[w("g1")]).is_empty());
        assert_eq!(registry.apply("b", &[w("g2")]), vec!["g1".to_string()]);
        assert!(registry.apply("a", &[]).is_empty());
        assert_eq!(registry.entries[0].libraries, vec!["b".to_string()]);
        assert_eq!(registry.apply("b", &[]), vec!["g2".to_string()]);
        assert!(registry.entries.is_empty());
    }
}
