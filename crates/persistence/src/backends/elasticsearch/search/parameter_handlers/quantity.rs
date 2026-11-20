//! Quantity parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::search::{Cmp, FhirNumberValue, FhirQuantityValue, QuantityBound};
use crate::types::SearchPrefix;

/// Builds an ES query clause for a quantity search parameter.
///
/// Format: `[prefix]number|system|code` or `[prefix]number|code` or
/// `[prefix]number`, read by the grammar every backend shares
/// ([`FhirQuantityValue`]): only an unescaped `|` separates.
///
/// Never returns `None` for a value it cannot read — see
/// [`number::build_clause`](super::number::build_clause): the query builder
/// refuses a parameter over a `None` (#2510), and used to drop the constraint
/// so that the search returned everything (#1319). It is
/// [`match_none`](super::date::match_none) instead.
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
    let number = &quantity.number;
    let num = number.value;
    let (system, code) = (quantity.system.as_deref(), quantity.code.as_deref());

    // Raw match against the stored value/unit/system.
    let mut raw_must = vec![
        json!({ "term": { "search_params.quantity.name": name } }),
        interval_condition(
            "search_params.quantity.value",
            "search_params.quantity.high",
            prefix,
            number,
            Some,
        )?,
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
        let range = interval_condition(
            "search_params.quantity.canonical_value",
            "search_params.quantity.canonical_high",
            prefix,
            number,
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

/// Builds the ES condition for the quantity interval predicate of `number`
/// and `prefix` (#2602), against the stored closed interval
/// `[low_field, high_field]`. The search-side bounds are transformed through
/// `map` (identity for the raw value, UCUM-canonicalization for the canonical
/// columns). Returns `None` if a transformed bound is unavailable.
///
/// Every bound on the upper end reads `high_field`, with a fallback to
/// `low_field` for a document that has no `high_field`: one indexed before
/// schema version 4 holds only points, so its `Quantity` and `Age` rows keep
/// their results until `$reindex` (a point's high end is its value).
fn interval_condition(
    low_field: &str,
    high_field: &str,
    prefix: SearchPrefix,
    number: &FhirNumberValue,
    map: impl Fn(f64) -> Option<f64>,
) -> Option<Value> {
    let predicate = number.interval_predicate(prefix);
    let mut groups = Vec::with_capacity(predicate.any_of.len());
    for group in &predicate.any_of {
        let mut clauses = Vec::with_capacity(group.len());
        for bound in group {
            clauses.push(match *bound {
                QuantityBound::Low(cmp, v) => range_on(low_field, cmp, map(v)?),
                QuantityBound::High(cmp, v) => {
                    let bound = map(v)?;
                    json!({
                        "bool": {
                            "should": [
                                range_on(high_field, cmp, bound),
                                { "bool": {
                                    "must": [ range_on(low_field, cmp, bound) ],
                                    "must_not": [ { "exists": { "field": high_field } } ]
                                } }
                            ],
                            "minimum_should_match": 1
                        }
                    })
                }
            });
        }
        groups.push(if clauses.len() == 1 {
            clauses.remove(0)
        } else {
            json!({ "bool": { "must": clauses } })
        });
    }
    Some(if groups.len() == 1 {
        groups.remove(0)
    } else {
        json!({ "bool": { "should": groups, "minimum_should_match": 1 } })
    })
}

/// `{"range": {field: {op: bound}}}` for a comparison operator.
fn range_on(field: &str, cmp: Cmp, bound: f64) -> Value {
    let op = match cmp {
        Cmp::Lt => "lt",
        Cmp::Le => "lte",
        Cmp::Gt => "gt",
        Cmp::Ge => "gte",
    };
    json!({ "range": { field: { op: bound } } })
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

    /// The range on the *high* field proper (not the fallback to `value` for
    /// documents that have no `high`), i.e. the first of the two `should`
    /// arms of a high-end bound.
    fn find_high_range<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
        if let Some(should) = value.get("should").and_then(Value::as_array) {
            if let Some(range) = should
                .first()
                .and_then(|c| c.get("range"))
                .and_then(|r| r.get(field))
            {
                return Some(range);
            }
        }
        match value {
            Value::Object(map) => map.values().find_map(|v| find_high_range(v, field)),
            Value::Array(arr) => arr.iter().find_map(|v| find_high_range(v, field)),
            _ => None,
        }
    }

    /// `eq` is the implicit-precision window `[s, e)` of the value as written,
    /// now against the interval: `s ≤ L ∧ H < e`.
    #[test]
    fn eq_keeps_text_precision_range() {
        let clause = build_clause("value-quantity", "60.0", SearchPrefix::Eq).unwrap();
        let low = find_range(&clause, "search_params.quantity.value")
            .expect("low-end range clause must be present");
        assert!((low["gte"].as_f64().unwrap() - 59.95).abs() < 1e-9);
        let high = find_high_range(&clause, "search_params.quantity.high")
            .expect("high-end range clause must be present");
        assert!((high["lt"].as_f64().unwrap() - 60.05).abs() < 1e-9);
    }

    /// `ne` is the complement: `L < s ∨ H ≥ e`, two alternatives.
    #[test]
    fn ne_is_below_or_above_the_precision_range() {
        let clause = build_clause("value-quantity", "60", SearchPrefix::Ne).unwrap();
        let raw = &clause["nested"]["query"]["bool"]["must"][1]["bool"]["should"];
        assert_eq!(
            raw[0],
            json!({ "range": { "search_params.quantity.value": { "lt": 59.5 } } })
        );
        let high = find_high_range(&raw[1], "search_params.quantity.high")
            .expect("high-end range clause must be present");
        assert_eq!(high, &json!({ "gte": 60.5 }));

        // With a UCUM unit the canonical branch negates the same way.
        let clause = build_clause(
            "value-quantity",
            "60|http://unitsofmeasure.org|kg",
            SearchPrefix::Ne,
        )
        .unwrap();
        let canon = |x: f64| {
            helios_fhirpath::ucum::canonicalize_quantity(x, "kg")
                .expect("kg must canonicalize")
                .0
        };
        let low = find_range(&clause, "search_params.quantity.canonical_value")
            .expect("canonical low-end clause must be present");
        assert!((low["lt"].as_f64().unwrap() - canon(59.5)).abs() < 1e-9);
        let high = find_high_range(&clause, "search_params.quantity.canonical_high")
            .expect("canonical high-end clause must be present");
        assert!((high["gte"].as_f64().unwrap() - canon(60.5)).abs() < 1e-9);
    }

    /// `ap` is the shared window (#1390) against the interval: `L ≤ v+m ∧
    /// H ≥ v−m`, on the raw and the canonical columns.
    #[test]
    fn ap_uses_the_shared_approximate_window() {
        let clause = build_clause("value-quantity", "5.4", SearchPrefix::Ap).unwrap();
        let low = find_range(&clause, "search_params.quantity.value")
            .expect("low-end range clause must be present");
        assert!((low["lte"].as_f64().unwrap() - 5.94).abs() < 1e-9);
        let high = find_high_range(&clause, "search_params.quantity.high")
            .expect("high-end range clause must be present");
        assert!((high["gte"].as_f64().unwrap() - 4.86).abs() < 1e-9);

        let clause = build_clause(
            "value-quantity",
            "5.4|http://unitsofmeasure.org|mg",
            SearchPrefix::Ap,
        )
        .unwrap();
        let canon = |x: f64| {
            helios_fhirpath::ucum::canonicalize_quantity(x, "mg")
                .expect("mg must canonicalize")
                .0
        };
        let low = find_range(&clause, "search_params.quantity.canonical_value")
            .expect("canonical low-end clause must be present");
        assert!((low["lte"].as_f64().unwrap() - canon(5.94)).abs() < 1e-12);
        let high = find_high_range(&clause, "search_params.quantity.canonical_high")
            .expect("canonical high-end clause must be present");
        assert!((high["gte"].as_f64().unwrap() - canon(4.86)).abs() < 1e-12);
    }

    /// `gt`/`ge`/`eb` read the upper end and `lt`/`le`/`sa` the lower one
    /// (#2602); a high-end bound falls back to `value` for a document with no
    /// `high` (indexed before schema version 4).
    #[test]
    fn comparators_read_the_matching_end_with_a_fallback() {
        let high_field = "search_params.quantity.high";
        let low_field = "search_params.quantity.value";
        for (prefix, op) in [
            (SearchPrefix::Gt, "gt"),
            (SearchPrefix::Ge, "gte"),
            (SearchPrefix::Eb, "lt"),
        ] {
            let clause = build_clause("onset-age", "45", prefix).unwrap();
            let should = &clause["nested"]["query"]["bool"]["must"][1]["bool"];
            assert_eq!(should["should"][0]["range"][high_field][op], json!(45.0));
            let fallback = &should["should"][1]["bool"];
            assert_eq!(fallback["must"][0]["range"][low_field][op], json!(45.0));
            assert_eq!(
                fallback["must_not"][0],
                json!({ "exists": { "field": high_field } })
            );
        }
        for (prefix, op) in [
            (SearchPrefix::Lt, "lt"),
            (SearchPrefix::Le, "lte"),
            (SearchPrefix::Sa, "gt"),
        ] {
            let clause = build_clause("onset-age", "45", prefix).unwrap();
            assert_eq!(
                clause["nested"]["query"]["bool"]["must"][1],
                json!({ "range": { low_field: { op: 45.0 } } }),
                "{prefix:?}"
            );
        }
    }

    /// The silent-drop trap (#1319), as for numbers: never `None`.
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
