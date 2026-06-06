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
        let param_num = param_offset + 1;

        // Parse the quantity value: [prefix]number|system|code or [prefix]number|code or [prefix]number
        let quantity_str = &value.value;
        let parts: Vec<&str> = quantity_str.split('|').collect();

        let (num_value, system, code) = match parts.len() {
            1 => {
                // Just a number
                let num: f64 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return SqlFragment::new("1 = 0"),
                };
                (num, None, None)
            }
            2 => {
                // number|code
                let num: f64 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return SqlFragment::new("1 = 0"),
                };
                (num, None, Some(parts[1]))
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
                (num, system, code)
            }
            _ => return SqlFragment::new("1 = 0"),
        };

        // Raw match: numeric comparison plus the stored unit/system verbatim.
        let raw = {
            let num = Self::build_numeric_condition(
                "value_quantity_value",
                num_value,
                value.prefix,
                param_num,
            );
            let mut conditions = vec![num.sql];
            let mut params = num.params;
            let mut next_param = param_num + params.len();

            if let Some(sys) = system {
                conditions.push(format!("value_quantity_system = ?{}", next_param));
                params.push(SqlParam::string(sys));
                next_param += 1;
            }
            if let Some(c) = code {
                conditions.push(format!("value_quantity_unit = ?{}", next_param));
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
                Self::build_canonical_condition(c, num_value, value.prefix, start)
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

    /// Builds the canonical-column predicate by canonicalizing the search value's
    /// numeric *bounds* (range endpoints) with the supplied UCUM code, so unit
    /// equivalence is honored without losing implicit-precision semantics to
    /// float rounding. Returns `None` if the unit cannot be canonicalized.
    fn build_canonical_condition(
        code: &str,
        value: f64,
        prefix: SearchPrefix,
        param_num: usize,
    ) -> Option<(String, Vec<SqlParam>)> {
        use helios_fhirpath::ucum::canonicalize_quantity as canon;
        let col = "value_quantity_canonical_value";

        // Canonicalize one bound, returning (canonical_value, canonical_unit).
        let (sql, mut params, unit) = match prefix {
            SearchPrefix::Eq | SearchPrefix::Ne => {
                let half = Self::get_implicit_precision(value) / 2.0;
                let (lo, unit) = canon(value - half, code)?;
                let (hi, _) = canon(value + half, code)?;
                let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
                let sql = if matches!(prefix, SearchPrefix::Eq) {
                    format!("{col} >= ?{} AND {col} < ?{}", param_num, param_num + 1)
                } else {
                    format!("({col} < ?{} OR {col} >= ?{})", param_num, param_num + 1)
                };
                (sql, vec![SqlParam::float(lo), SqlParam::float(hi)], unit)
            }
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
            format!("{sql} AND value_quantity_canonical_unit = ?{unit_param}"),
            params,
        ))
    }

    /// Builds the numeric comparison part of the condition against `column`.
    fn build_numeric_condition(
        column: &str,
        value: f64,
        prefix: SearchPrefix,
        param_num: usize,
    ) -> SqlFragment {
        match prefix {
            SearchPrefix::Eq => {
                // Implicit precision range
                let precision = Self::get_implicit_precision(value);
                let half = precision / 2.0;
                SqlFragment::with_params(
                    format!(
                        "{column} >= ?{} AND {column} < ?{}",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::float(value - half), SqlParam::float(value + half)],
                )
            }
            SearchPrefix::Ne => {
                let precision = Self::get_implicit_precision(value);
                let half = precision / 2.0;
                SqlFragment::with_params(
                    format!(
                        "({column} < ?{} OR {column} >= ?{})",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::float(value - half), SqlParam::float(value + half)],
                )
            }
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

    /// Gets the implicit precision of a number.
    fn get_implicit_precision(value: f64) -> f64 {
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
    fn test_quantity_gt() {
        let value = SearchValue::new(SearchPrefix::Gt, "5.4|mg");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("> ?1"));
    }

    #[test]
    fn test_quantity_ap() {
        let value = SearchValue::new(SearchPrefix::Ap, "100");
        let frag = QuantityHandler::build_sql(&value, 0);

        assert!(frag.sql.contains("BETWEEN"));
    }
}
