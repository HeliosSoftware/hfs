//! Quantity parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::SearchPrefix;

/// Builds an ES query clause for a quantity search parameter.
///
/// Format: `[prefix]number|system|code` or `[prefix]number|code` or `[prefix]number`
pub fn build_clause(name: &str, value: &str, prefix: SearchPrefix) -> Option<Value> {
    let (num_str, system, code) = parse_quantity_value(value);
    let num: f64 = num_str.parse().ok()?;

    // Raw match against the stored value/unit/system.
    let mut raw_must = vec![
        json!({ "term": { "search_params.quantity.name": name } }),
        range_condition("search_params.quantity.value", prefix, num, num_str, |x| {
            Some(x)
        })?,
    ];
    if let Some(sys) = system {
        raw_must.push(json!({ "term": { "search_params.quantity.system": sys } }));
    }
    if let Some(c) = code {
        raw_must.push(json!({ "term": { "search_params.quantity.code": c } }));
    }

    // Canonical match: when a UCUM code is supplied and convertible, also match
    // rows whose canonical unit/value are equivalent (e.g. g ⇄ mg). Bounds are
    // canonicalized so range/precision semantics survive unit conversion.
    let canonical = code.and_then(|c| {
        let (_, canon_unit) = helios_fhirpath::ucum::canonicalize_quantity(num, c)?;
        let canon = |x: f64| helios_fhirpath::ucum::canonicalize_quantity(x, c).map(|(v, _)| v);
        let range = range_condition(
            "search_params.quantity.canonical_value",
            prefix,
            num,
            num_str,
            canon,
        )?;
        Some(json!({
            "bool": {
                "must": [
                    { "term": { "search_params.quantity.name": name } },
                    range,
                    { "term": { "search_params.quantity.canonical_unit": canon_unit } }
                ]
            }
        }))
    });

    let query = match canonical {
        Some(canon_clause) => json!({
            "bool": {
                "should": [ { "bool": { "must": raw_must } }, canon_clause ],
                "minimum_should_match": 1
            }
        }),
        None => json!({ "bool": { "must": raw_must } }),
    };

    Some(json!({
        "nested": {
            "path": "search_params.quantity",
            "query": query
        }
    }))
}

/// Builds an ES `range` condition for `field`, transforming the numeric bounds
/// through `map` (identity for the raw value, UCUM-canonicalization for the
/// canonical column). Returns `None` if a transformed bound is unavailable.
fn range_condition(
    field: &str,
    prefix: SearchPrefix,
    num: f64,
    num_str: &str,
    map: impl Fn(f64) -> Option<f64>,
) -> Option<Value> {
    // gt/lt/ge/le/sa/eb ignore the implicit precision and compare against the
    // exact search value (FHIR spec). Only eq/ap below use the half-precision
    // `p` of the search value (e.g. "100" → 0.5).
    let p = super::number::implicit_range(num_str);
    let range = match prefix {
        SearchPrefix::Gt | SearchPrefix::Sa => json!({ "gt": map(num)? }),
        SearchPrefix::Lt | SearchPrefix::Eb => json!({ "lt": map(num)? }),
        SearchPrefix::Ge => json!({ "gte": map(num)? }),
        SearchPrefix::Le => json!({ "lte": map(num)? }),
        SearchPrefix::Ap => {
            let margin = (num * 0.1).abs().max(0.5);
            let (lo, hi) = ordered(map(num - margin)?, map(num + margin)?);
            json!({ "gte": lo, "lte": hi })
        }
        // Eq, Ne (treated as Eq range here), and any default
        _ => {
            let (lo, hi) = ordered(map(num - p)?, map(num + p)?);
            json!({ "gte": lo, "lt": hi })
        }
    };
    Some(json!({ "range": { field: range } }))
}

/// Returns the two values in ascending order (canonicalization factor is
/// positive, but guard against any inversion).
fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Parses a quantity value string into (number, system, code).
///
/// Formats:
/// - `5.4` -> ("5.4", None, None)
/// - `5.4|mg` -> ("5.4", None, Some("mg"))
/// - `5.4|http://unitsofmeasure.org|mg` -> ("5.4", Some("http://..."), Some("mg"))
fn parse_quantity_value(value: &str) -> (&str, Option<&str>, Option<&str>) {
    let parts: Vec<&str> = value.splitn(3, '|').collect();
    match parts.len() {
        1 => (parts[0], None, None),
        2 => (parts[0], None, Some(parts[1])),
        3 => {
            let system = if parts[1].is_empty() {
                None
            } else {
                Some(parts[1])
            };
            let code = if parts[2].is_empty() {
                None
            } else {
                Some(parts[2])
            };
            (parts[0], system, code)
        }
        _ => (value, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_quantity_value() {
        let (n, s, c) = parse_quantity_value("5.4");
        assert_eq!(n, "5.4");
        assert!(s.is_none());
        assert!(c.is_none());

        let (n, s, c) = parse_quantity_value("5.4|http://unitsofmeasure.org|mg");
        assert_eq!(n, "5.4");
        assert_eq!(s, Some("http://unitsofmeasure.org"));
        assert_eq!(c, Some("mg"));
    }

    #[test]
    fn test_quantity_clause() {
        let clause = build_clause(
            "value-quantity",
            "120|http://unitsofmeasure.org|mm[Hg]",
            SearchPrefix::Eq,
        )
        .unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("search_params.quantity"));
        assert!(s.contains("mm[Hg]"));
    }

    /// Recursively finds the `range` clause for `field` anywhere in `value`,
    /// regardless of how deeply it is nested inside the raw/canonical
    /// `bool.should` wrapping.
    fn find_range<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
        if let Some(range) = value.get("range").and_then(|r| r.get(field)) {
            return Some(range);
        }
        match value {
            Value::Object(map) => map.values().find_map(|v| find_range(v, field)),
            Value::Array(arr) => arr.iter().find_map(|v| find_range(v, field)),
            _ => None,
        }
    }

    #[test]
    fn raw_comparators_use_exact_value() {
        // gt/lt/ge/le/sa/eb ignore implicit precision and compare against the
        // exact search value on the raw (stored-unit) field.
        let clause = build_clause("value-quantity", "60|kg", SearchPrefix::Gt).unwrap();
        let range = find_range(&clause, "search_params.quantity.value")
            .expect("raw range clause must be present");
        assert_eq!(range, &json!({ "gt": 60.0 }));
    }

    #[test]
    fn canonical_comparator_uses_exact_canonical_value() {
        // Same rule on the canonical (UCUM-converted) field.
        let clause = build_clause(
            "value-quantity",
            "60|http://unitsofmeasure.org|kg",
            SearchPrefix::Gt,
        )
        .unwrap();
        let expected = helios_fhirpath::ucum::canonicalize_quantity(60.0, "kg")
            .expect("kg must canonicalize")
            .0;
        let range = find_range(&clause, "search_params.quantity.canonical_value")
            .expect("canonical range clause must be present");
        assert_eq!(range, &json!({ "gt": expected }));
    }

    #[test]
    fn eq_keeps_text_precision_range() {
        // eq is unaffected by this change: it still ranges over the
        // implicit-precision window derived from the value as written.
        let clause = build_clause("value-quantity", "60.0", SearchPrefix::Eq).unwrap();
        let range = find_range(&clause, "search_params.quantity.value")
            .expect("raw range clause must be present");
        let gte = range["gte"].as_f64().expect("gte must be a number");
        let lt = range["lt"].as_f64().expect("lt must be a number");
        assert!((gte - 59.95).abs() < 1e-9);
        assert!((lt - 60.05).abs() < 1e-9);
    }
}
