//! SQL Query Builder for FHIR Search.
//!
//! Translates FHIR search queries into SQL statements that can be executed
//! against the SQLite search_index table.

use std::collections::HashSet;

use crate::types::{SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue};

use super::parameter_handlers::{
    CompositeHandler, DateHandler, NumberHandler, QuantityHandler, ReferenceHandler, StringHandler,
    TokenHandler, UriHandler,
};

/// How a sort key's value is typed for cursor (keyset) binding and comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortValueKind {
    /// Text column (string/token/uri/reference/`_id`).
    Text,
    /// Floating-point column (number/quantity).
    Number,
    /// Timestamp column, stored as RFC3339 text (`_lastUpdated`, date).
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

/// A fragment of SQL with bound parameters.
#[derive(Debug, Clone)]
pub struct SqlFragment {
    /// The SQL clause.
    pub sql: String,
    /// Bound parameter values.
    pub params: Vec<SqlParam>,
}

/// A bound SQL parameter.
#[derive(Debug, Clone)]
pub enum SqlParam {
    /// String parameter.
    String(String),
    /// Integer parameter.
    Integer(i64),
    /// Float parameter.
    Float(f64),
    /// Null parameter.
    Null,
}

impl SqlParam {
    /// Creates a string parameter.
    pub fn string(s: impl Into<String>) -> Self {
        SqlParam::String(s.into())
    }

    /// Creates an integer parameter.
    pub fn integer(i: i64) -> Self {
        SqlParam::Integer(i)
    }

    /// Creates a float parameter.
    pub fn float(f: f64) -> Self {
        SqlParam::Float(f)
    }
}

impl SqlFragment {
    /// Creates a new SQL fragment.
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

    /// Adds a parameter placeholder and returns the placeholder string.
    pub fn add_param(&mut self, param: SqlParam) -> String {
        self.params.push(param);
        format!("?{}", self.params.len())
    }

    /// Combines with another fragment using AND.
    pub fn and(mut self, other: SqlFragment) -> Self {
        if !self.sql.is_empty() && !other.sql.is_empty() {
            self.sql = format!("({}) AND ({})", self.sql, other.sql);
        } else if !other.sql.is_empty() {
            self.sql = other.sql;
        }
        self.params.extend(other.params);
        self
    }

    /// Combines with another fragment using OR.
    pub fn or(mut self, other: SqlFragment) -> Self {
        if !self.sql.is_empty() && !other.sql.is_empty() {
            self.sql = format!("({}) OR ({})", self.sql, other.sql);
        } else if !other.sql.is_empty() {
            self.sql = other.sql;
        }
        self.params.extend(other.params);
        self
    }

    /// Returns true if this fragment is empty.
    pub fn is_empty(&self) -> bool {
        self.sql.is_empty()
    }
}

/// Builds SQL queries from FHIR search parameters.
pub struct QueryBuilder {
    /// The tenant ID for the query.
    tenant_id: String,
    /// The resource type being searched.
    resource_type: String,
    /// Base parameter offset for parameter placeholders.
    ///
    /// When the subquery is embedded in an outer query that already uses
    /// params ?1-?N, set this to N so search params start at ?(N+1).
    param_offset: usize,
    /// Whether to skip tenant/resource type params (they're shared with outer query).
    skip_base_params: bool,
}

impl QueryBuilder {
    /// Creates a new query builder.
    pub fn new(tenant_id: impl Into<String>, resource_type: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            resource_type: resource_type.into(),
            param_offset: 0,
            skip_base_params: false,
        }
    }

    /// Sets the parameter offset for embedded subqueries.
    ///
    /// When the generated SQL will be embedded in an outer query that already
    /// uses params ?1, ?2, etc., set this offset so the subquery's search
    /// params don't conflict.
    ///
    /// The offset should be the total number of params used by the outer query
    /// BEFORE the subquery. For example:
    /// - Outer query uses ?1 (tenant) and ?2 (type): offset = 2
    /// - Outer query uses ?1-?4 for cursor pagination: offset = 4
    ///
    /// Note: The subquery still references ?1 and ?2 for tenant/resource type
    /// since those bind to the same values as the outer query.
    pub fn with_param_offset(mut self, offset: usize) -> Self {
        self.param_offset = offset;
        self.skip_base_params = true;
        self
    }

    /// Builds a complete search query.
    ///
    /// Returns SQL that selects matching resource IDs from the search_index table.
    pub fn build(&self, query: &SearchQuery) -> SqlFragment {
        let mut conditions = Vec::new();

        // Base conditions: tenant and resource type
        // These always use ?1 and ?2 since they're shared with the outer query
        let mut base = SqlFragment::new(
            "SELECT DISTINCT resource_id FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2",
        );

        // Only include base params if not skipping (i.e., not embedded in outer query)
        if !self.skip_base_params {
            base.params.push(SqlParam::string(&self.tenant_id));
            base.params.push(SqlParam::string(&self.resource_type));
        }

        // Calculate the starting offset for search params
        // If embedded, use the provided offset; otherwise, start after base params
        let search_param_offset = if self.skip_base_params {
            self.param_offset
        } else {
            2 // After ?1 (tenant) and ?2 (resource_type)
        };

        // Build conditions for each parameter, tracking how many params we've added
        let mut current_offset = search_param_offset;
        for param in &query.parameters {
            if let Some(condition) = self.build_parameter_condition(param, current_offset) {
                current_offset += condition.params.len();
                conditions.push(condition);
            }
        }

        // Combine all conditions with AND
        if !conditions.is_empty() {
            let mut combined = conditions.remove(0);
            for cond in conditions {
                combined = combined.and(cond);
            }

            base.sql = format!("{} AND ({})", base.sql, combined.sql);
            base.params.extend(combined.params);
        }

        base
    }

    /// Builds a condition for a single search parameter.
    fn build_parameter_condition(
        &self,
        param: &SearchParameter,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        // Handle special parameters
        if param.name.starts_with('_') {
            return self.build_special_parameter_condition(param, param_offset);
        }

        // Composite parameters need a group-aware subquery (see below).
        if matches!(param.param_type, SearchParamType::Composite) {
            return self.build_composite_parameter_condition(param, param_offset);
        }

        // Multiple values are ORed together
        let mut or_conditions = Vec::new();
        let mut total_params = 0usize;

        for value in &param.values {
            let condition = self.build_value_condition(param, value, param_offset + total_params);
            if let Some(cond) = condition {
                total_params += cond.params.len();
                or_conditions.push(cond);
            }
        }

        if or_conditions.is_empty() {
            return None;
        }

        // Combine with OR
        let mut combined = or_conditions.remove(0);
        for cond in or_conditions {
            combined = combined.or(cond);
        }

        // Wrap in subquery to ensure proper AND/OR semantics
        Some(SqlFragment::with_params(
            format!(
                "resource_id IN (SELECT resource_id FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}' AND ({}))",
                param.name, combined.sql
            ),
            combined.params,
        ))
    }

    /// Builds a condition for a composite parameter.
    ///
    /// Each composite instance is indexed as a set of `search_index` rows that
    /// share a `composite_group`. A resource matches when there is a group in
    /// which every component is satisfied by some row, expressed as
    /// `GROUP BY resource_id, composite_group HAVING <every component present>`.
    fn build_composite_parameter_condition(
        &self,
        param: &SearchParameter,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if param.components.is_empty() {
            return None;
        }

        let mut or_conditions = Vec::new();
        let mut params = Vec::new();
        let mut total_params = 0usize;

        for value in &param.values {
            match CompositeHandler::build_component_fragments(
                value,
                &param.components,
                param_offset + total_params,
            ) {
                Some(fragments) if !fragments.is_empty() => {
                    let havings: Vec<String> = fragments
                        .iter()
                        .map(|f| format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", f.sql))
                        .collect();
                    for f in fragments {
                        total_params += f.params.len();
                        params.extend(f.params);
                    }
                    or_conditions.push(format!(
                        "resource_id IN (SELECT resource_id FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}' GROUP BY resource_id, composite_group HAVING {})",
                        param.name,
                        havings.join(" AND ")
                    ));
                }
                _ => or_conditions.push("0 = 1".to_string()),
            }
        }

        if or_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(or_conditions.join(" OR "), params))
    }

    /// Builds a condition for a special parameter (_id, _lastUpdated, etc.).
    fn build_special_parameter_condition(
        &self,
        param: &SearchParameter,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        match param.name.as_str() {
            "_id" => {
                // _id searches directly on the resources table
                let mut conditions = Vec::new();
                for (i, value) in param.values.iter().enumerate() {
                    conditions.push(SqlFragment::with_params(
                        format!("id = ?{}", param_offset + i + 1),
                        vec![SqlParam::string(&value.value)],
                    ));
                }

                if conditions.is_empty() {
                    return None;
                }

                let mut combined = conditions.remove(0);
                for cond in conditions {
                    combined = combined.or(cond);
                }

                Some(SqlFragment::with_params(
                    format!(
                        "resource_id IN (SELECT id FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND ({}))",
                        combined.sql
                    ),
                    combined.params,
                ))
            }
            "_lastUpdated" => {
                // _lastUpdated is stored in the resources table
                self.build_date_conditions_on_resources(&param.values, param_offset)
            }
            "_text" => {
                // _text searches the narrative text (text.div) via FTS5
                self.build_fts_condition(&param.values, "narrative_text", param_offset)
            }
            "_content" => {
                // _content searches all text content via FTS5
                self.build_fts_condition(&param.values, "full_content", param_offset)
            }
            "_filter" => {
                // _filter uses advanced filter expression syntax
                self.build_filter_condition(&param.values, param_offset)
            }
            _ => {
                // Other special parameters - fall through to regular handling
                None
            }
        }
    }

    /// Builds FTS5 conditions for _text and _content searches.
    fn build_fts_condition(
        &self,
        values: &[SearchValue],
        column: &str,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        use super::fts::Fts5Search;

        let mut conditions = Vec::new();

        for (i, value) in values.iter().enumerate() {
            // Escape and prepare the search term
            let search_term = Fts5Search::escape_fts_query(&value.value);
            if search_term.is_empty() {
                continue;
            }

            // Build the FTS match query
            // Use the column prefix to search only the specified column
            let param_num = param_offset + i + 1;
            conditions.push(SqlFragment::with_params(
                format!(
                    "resource_id IN (SELECT resource_id FROM resource_fts WHERE {} MATCH ?{})",
                    column, param_num
                ),
                vec![SqlParam::string(&search_term)],
            ));
        }

        if conditions.is_empty() {
            return None;
        }

        // OR together multiple search terms
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }

        Some(combined)
    }

    /// Builds date conditions for the resources table (for _lastUpdated).
    fn build_date_conditions_on_resources(
        &self,
        values: &[SearchValue],
        param_offset: usize,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();

        for (i, value) in values.iter().enumerate() {
            let cond = DateHandler::build_sql(value, param_offset + i);
            if !cond.is_empty() {
                conditions.push(cond);
            }
        }

        if conditions.is_empty() {
            return None;
        }

        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }

        Some(SqlFragment::with_params(
            format!(
                "resource_id IN (SELECT id FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND ({}))",
                combined.sql.replace("value_date", "last_updated")
            ),
            combined.params,
        ))
    }

    /// Builds conditions for _filter parameter.
    ///
    /// The _filter parameter allows complex filter expressions using a
    /// syntax similar to FHIRPath. See <https://build.fhir.org/search_filter.html>.
    ///
    /// # Examples
    ///
    /// ```text
    /// _filter=name eq "Smith"
    /// _filter=name eq "Smith" and birthdate gt 1980-01-01
    /// _filter=(status eq active or status eq pending) and category eq urgent
    /// ```
    fn build_filter_condition(
        &self,
        values: &[SearchValue],
        param_offset: usize,
    ) -> Option<SqlFragment> {
        use super::filter_parser::{FilterParser, FilterSqlGenerator};

        if values.is_empty() {
            return None;
        }

        let mut conditions = Vec::new();
        let mut current_offset = param_offset;

        for value in values {
            // Parse the filter expression
            match FilterParser::parse(&value.value) {
                Ok(expr) => {
                    // Generate SQL from the parsed expression
                    let mut generator = FilterSqlGenerator::new(current_offset);
                    let sql = generator.generate(&expr);
                    current_offset += sql.params.len();
                    conditions.push(sql);
                }
                Err(e) => {
                    // Log parse error but continue with other filters
                    tracing::warn!(
                        "Failed to parse _filter expression '{}': {}",
                        value.value,
                        e
                    );
                }
            }
        }

        if conditions.is_empty() {
            return None;
        }

        // AND together multiple _filter values
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }

        Some(combined)
    }

    /// Builds a condition for a single value.
    fn build_value_condition(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        // Handle :missing modifier
        if let Some(SearchModifier::Missing) = &param.modifier {
            return self.build_missing_condition(param, value);
        }

        // Build condition based on parameter type
        let fragment = match param.param_type {
            SearchParamType::String => {
                StringHandler::build_sql(value, param.modifier.as_ref(), param_offset)
            }
            SearchParamType::Token => {
                TokenHandler::build_sql(value, param.modifier.as_ref(), param_offset)
            }
            SearchParamType::Date => DateHandler::build_sql(value, param_offset),
            SearchParamType::Number => NumberHandler::build_sql(value, param_offset),
            SearchParamType::Quantity => QuantityHandler::build_sql(value, param_offset),
            SearchParamType::Reference => {
                ReferenceHandler::build_sql(value, param.modifier.as_ref(), param_offset)
            }
            SearchParamType::Uri => {
                UriHandler::build_sql(value, param.modifier.as_ref(), param_offset)
            }
            SearchParamType::Composite => {
                // Composite parameters require component definitions
                if param.components.is_empty() {
                    // No components defined, cannot process
                    return None;
                }
                CompositeHandler::build_composite_sql(
                    value,
                    &param.name,
                    &param.components,
                    param_offset,
                )
            }
            SearchParamType::Special => {
                // Should have been handled by build_special_parameter_condition
                return None;
            }
        };

        if fragment.is_empty() {
            None
        } else {
            Some(fragment)
        }
    }

    /// Builds a condition for the :missing modifier.
    fn build_missing_condition(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> Option<SqlFragment> {
        let is_missing = value.value.to_lowercase() == "true";

        if is_missing {
            // Missing = true: resources with NO index entry for this param
            Some(SqlFragment::new(format!(
                "resource_id NOT IN (SELECT resource_id FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}')",
                param.name
            )))
        } else {
            // Missing = false: resources WITH an index entry for this param
            Some(SqlFragment::new(format!(
                "resource_id IN (SELECT resource_id FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}')",
                param.name
            )))
        }
    }

    /// Builds an ORDER BY clause.
    ///
    /// Supports multiple sort directives (e.g., `_sort=name,-birthdate`).
    /// Each directive is processed in order, with a tie-breaker (`id ASC`) added
    /// at the end for stable pagination.
    ///
    /// # Supported Sort Parameters
    ///
    /// - `_id`: Sorts by resource logical ID
    /// - `_lastUpdated`: Sorts by last modification timestamp
    ///
    /// Other sort parameters are currently mapped to resource ID as a fallback.
    /// Full support for arbitrary search parameters would require additional
    /// SQL joins with the search_index table.
    pub fn build_order_by(&self, query: &SearchQuery) -> String {
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
                format!("{} {}", self.sort_expression(s), dir)
            })
            .collect();

        // Add tie-breaker for stable pagination if not already sorting by id
        let sorts_by_id = query.sort.iter().any(|s| s.parameter == "_id");
        if !sorts_by_id {
            clauses.push("id ASC".to_string());
        }

        format!("ORDER BY {}", clauses.join(", "))
    }

    /// Returns the keyset sort key for cursor pagination, or `None` when the
    /// query has multiple sort fields (those are returned as a single page
    /// rather than paged with a possibly-inconsistent keyset).
    pub fn primary_keyset_key(&self, query: &SearchQuery) -> Option<KeysetKey> {
        match query.sort.len() {
            0 => Some(KeysetKey {
                expr: "last_updated".to_string(),
                direction: crate::types::SortDirection::Descending,
                kind: SortValueKind::Timestamp,
            }),
            1 => {
                let directive = &query.sort[0];
                Some(KeysetKey {
                    expr: self.sort_expression(directive),
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
    fn sort_expression(&self, directive: &crate::types::SortDirective) -> String {
        match directive.parameter.as_str() {
            "_id" => return "id".to_string(),
            "_lastUpdated" => return "last_updated".to_string(),
            _ => {}
        }

        let column = directive.param_type.and_then(sort_value_column);
        match column {
            Some(col) => {
                let agg = match directive.direction {
                    crate::types::SortDirection::Ascending => "MIN",
                    crate::types::SortDirection::Descending => "MAX",
                };
                format!(
                    "(SELECT {}({}) FROM search_index si WHERE si.tenant_id = ?1 AND si.resource_type = ?2 AND si.resource_id = resources.id AND si.param_name = '{}')",
                    agg, col, directive.parameter
                )
            }
            // Unsortable (composite/special/unresolved) — stable fallback.
            None => "id".to_string(),
        }
    }

    /// Builds a LIMIT clause.
    pub fn build_limit(&self, query: &SearchQuery) -> String {
        let count = query.count.unwrap_or(100);
        if let Some(offset) = query.offset {
            format!("LIMIT {} OFFSET {}", count + 1, offset)
        } else {
            format!("LIMIT {}", count + 1)
        }
    }

    /// Returns the set of parameter names used in a query.
    pub fn get_used_params(query: &SearchQuery) -> HashSet<String> {
        let mut params = HashSet::new();
        for param in &query.parameters {
            params.insert(param.name.clone());
        }
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_fragment() {
        let mut frag = SqlFragment::new("value_string = ?1");
        frag.params.push(SqlParam::string("test"));

        assert!(!frag.is_empty());
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_fragment_and() {
        let frag1 = SqlFragment::with_params("a = ?1", vec![SqlParam::string("x")]);
        let frag2 = SqlFragment::with_params("b = ?2", vec![SqlParam::string("y")]);

        let combined = frag1.and(frag2);
        assert!(combined.sql.contains("AND"));
        assert_eq!(combined.params.len(), 2);
    }

    #[test]
    fn test_fragment_or() {
        let frag1 = SqlFragment::with_params("a = ?1", vec![SqlParam::string("x")]);
        let frag2 = SqlFragment::with_params("b = ?2", vec![SqlParam::string("y")]);

        let combined = frag1.or(frag2);
        assert!(combined.sql.contains("OR"));
    }

    #[test]
    fn test_query_builder_basic() {
        let builder = QueryBuilder::new("tenant1", "Patient");

        let query = SearchQuery::new("Patient");
        let fragment = builder.build(&query);

        assert!(fragment.sql.contains("search_index"));
        assert!(fragment.sql.contains("tenant_id"));
        assert!(fragment.sql.contains("resource_type"));
    }

    #[test]
    fn test_query_builder_with_param() {
        let builder = QueryBuilder::new("tenant1", "Patient");

        let mut query = SearchQuery::new("Patient");
        query.parameters.push(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("smith")],
            chain: vec![],
            components: vec![],
        });

        let fragment = builder.build(&query);

        assert!(fragment.sql.contains("param_name = 'name'"));
    }

    #[test]
    fn test_order_by_default() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let query = SearchQuery::new("Patient");

        let order_by = builder.build_order_by(&query);
        assert!(order_by.contains("last_updated DESC"));
        assert!(order_by.contains("id ASC")); // Tie-breaker for stable pagination
    }

    #[test]
    fn test_order_by_multiple_fields() {
        use crate::types::{SortDirection, SortDirective};

        let builder = QueryBuilder::new("tenant1", "Patient");
        let mut query = SearchQuery::new("Patient");
        query.sort = vec![
            SortDirective {
                parameter: "_lastUpdated".to_string(),
                direction: SortDirection::Descending,
                param_type: None,
            },
            SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            },
        ];

        let order_by = builder.build_order_by(&query);
        assert_eq!(order_by, "ORDER BY last_updated DESC, id ASC");
    }

    #[test]
    fn test_order_by_adds_tiebreaker() {
        use crate::types::{SortDirection, SortDirective};

        let builder = QueryBuilder::new("tenant1", "Patient");
        let mut query = SearchQuery::new("Patient");
        query.sort = vec![SortDirective {
            parameter: "_lastUpdated".to_string(),
            direction: SortDirection::Ascending,
            param_type: None,
        }];

        let order_by = builder.build_order_by(&query);
        // Should have id ASC as tie-breaker since _id is not in sort list
        assert_eq!(order_by, "ORDER BY last_updated ASC, id ASC");
    }

    #[test]
    fn test_limit_with_offset() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let mut query = SearchQuery::new("Patient");
        query.count = Some(10);
        query.offset = Some(20);

        let limit = builder.build_limit(&query);
        assert!(limit.contains("LIMIT 11"));
        assert!(limit.contains("OFFSET 20"));
    }

    #[test]
    fn test_reference_search_id_only() {
        // Test that ID-only reference search generates correct param numbers
        let builder = QueryBuilder::new("default", "Immunization");

        let mut query = SearchQuery::new("Immunization");
        query.parameters.push(SearchParameter {
            name: "patient".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("us-core-client-tests-patient")],
            chain: vec![],
            components: vec![],
        });

        let fragment = builder.build(&query);

        // ID-only reference search is version-agnostic: exact, `%/id`, and
        // `%/id/_history/%`, so three params (?3, ?4, ?5) after ?1 tenant / ?2
        // resource_type.
        assert!(fragment.sql.contains("?3"));
        assert!(fragment.sql.contains("?4"));
        assert!(fragment.sql.contains("?5"));
        // tenant, resource_type, + 3 ref_value bindings
        assert_eq!(fragment.params.len(), 5);
    }

    #[test]
    fn test_multiple_reference_values_correct_offsets() {
        // Test that multiple values get correct param offsets
        let builder = QueryBuilder::new("default", "Immunization");

        let mut query = SearchQuery::new("Immunization");
        query.parameters.push(SearchParameter {
            name: "patient".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("patient-1"), SearchValue::eq("patient-2")],
            chain: vec![],
            components: vec![],
        });

        let fragment = builder.build(&query);

        // Each ID-only value now binds 3 params (exact, `%/id`, `%/id/_history/%`):
        // first value ?3..?5, second value ?6..?8.
        for p in ["?3", "?4", "?5", "?6", "?7", "?8"] {
            assert!(fragment.sql.contains(p), "missing placeholder {p}");
        }
        // tenant, resource_type, + 3 per ID-only ref × 2 values
        assert_eq!(fragment.params.len(), 8);
    }
}
