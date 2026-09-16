//! Number parameter SQL handler.

use crate::types::{SearchPrefix, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles number parameter SQL generation.
pub struct NumberHandler;

impl NumberHandler {
    /// Builds SQL for a number parameter value.
    ///
    /// Supports all comparison prefixes: eq, ne, gt, lt, ge, le, sa, eb, ap.
    pub fn build_sql(value: &SearchValue, param_offset: usize) -> SqlFragment {
        let param_num = param_offset + 1;

        // Parse the number value
        let num_value: f64 = match value.value.parse() {
            Ok(v) => v,
            Err(_) => {
                // Invalid number - return impossible condition
                return SqlFragment::new("1 = 0");
            }
        };

        match value.prefix {
            SearchPrefix::Eq | SearchPrefix::Ne => {
                // eq/ne match the implicit-precision range [lo, hi) derived
                // from the search text as written (FHIR spec).
                let (lo, hi) = crate::search::implicit_range(num_value, &value.value);
                if matches!(value.prefix, SearchPrefix::Eq) {
                    Self::build_equals(lo, hi, param_num)
                } else {
                    Self::build_not_equals(lo, hi, param_num)
                }
            }
            // gt/lt/ge/le/sa/eb compare against the exact search value: per
            // the FHIR spec, the implicit precision is ignored for these
            // prefixes.
            SearchPrefix::Gt | SearchPrefix::Sa => Self::cmp(">", num_value, param_num),
            SearchPrefix::Lt | SearchPrefix::Eb => Self::cmp("<", num_value, param_num),
            SearchPrefix::Ge => Self::cmp(">=", num_value, param_num),
            SearchPrefix::Le => Self::cmp("<=", num_value, param_num),
            SearchPrefix::Ap => Self::build_approximately(num_value, param_num),
        }
    }

    /// Builds a single-boundary numeric comparison `value_number {op} ?`.
    fn cmp(op: &str, bound: f64, param_num: usize) -> SqlFragment {
        SqlFragment::with_params(
            format!("value_number {} ?{}", op, param_num),
            vec![SqlParam::float(bound)],
        )
    }

    /// Equality - matches the implicit-precision range `[lo, hi)` derived
    /// from the search text as written (e.g. "100" → [99.5, 100.5), "100.0"
    /// → [99.95, 100.05)).
    fn build_equals(lo: f64, hi: f64, param_num: usize) -> SqlFragment {
        SqlFragment::with_params(
            format!(
                "value_number >= ?{} AND value_number < ?{}",
                param_num,
                param_num + 1
            ),
            vec![SqlParam::float(lo), SqlParam::float(hi)],
        )
    }

    /// Not equals - outside the implicit-precision range `[lo, hi)` derived
    /// from the search text as written.
    fn build_not_equals(lo: f64, hi: f64, param_num: usize) -> SqlFragment {
        SqlFragment::with_params(
            format!(
                "(value_number < ?{} OR value_number >= ?{})",
                param_num,
                param_num + 1
            ),
            vec![SqlParam::float(lo), SqlParam::float(hi)],
        )
    }

    /// Approximately equals - +/- 10%.
    fn build_approximately(value: f64, param_num: usize) -> SqlFragment {
        let margin = (value.abs() * 0.1).max(0.0001); // At least 0.0001 for very small numbers

        SqlFragment::with_params(
            format!("value_number BETWEEN ?{} AND ?{}", param_num, param_num + 1),
            vec![
                SqlParam::float(value - margin),
                SqlParam::float(value + margin),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_eq() {
        let value = SearchValue::new(SearchPrefix::Eq, "100");
        let frag = NumberHandler::build_sql(&value, 0);

        assert!(frag.sql.contains(">="));
        assert!(frag.sql.contains("<"));
        assert_eq!(frag.params.len(), 2);
    }

    /// Asserts that `frag` carries exactly the two float params `[lo, hi]`
    /// (within a small tolerance for floating-point rounding).
    fn assert_range_params(frag: &SqlFragment, lo: f64, hi: f64) {
        assert_eq!(frag.params.len(), 2);
        match (&frag.params[0], &frag.params[1]) {
            (SqlParam::Float(a), SqlParam::Float(b)) => {
                assert!((*a - lo).abs() < 1e-9);
                assert!((*b - hi).abs() < 1e-9);
            }
            _ => panic!("expected float params"),
        }
    }

    #[test]
    fn comparators_use_exact_value() {
        // gt/lt/ge/le/sa/eb ignore implicit precision and compare against the
        // exact search value, per the FHIR spec.
        let cases = [
            (SearchPrefix::Gt, "value_number > ?1"),
            (SearchPrefix::Lt, "value_number < ?1"),
            (SearchPrefix::Ge, "value_number >= ?1"),
            (SearchPrefix::Le, "value_number <= ?1"),
            (SearchPrefix::Sa, "value_number > ?1"),
            (SearchPrefix::Eb, "value_number < ?1"),
        ];

        for (prefix, expected_sql) in cases {
            let value = SearchValue::new(prefix, "100");
            let frag = NumberHandler::build_sql(&value, 0);

            assert_eq!(frag.sql, expected_sql);
            assert_eq!(frag.params.len(), 1);
            match &frag.params[0] {
                SqlParam::Float(f) => assert!((*f - 100.0).abs() < 1e-9),
                _ => panic!("expected float param"),
            }
        }
    }

    #[test]
    fn comparator_ignores_trailing_zero_precision() {
        // "60" and "60.0" have different implicit precision, but gt ignores
        // it entirely: both must produce identical SQL and parameter.
        let plain = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Gt, "60"), 0);
        let trailing_zero =
            NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Gt, "60.0"), 0);

        assert_eq!(plain.sql, trailing_zero.sql);
        for frag in [&plain, &trailing_zero] {
            assert_eq!(frag.params.len(), 1);
            match &frag.params[0] {
                SqlParam::Float(f) => assert!((*f - 60.0).abs() < 1e-9),
                _ => panic!("expected float param"),
            }
        }
    }

    #[test]
    fn eq_range_follows_search_text_precision() {
        let eq_60 = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Eq, "60"), 0);
        assert_range_params(&eq_60, 59.5, 60.5);

        let eq_60_0 = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Eq, "60.0"), 0);
        assert_range_params(&eq_60_0, 59.95, 60.05);

        let ne_60_0 = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Ne, "60.0"), 0);
        assert_range_params(&ne_60_0, 59.95, 60.05);
    }

    #[test]
    fn test_number_ap() {
        let value = SearchValue::new(SearchPrefix::Ap, "100");
        let frag = NumberHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("BETWEEN"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_invalid_number() {
        let value = SearchValue::new(SearchPrefix::Eq, "not-a-number");
        let frag = NumberHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("1 = 0"));
    }
}
