//! Shared helpers for parsing query/form parameters while preserving repeated
//! keys.
//!
//! FHIR search uses repeated parameters for AND semantics (e.g.
//! `?language=en&language=fr` means *both*, and `?_include=A&_include=B` requests
//! two includes). Extracting parameters into a `HashMap<String, String>` would
//! collapse duplicate keys to a single (last-wins) value, silently dropping the
//! others. These helpers parse into an ordered `Vec<(String, String)>` that keeps
//! every occurrence.

/// Parses a raw query string into ordered key/value pairs (repeated keys kept).
pub(crate) fn parse_query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    match raw {
        None => Vec::new(),
        Some(q) => url::form_urlencoded::parse(q.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect(),
    }
}

/// Collects all values for `key`, splitting each on `,`.
///
/// Handles both FHIR multi-value forms: comma-separated (`?_include=A,B`) and
/// repeated keys (`?_include=A&_include=B`).
pub(crate) fn collect_multi(pairs: &[(String, String)], key: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(k, _)| k == key)
        .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Returns the first value for `key`, if any.
pub(crate) fn first_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Returns the last value for `key`, if any.
///
/// Use for genuinely single-valued control params (`_format`, `_count`,
/// `_summary`, …) where HTTP "last wins" is the conventional behavior.
pub(crate) fn last_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}
