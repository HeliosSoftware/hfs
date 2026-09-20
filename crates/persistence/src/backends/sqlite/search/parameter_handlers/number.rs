//! Number parameter SQL handler.

use crate::search::FhirNumberValue;
use crate::types::SearchValue;

use super::super::query_builder::SqlFragment;
use super::quantity::QuantityHandler;

/// Handles number parameter SQL generation.
pub struct NumberHandler;

impl NumberHandler {
    /// Builds SQL for a number parameter value.
    ///
    /// Supports all comparison prefixes: eq, ne, gt, lt, ge, le, sa, eb, ap.
    pub fn build_sql(value: &SearchValue, param_offset: usize) -> SqlFragment {
        Self::build_sql_for("value_number", value, param_offset)
    }

    /// [`Self::build_sql`] against an explicit column expression, for the
    /// chain builder's number terminal (`si2.value_number`, #1306).
    ///
    /// The per-prefix table is
    /// [`QuantityHandler::build_numeric_condition`], shared with quantity
    /// search: `eq`/`ne` match the implicit-precision range `[lo, hi)` derived
    /// from the search text as written ("100" → [99.5, 100.5)), the
    /// comparators use the exact value, and `ap` is +/- 10% of the magnitude.
    pub(crate) fn build_sql_for(
        column: &str,
        value: &SearchValue,
        param_offset: usize,
    ) -> SqlFragment {
        let param_num = param_offset + 1;

        // The grammar every backend shares. The search gate
        // (`validate_numeric_values`) rejects a value that is not a number
        // before any SQL is built, so this is defence in depth: an impossible
        // condition under every prefix, never a dropped or widened one.
        // `f64::from_str` used to stand here, and took `inf` and `nan`:
        // `value_number < Infinity` is true of every row (#1340).
        let number = match FhirNumberValue::parse(&value.value) {
            Ok(number) => number,
            Err(error) => {
                tracing::warn!(
                    "unvalidated number search value reached the SQLite handler: {error}"
                );
                return SqlFragment::new("1 = 0");
            }
        };

        QuantityHandler::build_numeric_condition(
            column,
            number.value,
            number.text(),
            value.prefix,
            param_num,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::query_builder::SqlParam;
    use super::*;
    use crate::types::SearchPrefix;

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

    /// Defence in depth behind `validate_numeric_values`: what `f64::from_str`
    /// takes for a number is an impossible condition too, under every prefix —
    /// `value_number < Infinity` is true of every row (#1340).
    #[test]
    fn non_finite_and_malformed_numbers_match_nothing() {
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
                let frag = NumberHandler::build_sql(&SearchValue::new(prefix, raw), 0);
                assert_eq!(frag.sql, "1 = 0", "{prefix:?} {raw:?}");
                assert!(frag.params.is_empty(), "{prefix:?} {raw:?}");
            }
        }
    }

    #[test]
    fn a_form_decoded_plus_is_read_as_written() {
        // `gt+5` and `1e+3` arrive as `gt 5` and `1e 3`.
        let frag = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Gt, " 5"), 0);
        assert_eq!(frag.sql, "value_number > ?1");
        let frag = NumberHandler::build_sql(&SearchValue::new(SearchPrefix::Lt, "1e 3"), 0);
        match &frag.params[0] {
            SqlParam::Float(f) => assert!((*f - 1000.0).abs() < 1e-9),
            _ => panic!("expected float param"),
        }
    }
}
