//! Quantity parameter SQL handler.

use crate::search::FhirQuantityValue;
use crate::types::{SearchPrefix, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles quantity parameter SQL generation.
pub struct QuantityHandler;

impl QuantityHandler {
    /// Builds SQL for a quantity parameter value.
    ///
    /// Quantity values can be:
    /// - `value` - matches any unit
    /// - `value|unit` - matches specific unit (code)
    /// - `value|system|code` - matches specific system and code
    pub fn build_sql(value: &SearchValue, param_offset: usize) -> SqlFragment {
        Self::build_sql_for("", value, param_offset)
    }

    /// [`Self::build_sql`] with every column qualified by `table` (`"si2."`),
    /// for the chain builder's quantity terminal (#1306). `build_sql` passes
    /// the empty string.
    pub(crate) fn build_sql_for(
        table: &str,
        value: &SearchValue,
        param_offset: usize,
    ) -> SqlFragment {
        let param_num = param_offset + 1;

        // Parse the quantity value — [prefix]number|system|code, number|code or
        // number — by the grammar every backend shares: only an unescaped `|`
        // separates, and the number must be a finite decimal. The search gate
        // (`validate_numeric_values`) rejects anything else before any SQL is
        // built, so this is defence in depth: an impossible condition under
        // every prefix, never a dropped or widened one. `f64::from_str` used
        // to stand here, and took `inf` and `nan` (#1340).
        let quantity = match FhirQuantityValue::parse(&value.value) {
            Ok(quantity) => quantity,
            Err(error) => {
                tracing::warn!(
                    "unvalidated quantity search value reached the SQLite handler: {error}"
                );
                return SqlFragment::new("1 = 0");
            }
        };
        let (num_str, num_value) = (quantity.number.text(), quantity.number.value);
        let (system, code) = (quantity.system.as_deref(), quantity.code.as_deref());

        // Raw match: numeric comparison plus the stored unit/system verbatim.
        let raw = {
            let predicate =
                crate::search::quantity_interval_predicate(value.prefix, num_value, num_str);
            let num = Self::build_interval_condition(
                &format!("{table}value_quantity_value"),
                &format!("{table}value_quantity_high"),
                &predicate,
                Some,
                param_num,
            )
            .expect("the identity mapping never fails");
            let mut conditions = vec![num.sql];
            let mut params = num.params;
            let mut next_param = param_num + params.len();

            if let Some(sys) = system {
                conditions.push(format!("{table}value_quantity_system = ?{}", next_param));
                params.push(SqlParam::string(sys));
                next_param += 1;
            }
            if let Some(c) = code {
                conditions.push(format!("{table}value_quantity_unit = ?{}", next_param));
                params.push(SqlParam::string(c));
            }
            SqlFragment::with_params(conditions.join(" AND "), params)
        };

        // Canonical match: when a UCUM code is supplied and convertible, also
        // match rows whose canonical unit/value are equivalent (e.g. `g` ⇄ `mg`).
        // ORed with the raw match so rows not yet reindexed (canonical columns
        // NULL) still match via their stored unit.
        if let Some(c) = code {
            let start = param_num + raw.params.len();
            if let Some((canon_sql, canon_params)) =
                Self::build_canonical_condition(table, c, num_value, num_str, value.prefix, start)
            {
                let mut params = raw.params;
                params.extend(canon_params);
                return SqlFragment::with_params(
                    format!("(({}) OR ({}))", raw.sql, canon_sql),
                    params,
                );
            }
        }

        raw
    }

    /// Builds the canonical-column predicate: the same interval predicate as
    /// the raw branch, with every bound of it mapped through UCUM
    /// canonicalization by the supplied code, over the canonical low and high
    /// columns. For `eq`/`ne` the bounds are the search value's
    /// implicit-precision range; the comparators use the exact value, since
    /// the FHIR spec says they ignore implicit precision. Returns `None` if
    /// the unit cannot be canonicalized.
    fn build_canonical_condition(
        table: &str,
        code: &str,
        value: f64,
        num_str: &str,
        prefix: SearchPrefix,
        param_num: usize,
    ) -> Option<(String, Vec<SqlParam>)> {
        use helios_fhirpath::ucum::canonicalize_quantity as canon;
        let (_, unit) = canon(value, code)?;
        let predicate = crate::search::quantity_interval_predicate(prefix, value, num_str);
        let fragment = Self::build_interval_condition(
            &format!("{table}value_quantity_canonical_value"),
            &format!("{table}value_quantity_canonical_high"),
            &predicate,
            |bound| canon(bound, code).map(|(c, _)| c),
            param_num,
        )?;
        let mut params = fragment.params;
        let unit_param = param_num + params.len();
        params.push(SqlParam::string(&unit));
        Some((
            format!(
                "{} AND {table}value_quantity_canonical_unit = ?{unit_param}",
                fragment.sql
            ),
            params,
        ))
    }

    /// Renders a [`QuantityIntervalPredicate`] over a stored closed interval
    /// whose lower end is `low_col` and upper end `high_col` (#2602). Every
    /// bound is passed through `map` (identity for the raw columns, UCUM
    /// canonicalization for the canonical ones); `None` from `map` aborts.
    /// A point (`high = low`) behaves exactly as the old single-column
    /// comparison; `gt`/`ge`/`eb`/`eq`/`ne` constrain the upper end, which
    /// the `idx_search_quantity_high` indexes lead with.
    fn build_interval_condition(
        low_col: &str,
        high_col: &str,
        predicate: &crate::search::QuantityIntervalPredicate,
        map: impl Fn(f64) -> Option<f64>,
        param_num: usize,
    ) -> Option<SqlFragment> {
        use crate::search::{Cmp, QuantityBound};
        let op = |cmp: Cmp| match cmp {
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        };
        let mut params = Vec::new();
        let mut groups = Vec::new();
        for group in &predicate.any_of {
            let mut terms = Vec::new();
            for bound in group {
                let (col, cmp, v) = match *bound {
                    QuantityBound::Low(cmp, v) => (low_col, cmp, v),
                    QuantityBound::High(cmp, v) => (high_col, cmp, v),
                };
                params.push(SqlParam::float(map(v)?));
                terms.push(format!(
                    "{col} {} ?{}",
                    op(cmp),
                    param_num + params.len() - 1
                ));
            }
            groups.push(terms.join(" AND "));
        }
        let sql = if groups.len() == 1 {
            groups.remove(0)
        } else {
            format!(
                "({})",
                groups
                    .iter()
                    .map(|g| format!("({g})"))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            )
        };
        Some(SqlFragment::with_params(sql, params))
    }

    /// Builds the numeric comparison part of the condition against `column`.
    /// `num_str` is the search value's textual form (used only by `eq`/`ne`
    /// and `ap` to derive the implicit-precision range and the `ap` window).
    ///
    /// This is the one numeric per-prefix table on SQLite: number search
    /// ([`super::number::NumberHandler`]) and, through the two handlers, the
    /// chain builder's numeric terminals use it too.
    pub(crate) fn build_numeric_condition(
        column: &str,
        value: f64,
        num_str: &str,
        prefix: SearchPrefix,
        param_num: usize,
    ) -> SqlFragment {
        match prefix {
            SearchPrefix::Eq => {
                let (lo, hi) = crate::search::implicit_range(value, num_str);
                SqlFragment::with_params(
                    format!(
                        "{column} >= ?{} AND {column} < ?{}",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::float(lo), SqlParam::float(hi)],
                )
            }
            SearchPrefix::Ne => {
                let (lo, hi) = crate::search::implicit_range(value, num_str);
                SqlFragment::with_params(
                    format!(
                        "({column} < ?{} OR {column} >= ?{})",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::float(lo), SqlParam::float(hi)],
                )
            }
            // gt/lt/ge/le/sa/eb compare against the exact search value: per
            // the FHIR spec, the implicit precision is ignored for these
            // prefixes.
            SearchPrefix::Gt | SearchPrefix::Sa => SqlFragment::with_params(
                format!("{column} > ?{}", param_num),
                vec![SqlParam::float(value)],
            ),
            SearchPrefix::Lt | SearchPrefix::Eb => SqlFragment::with_params(
                format!("{column} < ?{}", param_num),
                vec![SqlParam::float(value)],
            ),
            SearchPrefix::Ge => SqlFragment::with_params(
                format!("{column} >= ?{}", param_num),
                vec![SqlParam::float(value)],
            ),
            SearchPrefix::Le => SqlFragment::with_params(
                format!("{column} <= ?{}", param_num),
                vec![SqlParam::float(value)],
            ),
            // ap: the shared window, `max(10%, half the implicit precision)`
            // either side of the value (#1390).
            SearchPrefix::Ap => {
                let (lo, hi) = crate::search::approx_range(value, num_str);
                SqlFragment::with_params(
                    format!("{column} BETWEEN ?{} AND ?{}", param_num, param_num + 1),
                    vec![SqlParam::float(lo), SqlParam::float(hi)],
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantity_value_only() {
        let value = SearchValue::new(SearchPrefix::Eq, "5.4");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("value_quantity_value"));
        assert!(!frag.sql.contains("value_quantity_unit"));
    }

    #[test]
    fn test_quantity_with_code() {
        let value = SearchValue::new(SearchPrefix::Eq, "5.4|mg");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("value_quantity_value"));
        assert!(frag.sql.contains("value_quantity_unit"));
    }

    #[test]
    fn test_quantity_with_system_and_code() {
        let value = SearchValue::new(SearchPrefix::Eq, "5.4|http://unitsofmeasure.org|mg");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("value_quantity_value"));
        assert!(frag.sql.contains("value_quantity_system"));
        assert!(frag.sql.contains("value_quantity_unit"));
    }

    #[test]
    fn raw_comparators_use_exact_value() {
        // gt/le ignore implicit precision and compare against the exact
        // search value, per the FHIR spec.
        let gt = QuantityHandler::build_sql(&SearchValue::new(SearchPrefix::Gt, "60|kg"), 0);
        assert!(gt.sql.contains("value_quantity_high > ?1"));
        match &gt.params[0] {
            SqlParam::Float(f) => assert!((*f - 60.0).abs() < 1e-9),
            _ => panic!("expected float param"),
        }

        let le = QuantityHandler::build_sql(&SearchValue::new(SearchPrefix::Le, "60"), 0);
        assert!(le.sql.contains("value_quantity_value <= ?1"));
    }

    #[test]
    fn eq_range_follows_search_text_precision() {
        // "60.0" has one decimal of precision → [59.95, 60.05).
        let eq = QuantityHandler::build_sql(&SearchValue::new(SearchPrefix::Eq, "60.0"), 0);

        assert!(eq.params.len() >= 2);
        match (&eq.params[0], &eq.params[1]) {
            (SqlParam::Float(lo), SqlParam::Float(hi)) => {
                assert!((*lo - 59.95).abs() < 1e-9);
                assert!((*hi - 60.05).abs() < 1e-9);
            }
            _ => panic!("expected float params"),
        }
    }

    #[test]
    fn canonical_comparator_uses_exact_canonical_value() {
        use helios_fhirpath::ucum::canonicalize_quantity as canon;

        let value = SearchValue::new(SearchPrefix::Gt, "60|http://unitsofmeasure.org|kg");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("value_quantity_canonical_high > ?"));

        // The canonical bound must equal canon(60.0, "kg") exactly, not a
        // value widened by half the implicit precision (e.g. 60.5).
        let (expected, _) = canon(60.0, "kg").expect("kg canonicalizes");
        let has_exact_canonical_param = frag
            .params
            .iter()
            .any(|p| matches!(p, SqlParam::Float(f) if (*f - expected).abs() < 1e-9));
        assert!(
            has_exact_canonical_param,
            "expected a float param equal to canon(60.0, \"kg\") = {expected}, got {:?}",
            frag.params
        );
    }

    #[test]
    fn test_quantity_ap() {
        let value = SearchValue::new(SearchPrefix::Ap, "100");
        let frag = QuantityHandler::build_sql(&value, 0);

        // The shared window (#1390): [90, 110] around 100, as an interval
        // overlap: `low <= 110 AND high >= 90`.
        assert!(
            frag.sql.contains("value_quantity_value <= ?1"),
            "{}",
            frag.sql
        );
        assert!(
            frag.sql.contains("value_quantity_high >= ?2"),
            "{}",
            frag.sql
        );
        assert!(
            matches!(
                frag.params.as_slice(),
                [SqlParam::Float(hi), SqlParam::Float(lo), ..]
                    if (lo - 90.0).abs() < 1e-9 && (hi - 110.0).abs() < 1e-9
            ),
            "{:?}",
            frag.params
        );
    }

    /// Defence in depth behind `validate_numeric_values` (#1340): a number
    /// part that is missing, malformed or not finite is an impossible
    /// condition under every prefix, never a widened one.
    #[test]
    fn invalid_number_part_matches_nothing() {
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
                "5.4\\|mg",
            ] {
                let frag = QuantityHandler::build_sql(&SearchValue::new(prefix, raw), 0);
                assert_eq!(frag.sql, "1 = 0", "{prefix:?} {raw:?}");
                assert!(frag.params.is_empty(), "{prefix:?} {raw:?}");
            }
        }
    }

    #[test]
    fn an_escaped_pipe_is_part_of_the_code() {
        let value = SearchValue::new(SearchPrefix::Eq, "5.4|http://example.org|a\\|b");
        let frag = QuantityHandler::build_sql(&value, 0);
        assert!(
            frag.params
                .iter()
                .any(|p| matches!(p, SqlParam::String(s) if s == "a|b")),
            "{:?}",
            frag.params
        );
        assert!(
            frag.params
                .iter()
                .any(|p| matches!(p, SqlParam::String(s) if s == "http://example.org")),
            "{:?}",
            frag.params
        );
    }
}
