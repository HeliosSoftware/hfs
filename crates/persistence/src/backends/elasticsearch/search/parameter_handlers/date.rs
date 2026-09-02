//! Date parameter handler for Elasticsearch.

use serde_json::{Value, json};

use crate::types::SearchPrefix;

/// A precision-aware comparison on one ES date field, ready to be wrapped in
/// whatever query shape the caller needs (nested for indexed parameters, a
/// bare top-level clause for `_lastUpdated`).
#[derive(Debug)]
pub(crate) enum DateRange {
    /// `{ "range": { field: bounds } }` — the value must fall inside.
    Within(Value),
    /// `{ "range": { field: bounds } }` — the value must fall *outside*
    /// (`ne`). The caller negates it with `must_not` so the negation is
    /// applied at the right level of the enclosing query.
    Outside(Value),
}

/// Builds the `range` comparison for `field` at the value's inherent
/// precision: `eq` at day precision means `[day, day+1)`, `ne` its
/// complement, `gt`/`sa` start strictly after the whole period, `lt`/`eb`
/// end strictly before it, and `le` reaches the end of the period.
///
/// A full-precision instant (`2024-01-15T10:00:00Z`) has no period, so it
/// falls back to scalar comparison rather than an empty half-open range.
pub(crate) fn field_range(field: &str, value: &str, prefix: SearchPrefix) -> DateRange {
    let (lower, upper) = date_precision_range(value);
    let degenerate = lower == upper;
    let range = |bounds: Value| json!({ "range": { field: bounds } });

    match prefix {
        // `ap` on a date is the precision range itself: ES has no fuzzy
        // date matching, and the implied period is the natural tolerance.
        SearchPrefix::Eq | SearchPrefix::Ap if degenerate => {
            DateRange::Within(range(json!({ "gte": lower, "lte": lower })))
        }
        SearchPrefix::Eq | SearchPrefix::Ap => {
            DateRange::Within(range(json!({ "gte": lower, "lt": upper })))
        }
        SearchPrefix::Ne if degenerate => {
            DateRange::Outside(range(json!({ "gte": lower, "lte": lower })))
        }
        SearchPrefix::Ne => DateRange::Outside(range(json!({ "gte": lower, "lt": upper }))),
        SearchPrefix::Gt | SearchPrefix::Sa if degenerate => {
            DateRange::Within(range(json!({ "gt": lower })))
        }
        SearchPrefix::Gt | SearchPrefix::Sa => DateRange::Within(range(json!({ "gte": upper }))),
        SearchPrefix::Lt | SearchPrefix::Eb => DateRange::Within(range(json!({ "lt": lower }))),
        SearchPrefix::Ge => DateRange::Within(range(json!({ "gte": lower }))),
        SearchPrefix::Le if degenerate => DateRange::Within(range(json!({ "lte": lower }))),
        SearchPrefix::Le => DateRange::Within(range(json!({ "lt": upper }))),
    }
}

/// Builds an ES query clause for an indexed date search parameter.
pub fn build_clause(name: &str, value: &str, prefix: SearchPrefix) -> Option<Value> {
    let name_term = json!({ "term": { "search_params.date.name": name } });
    let bool_body = match field_range("search_params.date.value", value, prefix) {
        DateRange::Within(range) => json!({ "must": [name_term, range] }),
        DateRange::Outside(range) => json!({ "must": [name_term], "must_not": [range] }),
    };

    Some(json!({
        "nested": {
            "path": "search_params.date",
            "query": { "bool": bool_body }
        }
    }))
}

/// Computes the precision-based range for a date value.
///
/// Returns (lower_bound_inclusive, upper_bound_exclusive).
fn date_precision_range(value: &str) -> (String, String) {
    // Count characters to determine precision
    let clean = value.trim();

    if clean.len() == 4 {
        // Year precision: "2024" -> ["2024-01-01", "2025-01-01")
        let year: i32 = clean.parse().unwrap_or(2000);
        (
            format!("{:04}-01-01", year),
            format!("{:04}-01-01", year + 1),
        )
    } else if clean.len() == 7 {
        // Month precision: "2024-01" -> ["2024-01-01", "2024-02-01")
        let parts: Vec<&str> = clean.split('-').collect();
        let year: i32 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(2000);
        let month: u32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let (next_year, next_month) = if month >= 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        (
            format!("{:04}-{:02}-01", year, month),
            format!("{:04}-{:02}-01", next_year, next_month),
        )
    } else if clean.len() == 10 {
        // Day precision: "2024-01-15" -> ["2024-01-15", "2024-01-16")
        // Simple: parse and add one day
        let lower = clean.to_string();
        let parts: Vec<&str> = clean.split('-').collect();
        let year: i32 = parts.first().and_then(|p| p.parse().ok()).unwrap_or(2000);
        let month: u32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let day: u32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);

        // Use chrono for correct date arithmetic
        if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
            let next = date + chrono::Duration::days(1);
            (lower, next.format("%Y-%m-%d").to_string())
        } else {
            (lower.clone(), lower)
        }
    } else {
        // Full date-time precision: use the value directly
        (clean.to_string(), clean.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_year_precision() {
        let (lower, upper) = date_precision_range("2024");
        assert_eq!(lower, "2024-01-01");
        assert_eq!(upper, "2025-01-01");
    }

    #[test]
    fn test_month_precision() {
        let (lower, upper) = date_precision_range("2024-01");
        assert_eq!(lower, "2024-01-01");
        assert_eq!(upper, "2024-02-01");
    }

    #[test]
    fn test_day_precision() {
        let (lower, upper) = date_precision_range("2024-01-15");
        assert_eq!(lower, "2024-01-15");
        assert_eq!(upper, "2024-01-16");
    }

    #[test]
    fn test_eq_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Eq).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("gte"));
        assert!(s.contains("2024-01-15"));
        assert!(s.contains("2024-01-16"));
    }

    #[test]
    fn test_gt_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Gt).unwrap();
        let s = serde_json::to_string(&clause).unwrap();
        assert!(s.contains("gte"));
        assert!(s.contains("2024-01-16")); // starts after precision range
    }

    #[test]
    fn ne_is_the_negated_precision_range() {
        let clause = build_clause("birthdate", "2024-01-15", SearchPrefix::Ne).unwrap();
        let bool_body = &clause["nested"]["query"]["bool"];
        assert_eq!(
            bool_body["must"][0]["term"]["search_params.date.name"],
            "birthdate"
        );
        let range = &bool_body["must_not"][0]["range"]["search_params.date.value"];
        assert_eq!(range["gte"], "2024-01-15");
        assert_eq!(range["lt"], "2024-01-16");
    }

    #[test]
    fn sa_and_eb_mirror_gt_and_lt_on_the_whole_period() {
        let sa = field_range("f", "2024-01", SearchPrefix::Sa);
        let DateRange::Within(sa) = sa else {
            panic!("sa must be a plain range: {sa:?}")
        };
        assert_eq!(sa["range"]["f"], json!({ "gte": "2024-02-01" }));

        let eb = field_range("f", "2024-01", SearchPrefix::Eb);
        let DateRange::Within(eb) = eb else {
            panic!("eb must be a plain range: {eb:?}")
        };
        assert_eq!(eb["range"]["f"], json!({ "lt": "2024-01-01" }));
    }

    #[test]
    fn full_precision_instant_is_scalar_not_an_empty_range() {
        let instant = "2024-01-15T10:00:00Z";
        let DateRange::Within(eq) = field_range("f", instant, SearchPrefix::Eq) else {
            panic!("eq must be a plain range")
        };
        assert_eq!(eq["range"]["f"], json!({ "gte": instant, "lte": instant }));

        let DateRange::Within(gt) = field_range("f", instant, SearchPrefix::Gt) else {
            panic!("gt must be a plain range")
        };
        assert_eq!(gt["range"]["f"], json!({ "gt": instant }));

        let DateRange::Within(le) = field_range("f", instant, SearchPrefix::Le) else {
            panic!("le must be a plain range")
        };
        assert_eq!(le["range"]["f"], json!({ "lte": instant }));
    }
}
