//! Quantity parameter SQL handler.

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

        // Parse the quantity value: [prefix]number|system|code or [prefix]number|code or [prefix]number
        let quantity_str = &value.value;
        let parts: Vec<&str> = quantity_str.split('|').collect();

        let (num_str, num_value, system, code) = match parts.len() {
            1 => {
                // Just a number
                let num: f64 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return SqlFragment::new("1 = 0"),
                };
                (parts[0], num, None, None)
            }
            2 => {
                // number|code
                let num: f64 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return SqlFragment::new("1 = 0"),
                };
                (parts[0], num, None, Some(parts[1]))
            }
            3 => {
                // number|system|code
                let num: f64 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return SqlFragment::new("1 = 0"),
                };
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
                (parts[0], num, system, code)
            }
            _ => return SqlFragment::new("1 = 0"),
        };

        // Raw match: numeric comparison plus the stored unit/system verbatim.
        let raw = {
            let num = Self::build_numeric_condition(
                &format!("{table}value_quantity_value"),
                num_value,
                num_str,
                value.prefix,
                param_num,
            );
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

    /// Builds the canonical-column predicate. For `eq`/`ne` the search value's
    /// implicit-precision *bounds* (range endpoints, derived from `num_str`)
    /// are each canonicalized with the supplied UCUM code, so unit
    /// equivalence is honored without losing implicit-precision semantics to
    /// float rounding. For every other comparator the exact search value is
    /// canonicalized and compared with a single boundary, since the FHIR spec
    /// says those prefixes ignore implicit precision. Returns `None` if the
    /// unit cannot be canonicalized.
    fn build_canonical_condition(
        table: &str,
        code: &str,
        value: f64,
        num_str: &str,
        prefix: SearchPrefix,
        param_num: usize,
    ) -> Option<(String, Vec<SqlParam>)> {
        use helios_fhirpath::ucum::canonicalize_quantity as canon;
        let col = format!("{table}value_quantity_canonical_value");

        let (sql, mut params, unit) = match prefix {
            SearchPrefix::Eq | SearchPrefix::Ne => {
                let (lo, hi) = crate::search::implicit_range(value, num_str);
                let (lo, unit) = canon(lo, code)?;
                let (hi, _) = canon(hi, code)?;
                let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
                let sql = if matches!(prefix, SearchPrefix::Eq) {
                    format!("{col} >= ?{} AND {col} < ?{}", param_num, param_num + 1)
                } else {
                    format!("({col} < ?{} OR {col} >= ?{})", param_num, param_num + 1)
                };
                (sql, vec![SqlParam::float(lo), SqlParam::float(hi)], unit)
            }
            // gt/lt/ge/le/sa/eb compare against the exact canonicalized
            // value: per the FHIR spec, the implicit precision is ignored
            // for these prefixes.
            SearchPrefix::Gt | SearchPrefix::Sa => {
                let (b, unit) = canon(value, code)?;
                (
                    format!("{col} > ?{}", param_num),
                    vec![SqlParam::float(b)],
                    unit,
                )
            }
            SearchPrefix::Lt | SearchPrefix::Eb => {
                let (b, unit) = canon(value, code)?;
                (
                    format!("{col} < ?{}", param_num),
                    vec![SqlParam::float(b)],
                    unit,
                )
            }
            SearchPrefix::Ge => {
                let (b, unit) = canon(value, code)?;
                (
                    format!("{col} >= ?{}", param_num),
                    vec![SqlParam::float(b)],
                    unit,
                )
            }
            SearchPrefix::Le => {
                let (b, unit) = canon(value, code)?;
                (
                    format!("{col} <= ?{}", param_num),
                    vec![SqlParam::float(b)],
                    unit,
                )
            }
            SearchPrefix::Ap => {
                let margin = (value.abs() * 0.1).max(0.0001);
                let (lo, unit) = canon(value - margin, code)?;
                let (hi, _) = canon(value + margin, code)?;
                let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
                (
                    format!("{col} BETWEEN ?{} AND ?{}", param_num, param_num + 1),
                    vec![SqlParam::float(lo), SqlParam::float(hi)],
                    unit,
                )
            }
        };

        let unit_param = param_num + params.len();
        params.push(SqlParam::string(&unit));
        Some((
            format!("{sql} AND {table}value_quantity_canonical_unit = ?{unit_param}"),
            params,
        ))
    }

    /// Builds the numeric comparison part of the condition against `column`.
    /// `num_str` is the search value's textual form (used only by `eq`/`ne`
    /// to derive the implicit-precision range).
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
            SearchPrefix::Ap => {
                // +/- 10%
                let margin = (value.abs() * 0.1).max(0.0001);
                SqlFragment::with_params(
                    format!("{column} BETWEEN ?{} AND ?{}", param_num, param_num + 1),
                    vec![
                        SqlParam::float(value - margin),
                        SqlParam::float(value + margin),
                    ],
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
        assert!(gt.sql.contains("value_quantity_value > ?1"));
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

        assert!(frag.sql.contains("value_quantity_canonical_value > ?"));

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

        assert!(frag.sql.contains("BETWEEN"));
    }
}
