//! Cached fact: "this `(tenant, resource type, parameter)` has no `search_index`
//! row with `last_updated IS NULL`".
//!
//! The newest-first search fast path orders by `search_index.last_updated`, so it
//! is only valid when no row of the searched parameter lacks that column
//! (contained-resource rows are written with NULL, and so are rows older than
//! schema v18). Every fast-path statement used to carry an `index_eligible` CTE
//! that re-ran the whole search predicate to prove it, which on date searches
//! cost as much as the search itself. This module learns which `(type, param)`
//! pairs of a tenant have such rows and remembers the answer.
//!
//! # Safety model
//!
//! * A `false` answer ("NULL rows exist") or any probe failure keeps the
//!   original SQL, so the cache can only ever *remove* the probe, never add a
//!   result the old path would not return.
//! * Writes by this process that can create NULL rows ([`note_null_rows`]) pin
//!   the key to "NULL rows exist" for the life of the process. The note is made
//!   before the writing transaction commits, so a search cannot cache a stale
//!   "none" for a key this process has written NULLs for.
//! * Writes by *other* processes are only seen after [`TTL`]. Deployments that
//!   run several writers against one database and create contained resources
//!   can opt out with `HFS_PG_ELIGIBLE_CACHE=0`.
//!
//! # Why a tenant-wide set, not a per-parameter probe
//!
//! No index serves `param_name = $3 AND last_updated IS NULL` (the recent-first
//! indexes are partial on the value column), so a per-parameter probe scanned
//! 280k buffers (~1 s) on the 30% benchmark corpus. Under the `Denormalized`
//! layout the fast path already requires, the only rows without `last_updated`
//! are the contained-resource memberships, which the writer flags
//! `is_contained`; `idx_search_contained` is partial on exactly that flag. One
//! `SELECT DISTINCT resource_type, param_name ... WHERE tenant_id = $1 AND
//! is_contained` therefore yields every `(type, param)` with NULL rows, and the
//! answer for all parameters of the tenant is cached together. Loading is
//! single-flight: while one request loads, others keep the original SQL.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a "no NULL rows" answer is trusted.
const TTL: Duration = Duration::from_secs(30);
/// After [`TTL`], an answer is still served for this long to requests that find
/// another request already refreshing it, so a refresh never sends the whole
/// request stream back to the per-statement probe. The loader itself waits for
/// the fresh answer. Total staleness is therefore bounded by `TTL + GRACE`.
const GRACE: Duration = Duration::from_secs(10);
/// Upper bound on remembered keys; the table is cleared when it is exceeded.
const MAX_KEYS: usize = 4096;

type Key = (String, String, String);

struct TenantFact {
    at: Instant,
    /// `(resource_type, param_name)` pairs that have NULL-`last_updated` rows.
    with_nulls: HashSet<(String, String)>,
}

#[derive(Default)]
struct Facts {
    tenants: HashMap<String, TenantFact>,
    loading: HashSet<String>,
    pinned: HashSet<Key>,
}

static FACTS: OnceLock<Mutex<Facts>> = OnceLock::new();

fn facts() -> &'static Mutex<Facts> {
    FACTS.get_or_init(|| Mutex::new(Facts::default()))
}

/// `HFS_PG_ELIGIBLE_CACHE`, read once. Default on; `0`, `false`, `off`, `no`
/// disable it.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        !matches!(
            std::env::var("HFS_PG_ELIGIBLE_CACHE")
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Ok("0" | "false" | "off" | "no")
        )
    })
}

fn key(tenant: &str, resource_type: &str, param: &str) -> Key {
    (
        tenant.to_string(),
        resource_type.to_string(),
        param.to_string(),
    )
}

/// Records that this process is writing a NULL-`last_updated` row for the key.
pub(crate) fn note_null_rows(tenant: &str, resource_type: &str, param: &str) {
    if let Ok(mut f) = facts().lock() {
        if f.pinned.len() < MAX_KEYS * 4 {
            f.pinned.insert(key(tenant, resource_type, param));
        }
    }
}

/// Extracts the literal `param_name = '<name>' AND ...` the query builder puts
/// at the head of a single-parameter predicate. `None` for any other shape, in
/// which case the caller keeps the probe.
pub(crate) fn param_name_of(pred: &str) -> Option<String> {
    let rest = pred.strip_prefix("param_name = '")?;
    let mut name = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                name.push('\'');
                continue;
            }
            // Closing quote: the predicate must continue as a conjunction.
            let tail: String = chars.collect();
            return (tail.is_empty() || tail.starts_with(" AND ")).then_some(name);
        }
        name.push(c);
    }
    None
}

enum Cached {
    NoNulls,
    HasNulls,
    /// Past [`TTL`] but within [`GRACE`]; carries whether NULL rows were known.
    Stale { has_nulls: bool },
    Unknown,
}

fn lookup(tenant: &str, resource_type: &str, param: &str) -> Cached {
    let Ok(f) = facts().lock() else {
        return Cached::HasNulls;
    };
    if f.pinned.contains(&key(tenant, resource_type, param)) {
        return Cached::HasNulls;
    }
    let Some(t) = f.tenants.get(tenant) else {
        return Cached::Unknown;
    };
    let has_nulls = t
        .with_nulls
        .contains(&(resource_type.to_string(), param.to_string()));
    let age = t.at.elapsed();
    if age < TTL {
        if has_nulls {
            Cached::HasNulls
        } else {
            Cached::NoNulls
        }
    } else if age < TTL + GRACE {
        Cached::Stale { has_nulls }
    } else {
        Cached::Unknown
    }
}

/// Releases the single-flight claim even if the loading request is cancelled
/// mid-query (a dropped future would otherwise pin every later request to the
/// probe).
struct LoadClaim(String);

impl Drop for LoadClaim {
    fn drop(&mut self) {
        if let Ok(mut f) = facts().lock() {
            f.loading.remove(&self.0);
        }
    }
}

const LOAD_SQL: &str = "SELECT DISTINCT resource_type, param_name FROM search_index \
     WHERE tenant_id = $1 AND is_contained";

/// True only when it is known that the parameter has no NULL-`last_updated`
/// rows. Errors, unknowns and in-flight loads answer false.
pub(crate) async fn param_has_no_null_rows(
    client: &deadpool_postgres::Client,
    tenant: &str,
    resource_type: &str,
    param: &str,
) -> bool {
    let stale = match lookup(tenant, resource_type, param) {
        Cached::NoNulls => return true,
        Cached::HasNulls => return false,
        Cached::Stale { has_nulls } => Some(has_nulls),
        Cached::Unknown => None,
    };
    {
        let Ok(mut f) = facts().lock() else {
            return false;
        };
        if !f.loading.insert(tenant.to_string()) {
            // Someone else is refreshing: trust the recent answer if there is
            // one, otherwise keep the probe.
            return stale.is_some_and(|has_nulls| !has_nulls);
        }
    }
    let _claim = LoadClaim(tenant.to_string());
    let loaded = client.query(LOAD_SQL, &[&tenant]).await;
    let Ok(mut f) = facts().lock() else {
        return false;
    };
    let Ok(rows) = loaded else {
        return false;
    };
    if f.tenants.len() >= MAX_KEYS {
        f.tenants.clear();
    }
    f.tenants.insert(
        tenant.to_string(),
        TenantFact {
            at: Instant::now(),
            with_nulls: rows
                .iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
                .collect(),
        },
    );
    let pinned = f.pinned.contains(&key(tenant, resource_type, param));
    let hit = f.tenants.get(tenant).is_some_and(|t| {
        t.with_nulls
            .contains(&(resource_type.to_string(), param.to_string()))
    });
    !pinned && !hit
}

#[cfg(test)]
pub(crate) fn forget_tenant_for_tests(tenant: &str) {
    let mut f = facts().lock().unwrap();
    f.tenants.remove(tenant);
    f.pinned.retain(|k| k.0 != tenant);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learn(tenant: &str, nulls: &[(&str, &str)], age: Duration) -> bool {
        let Some(at) = Instant::now().checked_sub(age) else {
            return false;
        };
        facts().lock().unwrap().tenants.insert(
            tenant.to_string(),
            TenantFact {
                at,
                with_nulls: nulls
                    .iter()
                    .map(|(a, b)| (a.to_string(), b.to_string()))
                    .collect(),
            },
        );
        true
    }

    #[test]
    fn parses_param_name_from_predicates() {
        assert_eq!(
            param_name_of("param_name = 'birthdate' AND (value_date < $3)").as_deref(),
            Some("birthdate")
        );
        assert_eq!(
            param_name_of("param_name = 'it''s' AND x").as_deref(),
            Some("it's")
        );
        assert_eq!(param_name_of("param_name = 'a'").as_deref(), Some("a"));
        assert_eq!(param_name_of("param_name IN ('a','b') AND x"), None);
        assert_eq!(param_name_of("param_name = 'a' OR x"), None);
        assert_eq!(param_name_of("value_date < $3"), None);
    }

    #[test]
    fn learned_sets_answer_per_type_and_param() {
        assert!(learn(
            "t-set",
            &[("Observation", "subject")],
            Duration::ZERO
        ));
        assert!(matches!(
            lookup("t-set", "Observation", "subject"),
            Cached::HasNulls
        ));
        assert!(matches!(
            lookup("t-set", "Observation", "code"),
            Cached::NoNulls
        ));
        assert!(matches!(
            lookup("t-set", "Patient", "subject"),
            Cached::NoNulls
        ));
    }

    #[test]
    fn pinned_keys_never_report_no_nulls() {
        assert!(learn("t-pin", &[], Duration::ZERO));
        assert!(matches!(
            lookup("t-pin", "Patient", "name"),
            Cached::NoNulls
        ));
        note_null_rows("t-pin", "Patient", "name");
        assert!(matches!(
            lookup("t-pin", "Patient", "name"),
            Cached::HasNulls
        ));
        assert!(matches!(
            lookup("t-pin", "Patient", "other"),
            Cached::NoNulls
        ));
    }

    #[test]
    fn recently_expired_entries_are_stale_and_old_ones_unknown() {
        if learn("t-stale", &[("Patient", "name")], TTL + Duration::from_secs(1)) {
            assert!(matches!(
                lookup("t-stale", "Patient", "name"),
                Cached::Stale { has_nulls: true }
            ));
            assert!(matches!(
                lookup("t-stale", "Patient", "gender"),
                Cached::Stale { has_nulls: false }
            ));
        }
        if learn("t-old", &[], TTL + GRACE + Duration::from_secs(1)) {
            assert!(matches!(
                lookup("t-old", "Patient", "name"),
                Cached::Unknown
            ));
        }
    }
}
