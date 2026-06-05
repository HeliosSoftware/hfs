//! URI parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::{SearchModifier, SearchParameter};

/// Builds an ES query clause for a URI search parameter.
pub fn build_clause(param: &SearchParameter, value: &str) -> Option<Value> {
    let name = &param.name;

    let condition = match param.modifier {
        Some(SearchModifier::Contains) => {
            // :contains - case-insensitive substring match on the URI.
            json!({
                "wildcard": {
                    "search_params.uri.value": {
                        "value": format!("*{}*", value),
                        "case_insensitive": true
                    }
                }
            })
        }
        Some(SearchModifier::Below) => {
            // :below - Match URIs that start with the given value
            json!({
                "bool": {
                    "should": [
                        { "term": { "search_params.uri.value": value } },
                        { "prefix": { "search_params.uri.value": format!("{}/", value.trim_end_matches('/')) } }
                    ],
                    "minimum_should_match": 1
                }
            })
        }
        Some(SearchModifier::Above) => {
            // :above - Match URIs that are prefixes of the given value
            // This requires checking if the stored URI is a prefix of the search value
            // Use script query or build a set of possible prefixes
            let prefixes = compute_parent_uris(value);
            if prefixes.is_empty() {
                json!({ "term": { "search_params.uri.value": value } })
            } else {
                json!({ "terms": { "search_params.uri.value": prefixes } })
            }
        }
        _ => {
            // Default: exact match
            json!({ "term": { "search_params.uri.value": value } })
        }
    };

    Some(json!({
        "nested": {
            "path": "search_params.uri",
            "query": {
                "bool": {
                    "must": [
                        { "term": { "search_params.uri.name": name } },
                        condition
                    ]
                }
            }
        }
    }))
}

/// Computes all parent URIs for :above matching.
///
/// For "http://example.org/fhir/ValueSet/123", returns:
/// - "http://example.org/fhir/ValueSet/123"
/// - "http://example.org/fhir/ValueSet"
/// - "http://example.org/fhir"
/// - "http://example.org"
fn compute_parent_uris(uri: &str) -> Vec<String> {
    let mut result = vec![uri.to_string()];

    // Strip query and fragment
    let base = uri
        .split('?')
        .next()
        .unwrap_or(uri)
        .split('#')
        .next()
        .unwrap_or(uri);

    // Find the scheme+authority part
    let scheme_end = if let Some(idx) = base.find("://") {
        // Find the first / after the authority
        base[idx + 3..].find('/').map(|i| idx + 3 + i)
    } else {
        None
    };

    let min_len = scheme_end.unwrap_or(0);

    let mut current = base.to_string();
    while let Some(last_slash) = current.rfind('/') {
        if last_slash < min_len {
            break;
        }
        current.truncate(last_slash);
        if !current.is_empty() {
            result.push(current.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchParamType, SearchValue};

    fn make_param(modifier: Option<SearchModifier>) -> SearchParameter {
        SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier,
            values: vec![SearchValue::eq("http://example.org")],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn test_exact_match() {
        let param = make_param(None);
        let clause = build_clause(&param, "http://example.org/fhir").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("\"term\""));
        assert!(s.contains("http://example.org/fhir"));
    }

    #[test]
    fn test_below() {
        let param = make_param(Some(SearchModifier::Below));
        let clause = build_clause(&param, "http://example.org/fhir").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("prefix"));
    }

    #[test]
    fn test_parent_uris() {
        let parents = compute_parent_uris("http://example.org/fhir/ValueSet/123");
        assert!(parents.contains(&"http://example.org/fhir/ValueSet/123".to_string()));
        assert!(parents.contains(&"http://example.org/fhir/ValueSet".to_string()));
        assert!(parents.contains(&"http://example.org/fhir".to_string()));
        assert!(parents.contains(&"http://example.org".to_string()));
    }
}
