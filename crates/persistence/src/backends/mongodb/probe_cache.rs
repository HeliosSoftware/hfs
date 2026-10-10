//! Short-lived, bounded metadata for index-driver selection.
//!
//! These observations are never query results or proof that a predicate is
//! empty. Writes from this or another server can immediately invalidate them;
//! callers must execute every predicate and enforce their normal read bounds.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use mongodb::{
    bson::{self, Bson, Document, doc},
    options::Hint,
};
use parking_lot::Mutex;

const MAX_ENTRIES: usize = 1024;
const MAX_KEY_BYTES: usize = 4 * 1024 * 1024;
const MAX_SINGLE_KEY_BYTES: usize = 16 * 1024;
const TTL: Duration = Duration::from_secs(30);
// Avoid repeating an expired optional probe during a short burst of requests.
const UNKNOWN_TTL: Duration = Duration::from_secs(1);

/// Collision-free full BSON scope. Equivalent documents in another field
/// order may miss the cache, but cannot reuse another predicate's estimate.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ProbeKey(Vec<u8>);

impl ProbeKey {
    pub(super) fn offset_page(driver: &Self, remaining: &[Self]) -> Option<Self> {
        let binary = |key: &Self| {
            Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: key.0.clone(),
            })
        };
        let scope = doc! {
            "kind": "adaptive_offset_page",
            "driver": binary(driver),
            "remaining": remaining.iter().map(binary).collect::<Vec<_>>(),
        };
        let bytes = bson::to_vec(&scope).ok()?;
        (bytes.len() <= MAX_SINGLE_KEY_BYTES).then_some(Self(bytes))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        database: &str,
        collection: &str,
        tenant: &str,
        resource_type: &str,
        predicate: &Document,
        hint: Option<&Hint>,
        indexes_ready: bool,
        probe_limit: u64,
        definition_scope: &Bson,
    ) -> Option<Self> {
        let limit = i64::try_from(probe_limit).ok()?;
        let hint = match hint {
            None => Bson::Null,
            Some(Hint::Name(name)) => Bson::String(name.clone()),
            Some(Hint::Keys(keys)) => Bson::Document(keys.clone()),
            Some(_) => return None,
        };
        let scope = doc! {
            "database": database, "collection": collection, "tenant": tenant,
            "resource_type": resource_type, "predicate": predicate.clone(),
            "hint": hint, "indexes_ready": indexes_ready, "probe_limit": limit,
            "definition": definition_scope.clone(),
        };
        let bytes = bson::to_vec(&scope).ok()?;
        (bytes.len() <= MAX_SINGLE_KEY_BYTES).then_some(Self(bytes))
    }
}

/// Row cardinality observed by a bounded index probe, not resource cardinality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProbeEstimate {
    /// An optional probe timed out; this supplies no cardinality information.
    Unknown,
    /// The bounded probe finished below its cap. Still only a planning hint.
    Completed(u64),
    /// The cap was reached. The true row cardinality may be arbitrarily larger.
    AtLeast(u64),
}

impl ProbeEstimate {
    pub(super) fn from_bounded(rows: u64, limit: u64) -> Option<Self> {
        if rows == 0 || limit == 0 {
            // Never retain stale zeroes, including after concurrent insertion.
            None
        } else if rows >= limit {
            Some(Self::AtLeast(limit))
        } else {
            Some(Self::Completed(rows))
        }
    }

    pub(super) fn observed_rows(self) -> Option<u64> {
        match self {
            Self::Completed(rows) | Self::AtLeast(rows) => Some(rows),
            Self::Unknown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OffsetPageDecision {
    Bounded,
    Streaming,
}

#[derive(Clone, Copy)]
enum Observation {
    Cardinality(ProbeEstimate),
    OffsetPage(OffsetPageDecision),
}

struct Entry {
    observation: Observation,
    expires: Instant,
    last_used: u64,
}

#[derive(Default)]
struct State {
    entries: HashMap<ProbeKey, Entry>,
    key_bytes: usize,
    access: u64,
}

/// Per-backend cache: no cross-client sharing, persistence, or resource payloads.
/// Fixed entry and serialized-key budgets bound memory even for unique queries.
#[derive(Clone)]
pub(super) struct ProbeCache {
    state: Arc<Mutex<State>>,
    max_entries: usize,
    max_key_bytes: usize,
    ttl: Duration,
}

impl Default for ProbeCache {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            max_entries: MAX_ENTRIES,
            max_key_bytes: MAX_KEY_BYTES,
            ttl: TTL,
        }
    }
}

impl ProbeCache {
    pub(super) fn get(&self, key: &ProbeKey) -> Option<ProbeEstimate> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &ProbeKey, now: Instant) -> Option<ProbeEstimate> {
        match self.get_observation_at(key, now)? {
            Observation::Cardinality(estimate) => Some(estimate),
            Observation::OffsetPage(_) => None,
        }
    }

    pub(super) fn get_page_decision(&self, key: &ProbeKey) -> Option<OffsetPageDecision> {
        match self.get_observation_at(key, Instant::now())? {
            Observation::OffsetPage(decision) => Some(decision),
            Observation::Cardinality(_) => None,
        }
    }

    pub(super) fn remember_page_decision(&self, key: ProbeKey, decision: OffsetPageDecision) {
        self.store_at(key, Observation::OffsetPage(decision), Instant::now());
    }

    fn get_observation_at(&self, key: &ProbeKey, now: Instant) -> Option<Observation> {
        let mut state = self.state.lock();
        if state
            .entries
            .get(key)
            .is_some_and(|entry| now >= entry.expires)
        {
            state.entries.remove(key);
            state.key_bytes -= key.0.len();
            return None;
        }
        state.access = state.access.saturating_add(1);
        let access = state.access;
        let entry = state.entries.get_mut(key)?;
        entry.last_used = access;
        Some(entry.observation)
    }

    pub(super) fn insert(&self, key: ProbeKey, estimate: ProbeEstimate) {
        self.insert_at(key, estimate, Instant::now());
    }

    fn insert_at(&self, key: ProbeKey, estimate: ProbeEstimate, now: Instant) {
        if estimate.observed_rows() == Some(0) {
            return;
        }
        self.store_at(key, Observation::Cardinality(estimate), now);
    }

    fn store_at(&self, key: ProbeKey, observation: Observation, now: Instant) {
        if self.max_entries == 0 || key.0.len() > self.max_key_bytes {
            return;
        }
        let mut state = self.state.lock();
        state.entries.retain(|_, entry| now < entry.expires);
        state.key_bytes = state.entries.keys().map(|key| key.0.len()).sum();
        // A late timeout must not replace a still-fresh completed observation.
        if matches!(
            observation,
            Observation::Cardinality(ProbeEstimate::Unknown)
        ) && state.entries.get(&key).is_some_and(|entry| {
            matches!(
                entry.observation,
                Observation::Cardinality(ProbeEstimate::Completed(_) | ProbeEstimate::AtLeast(_))
            )
        }) {
            return;
        }
        if state.entries.remove(&key).is_some() {
            state.key_bytes -= key.0.len();
        }
        while state.entries.len() >= self.max_entries
            || state.key_bytes + key.0.len() > self.max_key_bytes
        {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            state.entries.remove(&oldest);
            state.key_bytes -= oldest.0.len();
        }
        state.access = state.access.saturating_add(1);
        let last_used = state.access;
        state.key_bytes += key.0.len();
        state.entries.insert(
            key,
            Entry {
                observation,
                expires: now
                    + match observation {
                        Observation::Cardinality(ProbeEstimate::Unknown) => UNKNOWN_TTL,
                        _ => self.ttl,
                    },
                last_used,
            },
        );
    }

    /// Definition refresh clears all metadata; a concurrent old probe may
    /// finish later, but its exact definition key cannot match a new definition.
    pub(super) fn clear(&self) {
        *self.state.lock() = State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: &str) -> ProbeKey {
        ProbeKey::new(
            "db",
            "search_index",
            "tenant",
            "Observation",
            &doc! { "param_name": "code", "value_token": value },
            None,
            true,
            10_001,
            &Bson::String("definition".to_owned()),
        )
        .unwrap()
    }

    #[test]
    fn every_scope_dimension_isolated_including_full_predicate_and_definition() {
        let base = key("a");
        let predicate = doc! { "param_name": "code", "value_token": "a" };
        let definition = Bson::String("definition".to_owned());
        let hint = Hint::Name("index".to_owned());
        for other in [
            ProbeKey::new(
                "other",
                "search_index",
                "tenant",
                "Observation",
                &predicate,
                None,
                true,
                10_001,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "contained",
                "tenant",
                "Observation",
                &predicate,
                None,
                true,
                10_001,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "search_index",
                "other",
                "Observation",
                &predicate,
                None,
                true,
                10_001,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Patient",
                &predicate,
                None,
                true,
                10_001,
                &definition,
            ),
            Some(key("b")),
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Observation",
                &predicate,
                Some(&hint),
                true,
                10_001,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Observation",
                &predicate,
                None,
                false,
                10_001,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Observation",
                &predicate,
                None,
                true,
                65,
                &definition,
            ),
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Observation",
                &predicate,
                None,
                true,
                10_001,
                &Bson::String("new expression".to_owned()),
            ),
        ] {
            assert_ne!(other.unwrap(), base);
        }
    }

    #[test]
    fn paging_keys_include_driver_and_every_full_condition() {
        let driver = key("driver");
        let base = ProbeKey::offset_page(&driver, &[key("a"), key("b")]).unwrap();
        assert_ne!(base, driver);
        for other in [
            ProbeKey::offset_page(&key("other-driver"), &[key("a"), key("b")]),
            ProbeKey::offset_page(&driver, &[key("other-value"), key("b")]),
            ProbeKey::offset_page(&driver, &[key("a")]),
            ProbeKey::offset_page(&driver, &[key("a"), key("b"), key("b")]),
        ] {
            assert_ne!(base, other.unwrap());
        }
        assert!(ProbeKey::offset_page(&driver, &vec![driver.clone(); 100]).is_none());
    }

    #[test]
    fn paging_decisions_expire_share_budgets_and_never_become_counts() {
        let cache = ProbeCache {
            max_entries: 2,
            ..ProbeCache::default()
        };
        let now = Instant::now();
        let page = ProbeKey::offset_page(&key("driver"), &[key("value")]).unwrap();
        cache.store_at(
            page.clone(),
            Observation::OffsetPage(OffsetPageDecision::Bounded),
            now,
        );
        assert_eq!(cache.get(&page), None);
        assert!(matches!(
            cache.get_observation_at(&page, now + TTL - Duration::from_nanos(1)),
            Some(Observation::OffsetPage(OffsetPageDecision::Bounded))
        ));
        assert!(cache.get_observation_at(&page, now + TTL).is_none());
        cache.insert(key("count"), ProbeEstimate::Completed(7));
        assert_eq!(cache.get_page_decision(&key("count")), None);
        cache.remember_page_decision(page.clone(), OffsetPageDecision::Streaming);
        let shared = cache.clone();
        assert_eq!(
            shared.get_page_decision(&page),
            Some(OffsetPageDecision::Streaming)
        );
        let other = ProbeKey::offset_page(&key("other"), &[]).unwrap();
        cache.remember_page_decision(other, OffsetPageDecision::Bounded);
        assert_eq!(cache.state.lock().entries.len(), 2);
        assert_eq!(cache.get(&key("count")), None);
        shared.clear();
        assert_eq!(cache.get_page_decision(&page), None);
        assert_eq!(cache.state.lock().key_bytes, 0);
        let byte_cache = ProbeCache {
            max_key_bytes: page.0.len() - 1,
            ..ProbeCache::default()
        };
        byte_cache.remember_page_decision(page.clone(), OffsetPageDecision::Bounded);
        assert_eq!(byte_cache.get_page_decision(&page), None);
    }

    #[test]
    fn lower_bounds_never_become_completed_counts_or_cached_zeroes() {
        assert_eq!(
            ProbeEstimate::from_bounded(10_001, 10_001),
            Some(ProbeEstimate::AtLeast(10_001))
        );
        assert_eq!(
            ProbeEstimate::from_bounded(37, 10_001),
            Some(ProbeEstimate::Completed(37))
        );
        assert_eq!(ProbeEstimate::from_bounded(0, 10_001), None);
        assert_eq!(ProbeEstimate::from_bounded(1, 0), None);
        let cache = ProbeCache::default();
        cache.insert(key("a"), ProbeEstimate::Completed(0));
        assert_eq!(cache.get(&key("a")), None);
    }

    #[test]
    fn ttl_expires_at_boundary_and_reads_do_not_extend_it() {
        let cache = ProbeCache::default();
        let now = Instant::now();
        cache.insert_at(key("a"), ProbeEstimate::Completed(3), now);
        assert_eq!(
            cache.get_at(&key("a"), now + TTL - Duration::from_nanos(1)),
            Some(ProbeEstimate::Completed(3))
        );
        assert_eq!(cache.get_at(&key("a"), now + TTL), None);
        assert_eq!(cache.state.lock().key_bytes, 0);
    }

    #[test]
    fn unknown_observations_expire_quickly_without_becoming_counts() {
        let cache = ProbeCache::default();
        let now = Instant::now();
        cache.insert_at(key("unknown"), ProbeEstimate::Unknown, now);
        cache.insert_at(key("known"), ProbeEstimate::Completed(3), now);
        assert_eq!(ProbeEstimate::Unknown.observed_rows(), None);
        assert_eq!(cache.get_page_decision(&key("unknown")), None);
        assert_eq!(cache.get_at(&key("other"), now), None);
        assert_eq!(
            cache.get_at(&key("unknown"), now + UNKNOWN_TTL - Duration::from_nanos(1)),
            Some(ProbeEstimate::Unknown)
        );
        assert_eq!(cache.get_at(&key("unknown"), now + UNKNOWN_TTL), None);
        assert_eq!(
            cache.get_at(&key("known"), now + UNKNOWN_TTL),
            Some(ProbeEstimate::Completed(3))
        );
        assert_eq!(cache.state.lock().key_bytes, key("known").0.len());
    }

    #[test]
    fn unknown_observations_share_eviction_budgets_and_refresh_invalidation() {
        let cache = ProbeCache {
            max_entries: 2,
            ..ProbeCache::default()
        };
        let now = Instant::now();
        cache.insert_at(key("a"), ProbeEstimate::Unknown, now);
        cache.insert_at(key("b"), ProbeEstimate::Completed(4), now);
        cache.insert_at(key("c"), ProbeEstimate::Unknown, now);
        assert_eq!(cache.get_at(&key("a"), now), None);
        assert_eq!(cache.get_at(&key("c"), now), Some(ProbeEstimate::Unknown));
        cache.clear();
        assert_eq!(cache.get_at(&key("c"), now), None);
        assert_eq!(cache.state.lock().key_bytes, 0);

        let cache = ProbeCache {
            max_key_bytes: key("a").0.len(),
            ..ProbeCache::default()
        };
        cache.insert_at(key("a"), ProbeEstimate::Unknown, now);
        cache.insert_at(key("b"), ProbeEstimate::Unknown, now);
        assert_eq!(cache.get_at(&key("a"), now), None);
        assert_eq!(cache.get_at(&key("b"), now), Some(ProbeEstimate::Unknown));
        assert_eq!(cache.state.lock().key_bytes, key("b").0.len());
    }

    #[test]
    fn late_timeouts_preserve_known_estimates_without_extending_expiry() {
        let cache = ProbeCache::default();
        let now = Instant::now();
        cache.insert_at(key("a"), ProbeEstimate::Unknown, now);
        cache.insert_at(key("a"), ProbeEstimate::Completed(4), now);
        cache.insert_at(key("a"), ProbeEstimate::Unknown, now + UNKNOWN_TTL);
        assert_eq!(
            cache.get_at(&key("a"), now + UNKNOWN_TTL),
            Some(ProbeEstimate::Completed(4))
        );
        assert_eq!(cache.get_at(&key("a"), now + TTL), None);
        cache.insert_at(key("a"), ProbeEstimate::Unknown, now + TTL);
        assert_eq!(
            cache.get_at(&key("a"), now + TTL),
            Some(ProbeEstimate::Unknown)
        );
    }

    #[test]
    fn entry_and_byte_budgets_evict_least_recently_used() {
        let cache = ProbeCache {
            max_entries: 2,
            ..ProbeCache::default()
        };
        cache.insert(key("a"), ProbeEstimate::Completed(1));
        cache.insert(key("b"), ProbeEstimate::AtLeast(10_001));
        assert!(cache.get(&key("a")).is_some());
        cache.insert(key("c"), ProbeEstimate::Completed(3));
        assert!(cache.get(&key("b")).is_none());
        assert!(cache.get(&key("a")).is_some());
        let byte_cache = ProbeCache {
            max_key_bytes: key("a").0.len(),
            ..ProbeCache::default()
        };
        byte_cache.insert(key("a"), ProbeEstimate::Completed(1));
        byte_cache.insert(key("b"), ProbeEstimate::Completed(2));
        assert!(byte_cache.get(&key("a")).is_none());
        assert_eq!(byte_cache.state.lock().entries.len(), 1);
        byte_cache.clear();
        assert_eq!(byte_cache.get(&key("b")), None);
        assert_eq!(byte_cache.state.lock().key_bytes, 0);
    }

    #[test]
    fn oversized_predicates_bypass_cache_and_lower_bound_round_trips() {
        assert!(
            ProbeKey::new(
                "db",
                "search_index",
                "tenant",
                "Observation",
                &doc! { "value": "x".repeat(MAX_SINGLE_KEY_BYTES) },
                None,
                true,
                10_001,
                &Bson::Null
            )
            .is_none()
        );
        let cache = ProbeCache::default();
        cache.insert(key("a"), ProbeEstimate::AtLeast(10_001));
        assert_eq!(cache.get(&key("a")), Some(ProbeEstimate::AtLeast(10_001)));
    }
}
