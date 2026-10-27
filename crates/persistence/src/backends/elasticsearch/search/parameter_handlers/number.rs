//! Number parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::search::FhirNumberValue;
use crate::types::SearchPrefix;

/// Builds an ES query clause for a number search parameter.
///
/// Never returns `None` for a value it cannot read. The query builder collects
/// these clauses with `filter_map`, so `None` does not mean "matches nothing",
/// it means "no constraint": `probability=abc` used to return every
/// RiskAssessment (#1319). The search gate (`validate_numeric_values`) rejects
/// such a value before a query is built; as defence in depth it is
/// [`match_none`](super::date::match_none) here, under every prefix.
/// `f64::from_str` used to stand here too, and took `inf` and `nan`, so
/// `ltinf` matched every indexed row (#1340).
pub fn build_clause(name: &str, value: &str, prefix: SearchPrefix) -> Option<Value> {
    let number = match FhirNumberValue::parse(value) {
        Ok(number) => number,
        Err(error) => {
            tracing::warn!(
                "unvalidated number search value reached the Elasticsearch handler: {error}"
            );
            return Some(super::date::match_none());
        }
    };
    let num = number.value;
    // The implicit-precision range eq/ne match: 100 is [99.5, 100.5), 100.0
    // is [99.95, 100.05).
    let (lo, hi) = number.implicit_range();

    let range_condition = match prefix {
        SearchPrefix::Eq => {
            json!({
                "range": { "search_params.number.value": { "gte": lo, "lt": hi } }
            })
        }
        SearchPrefix::Ne => {
            return Some(json!({
                "nested": {
                    "path": "search_params.number",
                    "query": {
                        "bool": {
                            "must": [
                                { "term": { "search_params.number.name": name } }
                            ],
                            "must_not": [
                                {
                                    "range": {
                                        "search_params.number.value": { "gte": lo, "lt": hi }
                                    }
                                }
                            ]
                        }
                    }
                }
            }));
        }
        // gt/lt/ge/le/sa/eb ignore the implicit precision and compare against
        // the exact search value (FHIR spec): "the implicit precision of the
        // number is ignored, and they are treated as if they have arbitrarily
        // high precision."
        SearchPrefix::Gt | SearchPrefix::Sa => {
            json!({
                "range": { "search_params.number.value": { "gt": num } }
            })
        }
        SearchPrefix::Lt | SearchPrefix::Eb => {
            json!({
                "range": { "search_params.number.value": { "lt": num } }
            })
        }
        SearchPrefix::Ge => {
            json!({
                "range": { "search_params.number.value": { "gte": num } }
            })
        }
        SearchPrefix::Le => {
            json!({
                "range": { "search_params.number.value": { "lte": num } }
            })
        }
        SearchPrefix::Ap => {
            // The shared `ap` window: ±10% of the value, never narrower than
            // the implicit-precision range (#1390).
            let (lo, hi) = number.approx_range();
            json!({
                "range": { "search_params.number.value": { "gte": lo, "lte": hi } }
            })
        }
    };

    Some(json!({
        "nested": {
            "path": "search_params.number",
            "query": {
                "bool": {
                    "must": [
                        { "term": { "search_params.number.name": name } },
                        range_condition
                    ]
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_range(clause: &Value) -> &Value {
        &clause["nested"]["query"]["bool"]["must"][1]["range"]["search_params.number.value"]
    }

    #[test]
    fn test_eq_range() {
        let clause = build_clause("length", "100", SearchPrefix::Eq).unwrap();
        assert_eq!(value_range(&clause), &json!({ "gte": 99.5, "lt": 100.5 }));
        let clause = build_clause("length", "100.0", SearchPrefix::Eq).unwrap();
        let range = value_range(&clause);
        assert!((range["gte"].as_f64().unwrap() - 99.95).abs() < 1e-9);
        assert!((range["lt"].as_f64().unwrap() - 100.05).abs() < 1e-9);
    }

    /// `ap` is the shared window (#1390): ±10% of the value, floored at the
    /// implicit-precision range, closed at both ends.
    #[test]
    fn ap_uses_the_shared_approximate_window() {
        for (raw, lo, hi) in [
            ("100", 90.0, 110.0),
            ("-100", -110.0, -90.0),
            ("0", -0.5, 0.5),
            ("1e2", 50.0, 150.0),
            ("-5.4", -5.94, -4.86),
        ] {
            let clause = build_clause("length", raw, SearchPrefix::Ap).unwrap();
            let range = value_range(&clause);
            let gte = range["gte"].as_f64().expect("gte must be a number");
            let lte = range["lte"].as_f64().expect("lte must be a number");
            assert!((gte - lo).abs() < 1e-9, "ap{raw}: gte {gte}");
            assert!((lte - hi).abs() < 1e-9, "ap{raw}: lte {lte}");
        }
    }

    #[test]
    fn comparators_use_exact_value() {
        // gt/lt/ge/le/sa/eb ignore implicit precision and compare against the
        // exact search value, per the FHIR spec.
        let cases = [
            (SearchPrefix::Gt, "gt"),
            (SearchPrefix::Sa, "gt"),
            (SearchPrefix::Lt, "lt"),
            (SearchPrefix::Eb, "lt"),
            (SearchPrefix::Ge, "gte"),
            (SearchPrefix::Le, "lte"),
        ];

        for (prefix, expected_key) in cases {
            let clause = build_clause("length", "100", prefix).unwrap();
            assert_eq!(
                value_range(&clause),
                &json!({ expected_key: 100.0 }),
                "{prefix:?} must emit {{\"{expected_key}\": 100.0}}"
            );
        }
    }

    #[test]
    fn comparator_ignores_trailing_zero_precision() {
        // "60" and "60.0" have different implicit precision, but gt ignores
        // it entirely: both must produce identical JSON.
        let plain = build_clause("length", "60", SearchPrefix::Gt).unwrap();
        let trailing_zero = build_clause("length", "60.0", SearchPrefix::Gt).unwrap();
        assert_eq!(plain, trailing_zero);
    }

    /// The `filter_map` trap (#1319): `None` from here does not mean "matches
    /// nothing", it means "no constraint". A value that is not a number — by
    /// the shared grammar, so `inf` and `nan` included (#1340) — must be a
    /// clause, and one that matches nothing, under every prefix.
    #[test]
    fn a_value_that_is_not_a_number_is_match_none_never_none() {
        for prefix in [
            SearchPrefix::Eq,
            SearchPrefix::Ne,
            SearchPrefix::Gt,
            SearchPrefix::Lt,
            SearchPrefix::Ge,
            SearchPrefix::Le,
            SearchPrefix::Ap,
        ] {
            for raw in [
                "abc", "", "1e", "inf", "-inf", "Infinity", "nan", "NaN", "1e999", "0x10",
            ] {
                assert_eq!(
                    build_clause("probability", raw, prefix),
                    Some(json!({ "match_none": {} })),
                    "{prefix:?} {raw:?}"
                );
            }
        }
    }
}
