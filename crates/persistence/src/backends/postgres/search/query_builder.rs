//! PostgreSQL search query builder.
//!
//! Builds SQL queries for FHIR search operations using PostgreSQL syntax
//! with $N parameter placeholders, ILIKE for case-insensitive matching,
//! and native TIMESTAMPTZ comparisons.

use chrono::{DateTime, Utc};

use crate::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};

/// How a sort key's value is typed for cursor (keyset) binding and comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortValueKind {
    /// Text column (string/token/uri/reference/`_id`).
    Text,
    /// Floating-point column (number/quantity).
    Number,
    /// Timestamp column (`_lastUpdated`, date).
    Timestamp,
}

/// A single keyset sort key: the SQL value expression, its direction, and the
/// value kind used to bind/read the cursor boundary value.
#[derive(Debug, Clone)]
pub struct KeysetKey {
    /// SQL expression yielding the sort value (column or correlated subquery).
    pub expr: String,
    /// Sort direction.
    pub direction: crate::types::SortDirection,
    /// How the value is typed for binding/reading.
    pub kind: SortValueKind,
}

/// Determines the value kind for a sort parameter.
pub(crate) fn sort_value_kind(
    parameter: &str,
    param_type: Option<SearchParamType>,
) -> SortValueKind {
    match parameter {
        "_id" => SortValueKind::Text,
        "_lastUpdated" => SortValueKind::Timestamp,
        _ => match param_type {
            Some(SearchParamType::Number) | Some(SearchParamType::Quantity) => {
                SortValueKind::Number
            }
            Some(SearchParamType::Date) => SortValueKind::Timestamp,
            _ => SortValueKind::Text,
        },
    }
}

/// Maps a search-parameter type to the `search_index` value column used when
/// sorting on that parameter. Returns `None` for types that are not sortable via
/// a single value column (composite, special).
pub(crate) fn sort_value_column(param_type: SearchParamType) -> Option<&'static str> {
    match param_type {
        SearchParamType::String => Some("value_string"),
        SearchParamType::Token => Some("value_token_code"),
        SearchParamType::Date => Some("value_date"),
        SearchParamType::Number => Some("value_number"),
        SearchParamType::Quantity => Some("value_quantity_value"),
        SearchParamType::Reference => Some("value_reference"),
        SearchParamType::Uri => Some("value_uri"),
        SearchParamType::Composite | SearchParamType::Special => None,
    }
}

/// A SQL fragment with associated parameters.
#[derive(Debug, Clone)]
pub struct SqlFragment {
    /// The SQL string with $N placeholders.
    pub sql: String,
    /// The parameter values.
    pub params: Vec<SqlParam>,
}

/// A SQL parameter value.
#[derive(Debug, Clone)]
pub enum SqlParam {
    /// Text parameter.
    Text(String),
    /// Floating point parameter.
    Float(f64),
    /// Integer parameter.
    Integer(i64),
    /// Boolean parameter.
    Bool(bool),
    /// Timestamp parameter.
    Timestamp(DateTime<Utc>),
    /// Null parameter.
    Null,
}

impl SqlParam {
    /// Creates a text parameter.
    pub fn text(s: &str) -> Self {
        SqlParam::Text(s.to_string())
    }
}

impl SqlFragment {
    /// Creates a new fragment with no parameters.
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
        }
    }

    /// Creates a fragment with parameters.
    pub fn with_params(sql: impl Into<String>, params: Vec<SqlParam>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }

    /// Combines two fragments with AND.
    pub fn and(self, other: SqlFragment) -> SqlFragment {
        SqlFragment {
            sql: format!("({}) AND ({})", self.sql, other.sql),
            params: [self.params, other.params].concat(),
        }
    }

    /// Combines two fragments with OR.
    pub fn or(self, other: SqlFragment) -> SqlFragment {
        SqlFragment {
            sql: format!("({}) OR ({})", self.sql, other.sql),
            params: [self.params, other.params].concat(),
        }
    }
}

/// PostgreSQL search query builder.
pub struct PostgresQueryBuilder;

impl PostgresQueryBuilder {
    /// Builds a search query for finding matching resource IDs.
    ///
    /// Returns a SQL fragment that selects DISTINCT resource_ids from search_index
    /// matching the given search parameters.
    pub fn build_search_query(query: &SearchQuery, param_offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut current_offset = param_offset;

        for param in &query.parameters {
            if let Some(condition) = Self::build_parameter_condition(param, current_offset) {
                current_offset += condition.params.len();
                conditions.push(condition);
            }
        }

        if conditions.is_empty() {
            return None;
        }

        // AND all conditions together
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }

        Some(combined)
    }

    /// Builds an `ORDER BY` clause from the query's `_sort` directives.
    ///
    /// Mirrors the SQLite backend's ordering semantics so the two backends stay
    /// at parity. Multiple directives (e.g. `_sort=_lastUpdated,-_id`) are
    /// honored in order, with an `id ASC` tie-breaker appended for stable
    /// pagination when `_id` is not already part of the sort.
    ///
    /// # Supported sort parameters
    ///
    /// - `_id` → `id`
    /// - `_lastUpdated` → `last_updated`
    ///
    /// Any other parameter currently falls back to `id`. Sorting by arbitrary
    /// search parameters would require an additional join against `search_index`
    /// and is not yet implemented (see the search spec assessment).
    ///
    /// Note: this is applied to the first-page and offset paths only. The
    /// cursor (keyset) paths keep their `(last_updated, id)` ordering, which is
    /// required by the keyset `WHERE` comparison; cursor pages therefore always
    /// use the default ordering.
    pub fn build_order_by(query: &SearchQuery) -> String {
        if query.sort.is_empty() {
            return "ORDER BY last_updated DESC, id ASC".to_string();
        }

        let mut clauses: Vec<String> = query
            .sort
            .iter()
            .map(|s| {
                let dir = match s.direction {
                    crate::types::SortDirection::Ascending => "ASC",
                    crate::types::SortDirection::Descending => "DESC",
                };
                format!("{} {}", Self::sort_expression(s), dir)
            })
            .collect();

        let sorts_by_id = query.sort.iter().any(|s| s.parameter == "_id");
        if !sorts_by_id {
            clauses.push("id ASC".to_string());
        }

        format!("ORDER BY {}", clauses.join(", "))
    }

    /// Returns the keyset sort key for cursor pagination, or `None` when the
    /// query has multiple sort fields (those are returned as a single page
    /// rather than paged with a possibly-inconsistent keyset).
    pub fn primary_keyset_key(query: &SearchQuery) -> Option<KeysetKey> {
        match query.sort.len() {
            0 => Some(KeysetKey {
                expr: "last_updated".to_string(),
                direction: crate::types::SortDirection::Descending,
                kind: SortValueKind::Timestamp,
            }),
            1 => {
                let directive = &query.sort[0];
                Some(KeysetKey {
                    expr: Self::sort_expression(directive),
                    direction: directive.direction,
                    kind: sort_value_kind(&directive.parameter, directive.param_type),
                })
            }
            _ => None,
        }
    }

    /// Builds the ORDER BY expression for a single sort directive.
    ///
    /// `_id`/`_lastUpdated` map to `resources` columns. Any other indexed search
    /// parameter sorts on a correlated subquery into `search_index`, taking the
    /// MIN value for ascending and MAX for descending (FHIR multi-value sort).
    fn sort_expression(directive: &crate::types::SortDirective) -> String {
        match directive.parameter.as_str() {
            "_id" => return "id".to_string(),
            "_lastUpdated" => return "last_updated".to_string(),
            _ => {}
        }

        match directive.param_type.and_then(sort_value_column) {
            Some(col) => {
                let agg = match directive.direction {
                    crate::types::SortDirection::Ascending => "MIN",
                    crate::types::SortDirection::Descending => "MAX",
                };
                format!(
                    "(SELECT {}({}) FROM search_index si WHERE si.tenant_id = $1 AND si.resource_type = $2 AND si.resource_id = resources.id AND si.param_name = '{}')",
                    agg, col, directive.parameter
                )
            }
            // Unsortable (composite/special/unresolved) — stable fallback.
            None => "id".to_string(),
        }
    }

    /// Builds a condition for a single search parameter.
    fn build_parameter_condition(
        param: &SearchParameter,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if param.values.is_empty() {
            return None;
        }

        // The `:missing` modifier is type-agnostic and resolved purely from the
        // presence/absence of a search_index entry for the parameter.
        if let Some(SearchModifier::Missing) = param.modifier {
            return Some(Self::build_missing_condition(param));
        }

        // Handle special parameters
        match param.name.as_str() {
            "_id" => return Self::build_id_condition(&param.values, param_offset),
            "_lastUpdated" => {
                return Self::build_last_updated_condition(&param.values, param_offset);
            }
            _ => {}
        }

        // Build conditions based on parameter type
        match param.param_type {
            SearchParamType::String => Self::build_string_condition(param, param_offset),
            SearchParamType::Token => Self::build_token_condition(param, param_offset),
            SearchParamType::Date => Self::build_date_condition(param, param_offset),
            SearchParamType::Number => Self::build_number_condition(param, param_offset),
            SearchParamType::Quantity => Self::build_quantity_condition(param, param_offset),
            SearchParamType::Reference => Self::build_reference_condition(param, param_offset),
            SearchParamType::Uri => Self::build_uri_condition(param, param_offset),
            SearchParamType::Composite => Self::build_composite_condition(param, param_offset),
            SearchParamType::Special => None,
        }
    }

    fn build_id_condition(values: &[SearchValue], offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        for (i, value) in values.iter().enumerate() {
            let param_num = offset + i + 1;
            conditions.push(SqlFragment::with_params(
                format!("id = ${}", param_num),
                vec![SqlParam::text(&value.value)],
            ));
        }
        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    fn build_last_updated_condition(values: &[SearchValue], offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        for (i, value) in values.iter().enumerate() {
            let param_num = offset + i + 1;
            let op = Self::prefix_to_operator(&value.prefix);
            conditions.push(SqlFragment::with_params(
                format!("last_updated {} ${}", op, param_num),
                vec![SqlParam::text(&value.value)],
            ));
        }
        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }
        Some(combined)
    }

    /// Builds an `id IN/NOT IN` condition for the `:missing` modifier.
    ///
    /// `param:missing=true` matches resources with **no** `search_index` entry
    /// for the parameter; `:missing=false` matches resources that **have** one.
    /// Uses only the always-present `$1`/`$2` (tenant, resource type) bind
    /// params, so it adds no parameters to the surrounding query.
    fn build_missing_condition(param: &SearchParameter) -> SqlFragment {
        let is_missing = param
            .values
            .first()
            .map(|v| v.value == "true")
            .unwrap_or(false);
        let inner = format!(
            "SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}'",
            param.name
        );
        let sql = if is_missing {
            format!("id NOT IN ({})", inner)
        } else {
            format!("id IN ({})", inner)
        };
        SqlFragment::new(sql)
    }

    fn build_string_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let modifier = param.modifier.as_ref();
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let condition = match modifier {
                Some(SearchModifier::Exact) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_string = ${})",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&value.value)],
                ),
                Some(SearchModifier::Contains) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_string ILIKE ${})",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&format!("%{}%", value.value))],
                ),
                _ => {
                    // Default: starts-with (case-insensitive)
                    SqlFragment::with_params(
                        format!(
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_string ILIKE ${})",
                            param.name, param_num
                        ),
                        vec![SqlParam::text(&format!("{}%", value.value))],
                    )
                }
            };
            conditions.push(condition);
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    fn build_token_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        // `:of-type` matches an Identifier by its type (system|code) and value,
        // in the three-part form `type-system|type-code|identifier-value`.
        if let Some(SearchModifier::OfType) = param.modifier {
            return Self::build_of_type_condition(param, offset);
        }

        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let base_offset = offset + i * 2;
            let condition = if let Some((system, code)) = value.value.split_once('|') {
                if system.is_empty() {
                    // |code - match any system
                    SqlFragment::with_params(
                        format!(
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_token_code = ${})",
                            param.name,
                            base_offset + 1
                        ),
                        vec![SqlParam::text(code)],
                    )
                } else if code.is_empty() {
                    // system| - match any code in system
                    SqlFragment::with_params(
                        format!(
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_token_system = ${})",
                            param.name,
                            base_offset + 1
                        ),
                        vec![SqlParam::text(system)],
                    )
                } else {
                    // system|code - exact match
                    SqlFragment::with_params(
                        format!(
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_token_system = ${} AND value_token_code = ${})",
                            param.name,
                            base_offset + 1,
                            base_offset + 2
                        ),
                        vec![SqlParam::text(system), SqlParam::text(code)],
                    )
                }
            } else {
                // code only - match any system
                SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_token_code = ${})",
                        param.name,
                        base_offset + 1
                    ),
                    vec![SqlParam::text(&value.value)],
                )
            };
            conditions.push(condition);
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }

        // `:not` reverses the match: include resources that have no matching
        // value (including those with no value at all). Negating the whole
        // OR-set gives `NOT (id IN (...) OR id IN (...))`.
        if let Some(SearchModifier::Not) = param.modifier {
            let sql = format!("NOT ({})", combined.sql);
            combined = SqlFragment::with_params(sql, combined.params);
        }

        Some(combined)
    }

    /// Builds an `:of-type` identifier condition (token modifier).
    ///
    /// Value form: `type-system|type-code|identifier-value`. Empty parts are
    /// dropped, so e.g. `||MR12345` matches by identifier value only.
    fn build_of_type_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut value_conditions = Vec::new();
        let mut all_params: Vec<SqlParam> = Vec::new();
        let mut current = offset;

        for value in &param.values {
            let parts: Vec<&str> = value.value.splitn(3, '|').collect();
            if parts.len() != 3 {
                continue;
            }
            let (type_system, type_code, identifier_value) = (parts[0], parts[1], parts[2]);

            let mut conds = Vec::new();
            // Identifier value (matched against the token code column).
            if !identifier_value.is_empty() {
                current += 1;
                conds.push(format!("value_token_code = ${}", current));
                all_params.push(SqlParam::text(identifier_value));
            }
            if !type_system.is_empty() {
                current += 1;
                conds.push(format!("value_identifier_type_system = ${}", current));
                all_params.push(SqlParam::text(type_system));
            }
            if !type_code.is_empty() {
                current += 1;
                conds.push(format!("value_identifier_type_code = ${}", current));
                all_params.push(SqlParam::text(type_code));
            }
            if conds.is_empty() {
                continue;
            }

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                param.name,
                conds.join(" AND ")
            ));
        }

        if value_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(
            value_conditions.join(" OR "),
            all_params,
        ))
    }

    /// Builds a composite-parameter condition.
    ///
    /// Composite values join their components with `$` (e.g.
    /// `http://loinc.org|8480-6$lt60`). Each component of a composite instance
    /// is indexed as its own `search_index` row sharing a `composite_group`, so
    /// a resource matches when there is a group in which every component is
    /// satisfied by some row. This is expressed with
    /// `GROUP BY resource_id, composite_group HAVING <every component present>`.
    fn build_composite_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        if param.components.is_empty() {
            return None;
        }

        let mut value_conditions = Vec::new();
        let mut all_params: Vec<SqlParam> = Vec::new();
        let mut current = offset;

        for value in &param.values {
            let parts: Vec<&str> = value.value.split('$').collect();
            if parts.len() != param.components.len() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            let mut havings = Vec::new();
            let mut ok = true;
            for (part, component) in parts.iter().zip(param.components.iter()) {
                let cv = Self::parse_component_value(part);
                match Self::build_composite_component(&cv, component.param_type, current) {
                    Some((sql, params)) => {
                        current += params.len();
                        all_params.extend(params);
                        havings.push(format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", sql));
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }

            if !ok || havings.is_empty() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' GROUP BY resource_id, composite_group HAVING {})",
                param.name,
                havings.join(" AND ")
            ));
        }

        if value_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(
            value_conditions.join(" OR "),
            all_params,
        ))
    }

    /// Parses a composite component value, stripping any comparison prefix.
    fn parse_component_value(part: &str) -> SearchValue {
        let prefixes = [
            ("ne", SearchPrefix::Ne),
            ("gt", SearchPrefix::Gt),
            ("lt", SearchPrefix::Lt),
            ("ge", SearchPrefix::Ge),
            ("le", SearchPrefix::Le),
            ("sa", SearchPrefix::Sa),
            ("eb", SearchPrefix::Eb),
            ("ap", SearchPrefix::Ap),
            ("eq", SearchPrefix::Eq),
        ];
        for (prefix_str, prefix) in prefixes {
            if let Some(stripped) = part.strip_prefix(prefix_str) {
                return SearchValue {
                    prefix,
                    value: stripped.to_string(),
                };
            }
        }
        SearchValue {
            prefix: SearchPrefix::Eq,
            value: part.to_string(),
        }
    }

    /// Builds a single composite component as a bare column predicate (no
    /// `id IN (...)` wrapper — the caller scopes it to the composite row).
    /// Returns `None` if the value cannot be parsed for the component type.
    fn build_composite_component(
        value: &SearchValue,
        param_type: SearchParamType,
        offset: usize,
    ) -> Option<(String, Vec<SqlParam>)> {
        match param_type {
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        Some((
                            format!("value_token_code = ${}", offset + 1),
                            vec![SqlParam::text(code)],
                        ))
                    } else if code.is_empty() {
                        Some((
                            format!("value_token_system = ${}", offset + 1),
                            vec![SqlParam::text(system)],
                        ))
                    } else {
                        Some((
                            format!(
                                "value_token_system = ${} AND value_token_code = ${}",
                                offset + 1,
                                offset + 2
                            ),
                            vec![SqlParam::text(system), SqlParam::text(code)],
                        ))
                    }
                } else {
                    Some((
                        format!("value_token_code = ${}", offset + 1),
                        vec![SqlParam::text(&value.value)],
                    ))
                }
            }
            SearchParamType::String => Some((
                format!("value_string ILIKE ${}", offset + 1),
                vec![SqlParam::text(&format!("{}%", value.value))],
            )),
            SearchParamType::Number => {
                let num = value.value.parse::<f64>().ok()?;
                let op = Self::prefix_to_operator(&value.prefix);
                Some((
                    format!("value_number {} ${}", op, offset + 1),
                    vec![SqlParam::Float(num)],
                ))
            }
            SearchParamType::Quantity => {
                let parts: Vec<&str> = value.value.splitn(3, '|').collect();
                let num = parts.first().and_then(|s| s.parse::<f64>().ok())?;
                let op = Self::prefix_to_operator(&value.prefix);
                if parts.len() >= 3 {
                    Some((
                        format!(
                            "value_quantity_value {} ${} AND value_quantity_unit = ${}",
                            op,
                            offset + 1,
                            offset + 2
                        ),
                        vec![SqlParam::Float(num), SqlParam::text(parts[2])],
                    ))
                } else {
                    Some((
                        format!("value_quantity_value {} ${}", op, offset + 1),
                        vec![SqlParam::Float(num)],
                    ))
                }
            }
            SearchParamType::Date => {
                let op = Self::prefix_to_operator(&value.prefix);
                let ts = Self::parse_date_value(&value.value);
                Some((
                    format!("value_date {} ${}", op, offset + 1),
                    vec![SqlParam::Timestamp(ts)],
                ))
            }
            _ => None,
        }
    }

    fn build_date_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let op = Self::prefix_to_operator(&value.prefix);
            let timestamp = Self::parse_date_value(&value.value);
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_date {} ${})",
                    param.name, op, param_num
                ),
                vec![SqlParam::Timestamp(timestamp)],
            ));
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }
        Some(combined)
    }

    fn build_number_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let op = Self::prefix_to_operator(&value.prefix);
            if let Ok(num) = value.value.parse::<f64>() {
                conditions.push(SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_number {} ${})",
                        param.name, op, param_num
                    ),
                    vec![SqlParam::Float(num)],
                ));
            }
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }
        Some(combined)
    }

    fn build_quantity_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let base_offset = offset + i * 2;
            // Parse quantity: [prefix]number|system|code
            let parts: Vec<&str> = value.value.splitn(3, '|').collect();
            if let Some(num_str) = parts.first() {
                if let Ok(num) = num_str.parse::<f64>() {
                    let op = Self::prefix_to_operator(&value.prefix);
                    if parts.len() >= 3 {
                        conditions.push(SqlFragment::with_params(
                            format!(
                                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_quantity_value {} ${} AND value_quantity_unit = ${})",
                                param.name, op, base_offset + 1, base_offset + 2
                            ),
                            vec![SqlParam::Float(num), SqlParam::text(parts[2])],
                        ));
                    } else {
                        conditions.push(SqlFragment::with_params(
                            format!(
                                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_quantity_value {} ${})",
                                param.name, op, base_offset + 1
                            ),
                            vec![SqlParam::Float(num)],
                        ));
                    }
                }
            }
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }
        Some(combined)
    }

    fn build_reference_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_reference = ${})",
                    param.name, param_num
                ),
                vec![SqlParam::text(&value.value)],
            ));
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    fn build_uri_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let modifier = param.modifier.as_ref();
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let condition = match modifier {
                Some(SearchModifier::Below) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_uri LIKE ${} || '%')",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&value.value)],
                ),
                Some(SearchModifier::Above) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND ${} LIKE value_uri || '%')",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&value.value)],
                ),
                _ => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_uri = ${})",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&value.value)],
                ),
            };
            conditions.push(condition);
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// Converts a FHIR search prefix to a SQL comparison operator.
    fn prefix_to_operator(prefix: &SearchPrefix) -> &'static str {
        match prefix {
            SearchPrefix::Eq => "=",
            SearchPrefix::Ne => "!=",
            SearchPrefix::Gt => ">",
            SearchPrefix::Lt => "<",
            SearchPrefix::Ge => ">=",
            SearchPrefix::Le => "<=",
            SearchPrefix::Sa => ">", // starts after
            SearchPrefix::Eb => "<", // ends before
            SearchPrefix::Ap => "=", // approximately (simplified)
        }
    }

    /// Parses a FHIR date search value into a `DateTime<Utc>`.
    ///
    /// Handles partial dates (year, year-month, date) and full date-times.
    fn parse_date_value(value: &str) -> DateTime<Utc> {
        let normalized = if value.contains('T') {
            if value.contains('+') || value.contains('Z') || value.ends_with("-00:00") {
                value.to_string()
            } else {
                format!("{}+00:00", value)
            }
        } else if value.len() == 10 {
            format!("{}T00:00:00+00:00", value)
        } else if value.len() == 7 {
            format!("{}-01T00:00:00+00:00", value)
        } else if value.len() == 4 {
            format!("{}-01-01T00:00:00+00:00", value)
        } else {
            value.to_string()
        };

        DateTime::parse_from_rfc3339(&normalized)
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|_| normalized.parse::<DateTime<Utc>>())
            .unwrap_or_else(|_| Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompositeSearchComponent, SearchModifier, SearchQuery};

    #[test]
    fn composite_token_quantity_sql() {
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$lt60",
            )],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        // Non-cursor search binds $1=tenant, $2=type, so params start at $3.
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("composite should produce a condition");

        assert!(frag.sql.contains("param_name = 'code-value-quantity'"));
        assert!(frag.sql.contains("value_token_system = $3"));
        assert!(frag.sql.contains("value_token_code = $4"));
        assert!(frag.sql.contains("value_quantity_value < $5"));
        // token (system+code) = 2 params, quantity (no unit) = 1 param.
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn composite_returns_none_without_components() {
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "a$b")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        assert!(PostgresQueryBuilder::build_search_query(&query, 2).is_none());
    }

    #[test]
    fn of_type_identifier_sql() {
        let param = SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::OfType),
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://terminology.hl7.org/CodeSystem/v2-0203|MR|12345",
            )],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Patient").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect(":of-type should produce a condition");

        assert!(frag.sql.contains("param_name = 'identifier'"));
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_identifier_type_system = $4"));
        assert!(frag.sql.contains("value_identifier_type_code = $5"));
        assert_eq!(frag.params.len(), 3);
    }
}
