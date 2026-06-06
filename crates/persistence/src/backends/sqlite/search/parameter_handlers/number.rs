//! Number parameter SQL handler.

use crate::types::{SearchPrefix, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles number parameter SQL generation.
pub struct NumberHandler;

impl NumberHandler {
    /// Builds SQL for a number parameter value.
    ///
    /// Supports all comparison prefixes: eq, ne, gt, lt, ge, le, ap.
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

        // Implicit-precision range [lo, hi) of the search value; comparators
        // match against its boundaries per the FHIR spec.
        let (lo, hi) = crate::search::implicit_range(num_value, &value.value);

        match value.prefix {
            SearchPrefix::Eq => Self::build_equals(num_value, param_num),
            SearchPrefix::Ne => Self::build_not_equals(num_value, param_num),
            // gt / sa: strictly above the whole search range.
            SearchPrefix::Gt | SearchPrefix::Sa => Self::cmp(">=", hi, param_num),
            // lt / eb: strictly below the whole search range.
            SearchPrefix::Lt | SearchPrefix::Eb => Self::cmp("<", lo, param_num),
            SearchPrefix::Ge => Self::cmp(">=", lo, param_num),
            SearchPrefix::Le => Self::cmp("<", hi, param_num),
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

    /// Equality - exact match with implicit precision.
    ///
    /// For numbers like "100", this matches values in range [99.5, 100.5).
    fn build_equals(value: f64, param_num: usize) -> SqlFragment {
        // Determine precision from the value
        let precision = Self::get_implicit_precision(value);
        let half_precision = precision / 2.0;

        SqlFragment::with_params(
            format!(
                "value_number >= ?{} AND value_number < ?{}",
                param_num,
                param_num + 1
            ),
            vec![
                SqlParam::float(value - half_precision),
                SqlParam::float(value + half_precision),
            ],
        )
    }

    /// Not equals.
    fn build_not_equals(value: f64, param_num: usize) -> SqlFragment {
        let precision = Self::get_implicit_precision(value);
        let half_precision = precision / 2.0;

        SqlFragment::with_params(
            format!(
                "(value_number < ?{} OR value_number >= ?{})",
                param_num,
                param_num + 1
            ),
            vec![
                SqlParam::float(value - half_precision),
                SqlParam::float(value + half_precision),
            ],
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

    /// Gets the implicit precision of a number.
    ///
    /// For "100", precision is 1. For "100.0", precision is 0.1. For "100.00", precision is 0.01.
    fn get_implicit_precision(value: f64) -> f64 {
        // Determine precision from string representation
        let s = value.to_string();
        if let Some(dot_pos) = s.find('.') {
            let decimal_places = s.len() - dot_pos - 1;
            10_f64.powi(-(decimal_places as i32))
        } else {
            1.0
        }
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

    #[test]
    fn test_number_gt() {
        // gt matches strictly above the search range; for "100" that is >= 100.5.
        let value = SearchValue::new(SearchPrefix::Gt, "100");
        let frag = NumberHandler::build_sql(&value, 0);

        assert!(frag.sql.contains(">= ?1"));
        assert_eq!(frag.params.len(), 1);
        match &frag.params[0] {
            SqlParam::Float(f) => assert!((*f - 100.5).abs() < 1e-9),
            _ => panic!("expected float bound"),
        }
    }

    #[test]
    fn test_number_le() {
        // le matches up to the top of the search range; for "100" that is < 100.5.
        let value = SearchValue::new(SearchPrefix::Le, "100");
        let frag = NumberHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("< ?1"));
        match &frag.params[0] {
            SqlParam::Float(f) => assert!((*f - 100.5).abs() < 1e-9),
            _ => panic!("expected float bound"),
        }
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
