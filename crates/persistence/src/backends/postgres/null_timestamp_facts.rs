//! Cached fact: "this `(tenant, resource type, parameter)` has no `search_index`
//! row with `last_updated IS NULL`".
//!
//! The newest-first search fast path orders by `search_index.last_updated`, so it
//! is only valid when no row of the searched parameter lacks that column
//! (contained-resource rows are written with NULL, and so are rows older than
//! schema v18). Every fast-path statement used to carry an `index_eligible` CTE
//! that re-ran the whole search predicate to prove it, which on date searches
//! cost as much as the search itself. This module answers the question once per
//! `(tenant, type, param)` with a bounded index probe and remembers the answer.
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
//! The per-parameter probe is one seek in the `(tenant_id, resource_type,
//! param_name, last_updated DESC, ...)` indexes, where NULLs sort first.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a "no NULL rows" answer is trusted.
const TTL: Duration = Duration::from_secs(30);
/// Upper bound on remembered keys; the table is cleared when it is exceeded.
const MAX_KEYS: usize = 4096;

type Key = (String, String, String);

#[derive(Default)]
struct Facts {
    checked: HashMap<Key, (Instant, bool)>,
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
        let k = key(tenant, resource_type, param);
        f.checked.remove(&k);
        if f.pinned.len() < MAX_KEYS * 4 {
            f.pinned.insert(k);
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
    Unknown,
}

fn lookup(k: &Key) -> Cached {
    let Ok(f) = facts().lock() else {
        return Cached::HasNulls;
    };
    if f.pinned.contains(k) {
        return Cached::HasNulls;
    }
    match f.checked.get(k) {
        Some((at, none)) if at.elapsed() < TTL => {
            if *none {
                Cached::NoNulls
            } else {
                Cached::HasNulls
            }
        }
        _ => Cached::Unknown,
    }
}

const PROBE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM search_index \
     WHERE tenant_id = $1 AND resource_type = $2 AND param_name = $3 \
       AND last_updated IS NULL)";

/// True only when it is known that the parameter has no NULL-`last_updated`
/// rows. Errors and unknowns answer false.
pub(crate) async fn param_has_no_null_rows(
    client: &deadpool_postgres::Client,
    tenant: &str,
    resource_type: &str,
    param: &str,
) -> bool {
    let k = key(tenant, resource_type, param);
    match lookup(&k) {
        Cached::NoNulls => return true,
        Cached::HasNulls => return false,
        Cached::Unknown => {}
    }
    let Ok(row) = client
        .query_one(PROBE_SQL, &[&tenant, &resource_type, &param])
        .await
    else {
        return false;
    };
    let has_nulls: bool = row.get(0);
    if let Ok(mut f) = facts().lock() {
        if f.pinned.contains(&k) {
            return false;
        }
        if f.checked.len() >= MAX_KEYS {
            f.checked.clear();
        }
        f.checked.insert(k, (Instant::now(), !has_nulls));
    }
    !has_nulls
}

#[cfg(test)]
pub(crate) fn forget_tenant_for_tests(tenant: &str) {
    let mut f = facts().lock().unwrap();
    f.checked.retain(|k, _| k.0 != tenant);
    f.pinned.retain(|k| k.0 != tenant);
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pinned_keys_never_report_no_nulls() {
        let k = key("t-pin", "Patient", "name");
        facts()
            .lock()
            .unwrap()
            .checked
            .insert(k.clone(), (Instant::now(), true));
        assert!(matches!(lookup(&k), Cached::NoNulls));
        note_null_rows("t-pin", "Patient", "name");
        assert!(matches!(lookup(&k), Cached::HasNulls));
    }

    #[test]
    fn expired_entries_are_unknown() {
        let k = key("t-old", "Patient", "name");
        let Some(old) = Instant::now().checked_sub(TTL + Duration::from_secs(1)) else {
            return; // monotonic clock too young to represent an expired entry
        };
        facts()
            .lock()
            .unwrap()
            .checked
            .insert(k.clone(), (old, true));
        assert!(matches!(lookup(&k), Cached::Unknown));
    }
}
