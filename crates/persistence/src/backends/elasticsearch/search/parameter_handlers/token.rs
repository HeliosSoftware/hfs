//! Token parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::{SearchModifier, SearchParameter};

/// Builds an ES query clause for a token search parameter.
pub fn build_clause(param: &SearchParameter, value: &str) -> Option<Value> {
    let name = &param.name;

    // Handle modifiers first
    match param.modifier {
        // `:not` builds the POSITIVE clause here; the query builder negates once,
        // around the OR of all values (#473). Negating per value made
        // `:not=a,b` mean "NOT a OR NOT b", so any resource carrying both values
        // satisfied one half of the OR and leaked back into the result set.
        Some(SearchModifier::Text) => {
            return build_text_clause(name, value);
        }
        Some(SearchModifier::TextAdvanced) => {
            return build_text_advanced_clause(name, value);
        }
        Some(SearchModifier::CodeText) => {
            return build_code_text_clause(name, value);
        }
        Some(SearchModifier::OfType) => {
            return build_of_type_clause(name, value);
        }
        _ => {}
    }

    build_token_condition(name, value, param.modifier.as_ref())
}

/// Builds the core token matching condition.
fn build_token_condition(
    name: &str,
    value: &str,
    _modifier: Option<&SearchModifier>,
) -> Option<Value> {
    let mut must_conditions = vec![json!({ "term": { "search_params.token.name": name } })];

    if let Some((system, code)) = value.split_once('|') {
        if system.is_empty() && !code.is_empty() {
            // |code - code with no system
            must_conditions.push(json!({ "term": { "search_params.token.code": code } }));
            must_conditions.push(json!({
                "bool": {
                    "must_not": [
                        { "exists": { "field": "search_params.token.system" } }
                    ]
                }
            }));
        } else if !system.is_empty() && code.is_empty() {
            // system| - any code in system
            must_conditions.push(json!({ "term": { "search_params.token.system": system } }));
        } else {
            // system|code - both must match
            must_conditions.push(json!({ "term": { "search_params.token.system": system } }));
            must_conditions.push(json!({ "term": { "search_params.token.code": code } }));
        }
    } else {
        // code only - match code in any system
        must_conditions.push(json!({ "term": { "search_params.token.code": value } }));
    }

    Some(json!({
        "nested": {
            "path": "search_params.token",
            "query": {
                "bool": {
                    "must": must_conditions
                }
            }
        }
    }))
}

/// Builds a :text modifier clause (search on display).
fn build_text_clause(name: &str, value: &str) -> Option<Value> {
    Some(json!({
        "nested": {
            "path": "search_params.token",
            "query": {
                "bool": {
                    "must": [
                        { "term": { "search_params.token.name": name } },
                        {
                            "match": {
                                "search_params.token.display": {
                                    "query": value,
                                    "operator": "and"
                                }
                            }
                        }
                    ]
                }
            }
        }
    }))
}

/// Builds a :text-advanced modifier clause (ES query string on display).
fn build_text_advanced_clause(name: &str, value: &str) -> Option<Value> {
    Some(json!({
        "nested": {
            "path": "search_params.token",
            "query": {
                "bool": {
                    "must": [
                        { "term": { "search_params.token.name": name } },
                        {
                            "query_string": {
                                "default_field": "search_params.token.display",
                                "query": value
                            }
                        }
                    ]
                }
            }
        }
    }))
}

/// Builds a :code-text modifier clause (case-insensitive starts-with on display).
fn build_code_text_clause(name: &str, value: &str) -> Option<Value> {
    Some(json!({
        "nested": {
            "path": "search_params.token",
            "query": {
                "bool": {
                    "must": [
                        { "term": { "search_params.token.name": name } },
                        { "match_phrase_prefix": { "search_params.token.display": value } }
                    ]
                }
            }
        }
    }))
}

/// Builds a :of-type modifier clause for Identifier types.
/// Format: type-system|type-code|value
fn build_of_type_clause(name: &str, value: &str) -> Option<Value> {
    let parts: Vec<&str> = value.splitn(3, '|').collect();
    if parts.len() < 3 {
        return None;
    }

    let type_system = parts[0];
    let type_code = parts[1];
    let identifier_value = parts[2];

    let mut must_conditions = vec![
        json!({ "term": { "search_params.token.name": name } }),
        json!({ "term": { "search_params.token.code": identifier_value } }),
    ];

    if !type_system.is_empty() {
        must_conditions
            .push(json!({ "term": { "search_params.token.identifier_type_system": type_system } }));
    }
    if !type_code.is_empty() {
        must_conditions
            .push(json!({ "term": { "search_params.token.identifier_type_code": type_code } }));
    }

    Some(json!({
        "nested": {
            "path": "search_params.token",
            "query": {
                "bool": {
                    "must": must_conditions
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchParamType, SearchValue};

    fn make_param(name: &str, modifier: Option<SearchModifier>) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: vec![SearchValue::eq("test")],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn test_code_only() {
        let param = make_param("code", None);
        let clause = build_clause(&param, "8867-4").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("8867-4"));
        assert!(s.contains("search_params.token.code"));
    }

    #[test]
    fn test_system_code() {
        let param = make_param("code", None);
        let clause = build_clause(&param, "http://loinc.org|8867-4").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("http://loinc.org"));
        assert!(s.contains("8867-4"));
    }

    #[test]
    fn test_not_modifier_builds_positive_clause() {
        // The query builder applies the negation once, around all values (#473);
        // the per-value clause must stay positive.
        let param = make_param("gender", Some(SearchModifier::Not));
        let clause = build_clause(&param, "male").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(!s.contains("must_not"));
        assert!(s.contains("search_params.token.code"));
        assert!(s.contains("male"));
    }

    #[test]
    fn test_text_modifier() {
        let param = make_param("code", Some(SearchModifier::Text));
        let clause = build_clause(&param, "headache").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("display"));
        assert!(s.contains("headache"));
    }
}
