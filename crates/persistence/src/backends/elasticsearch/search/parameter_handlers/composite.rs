//! Composite parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::{SearchParamType, SearchParameter};

/// Builds an ES query clause for a composite search parameter.
///
/// Composite parameters combine two or more components that must all match
/// within the same composite instance. Each instance is indexed as one nested
/// object under `search_params.composite` with the component values inline (as
/// arrays), so a single nested query with one condition per component enforces
/// same-instance matching. The value format is `value1$value2$...`.
pub fn build_clause(param: &SearchParameter, value: &str) -> Option<Value> {
    let name = &param.name;
    let parts: Vec<&str> = value.split('$').collect();

    let mut must: Vec<Value> = vec![json!({ "term": { "search_params.composite.name": name } })];

    if param.components.is_empty() {
        // Component definitions not resolved (e.g. standalone ES registry) —
        // fall back to matching the composite name only.
        return Some(nested(must));
    }

    if parts.len() != param.components.len() {
        return Some(nested(never_match(must)));
    }

    for (part, component) in parts.iter().zip(param.components.iter()) {
        match component_conditions(component.param_type, part) {
            Some(conds) => must.extend(conds),
            None => return Some(nested(never_match(must))),
        }
    }

    Some(nested(must))
}

/// Wraps the must-conditions in a nested query on the composite path.
fn nested(must: Vec<Value>) -> Value {
    json!({
        "nested": {
            "path": "search_params.composite",
            "query": { "bool": { "must": must } }
        }
    })
}

/// Appends an impossible condition so the clause matches nothing.
fn never_match(mut must: Vec<Value>) -> Vec<Value> {
    must.push(json!({ "term": { "search_params.composite.group_id": -1 } }));
    must
}

/// Builds the ES condition(s) for a single component value.
fn component_conditions(param_type: SearchParamType, part: &str) -> Option<Vec<Value>> {
    let field = |f: &str| format!("search_params.composite.{}", f);
    match param_type {
        SearchParamType::Token => {
            let conds = if let Some((system, code)) = part.split_once('|') {
                let mut v = Vec::new();
                if !system.is_empty() {
                    v.push(json!({ "term": { field("token_system"): system } }));
                }
                if !code.is_empty() {
                    v.push(json!({ "term": { field("token_code"): code } }));
                }
                v
            } else {
                vec![json!({ "term": { field("token_code"): part } })]
            };
            Some(conds)
        }
        SearchParamType::String => Some(vec![json!({
            "term": { field("string.lowercase"): part.to_lowercase() }
        })]),
        SearchParamType::Number => {
            let (op, num) = parse_prefixed_number(part)?;
            Some(vec![numeric_condition(&field("number"), op, num)])
        }
        SearchParamType::Quantity => {
            let mut comps = part.splitn(3, '|');
            let (op, num) = parse_prefixed_number(comps.next()?)?;
            let mut v = vec![numeric_condition(&field("quantity_value"), op, num)];
            let _system = comps.next();
            if let Some(code) = comps.next() {
                if !code.is_empty() {
                    v.push(json!({ "term": { field("quantity_unit"): code } }));
                }
            }
            Some(v)
        }
        SearchParamType::Date => {
            let (op, val) = split_prefix(part);
            Some(vec![range_condition(&field("date"), op, json!(val))])
        }
        SearchParamType::Reference => Some(vec![json!({ "term": { field("reference"): part } })]),
        SearchParamType::Uri => Some(vec![json!({ "term": { field("uri"): part } })]),
        SearchParamType::Composite | SearchParamType::Special => None,
    }
}

/// Splits a leading FHIR comparison prefix (`eq`/`gt`/…) from a value.
fn split_prefix(part: &str) -> (&str, &str) {
    for p in ["eq", "ne", "gt", "lt", "ge", "le", "sa", "eb", "ap"] {
        if let Some(rest) = part.strip_prefix(p) {
            return (p, rest);
        }
    }
    ("eq", part)
}

fn parse_prefixed_number(part: &str) -> Option<(&str, f64)> {
    let (op, rest) = split_prefix(part);
    rest.parse::<f64>().ok().map(|n| (op, n))
}

/// Builds a numeric term/range condition honoring the comparison prefix.
fn numeric_condition(field: &str, op: &str, num: f64) -> Value {
    match op {
        "gt" | "sa" => json!({ "range": { field: { "gt": num } } }),
        "lt" | "eb" => json!({ "range": { field: { "lt": num } } }),
        "ge" => json!({ "range": { field: { "gte": num } } }),
        "le" => json!({ "range": { field: { "lte": num } } }),
        _ => json!({ "term": { field: num } }),
    }
}

/// Builds a date range condition honoring the comparison prefix.
fn range_condition(field: &str, op: &str, val: Value) -> Value {
    match op {
        "gt" | "sa" => json!({ "range": { field: { "gt": val } } }),
        "lt" | "eb" => json!({ "range": { field: { "lt": val } } }),
        "ge" => json!({ "range": { field: { "gte": val } } }),
        "le" => json!({ "range": { field: { "lte": val } } }),
        _ => json!({ "range": { field: { "gte": val, "lte": val } } }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompositeSearchComponent, SearchValue};

    fn composite_param(components: Vec<CompositeSearchComponent>) -> SearchParameter {
        SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq("8867-4$120")],
            chain: vec![],
            components,
        }
    }

    #[test]
    fn name_only_when_no_components() {
        let clause = build_clause(&composite_param(vec![]), "8867-4$120").unwrap();
        let s = clause.to_string();
        assert!(s.contains("search_params.composite.name"));
        assert!(!s.contains("token_code"));
    }

    #[test]
    fn evaluates_token_and_quantity_components() {
        let param = composite_param(vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "code".to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".to_string(),
            },
        ]);
        let clause = build_clause(&param, "http://loinc.org|8480-6$ge100").unwrap();
        let s = clause.to_string();
        assert!(s.contains("token_system"));
        assert!(s.contains("token_code"));
        assert!(s.contains("quantity_value"));
        assert!(s.contains("gte"));
    }

    #[test]
    fn arity_mismatch_never_matches() {
        let param = composite_param(vec![CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: "code".to_string(),
        }]);
        let clause = build_clause(&param, "a$b").unwrap();
        assert!(clause.to_string().contains("group_id"));
    }
}
