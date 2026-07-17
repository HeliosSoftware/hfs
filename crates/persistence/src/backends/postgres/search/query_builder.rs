//! PostgreSQL search query builder.
//!
//! Builds SQL queries for FHIR search operations using PostgreSQL syntax
//! with $N parameter placeholders, ILIKE for case-insensitive matching,
//! and native TIMESTAMPTZ comparisons.

use chrono::{DateTime, Utc};

use crate::search::fold_text;
use crate::types::{
    CompartmentMembership, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, strip_reference_version,
};

/// Returns the implicit precision of a decimal search value from its string form
/// (e.g. `"100"` → 1.0, `"100.0"` → 0.1), used to build `eq` ranges.
fn quantity_implicit_precision(num_str: &str) -> f64 {
    crate::search::implicit_precision(num_str)
}

/// Escapes LIKE metacharacters in a user-supplied search value.
///
/// Without this, `?name=100%` is a wildcard rather than a literal — the value is
/// interpolated straight into the pattern. Paired with `ESCAPE '\'` at the call
/// site. (`chain_builder.rs` already does this; the main builder did not.)
fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The SQL expression a string search matches against.
///
/// `value_string_folded` (case- and accent-folded, added in schema v10) is
/// populated on write but **never backfilled**, so rows written before the v10
/// upgrade have NULL there and must fall back to the raw column. `lower()` on that
/// fallback reproduces exactly what the previous `ILIKE` did — both sides of the
/// comparison end up lowercased — while keeping the whole expression indexable.
/// Plain `ILIKE` can never use a btree; this expression, matched by
/// `idx_search_string_folded_pattern` under `text_pattern_ops`, can.
///
/// Must stay character-for-character identical to the index definition in
/// `schema.rs::migrate_v13_to_v14`, or Postgres will not use the index.
const FOLDED_STRING_EXPR: &str = "COALESCE(value_string_folded, lower(value_string))";

/// Builds the numeric comparison SQL for `col` against the implicit-precision
/// range `[lo, hi)` per the FHIR prefix semantics, advancing `next` and
/// returning the SQL plus its bound params. `num` is used only for the `ap`
/// margin.
fn numeric_predicate(
    col: &str,
    prefix: SearchPrefix,
    num: f64,
    lo: f64,
    hi: f64,
    next: &mut usize,
) -> (String, Vec<SqlParam>) {
    match prefix {
        SearchPrefix::Eq => {
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("{col} >= ${a} AND {col} < ${next}"),
                vec![SqlParam::Float(lo), SqlParam::Float(hi)],
            )
        }
        SearchPrefix::Ne => {
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("({col} < ${a} OR {col} >= ${next})"),
                vec![SqlParam::Float(lo), SqlParam::Float(hi)],
            )
        }
        SearchPrefix::Gt | SearchPrefix::Sa => {
            *next += 1;
            (format!("{col} >= ${next}"), vec![SqlParam::Float(hi)])
        }
        SearchPrefix::Lt | SearchPrefix::Eb => {
            *next += 1;
            (format!("{col} < ${next}"), vec![SqlParam::Float(lo)])
        }
        SearchPrefix::Ge => {
            *next += 1;
            (format!("{col} >= ${next}"), vec![SqlParam::Float(lo)])
        }
        SearchPrefix::Le => {
            *next += 1;
            (format!("{col} < ${next}"), vec![SqlParam::Float(hi)])
        }
        SearchPrefix::Ap => {
            let margin = (num.abs() * 0.1).max(0.0001);
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("{col} BETWEEN ${a} AND ${next}"),
                vec![SqlParam::Float(num - margin), SqlParam::Float(num + margin)],
            )
        }
    }
}

/// Returns the `[start, end)` timestamp range for a date search value at its
/// inherent precision (year/month/day). Full-precision instants return a
/// degenerate range (`start == end`).
fn date_precision_range(value: &str) -> (DateTime<Utc>, DateTime<Utc>) {
    use chrono::Datelike;
    let start = PostgresQueryBuilder::parse_date_value(value);
    let end = if value.contains('T') {
        start
    } else if value.len() == 4 {
        start.with_year(start.year() + 1).unwrap_or(start)
    } else if value.len() == 7 {
        let (y, m) = (start.year(), start.month());
        let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        start
            .with_month(1)
            .and_then(|d| d.with_year(ny))
            .and_then(|d| d.with_month(nm))
            .unwrap_or(start)
    } else if value.len() == 10 {
        start + chrono::Duration::days(1)
    } else {
        start
    };
    (start, end)
}

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

        // Compartment membership: match resources that reference the compartment
        // through ANY of the membership params (OR), per the CompartmentDefinition.
        if let Some(comp) = &query.compartment {
            // Last condition appended, so no need to advance `current_offset`.
            if let Some(condition) = Self::build_compartment_condition(comp, current_offset) {
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

    /// Builds the compartment-membership subquery: matches resources that
    /// reference `comp.reference` via ANY of `comp.params` (logical OR), mirroring
    /// the SQLite backend. Returns `None` when there are no params or reference.
    fn build_compartment_condition(
        comp: &CompartmentMembership,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return None;
        }

        // Membership params come from the bundled FHIR CompartmentDefinitions
        // (trusted); escape single quotes defensively before inlining.
        let in_list = comp
            .params
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");

        let base = strip_reference_version(&comp.reference).to_string();
        let p1 = param_offset + 1;
        let p2 = param_offset + 2;

        Some(SqlFragment::with_params(
            format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name IN ({in_list}) \
                 AND (value_reference = ${p1} OR value_reference LIKE ${p2} || '/_history/%'))"
            ),
            vec![SqlParam::text(&base), SqlParam::text(&base)],
        ))
    }

    /// Builds the `_contained` match subquery (mirrors the SQLite backend's
    /// `build_contained`).
    ///
    /// Returns SQL selecting `(resource_type, resource_id, contained_local_id)`
    /// from `search_index` for contained resources (`is_contained = TRUE`) of the
    /// searched type (`contained_type = $2`) matching every standard parameter,
    /// keyed on the contained entity `(resource_id, contained_local_id)` via
    /// `GROUP BY ... HAVING COUNT(DISTINCT param_name) >= n`. Value predicates are
    /// the bare column conditions shared with composite-component matching.
    ///
    /// Param layout: `$1` = tenant, `$2` = contained type, then value params.
    /// Returns `None` when no standard parameter contributes a condition
    /// (special `_`-params and composites are not applied to contained matching).
    pub fn build_contained(query: &SearchQuery) -> Option<SqlFragment> {
        let mut branches: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        let mut distinct_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // $1 = tenant, $2 = contained_type are bound by the caller.
        let mut offset = 2;

        for param in &query.parameters {
            if param.name.starts_with('_')
                || matches!(
                    param.param_type,
                    SearchParamType::Composite | SearchParamType::Special
                )
            {
                continue;
            }

            let mut or_parts: Vec<String> = Vec::new();
            for value in &param.values {
                let predicate = match param.param_type {
                    SearchParamType::Token
                    | SearchParamType::String
                    | SearchParamType::Number
                    | SearchParamType::Quantity
                    | SearchParamType::Date => {
                        Self::build_composite_component(value, param.param_type, offset)
                    }
                    SearchParamType::Reference => Some((
                        format!("value_reference = ${}", offset + 1),
                        vec![SqlParam::text(strip_reference_version(&value.value))],
                    )),
                    SearchParamType::Uri => Some((
                        format!("value_uri = ${}", offset + 1),
                        vec![SqlParam::text(&value.value)],
                    )),
                    SearchParamType::Composite | SearchParamType::Special => None,
                };
                if let Some((sql, ps)) = predicate {
                    offset += ps.len();
                    or_parts.push(sql);
                    params.extend(ps);
                }
            }
            if or_parts.is_empty() {
                continue;
            }
            branches.push(format!(
                "(param_name = '{}' AND ({}))",
                param.name,
                or_parts.join(" OR ")
            ));
            distinct_names.insert(param.name.clone());
        }

        if branches.is_empty() {
            return None;
        }
        let sql = format!(
            "SELECT resource_type, resource_id, contained_local_id FROM search_index \
             WHERE tenant_id = $1 AND is_contained = TRUE AND contained_type = $2 AND ({}) \
             GROUP BY resource_type, resource_id, contained_local_id \
             HAVING COUNT(DISTINCT param_name) >= {}",
            branches.join(" OR "),
            distinct_names.len()
        );
        Some(SqlFragment::with_params(sql, params))
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
                // `:text` on a string is a case-insensitive partial match,
                // implemented here as a substring match (same as `:contains`).
                // Match the accent-folded column (falling back to the raw column
                // for not-yet-reindexed rows) against a folded pattern.
                //
                // A leading `%` is not btree-sargable; this stays a scan unless the
                // optional pg_trgm index is present. It is at least now scoped to
                // one parameter's slice of the index rather than the whole table.
                Some(SearchModifier::Contains | SearchModifier::Text) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {} LIKE ${} ESCAPE '\\')",
                        param.name, FOLDED_STRING_EXPR, param_num
                    ),
                    vec![SqlParam::text(&format!(
                        "%{}%",
                        like_escape(&fold_text(&value.value))
                    ))],
                ),
                _ => {
                    // Default: starts-with (case- and accent-insensitive).
                    //
                    // `LIKE`, not `ILIKE`: `fold_text` already lowercases both the
                    // stored column and the pattern, so `ILIKE` was doing no work
                    // that `LIKE` doesn't — but it made the predicate unindexable.
                    SqlFragment::with_params(
                        format!(
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {} LIKE ${} ESCAPE '\\')",
                            param.name, FOLDED_STRING_EXPR, param_num
                        ),
                        vec![SqlParam::text(&format!(
                            "{}%",
                            like_escape(&fold_text(&value.value))
                        ))],
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

        // `:text` (contains) and `:code-text` (starts-with) match the token's
        // display text (Coding.display / CodeableConcept.text).
        if matches!(
            param.modifier,
            Some(SearchModifier::Text | SearchModifier::CodeText)
        ) {
            let starts_with = matches!(param.modifier, Some(SearchModifier::CodeText));
            let mut conditions = Vec::new();
            for (i, value) in param.values.iter().enumerate() {
                let param_num = offset + i + 1;
                let pattern = if starts_with {
                    format!("{}%", value.value)
                } else {
                    format!("%{}%", value.value)
                };
                conditions.push(SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_token_display ILIKE ${})",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&pattern)],
                ));
            }
            if conditions.is_empty() {
                return None;
            }
            let mut combined = conditions.remove(0);
            for cond in conditions {
                combined = combined.or(cond);
            }
            return Some(combined);
        }

        // An OR-list (`status=finished,in-progress`) becomes ONE subquery with the
        // values OR'd inside it, rather than one `id IN (…)` per value OR'd together.
        //
        // This is a plan fix, not cosmetics. PostgreSQL only pulls a sublink up into
        // a semi-join when it sits in the top-level AND-list of the WHERE clause.
        // Sublinks under an OR stay `SubPlan`s — hashed if the planner's estimate
        // fits in `work_mem`, and otherwise **re-executed for every outer row**. At
        // 1.3M index rows that mis-estimate is easy to hit, and it is the second
        // timeout generator after the composite aggregate. With the OR inside there
        // is a single sublink, so it becomes a semi-join the planner can drive from
        // an index scan.
        //
        // Each value binds a variable number of params: `system|code` binds 2, every
        // other form (code-only, `|code`, `system|`) binds 1. Placeholder numbers
        // must be sequential and gap-free because the caller advances the global
        // offset by `params.len()`; a fixed 2-slot stride once produced a placeholder
        // past the end of the bound params, which Postgres rejects.
        let mut predicates: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        let mut next = offset;

        for value in param.values.iter() {
            if let Some((system, code)) = value.value.split_once('|') {
                if system.is_empty() {
                    // |code - match any system
                    next += 1;
                    predicates.push(format!("value_token_code = ${}", next));
                    params.push(SqlParam::text(code));
                } else if code.is_empty() {
                    // system| - match any code in system
                    next += 1;
                    predicates.push(format!("value_token_system = ${}", next));
                    params.push(SqlParam::text(system));
                } else {
                    // system|code - exact match
                    let s = next + 1;
                    let c = next + 2;
                    next += 2;
                    predicates.push(format!(
                        "(value_token_system = ${} AND value_token_code = ${})",
                        s, c
                    ));
                    params.push(SqlParam::text(system));
                    params.push(SqlParam::text(code));
                }
            } else {
                // code only - match any system
                next += 1;
                predicates.push(format!("value_token_code = ${}", next));
                params.push(SqlParam::text(&value.value));
            }
        }

        if predicates.is_empty() {
            return None;
        }

        let mut combined = SqlFragment::with_params(
            format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND ({}))",
                param.name,
                predicates.join(" OR ")
            ),
            params,
        );

        // `:not` reverses the match: include resources that have no matching
        // value (including those with no value at all). Negating the single sublink
        // is equivalent to negating the OR-set of sublinks it replaced.
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

            // Stage this value's params locally and commit them only once every
            // component has parsed. A component that fails midway used to leave the
            // params of its predecessors bound while emitting `1 = 0`, so those
            // params had no placeholder in the final statement and Postgres rejected
            // the whole query ("bind message supplies N parameters, but prepared
            // statement requires M"). e.g. `?code-value-quantity=8867-4$abc` → 500.
            let mut staged_params: Vec<SqlParam> = Vec::new();
            let mut predicates: Vec<String> = Vec::new();
            let mut next = current;
            let mut ok = true;

            for (part, component) in parts.iter().zip(param.components.iter()) {
                let cv = Self::parse_component_value(part);
                match Self::build_composite_component(&cv, component.param_type, next) {
                    Some((sql, params)) => {
                        next += params.len();
                        staged_params.extend(params);
                        predicates.push(sql);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }

            if !ok || predicates.is_empty() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            current = next;
            all_params.extend(staged_params);

            // Each component of a composite instance is a separate `search_index`
            // row, so a resource matches only when every component is satisfied by
            // some row *within the same* `composite_group`. That grouping is what
            // stops a blood-pressure panel's systolic value from pairing with its
            // diastolic code, so the GROUP BY / HAVING structure is load-bearing and
            // is kept exactly as-is.
            //
            // What is new is the WHERE prefilter. Previously the subquery had no
            // value predicate at all: it read every composite row for the parameter
            // and only then filtered inside the aggregate, which meant a full
            // aggregation over the resource type's entire composite slice on every
            // request — the `combo-code-value-quantity` timeout.
            //
            // Dropping rows that satisfy *no* component is provably result-
            // preserving: such a row contributes 0 to every `MAX(CASE …)` in the
            // HAVING, so it can never turn a 0 into a 1, and a group left empty by
            // the prefilter could not have satisfied the HAVING anyway.
            //
            // Measured on the real benchmark dataset (656,737 Observations,
            // 1.8M `code-value-quantity` index rows), which is what these numbers
            // refer to — an earlier, smaller replica gave badly misleading answers
            // here and every alternative below was ranked wrong by it.
            //
            // Three alternatives were tried and REJECTED against real data:
            //
            //   • Dropping the prefilter entirely (the pre-#224 form) and reading the
            //     whole parameter slice index-only from a covering index: 16.5 s. The
            //     prefilter is load-bearing, not incidental.
            //
            //   • A flat self-join of the components correlated on `composite_group`,
            //     driving from the token. Postgres flattens it, then drives from an
            //     ordered scan of `resources` and probes — see below for why that is
            //     fatal here.
            //
            //   • A correlated `EXISTS` (`s.resource_id = resources.id`, grouped per
            //     resource). Only 222 of 656,737 Observations match this composite, so
            //     an ordered scan that probes per row must walk ~190k resources to
            //     collect 21 hits: 8.9 s, and 20 s when nothing matches at all.
            //
            // The common thread: the composite is extremely *sparse*, so the plan must
            // build the small candidate set first and probe `resources` by primary key
            // — never walk `resources` and test each row. The grouped subquery below
            // cannot be flattened or parameterized, which forces exactly that shape.
            // Its unflattenability is a feature; do not "optimize" it into an EXISTS.
            //
            // What remains: this shape still costs ~1 s cold because the Bitmap Heap
            // Scan fetches ~113k rows in ~112k buffer reads — close to one random page
            // per row, since a parameter's rows are scattered across a ~10M-row table.
            // That is I/O, not CPU, and it is why the shape is still ~12 s at 30 VUs.
            // A covering index does not fix it (the OR-prefilter forces heap access by
            // construction). The real fix is a storage change: write one row per
            // composite group with every component's value column populated, so a
            // single index answers "code = X AND value > Y within one group" directly.
            // Tracked separately — it needs a writer change, a migration and a backfill.
            let havings = predicates
                .iter()
                .map(|p| format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", p))
                .collect::<Vec<_>>()
                .join(" AND ");
            // Parenthesize each component before OR-ing: a token component emits
            // `sys = $n AND code = $m`, and while SQL's AND-before-OR precedence
            // happens to group that correctly, relying on it is a trap for the next
            // component type someone adds.
            let prefilter = predicates
                .iter()
                .map(|p| format!("({})", p))
                .collect::<Vec<_>>()
                .join(" OR ");

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' \
                 AND ({}) \
                 GROUP BY resource_id, composite_group HAVING {})",
                param.name, prefilter, havings
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
        let mut next = offset;

        for value in &param.values {
            let (start, end) = date_precision_range(&value.value);
            // Comparators match against the precision-range boundaries; a
            // full-precision instant (degenerate range) falls back to scalar.
            let degenerate = start == end;
            let (sql, params): (String, Vec<SqlParam>) = match value.prefix {
                SearchPrefix::Eq if !degenerate => {
                    next += 1;
                    let a = next;
                    next += 1;
                    (
                        format!("value_date >= ${a} AND value_date < ${next}"),
                        vec![SqlParam::Timestamp(start), SqlParam::Timestamp(end)],
                    )
                }
                SearchPrefix::Ne if !degenerate => {
                    next += 1;
                    let a = next;
                    next += 1;
                    (
                        format!("(value_date < ${a} OR value_date >= ${next})"),
                        vec![SqlParam::Timestamp(start), SqlParam::Timestamp(end)],
                    )
                }
                SearchPrefix::Gt | SearchPrefix::Sa if !degenerate => {
                    next += 1;
                    (
                        format!("value_date >= ${next}"),
                        vec![SqlParam::Timestamp(end)],
                    )
                }
                SearchPrefix::Lt | SearchPrefix::Eb if !degenerate => {
                    next += 1;
                    (
                        format!("value_date < ${next}"),
                        vec![SqlParam::Timestamp(start)],
                    )
                }
                SearchPrefix::Ge if !degenerate => {
                    next += 1;
                    (
                        format!("value_date >= ${next}"),
                        vec![SqlParam::Timestamp(start)],
                    )
                }
                SearchPrefix::Le if !degenerate => {
                    next += 1;
                    (
                        format!("value_date < ${next}"),
                        vec![SqlParam::Timestamp(end)],
                    )
                }
                // Degenerate (full-precision) or unhandled: scalar comparison.
                other => {
                    let op = Self::prefix_to_operator(&other);
                    next += 1;
                    (
                        format!("value_date {op} ${next}"),
                        vec![SqlParam::Timestamp(start)],
                    )
                }
            };
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, sql
                ),
                params,
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
        let mut next = offset;

        for value in &param.values {
            let num: f64 = match value.value.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            let (lo, hi) = crate::search::implicit_range(num, &value.value);
            let (sql, params) =
                numeric_predicate("value_number", value.prefix, num, lo, hi, &mut next);
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, sql
                ),
                params,
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

    fn build_quantity_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        // Running placeholder counter: the outer builder advances by the
        // fragment's params.len(), so numbering must be sequential and gap-free.
        let mut next = offset;

        for value in &param.values {
            // Parse quantity: [prefix]number|system|code (or number|code, or number).
            let parts: Vec<&str> = value.value.splitn(3, '|').collect();
            let num: f64 = match parts.first().and_then(|s| s.parse::<f64>().ok()) {
                Some(n) => n,
                None => continue,
            };
            let num_str = parts[0];
            let (system, code) = match parts.len() {
                3 => (
                    (!parts[1].is_empty()).then_some(parts[1]),
                    (!parts[2].is_empty()).then_some(parts[2]),
                ),
                2 => (None, (!parts[1].is_empty()).then_some(parts[1])),
                _ => (None, None),
            };

            // Raw branch: range-boundary value comparison + the stored unit/system.
            let (lo, hi) = crate::search::implicit_range(num, num_str);
            let (raw_num, mut params) =
                numeric_predicate("value_quantity_value", value.prefix, num, lo, hi, &mut next);
            let mut raw = raw_num;
            if let Some(c) = code {
                next += 1;
                params.push(SqlParam::text(c));
                raw.push_str(&format!(" AND value_quantity_unit = ${next}"));
            }
            if let Some(s) = system {
                next += 1;
                params.push(SqlParam::text(s));
                raw.push_str(&format!(" AND value_quantity_system = ${next}"));
            }

            // Canonical branch (range-based on the canonical columns) so unit
            // equivalents match (g ⇄ mg). Bounds are canonicalized to preserve
            // range/precision and absorb float-conversion noise. Skipped for
            // `ne` and non-convertible units.
            let mut predicate = format!("({raw})");
            if let Some(c) = code {
                if !matches!(value.prefix, SearchPrefix::Ne) {
                    if let Some((_, cunit)) = helios_fhirpath::ucum::canonicalize_quantity(num, c) {
                        let canon = |x: f64| {
                            helios_fhirpath::ucum::canonicalize_quantity(x, c).map(|(v, _)| v)
                        };
                        let col = "value_quantity_canonical_value";
                        // Comparators match canonicalized range boundaries:
                        // gt/sa → ≥ canon(hi), lt/eb → < canon(lo),
                        // ge → ≥ canon(lo), le → < canon(hi).
                        let half = quantity_implicit_precision(num_str) / 2.0;
                        let range: Option<String> = match value.prefix {
                            SearchPrefix::Gt | SearchPrefix::Sa => canon(num + half).map(|b| {
                                next += 1;
                                params.push(SqlParam::Float(b));
                                format!("{col} >= ${next}")
                            }),
                            SearchPrefix::Lt | SearchPrefix::Eb => canon(num - half).map(|b| {
                                next += 1;
                                params.push(SqlParam::Float(b));
                                format!("{col} < ${next}")
                            }),
                            SearchPrefix::Ge => canon(num - half).map(|b| {
                                next += 1;
                                params.push(SqlParam::Float(b));
                                format!("{col} >= ${next}")
                            }),
                            SearchPrefix::Le => canon(num + half).map(|b| {
                                next += 1;
                                params.push(SqlParam::Float(b));
                                format!("{col} < ${next}")
                            }),
                            SearchPrefix::Ap => {
                                let margin = (num.abs() * 0.1).max(0.0001);
                                match (canon(num - margin), canon(num + margin)) {
                                    (Some(a), Some(b)) => {
                                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                        next += 1;
                                        params.push(SqlParam::Float(lo));
                                        let lo_p = next;
                                        next += 1;
                                        params.push(SqlParam::Float(hi));
                                        Some(format!("{col} BETWEEN ${lo_p} AND ${next}"))
                                    }
                                    _ => None,
                                }
                            }
                            // Eq + default: implicit-precision range.
                            _ => {
                                let half = quantity_implicit_precision(num_str) / 2.0;
                                match (canon(num - half), canon(num + half)) {
                                    (Some(a), Some(b)) => {
                                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                                        next += 1;
                                        params.push(SqlParam::Float(lo));
                                        let lo_p = next;
                                        next += 1;
                                        params.push(SqlParam::Float(hi));
                                        Some(format!("{col} >= ${lo_p} AND {col} < ${next}"))
                                    }
                                    _ => None,
                                }
                            }
                        };
                        if let Some(range) = range {
                            next += 1;
                            params.push(SqlParam::text(&cunit));
                            predicate = format!(
                                "(({raw}) OR ({range} AND value_quantity_canonical_unit = ${next}))"
                            );
                        }
                    }
                }
            }

            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, predicate
                ),
                params,
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

    /// Builds the `:identifier` condition: match references whose target
    /// resource has an identifier equal to the supplied `system|value`. Mirrors
    /// the SQLite implementation using PG's `SUBSTRING`/`POSITION`.
    fn build_reference_identifier_condition(
        param: &SearchParameter,
        offset: usize,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset; // running 0-based param offset

        for value in &param.values {
            // Correlate the target resource id (the part after '/') with an
            // 'identifier' index row for that resource.
            let target = "idx.resource_id = SUBSTRING(ref.value_reference FROM POSITION('/' IN ref.value_reference) + 1)";
            let (filter, params): (String, Vec<SqlParam>) = match value.value.split_once('|') {
                Some(("", code)) => {
                    next += 1;
                    (
                        format!(
                            "(idx.value_token_system IS NULL OR idx.value_token_system = '') AND idx.value_token_code = ${next}"
                        ),
                        vec![SqlParam::text(code)],
                    )
                }
                Some((system, "")) => {
                    next += 1;
                    (
                        format!("idx.value_token_system = ${next}"),
                        vec![SqlParam::text(system)],
                    )
                }
                Some((system, code)) => {
                    let s = next + 1;
                    let c = next + 2;
                    next += 2;
                    (
                        format!("idx.value_token_system = ${s} AND idx.value_token_code = ${c}"),
                        vec![SqlParam::text(system), SqlParam::text(code)],
                    )
                }
                None => {
                    next += 1;
                    (
                        format!("idx.value_token_code = ${next}"),
                        vec![SqlParam::text(&value.value)],
                    )
                }
            };
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT ref.resource_id FROM search_index ref \
                     WHERE ref.tenant_id = $1 AND ref.resource_type = $2 AND ref.param_name = '{}' \
                     AND EXISTS (SELECT 1 FROM search_index idx \
                       WHERE idx.tenant_id = $1 AND idx.param_name = 'identifier' \
                       AND {target} AND {filter}))",
                    param.name
                ),
                params,
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

    fn build_reference_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Identifier)) {
            return Self::build_reference_identifier_condition(param, offset);
        }

        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        // :contains - case-insensitive substring on the stored reference.
        // :text (contains) / :code-text (starts-with) match the indexed
        // Reference.display text.
        let modifier = param.modifier.as_ref();
        let is_contains = matches!(modifier, Some(SearchModifier::Contains));
        let is_text = matches!(modifier, Some(SearchModifier::Text));
        let is_code_text = matches!(modifier, Some(SearchModifier::CodeText));
        let is_below = matches!(modifier, Some(SearchModifier::Below));
        let is_above = matches!(modifier, Some(SearchModifier::Above));

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let predicate = if is_text {
                format!("value_reference_display ILIKE '%' || ${} || '%'", param_num)
            } else if is_code_text {
                format!("value_reference_display ILIKE ${} || '%'", param_num)
            } else if is_contains {
                format!("value_reference ILIKE '%' || ${} || '%'", param_num)
            } else if is_below {
                // URL/path-prefix hierarchy (canonical |version not handled).
                format!(
                    "(value_reference = ${0} OR value_reference LIKE ${0} || '/%')",
                    param_num
                )
            } else if is_above {
                format!(
                    "(${0} = value_reference OR ${0} LIKE value_reference || '/%')",
                    param_num
                )
            } else {
                // Plain reference match, version-agnostic: the bound value is
                // the version-stripped base; match it exactly or carrying any
                // `_history` version.
                format!(
                    "(value_reference = ${0} OR value_reference LIKE ${0} || '/_history/%')",
                    param_num
                )
            };
            // For the plain match the bound value is version-stripped; modifiers
            // keep the literal value.
            let bound = if is_text || is_code_text || is_contains || is_below || is_above {
                value.value.clone()
            } else {
                strip_reference_version(&value.value).to_string()
            };
            conditions.push(predicate);
            params.push(SqlParam::text(&bound));
        }

        if conditions.is_empty() {
            return None;
        }

        // One sublink with the values OR'd inside, not one sublink per value OR'd
        // together — see `build_token_condition` for why (an OR'd sublink is left as
        // a re-executable SubPlan instead of being pulled up into a semi-join). The
        // benchmark fires 2- and 3-id reference OR-lists, so this path is hot.
        Some(SqlFragment::with_params(
            format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND ({}))",
                param.name,
                conditions.join(" OR ")
            ),
            params,
        ))
    }

    fn build_uri_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let modifier = param.modifier.as_ref();
        let mut conditions = Vec::new();

        for (i, value) in param.values.iter().enumerate() {
            let param_num = offset + i + 1;
            let condition = match modifier {
                Some(SearchModifier::Contains) => SqlFragment::with_params(
                    format!(
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_uri ILIKE '%' || ${} || '%')",
                        param.name, param_num
                    ),
                    vec![SqlParam::text(&value.value)],
                ),
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
    fn token_or_list_placeholders_are_gap_free() {
        // Regression: `status=finished,in-progress` — two code-only token values,
        // each binding one param. With offset 2 the placeholders must be $3 and $4
        // with exactly 2 params bound. The old fixed 2-slot stride emitted $3 and
        // $5 while binding only 2 params, which Postgres rejected at execute time
        // ("Failed to execute search: db error").
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "finished"),
                SearchValue::new(SearchPrefix::Eq, "in-progress"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("token OR-list should produce a condition");

        assert_eq!(frag.params.len(), 2, "two code-only values bind two params");
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_token_code = $4"));
        assert!(
            !frag.sql.contains("$5"),
            "placeholder numbering must be gap-free: {}",
            frag.sql
        );
    }

    #[test]
    fn token_or_list_mixed_forms_placeholders_sequential() {
        // code-only (1 param) then system|code (2 params): placeholders must run
        // $3, $4, $5 with 3 params bound — no gaps across differing per-value arity.
        let param = SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "8302-2"),
                SearchValue::new(SearchPrefix::Eq, "http://loinc.org|29463-7"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("mixed token OR-list should produce a condition");

        assert_eq!(frag.params.len(), 3);
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_token_system = $4"));
        assert!(frag.sql.contains("value_token_code = $5"));
        assert!(!frag.sql.contains("$6"));
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
    fn composite_prefilters_but_keeps_group_semantics() {
        // The WHERE gains an OR-prefilter so the subquery stops reading every
        // composite row for the parameter, but the GROUP BY / HAVING must survive
        // intact — it is what forces both components into the SAME composite_group.
        let param = SearchParameter {
            name: "combo-code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$gt140",
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
        let frag =
            PostgresQueryBuilder::build_search_query(&query, 2).expect("composite condition");

        assert!(
            frag.sql
                .contains("GROUP BY resource_id, composite_group HAVING"),
            "composite_group grouping must be preserved — without it, components \
             match across different composite groups: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains("MAX(CASE WHEN"),
            "per-component HAVING must be preserved: {}",
            frag.sql
        );
        // The prefilter: components OR'd inside the WHERE.
        assert!(
            frag.sql.contains(") OR (value_quantity_value"),
            "expected an OR-prefilter over the components in the WHERE clause: {}",
            frag.sql
        );
    }

    #[test]
    fn composite_unparseable_component_binds_no_params() {
        // `8867-4$abc` — the quantity component fails to parse. The value must
        // collapse to `1 = 0` and contribute ZERO bound params. Previously the token
        // component's params were already pushed before the quantity component
        // bailed, leaving them bound with no placeholder to reference them, which
        // Postgres rejects outright ("bind message supplies N parameters...") — a
        // 500 on a malformed query rather than an empty result.
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "8867-4$abc")],
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
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("unparseable composite still yields a condition");

        assert_eq!(
            frag.params.len(),
            0,
            "a bailed-out composite value must leave no orphaned bind params: {}",
            frag.sql
        );
        assert!(frag.sql.contains("1 = 0"));
    }

    #[test]
    fn token_or_list_is_a_single_sublink() {
        // OR'd sublinks (`id IN (..) OR id IN (..)`) are not pulled up into a
        // semi-join by Postgres; they stay SubPlans that can be re-executed per
        // outer row. The OR must live INSIDE one sublink.
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "finished"),
                SearchValue::new(SearchPrefix::Eq, "in-progress"),
                SearchValue::new(SearchPrefix::Eq, "planned"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token OR-list");

        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "a 3-value OR-list must produce exactly one sublink, not three: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn string_search_is_sargable_and_escapes_wildcards() {
        // `ILIKE` can never use a btree, and the raw `COALESCE(folded, value_string)`
        // could not be matched by any index. The emitted predicate must be a `LIKE`
        // against the exact expression the v14 index is built on.
        let param = SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            // A literal '%' in the search value must not become a wildcard.
            values: vec![SearchValue::new(SearchPrefix::Eq, "50%Off")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Organization").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");

        assert!(
            frag.sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE $3 ESCAPE"),
            "string search must LIKE against the indexed folded expression: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("ILIKE"),
            "ILIKE is not btree-sargable: {}",
            frag.sql
        );
        match &frag.params[0] {
            SqlParam::Text(p) => assert_eq!(
                p, "50\\%off%",
                "the value's own '%' must be escaped; only the trailing \
                 starts-with wildcard is live"
            ),
            other => panic!("expected a text param, got {:?}", other),
        }
    }

    #[test]
    fn reference_or_list_is_a_single_sublink() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "Patient/a"),
                SearchValue::new(SearchPrefix::Eq, "Patient/b"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("reference OR-list");

        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "reference OR-list must be one sublink: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
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
