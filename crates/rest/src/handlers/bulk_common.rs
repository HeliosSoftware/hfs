//! Shared helpers for the Bulk Data **Export** and **Submit** handlers.
//!
//! Both operations parse `Prefer` headers and `Parameters` request bodies the same
//! way; these helpers live here so the two handler modules don't duplicate them.

use axum::http::HeaderMap;
use chrono::Utc;

use crate::error::RestError;

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

/// Parses a FHIR `instant` into a UTC datetime.
pub(crate) fn parse_instant(s: &str) -> Result<chrono::DateTime<Utc>, RestError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RestError::BadRequest {
            message: format!("invalid instant '{s}': {e}"),
        })
}

/// Reads the `Prefer: handling=` directive (`strict` / `lenient`).
pub(crate) fn prefer_handling(headers: &HeaderMap) -> Option<String> {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .and_then(|p| {
            p.split(',')
                .map(|s| s.trim())
                .find_map(|s| s.strip_prefix("handling="))
                .map(|s| s.to_ascii_lowercase())
        })
}

/// Returns true if `Prefer: respond-async` is present.
pub(crate) fn has_respond_async(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .map(|p| {
            p.split(',')
                .any(|s| s.trim().eq_ignore_ascii_case("respond-async"))
        })
        .unwrap_or(false)
}

/// Builds flat parameter pairs from a POST `Parameters` resource body.
///
/// Handles the common scalar `value[x]` shapes plus `valueReference.reference`.
/// Structured/nested parameters (`part`, `valueIdentifier`, `valueCoding`) are
/// parsed by the caller directly from the JSON when needed.
pub(crate) fn pairs_from_parameters(body: &serde_json::Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(arr) = body.get("parameter").and_then(|p| p.as_array()) {
        for p in arr {
            let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let value = p
                .get("valueString")
                .or_else(|| p.get("valueUri"))
                .or_else(|| p.get("valueUrl"))
                .or_else(|| p.get("valueInstant"))
                .or_else(|| p.get("valueCode"))
                .or_else(|| p.get("valueDateTime"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    p.get("valueReference")
                        .and_then(|r| r.get("reference"))
                        .and_then(|r| r.as_str())
                });
            if let Some(v) = value {
                pairs.push((name.to_string(), v.to_string()));
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use serde_json::json;

    #[test]
    fn test_parse_query_pairs_none_and_repeated() {
        assert!(parse_query_pairs(None).is_empty());
        let pairs = parse_query_pairs(Some("_type=Patient&_type=Observation&_since=2020"));
        assert_eq!(
            pairs,
            vec![
                ("_type".to_string(), "Patient".to_string()),
                ("_type".to_string(), "Observation".to_string()),
                ("_since".to_string(), "2020".to_string()),
            ]
        );
    }

    #[test]
    fn test_collect_multi_splits_and_trims() {
        let pairs = vec![
            ("_type".to_string(), "Patient, Observation".to_string()),
            ("_type".to_string(), "Condition".to_string()),
            ("other".to_string(), "ignored".to_string()),
        ];
        assert_eq!(
            collect_multi(&pairs, "_type"),
            vec!["Patient", "Observation", "Condition"]
        );
    }

    #[test]
    fn test_collect_multi_drops_empty_segments() {
        let pairs = vec![("_type".to_string(), "Patient,,Observation".to_string())];
        assert_eq!(
            collect_multi(&pairs, "_type"),
            vec!["Patient", "Observation"]
        );
    }

    #[test]
    fn test_first_value() {
        let pairs = vec![
            ("a".to_string(), "1".to_string()),
            ("a".to_string(), "2".to_string()),
        ];
        assert_eq!(first_value(&pairs, "a"), Some("1".to_string()));
        assert_eq!(first_value(&pairs, "missing"), None);
    }

    #[test]
    fn test_parse_instant_valid_and_invalid() {
        assert!(parse_instant("2021-01-01T00:00:00Z").is_ok());
        let err = parse_instant("not-a-date").unwrap_err();
        assert!(matches!(err, RestError::BadRequest { .. }));
    }

    #[test]
    fn test_prefer_handling() {
        let mut headers = HeaderMap::new();
        headers.insert("prefer", "respond-async, handling=STRICT".parse().unwrap());
        assert_eq!(prefer_handling(&headers), Some("strict".to_string()));

        let empty = HeaderMap::new();
        assert_eq!(prefer_handling(&empty), None);
    }

    #[test]
    fn test_has_respond_async() {
        let mut headers = HeaderMap::new();
        headers.insert("prefer", "respond-async".parse().unwrap());
        assert!(has_respond_async(&headers));

        let mut other = HeaderMap::new();
        other.insert("prefer", "handling=lenient".parse().unwrap());
        assert!(!has_respond_async(&other));

        assert!(!has_respond_async(&HeaderMap::new()));
    }

    #[test]
    fn test_pairs_from_parameters_scalar_and_reference() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_type", "valueString": "Patient"},
                {"name": "manifest", "valueUrl": "http://example.com/m.json"},
                {"name": "_since", "valueInstant": "2021-01-01T00:00:00Z"},
                {"name": "group", "valueReference": {"reference": "Group/123"}},
                {"name": "no-value"},
            ]
        });
        let pairs = pairs_from_parameters(&body);
        assert_eq!(first_value(&pairs, "_type"), Some("Patient".to_string()));
        assert_eq!(
            first_value(&pairs, "manifest"),
            Some("http://example.com/m.json".to_string())
        );
        assert_eq!(
            first_value(&pairs, "_since"),
            Some("2021-01-01T00:00:00Z".to_string())
        );
        assert_eq!(first_value(&pairs, "group"), Some("Group/123".to_string()));
        assert_eq!(first_value(&pairs, "no-value"), None);
    }

    #[test]
    fn test_pairs_from_parameters_empty() {
        assert!(pairs_from_parameters(&json!({})).is_empty());
    }
}
