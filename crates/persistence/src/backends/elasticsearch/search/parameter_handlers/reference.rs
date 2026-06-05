//! Reference parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::{SearchModifier, SearchParameter};

/// Builds an ES query clause for a reference search parameter.
pub fn build_clause(param: &SearchParameter, value: &str) -> Option<Value> {
    let name = &param.name;

    if param.modifier == Some(SearchModifier::Identifier) {
        return build_identifier_clause(name, value);
    }

    // :contains - case-insensitive substring match on the stored reference.
    if param.modifier == Some(SearchModifier::Contains) {
        return Some(json!({
            "nested": {
                "path": "search_params.reference",
                "query": {
                    "bool": {
                        "must": [
                            { "term": { "search_params.reference.name": name } },
                            {
                                "wildcard": {
                                    "search_params.reference.reference": {
                                        "value": format!("*{}*", value),
                                        "case_insensitive": true
                                    }
                                }
                            }
                        ]
                    }
                }
            }
        }));
    }

    let mut must_conditions = vec![json!({ "term": { "search_params.reference.name": name } })];

    // Parse reference value
    if value.contains('/') {
        // Type/id format (e.g., "Patient/123") or full URL
        must_conditions.push(json!({ "term": { "search_params.reference.reference": value } }));
    } else {
        // Just an ID - match either resource_id or reference ending with /id
        must_conditions.push(json!({
            "bool": {
                "should": [
                    { "term": { "search_params.reference.resource_id": value } },
                    { "term": { "search_params.reference.reference": value } }
                ],
                "minimum_should_match": 1
            }
        }));
    }

    // If there's a type modifier, filter by resource_type
    if let Some(SearchModifier::Type(type_name)) = &param.modifier {
        must_conditions
            .push(json!({ "term": { "search_params.reference.resource_type": type_name } }));
    }

    Some(json!({
        "nested": {
            "path": "search_params.reference",
            "query": {
                "bool": {
                    "must": must_conditions
                }
            }
        }
    }))
}

/// Builds a :identifier clause that searches for references by identifier.
fn build_identifier_clause(name: &str, value: &str) -> Option<Value> {
    let mut must_conditions = vec![json!({ "term": { "search_params.token.name": name } })];

    if let Some((system, code)) = value.split_once('|') {
        if !system.is_empty() {
            must_conditions.push(json!({ "term": { "search_params.token.system": system } }));
        }
        must_conditions.push(json!({ "term": { "search_params.token.code": code } }));
    } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchParamType, SearchValue};

    fn make_param(name: &str, modifier: Option<SearchModifier>) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Reference,
            modifier,
            values: vec![SearchValue::eq("Patient/123")],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn test_relative_reference() {
        let param = make_param("subject", None);
        let clause = build_clause(&param, "Patient/123").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("Patient/123"));
    }

    #[test]
    fn test_id_only_reference() {
        let param = make_param("subject", None);
        let clause = build_clause(&param, "123").unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("resource_id"));
    }
}
