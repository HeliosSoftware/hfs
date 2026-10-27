//! Modifier handlers for Elasticsearch queries.
//!
//! Handles cross-cutting modifiers like `:missing`.

use serde_json::{Value, json};

use crate::types::{SearchParamType, SearchParameter};

/// Builds a clause for the `:missing` modifier.
///
/// `:missing=true` matches resources that do NOT have a value for the parameter.
/// `:missing=false` matches resources that DO have a value for the parameter.
pub fn build_missing_clause(param: &SearchParameter) -> Option<Value> {
    let is_missing = param
        .values
        .first()
        .map(|v| v.value == "true")
        .unwrap_or(true);

    let exists_query = match param.name.as_str() {
        "_id" => json!({ "exists": { "field": "resource_id" } }),
        "_lastUpdated" => json!({ "exists": { "field": "last_updated" } }),
        _ => {
            let path = nested_path_for_type(param.param_type);
            let name_field = format!("{}.name", path);
            json!({
                "nested": {
                    "path": path,
                    "query": {
                        "term": { name_field: &param.name }
                    }
                }
            })
        }
    };

    if is_missing {
        // Resource does NOT have this parameter
        Some(json!({
            "bool": {
                "must_not": [exists_query]
            }
        }))
    } else {
        // Resource DOES have this parameter
        Some(exists_query)
    }
}

/// Returns the nested path for a search parameter type.
fn nested_path_for_type(param_type: SearchParamType) -> &'static str {
    match param_type {
        SearchParamType::String => "search_params.string",
        SearchParamType::Token => "search_params.token",
        SearchParamType::Date => "search_params.date",
        SearchParamType::Number => "search_params.number",
        SearchParamType::Quantity => "search_params.quantity",
        SearchParamType::Reference => "search_params.reference",
        SearchParamType::Uri => "search_params.uri",
        SearchParamType::Composite => "search_params.composite",
        SearchParamType::Special => "search_params.string",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchModifier, SearchValue};

    #[test]
    fn test_missing_true() {
        let param = SearchParameter {
            name: "email".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            chain: vec![],
            components: vec![],
        };
        let clause = build_missing_clause(&param).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("must_not"));
    }

    #[test]
    fn test_missing_false() {
        let param = SearchParameter {
            name: "email".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("false")],
            chain: vec![],
            components: vec![],
        };
        let clause = build_missing_clause(&param).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(!s.contains("must_not"));
    }
}
