//! Quantity parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::search::FhirQuantityValue;
use crate::types::SearchPrefix;

/// Builds an ES query clause for a quantity search parameter.
///
/// Format: `[prefix]number|system|code` or `[prefix]number|code` or
/// `[prefix]number`, read by the grammar every backend shares
/// ([`FhirQuantityValue`]): only an unescaped `|` separates.
///
/// Never returns `None` for a value it cannot read — see
/// [`number::build_clause`](super::number::build_clause): the query builder's
/// `filter_map` would drop the constraint and the search would return
/// everything (#1319). It is [`match_none`](super::date::match_none) instead.
pub fn build_clause(name: &str, value: &str, prefix: SearchPrefix) -> Option<Value> {
    let quantity = match FhirQuantityValue::parse(value) {
        Ok(quantity) => quantity,
        Err(error) => {
            tracing::warn!(
                "unvalidated quantity search value reached the Elasticsearch handler: {error}"
            );
            return Some(super::date::match_none());
        }
    };
    let (num_str, num) = (quantity.number.text(), quantity.number.value);
    let (system, code) = (quantity.system.as_deref(), quantity.code.as_deref());

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
    // exact search value (FHIR spec). Only eq/ne/ap below use the half-precision
    // `p` of the search value (e.g. "100" → 0.5).
    let p = super::number::implicit_range(num_str);
    if matches!(prefix, SearchPrefix::Ne) {
        // ne matches values outside the implicit-precision window: negate the
        // same [lo, hi) range eq builds for this field. Since this clause
        // stays inside the same raw/canonical `must` list eq uses (unit,
        // system, and code terms untouched), a resource only matches ne when
        // it has a quantity entry meeting eq's unit/system criteria whose
        // value falls outside the range; resources without a matching entry
        // never satisfy the surrounding `must`, so they are correctly
        // excluded.
        let (lo, hi) = ordered(map(num - p)?, map(num + p)?);
        return Some(json!({
            "bool": {
                "must_not": [
                    { "range": { field: { "gte": lo, "lt": hi } } }
                ]
            }
        }));
    }
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
        // Eq and any default.
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Recursively finds a `{"bool": {"must_not": [{"range": {field: ...}}]}}`
    /// clause for `field` anywhere in `value`, returning the negated range's
    /// inner bounds object (e.g. `{"gte": ..., "lt": ...}`).
    fn find_negated_range<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
        if let Some(must_not) = value.get("bool").and_then(|b| b.get("must_not")) {
            if let Some(range) = must_not
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|item| item.get("range").and_then(|r| r.get(field)))
            {
                return Some(range);
            }
        }
        match value {
            Value::Object(map) => map.values().find_map(|v| find_negated_range(v, field)),
            Value::Array(arr) => arr.iter().find_map(|v| find_negated_range(v, field)),
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

    #[test]
    fn ne_excludes_text_precision_range() {
        // ne "60" negates the same [59.5, 60.5) range eq would match, on the
        // raw field.
        let clause = build_clause("value-quantity", "60", SearchPrefix::Ne).unwrap();
        let raw_range = find_negated_range(&clause, "search_params.quantity.value")
            .expect("raw must_not range clause must be present");
        assert_eq!(raw_range, &json!({ "gte": 59.5, "lt": 60.5 }));

        // With a UCUM unit, the canonical branch's range is negated too.
        let clause = build_clause(
            "value-quantity",
            "60|http://unitsofmeasure.org|kg",
            SearchPrefix::Ne,
        )
        .unwrap();
        let expected_lo = helios_fhirpath::ucum::canonicalize_quantity(59.5, "kg")
            .expect("kg must canonicalize")
            .0;
        let expected_hi = helios_fhirpath::ucum::canonicalize_quantity(60.5, "kg")
            .expect("kg must canonicalize")
            .0;
        let raw_range = find_negated_range(&clause, "search_params.quantity.value")
            .expect("raw must_not range clause must be present");
        assert_eq!(raw_range, &json!({ "gte": 59.5, "lt": 60.5 }));

        let canonical_range = find_negated_range(&clause, "search_params.quantity.canonical_value")
            .expect("canonical must_not range clause must be present");
        let gte = canonical_range["gte"]
            .as_f64()
            .expect("gte must be a number");
        let lt = canonical_range["lt"].as_f64().expect("lt must be a number");
        assert!((gte - expected_lo).abs() < 1e-9);
        assert!((lt - expected_hi).abs() < 1e-9);
    }

    /// The `filter_map` trap (#1319), as for numbers: never `None`.
    #[test]
    fn an_invalid_number_part_is_match_none_never_none() {
        for prefix in [SearchPrefix::Eq, SearchPrefix::Ne, SearchPrefix::Lt] {
            for raw in [
                "abc",
                "",
                "||mg",
                "|http://unitsofmeasure.org|mg",
                "abc|http://unitsofmeasure.org|mg",
                "inf||mg",
                "nan",
                "1e999|mg",
            ] {
                assert_eq!(
                    build_clause("value-quantity", raw, prefix),
                    Some(json!({ "match_none": {} })),
                    "{prefix:?} {raw:?}"
                );
            }
        }
    }

    #[test]
    fn an_escaped_pipe_is_part_of_the_code() {
        let clause = build_clause(
            "value-quantity",
            "5.4|http://example.org|a\\|b",
            SearchPrefix::Eq,
        )
        .unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains(r#""search_params.quantity.code":"a|b""#), "{s}");
        assert!(
            s.contains(r#""search_params.quantity.system":"http://example.org""#),
            "{s}"
        );
    }
}
