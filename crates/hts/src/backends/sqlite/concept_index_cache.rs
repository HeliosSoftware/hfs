//! A bounded, least-recently-used cache of in-memory concept indexes.
//!
//! The SQLite backend keeps several process-local maps from a caller-derived
//! key (an implicit ValueSet URL, or the hash of an inline compose body) to
//! an [`ImplicitConceptIndex`]. The keys are chosen by whoever sends the
//! request, so each map is bounded both by the number of keys and by the
//! bytes its indexes retain; when either budget is exceeded the entry used
//! longest ago is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::value_set::ImplicitConceptIndex;

/// Keys one cache keeps. Matches the key budget of the persisted
/// `implicit_expansion_cache` table.
pub(crate) const CONCEPT_INDEX_CACHE_MAX_KEYS: usize = 256;

/// Bytes one cache retains across all its indexes (512 MiB). One index over
/// a 350 K-concept system is roughly 150 MiB, so a few such sets fit.
pub(crate) const CONCEPT_INDEX_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;

/// Milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Slot {
    index: Arc<ImplicitConceptIndex>,
    bytes: usize,
    /// Updated under the shared read lock, so a hit never takes the write lock.
    last_used_ms: AtomicU64,
}

#[derive(Default)]
struct Slots {
    map: HashMap<String, Slot>,
    bytes: usize,
}

/// See the module documentation.
pub(crate) struct ConceptIndexCache {
    slots: RwLock<Slots>,
    max_keys: usize,
    max_bytes: usize,
}

impl Default for ConceptIndexCache {
    fn default() -> Self {
        Self::new(CONCEPT_INDEX_CACHE_MAX_KEYS, CONCEPT_INDEX_CACHE_MAX_BYTES)
    }
}

impl ConceptIndexCache {
    pub(crate) fn new(max_keys: usize, max_bytes: usize) -> Self {
        Self {
            slots: RwLock::new(Slots::default()),
            max_keys,
            max_bytes,
        }
    }

    /// The index stored under `key`, marking it as just used.
    pub(crate) fn get(&self, key: &str) -> Option<Arc<ImplicitConceptIndex>> {
        let guard = self.slots.read().ok()?;
        let slot = guard.map.get(key)?;
        slot.last_used_ms.store(now_ms(), Ordering::Relaxed);
        Some(slot.index.clone())
    }

    /// Store `index` under `key` and return the index now cached for it: the
    /// one already there when a concurrent caller won the race, otherwise
    /// `index`. An index larger than the whole byte budget is returned without
    /// being stored. Entries used longest ago are dropped to make room.
    pub(crate) fn insert(
        &self,
        key: &str,
        index: Arc<ImplicitConceptIndex>,
    ) -> Arc<ImplicitConceptIndex> {
        let bytes = key.len() + index.approx_bytes();
        if bytes > self.max_bytes || self.max_keys == 0 {
            return index;
        }
        let Ok(mut guard) = self.slots.write() else {
            return index;
        };
        if let Some(existing) = guard.map.get(key) {
            existing.last_used_ms.store(now_ms(), Ordering::Relaxed);
            return existing.index.clone();
        }
        while guard.map.len() >= self.max_keys || guard.bytes + bytes > self.max_bytes {
            let Some(oldest) = guard
                .map
                .iter()
                .min_by_key(|(_, slot)| slot.last_used_ms.load(Ordering::Relaxed))
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(evicted) = guard.map.remove(&oldest) {
                guard.bytes -= evicted.bytes;
            }
        }
        guard.bytes += bytes;
        guard.map.insert(
            key.to_owned(),
            Slot {
                index: index.clone(),
                bytes,
                last_used_ms: AtomicU64::new(now_ms()),
            },
        );
        index
    }

    /// Drop every entry.
    pub(crate) fn clear(&self) {
        if let Ok(mut guard) = self.slots.write() {
            guard.map.clear();
            guard.bytes = 0;
        }
    }

    /// Each key with the time it was last used, in milliseconds since the
    /// Unix epoch.
    pub(crate) fn recency(&self) -> Vec<(String, u64)> {
        self.slots
            .read()
            .map(|guard| {
                guard
                    .map
                    .iter()
                    .map(|(k, slot)| (k.clone(), slot.last_used_ms.load(Ordering::Relaxed)))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots.read().map(|g| g.map.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.slots.read().map(|g| g.bytes).unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.slots
            .read()
            .map(|g| g.map.contains_key(key))
            .unwrap_or(false)
    }

    /// Backdate `key` so tests can order entries without sleeping.
    #[cfg(test)]
    pub(crate) fn set_last_used(&self, key: &str, ms: u64) {
        if let Ok(guard) = self.slots.read() {
            if let Some(slot) = guard.map.get(key) {
                slot.last_used_ms.store(ms, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::value_set::{ImplicitConceptEntry, build_concept_index_from_entries};
    use super::*;

    fn index(codes: usize) -> Arc<ImplicitConceptIndex> {
        let entries = (0..codes)
            .map(|i| ImplicitConceptEntry {
                system_url: "http://example.org/cs".into(),
                code: format!("code-{i}"),
                display: Some(format!("display {i}")),
                code_lower: format!("code-{i}"),
                display_lower: format!("display {i}"),
            })
            .collect();
        Arc::new(build_concept_index_from_entries(entries))
    }

    #[test]
    fn the_key_budget_drops_the_entry_used_longest_ago() {
        let cache = ConceptIndexCache::new(2, usize::MAX);
        cache.insert("a", index(1));
        cache.insert("b", index(1));
        cache.set_last_used("a", 1);
        cache.set_last_used("b", 2);
        // Reading "a" makes "b" the least recently used entry.
        assert!(cache.get("a").is_some());
        cache.insert("c", index(1));
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key("a"));
        assert!(!cache.contains_key("b"));
        assert!(cache.contains_key("c"));
    }

    #[test]
    fn the_byte_budget_bounds_what_is_retained() {
        let one = index(50);
        let each = 1 + one.approx_bytes();
        let cache = ConceptIndexCache::new(1_000, each * 3);
        for key in ["a", "b", "c", "d", "e", "f"] {
            cache.insert(key, index(50));
        }
        assert_eq!(cache.len(), 3);
        assert!(cache.bytes() <= each * 3);
    }

    #[test]
    fn an_index_over_the_whole_budget_is_served_but_not_kept() {
        let big = index(500);
        let cache = ConceptIndexCache::new(10, big.approx_bytes() / 2);
        let served = cache.insert("big", big.clone());
        assert!(Arc::ptr_eq(&served, &big));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn a_second_insert_keeps_the_first_index() {
        let cache = ConceptIndexCache::new(4, usize::MAX);
        let first = cache.insert("a", index(1));
        let second = cache.insert("a", index(2));
        assert!(Arc::ptr_eq(&first, &second));
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.bytes(), 0);
    }
}
