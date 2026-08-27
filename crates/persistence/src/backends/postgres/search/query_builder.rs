//! PostgreSQL search query builder.
//!
//! Builds SQL queries for FHIR search operations using PostgreSQL syntax
//! with $N parameter placeholders, ILIKE for case-insensitive matching,
//! and native TIMESTAMPTZ comparisons.

use chrono::{DateTime, Utc};

use crate::backends::postgres::schema::IndexLayout;
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
/// `schema.rs::migrate_v23_to_v24`, or Postgres will not use the index.
///
/// Every predicate built from this expression is emitted with a leading
/// `value_string IS NOT NULL` conjunct. `idx_search_string_folded_pattern` is
/// partial on exactly that, and a partial index is only usable when the query
/// *implies* its predicate — which `COALESCE(a, b) LIKE …` does not, COALESCE
/// not being strict. Run 33029355759 measured the consequence: the pattern index
/// had 0 scans while `idx_search_string_folded`, which can only supply the
/// `(tenant_id, resource_type, param_name)` prefix and then filter, took 38,694
/// scans for 142,058,567 tuples — 3,671 tuples read per query.
///
/// The conjunct loses no row. `writer.rs` sets `value_string_folded` only
/// together with `value_string` (and the contained-row path sets `value_string`
/// alone), so `COALESCE(value_string_folded, lower(value_string)) IS NOT NULL`
/// and `value_string IS NOT NULL` select the same rows.
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
        Self::build_search_query_for(query, param_offset, IndexLayout::Denormalized)
    }

    /// As [`Self::build_search_query`], but emitting the form that matches the
    /// database's actual `search_index` layout.
    ///
    /// Only composite search differs. Under [`IndexLayout::Denormalized`] every
    /// component of one composite instance shares a row, so the match is a plain
    /// conjunction. A database that predates v17 still stores one row per
    /// component, where that conjunction matches nothing at all — the search
    /// silently returns an empty bundle rather than failing.
    pub fn build_search_query_for(
        query: &SearchQuery,
        param_offset: usize,
        layout: IndexLayout,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut current_offset = param_offset;

        for param in &query.parameters {
            if let Some(condition) = Self::build_parameter_condition(param, current_offset, layout)
            {
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
                        // Not a composite: this reuses the component predicate
                        // builder for an ordinary single-valued parameter, which
                        // always lives in slot 1.
                        Self::build_composite_component(value, param.param_type, offset, 1)
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

    /// The membership wrapper every single-parameter condition is built with.
    ///
    /// `SqlFragment::and` parenthesizes both sides, so a conjunction never
    /// matches this prefix — only a lone membership test does.
    const INDEX_MEMBERSHIP_PREFIX: &'static str = "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND ";

    /// Returns the bare `search_index` predicate when `sql` is exactly one
    /// membership test, so the caller can resolve the page against
    /// `search_index` directly instead of through a subquery on `resources`.
    pub fn single_index_predicate(sql: &str) -> Option<&str> {
        let inner = sql
            .strip_prefix(Self::INDEX_MEMBERSHIP_PREFIX)?
            .strip_suffix(')')?;
        // Defensive: nothing that still mentions the table can be a lone test,
        // and the legacy composite form is an aggregate rather than a row
        // predicate — splicing it into a `SELECT DISTINCT ... ORDER BY` would be
        // nonsense. (Layouts are mutually exclusive, so this is belt and braces.)
        if inner.contains("FROM search_index") || inner.contains("GROUP BY") {
            return None;
        }
        Some(inner)
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
        layout: IndexLayout,
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
            // Full text over the generated narrative (`_text`) and over the whole
            // serialized resource (`_content`), against the same `resource_fts`
            // tsvector columns the write path populates.
            "_text" => {
                return Self::build_fts_condition(
                    &param.values,
                    "narrative_tsvector",
                    param_offset,
                );
            }
            "_content" => {
                return Self::build_fts_condition(&param.values, "content_tsvector", param_offset);
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
            SearchParamType::Composite => match layout {
                IndexLayout::Denormalized => Self::build_composite_condition(param, param_offset),
                IndexLayout::Legacy => Self::build_composite_condition_legacy(param, param_offset),
            },
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

    /// Builds the `_text` / `_content` full-text condition against
    /// `resource_fts`.
    ///
    /// Without this, `SearchParamType::Special` fell through to the `None` arm
    /// below and the parameter contributed **no** condition. A search whose only
    /// parameter was `_text` therefore produced an empty filter and returned
    /// every resource of the type — a text query answered with the whole
    /// compartment, not a narrower or empty result. SQLite has handled both
    /// parameters (via FTS5 `MATCH`) all along; this is the PostgreSQL half.
    ///
    /// `plainto_tsquery('english', …)` matches `search_text`/`search_content` in
    /// `search_impl.rs`, and it parameterises the user's term rather than
    /// splicing it, so a term containing tsquery operators is data, not syntax.
    ///
    /// `tenant_id = $1` is not optional: the sub-select yields a bare
    /// `resource_id` set that the outer query intersects with *this* tenant's
    /// resources, so omitting it would let tenant B's Patient/123 select tenant
    /// A's Patient/123 — a cross-tenant match oracle. `resource_type = $2`
    /// likewise keeps an Observation's narrative from selecting a Patient of the
    /// same id. Both are already bound as tenant and resource type by every
    /// caller of `build_search_query` (`search`, `search_count`), the same
    /// invariant `build_missing_condition` and `build_compartment_condition`
    /// rely on.
    fn build_fts_condition(
        values: &[SearchValue],
        column: &str,
        offset: usize,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        // Numbered off the running count of *accepted* values, not the loop
        // index: a skipped value must not leave a gap, because the caller binds
        // this fragment's params consecutively from `offset`.
        let mut param_num = offset;
        for value in values {
            let term = value.value.trim();
            if term.is_empty() {
                continue;
            }
            param_num += 1;
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM resource_fts \
                     WHERE tenant_id = $1 AND resource_type = $2 \
                     AND {} @@ plainto_tsquery('english', ${}))",
                    column, param_num
                ),
                vec![SqlParam::text(term)],
            ));
        }
        if conditions.is_empty() {
            // Every value was blank. Returning `None` here would drop the
            // parameter and hand back the entire resource type — the defect this
            // function exists to fix — so fail closed instead. A term of nothing
            // matches nothing, which is also what a stopword-only term already
            // does through `plainto_tsquery`.
            return Some(SqlFragment::new("FALSE".to_string()));
        }
        // Repeated values of one parameter are a logical OR, as elsewhere in
        // this builder.
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
    /// `param:missing=true` matches resources with **no top-level** `search_index`
    /// entry for the parameter; `:missing=false` matches resources that **have**
    /// one. Index rows extracted from contained resources do not establish
    /// presence on their container.
    /// Uses only the always-present `$1`/`$2` (tenant, resource type) bind
    /// params, so it adds no parameters to the surrounding query.
    fn build_missing_condition(param: &SearchParameter) -> SqlFragment {
        let is_missing = param
            .values
            .first()
            .map(|v| v.value == "true")
            .unwrap_or(false);
        let inner = match param.name.as_str() {
            "_id" => "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id IS NOT NULL".to_string(),
            "_lastUpdated" => "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND last_updated IS NOT NULL".to_string(),
            _ => format!(
                "SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND is_contained = FALSE AND param_name = '{}'",
                param.name
            ),
        };
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
                        "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_string IS NOT NULL AND {} LIKE ${} ESCAPE '\\')",
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
                            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND value_string IS NOT NULL AND {} LIKE ${} ESCAPE '\\')",
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

        // Slot each component within its column family, in the parameter's own
        // component order — the identical rule the extractor uses when it writes
        // the denormalized row (`ExtractedValue::composite_slot`). Both sides
        // derive it from the same registry ordering, so no mapping is stored.
        let component_slots: Vec<u8> = {
            let mut seen: std::collections::HashMap<SearchParamType, u8> =
                std::collections::HashMap::new();
            param
                .components
                .iter()
                .map(|c| {
                    let slot = seen.entry(c.param_type).or_insert(0);
                    *slot += 1;
                    *slot
                })
                .collect()
        };

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

            for ((part, component), slot) in parts
                .iter()
                .zip(param.components.iter())
                .zip(component_slots.iter())
            {
                let cv = Self::parse_component_value(part);
                match Self::build_composite_component(&cv, component.param_type, next, *slot) {
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
            // With the denormalized layout every component of one composite
            // instance lives in a single row, so "code = X AND value > Y within
            // the same group" is a plain conjunction — no prefilter, no
            // GROUP BY, no HAVING. The covering index
            // `idx_search_composite_flat` answers it directly and the LIMIT can
            // stop early, which is what removes the ~110k-row / ~108k-heap-block
            // scan the grouped form performed to return 21 rows.
            //
            // Parenthesize each component: a token component emits
            // `sys = $n AND code = $m`, and relying on AND-before-OR precedence
            // is a trap for the next component type someone adds.
            let conjunction = predicates
                .iter()
                .map(|p| format!("({})", p))
                .collect::<Vec<_>>()
                .join(" AND ");

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' \
                 AND composite_group IS NOT NULL AND {})",
                param.name, conjunction
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

    /// Composite search against a pre-v17 `search_index`, where each component of
    /// a composite instance is its own row.
    ///
    /// A resource matches only when every component is satisfied by some row
    /// *within the same* `composite_group` — that grouping is what stops a blood
    /// pressure panel's systolic value from pairing with its diastolic code, so
    /// the GROUP BY / HAVING structure is load-bearing.
    ///
    /// The WHERE prefilter is result-preserving: a row satisfying no component
    /// contributes 0 to every `MAX(CASE ...)` in the HAVING, so it can never turn
    /// a 0 into a 1, and a group the prefilter empties could not have satisfied
    /// the HAVING anyway. Without it the subquery aggregates the resource type's
    /// entire composite slice on every request (#224's timeout).
    ///
    /// This is the pre-#279 form, restored verbatim in behaviour. It is slow —
    /// that slowness is exactly why the denormalized layout exists — but it is
    /// correct against rows the denormalized conjunction cannot match at all.
    /// Components read the base value columns (slot 1): the `_2` columns only
    /// exist to pack two same-type components into one denormalized row.
    fn build_composite_condition_legacy(
        param: &SearchParameter,
        offset: usize,
    ) -> Option<SqlFragment> {
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

            // Stage this value's params and commit them only once every component
            // has parsed, so a component that fails midway cannot leave its
            // predecessors bound with no placeholder in the final statement.
            let mut staged_params: Vec<SqlParam> = Vec::new();
            let mut predicates: Vec<String> = Vec::new();
            let mut next = current;
            let mut ok = true;

            for (part, component) in parts.iter().zip(param.components.iter()) {
                let cv = Self::parse_component_value(part);
                match Self::build_composite_component(&cv, component.param_type, next, 1) {
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

            let havings = predicates
                .iter()
                .map(|p| format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", p))
                .collect::<Vec<_>>()
                .join(" AND ");
            // Parenthesize each component before OR-ing: a token component emits
            // `sys = $n AND code = $m`, and relying on AND-before-OR precedence is
            // a trap for the next component type someone adds.
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

    /// The column a composite component's value lives in, for a given slot.
    ///
    /// Slot 1 is the ordinary value column. Slot 2 is the `_2` variant, used
    /// when a composite has two components of the same type (24 of the 46 R4
    /// composites do — see `ExtractedValue::composite_slot`). The writer
    /// assigns slots from the same registry component order, so both sides
    /// agree without storing a mapping.
    fn composite_col(base: &str, slot: u8) -> String {
        if slot >= 2 {
            format!("{base}_{slot}")
        } else {
            base.to_string()
        }
    }

    /// Builds a single composite component as a bare column predicate (no
    /// `id IN (...)` wrapper — the caller scopes it to the composite row).
    /// Returns `None` if the value cannot be parsed for the component type.
    fn build_composite_component(
        value: &SearchValue,
        param_type: SearchParamType,
        offset: usize,
        slot: u8,
    ) -> Option<(String, Vec<SqlParam>)> {
        let token_code = Self::composite_col("value_token_code", slot);
        let token_system = Self::composite_col("value_token_system", slot);
        let number = Self::composite_col("value_number", slot);
        match param_type {
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        Some((
                            format!("{token_code} = ${}", offset + 1),
                            vec![SqlParam::text(code)],
                        ))
                    } else if code.is_empty() {
                        Some((
                            format!("{token_system} = ${}", offset + 1),
                            vec![SqlParam::text(system)],
                        ))
                    } else {
                        Some((
                            format!(
                                "{token_system} = ${} AND {token_code} = ${}",
                                offset + 1,
                                offset + 2
                            ),
                            vec![SqlParam::text(system), SqlParam::text(code)],
                        ))
                    }
                } else {
                    Some((
                        format!("{token_code} = ${}", offset + 1),
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
                    format!("{} {} ${}", number, op, offset + 1),
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

    /// Builds the condition for a reference parameter.
    ///
    /// # Bare ids
    ///
    /// Per [FHIR R4 search on references](https://hl7.org/fhir/R4/search.html#reference),
    /// `[parameter]=[id]` — a bare logical id — is the *primary* form, with
    /// `[type]/[id]` an additional one. References are indexed as written
    /// (`search/writer.rs` stores `Reference.reference` verbatim), so the
    /// overwhelmingly common `Patient/<id>` literal is what sits in
    /// `value_reference`; matching a bare id therefore needs a suffix match, not
    /// just equality. Comparing only the raw value made `Observation?patient=<id>`
    /// — the single most common search shape in FHIR — return an empty Bundle
    /// (#490), while `patient=Patient/<id>` worked.
    ///
    /// A `:Type` modifier resolves the ambiguity up front: `subject:Patient=<id>`
    /// is normalized to `Patient/<id>` and matched as a type-prefixed reference,
    /// mirroring the SQLite handler.
    ///
    /// Matching stays version-agnostic throughout: the search value is stripped of
    /// any `/_history/<vid>`, and a stored versioned reference still matches.
    ///
    /// # LIKE escaping
    ///
    /// The plain path binds fully-formed patterns built with [`like_escape`] and
    /// `ESCAPE '\'` rather than concatenating the raw value into a pattern in SQL.
    /// The suffix match makes this load-bearing: an unescaped `%` would turn
    /// `patient=%` into `LIKE '%/%'` and match every reference. (The `:contains`,
    /// `:below` and `:above` paths keep their existing unescaped behavior.)
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
        let type_modifier = match modifier {
            Some(SearchModifier::Type(type_name)) => Some(type_name.as_str()),
            _ => None,
        };

        // The plain path binds a variable number of parameters per value (two for
        // a type-prefixed reference, three for a bare id), so `param_num` runs as
        // a counter rather than `offset + i`. `build_search_query` advances the
        // next parameter's offset by `params.len()`, so this stays consistent.
        let mut param_num = offset;
        for value in &param.values {
            let predicate = if is_text {
                param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference_display ILIKE '%' || ${} || '%'", param_num)
            } else if is_code_text {
                param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference_display ILIKE ${} || '%'", param_num)
            } else if is_contains {
                param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference ILIKE '%' || ${} || '%'", param_num)
            } else if is_below {
                // URL/path-prefix hierarchy (canonical |version not handled).
                param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!(
                    "(value_reference = ${0} OR value_reference LIKE ${0} || '/%')",
                    param_num
                )
            } else if is_above {
                param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!(
                    "(${0} = value_reference OR ${0} LIKE value_reference || '/%')",
                    param_num
                )
            } else {
                // Plain reference match. Normalize `:Type` + bare id to `Type/id`,
                // then match version-agnostically off the version-stripped base.
                let stripped = strip_reference_version(&value.value);
                let base = match type_modifier {
                    Some(type_name) if !stripped.contains('/') => {
                        format!("{}/{}", type_name, stripped)
                    }
                    _ => stripped.to_string(),
                };
                let escaped = like_escape(&base);

                // `\_history` — the escape keeps the underscore literal.
                if base.contains('/') {
                    // `Type/id` or an absolute URL: match the base reference, or
                    // the same reference carrying any `_history` version.
                    let exact = param_num + 1;
                    let versioned = param_num + 2;
                    param_num += 2;
                    params.push(SqlParam::text(&base));
                    params.push(SqlParam::text(&format!("{}/\\_history/%", escaped)));
                    format!(
                        "(value_reference = ${exact} OR value_reference LIKE ${versioned} ESCAPE '\\')"
                    )
                } else {
                    // Bare logical id: also match any reference ending in `/id`,
                    // with or without a trailing `_history` version.
                    let exact = param_num + 1;
                    let suffix = param_num + 2;
                    let versioned = param_num + 3;
                    param_num += 3;
                    params.push(SqlParam::text(&base));
                    params.push(SqlParam::text(&format!("%/{}", escaped)));
                    params.push(SqlParam::text(&format!("%/{}/\\_history/%", escaped)));
                    format!(
                        "(value_reference = ${exact} \
                          OR value_reference LIKE ${suffix} ESCAPE '\\' \
                          OR value_reference LIKE ${versioned} ESCAPE '\\')"
                    )
                }
            };
            conditions.push(predicate);
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

    fn date_param(name: &str, prefix: SearchPrefix, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn single_membership_test_is_extractable() {
        let query = SearchQuery::new("Encounter").with_parameter(date_param(
            "date",
            SearchPrefix::Gt,
            "2010-01-01",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(pred, "param_name = 'date' AND value_date >= $3");
        // The extracted predicate must stand alone against `search_index`.
        assert!(!pred.contains("resources"));
        assert!(!pred.contains("SELECT"));
    }

    #[test]
    fn conjunction_is_not_extractable() {
        // Two parameters AND together, so no single index predicate resolves the
        // page — taking either one alone would over-match.
        let query = SearchQuery::new("Encounter")
            .with_parameter(date_param("date", SearchPrefix::Gt, "2010-01-01"))
            .with_parameter(date_param("date", SearchPrefix::Lt, "2020-01-01"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    #[test]
    fn repeated_values_of_one_param_are_not_extractable() {
        // `date=gt..&date=lt..` on one parameter also ANDs at resource level: two
        // index rows may satisfy it jointly, which a single-row predicate cannot
        // express.
        let param = SearchParameter {
            name: "date".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Gt, "2010-01-01"),
                SearchValue::new(SearchPrefix::Lt, "2020-01-01"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    #[test]
    fn missing_modifier_is_not_extractable() {
        // `:missing=true` is an absence test, so the rows it selects are exactly
        // the ones `search_index` does not hold.
        let mut param = date_param("date", SearchPrefix::Eq, "true");
        param.modifier = Some(SearchModifier::Missing);
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    #[test]
    fn legacy_layout_keeps_the_grouped_composite_form() {
        // A database that predates v17 stores one row per composite component.
        // The denormalized conjunction matches none of them, and would return an
        // empty bundle rather than an error — so the layout must select the form.
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

        let legacy = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("legacy composite condition");
        assert!(
            legacy
                .sql
                .contains("GROUP BY resource_id, composite_group HAVING"),
            "grouping is what confines components to one composite instance: {}",
            legacy.sql
        );
        assert!(
            legacy.sql.contains("MAX(CASE WHEN"),
            "per-component HAVING must survive: {}",
            legacy.sql
        );
        // Components read the base columns; the `_2` columns exist only to pack
        // two same-type components into one denormalized row.
        assert!(!legacy.sql.contains("value_token_code_2"), "{}", legacy.sql);

        let flat =
            PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Denormalized)
                .expect("denormalized composite condition");
        assert!(!flat.sql.contains("GROUP BY"), "{}", flat.sql);
        assert!(
            flat.sql.contains("composite_group IS NOT NULL"),
            "{}",
            flat.sql
        );

        // Both forms must bind the same values, or the two layouts would disagree
        // about what was searched for.
        assert_eq!(legacy.params.len(), flat.params.len());
    }

    #[test]
    fn the_legacy_composite_form_is_never_taken_as_a_fast_path() {
        // It is an aggregate, not a row predicate; splicing it into a
        // `SELECT DISTINCT ... ORDER BY` would be nonsense.
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
        let legacy = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("condition");
        assert!(PostgresQueryBuilder::single_index_predicate(&legacy.sql).is_none());
    }

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

    /// The two composite spellings, pinned. These are the exact texts the v27
    /// composite indexes are shaped for, and the exact texts whose partial
    /// predicates `predtest.c` has to prove:
    ///
    /// - `composite_group IS NOT NULL` appears literally, so
    ///   `WHERE composite_group IS NOT NULL` is provable;
    /// - the quantity component is a strict operator over
    ///   `value_quantity_value`, so `WHERE value_quantity_value IS NOT NULL` is
    ///   provable;
    /// - the token component is an equality on `value_token_code`, which is why
    ///   that column leads the index key ahead of the sort key (v20's rule).
    ///
    /// If any of these change, the v27 indexes silently stop being reachable
    /// and composite search falls back to a full sort. Reachability is not
    /// something a `#[test]` can observe, so it is pinned here instead.
    fn composite_param(name: &str, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
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
        }
    }

    #[test]
    fn composite_bare_code_emits_what_the_v27_index_is_keyed_for() {
        // `combo-code-value-quantity=8867-4$gt100` — 21 of the 27 composite
        // values in `k6/searchConfig.js` are this spelling.
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param("combo-code-value-quantity", "8867-4$gt100"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 \
             AND resource_type = $2 AND param_name = 'combo-code-value-quantity' \
             AND composite_group IS NOT NULL \
             AND (value_token_code = $3) AND (value_quantity_value > $4))",
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn composite_system_qualified_code_keeps_the_system_in_the_predicate() {
        // The other 6 values, and the second `pg_stat_statements` entry (7,767
        // calls). `value_token_system` is payload on the v27 index so this
        // stays index-only rather than fetching the heap per candidate.
        let query = SearchQuery::new("Observation").with_parameter(composite_param(
            "combo-code-value-quantity",
            "http://loinc.org|8480-6$gt140",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 \
             AND resource_type = $2 AND param_name = 'combo-code-value-quantity' \
             AND composite_group IS NOT NULL \
             AND (value_token_system = $3 AND value_token_code = $4) \
             AND (value_quantity_value > $5))",
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn every_composite_quantity_prefix_is_a_strict_operator() {
        // `WHERE value_quantity_value IS NOT NULL` on the composite index is
        // only provable from a STRICT operator over that column. Every prefix
        // the benchmark can send — and every prefix `parse_component_value`
        // recognises — must therefore emit one. `IS DISTINCT FROM` or a
        // `COALESCE` here would strand the index without any test failing.
        for (spelling, op) in [
            ("gt100", ">"),
            ("lt100", "<"),
            ("ge100", ">="),
            ("le100", "<="),
            ("ne100", "!="),
            ("sa100", ">"),
            ("eb100", "<"),
            ("ap100", "="),
            ("100", "="),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(composite_param(
                "combo-code-value-quantity",
                &format!("8867-4${spelling}"),
            ));
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
            assert!(
                frag.sql
                    .contains(&format!("(value_quantity_value {op} $4)")),
                "{spelling} must emit a strict operator: {}",
                frag.sql
            );
        }
    }

    #[test]
    fn the_composite_fast_path_predicate_is_extractable() {
        // The whole point of keying the composite index on the fast path's sort
        // key is that composite search TAKES the fast path. It does, because a
        // lone composite parameter is exactly one membership test — this is the
        // gate, and it is the reason `ORDER BY last_updated DESC, resource_id
        // ASC` is what the index has to serve.
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param("combo-code-value-quantity", "8867-4$gt100"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone composite test is extractable");

        assert_eq!(
            pred,
            "param_name = 'combo-code-value-quantity' AND composite_group IS NOT NULL \
             AND (value_token_code = $3) AND (value_quantity_value > $4)"
        );
        // It has to stand alone against `search_index`.
        assert!(!pred.contains("resources"));
        assert!(!pred.contains("SELECT"));
    }

    fn special_param(name: &str, values: Vec<SearchValue>) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Special,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn text_search_filters_instead_of_returning_everything() {
        // Regression: `SearchParamType::Special` fell through to `None`, so a
        // `_text`-only query built no filter at all and `search` answered a
        // full-text query with every resource of the type.
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_text",
            vec![SearchValue::eq("Zebracrossingdiagnosis")],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_text must produce a condition, not an empty filter");

        assert!(
            frag.sql.contains("FROM resource_fts"),
            "must resolve through the full-text table: {}",
            frag.sql
        );
        assert!(
            frag.sql
                .contains("narrative_tsvector @@ plainto_tsquery('english', $3)"),
            "_text matches the narrative column: {}",
            frag.sql
        );
        // Tenant and type scoping keep the sub-select from selecting another
        // tenant's — or another resource type's — row of the same id.
        assert!(frag.sql.contains("tenant_id = $1"), "{}", frag.sql);
        assert!(frag.sql.contains("resource_type = $2"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn content_search_uses_the_content_column() {
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_content",
            vec![SearchValue::eq("Quokkaflavoured")],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_content must produce a condition");

        assert!(
            frag.sql
                .contains("content_tsvector @@ plainto_tsquery('english', $3)"),
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn text_or_list_placeholders_are_gap_free() {
        // Two terms OR together, and a blank one must not consume a placeholder
        // number it never binds — the caller binds this fragment's params
        // consecutively.
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_text",
            vec![
                SearchValue::eq("   "),
                SearchValue::eq("fracture"),
                SearchValue::eq("sprain"),
            ],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_text OR-list should produce a condition");

        assert_eq!(frag.params.len(), 2, "the blank term binds nothing");
        assert!(frag.sql.contains("$3"), "{}", frag.sql);
        assert!(frag.sql.contains("$4"), "{}", frag.sql);
        assert!(
            !frag.sql.contains("$5"),
            "placeholder numbering must be gap-free: {}",
            frag.sql
        );
        assert!(frag.sql.contains(" OR "), "{}", frag.sql);
    }

    #[test]
    fn blank_text_term_fails_closed() {
        // Dropping the parameter would return the whole resource type, which is
        // the exact failure mode being fixed.
        let query = SearchQuery::new("Patient")
            .with_parameter(special_param("_text", vec![SearchValue::eq("")]));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("a blank _text must still constrain the query");

        assert_eq!(frag.sql, "FALSE");
        assert!(frag.params.is_empty());
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
    fn composite_confines_components_to_one_group() {
        // The invariant under test is unchanged from the grouped form: a
        // resource may only match when BOTH components are satisfied within the
        // SAME composite instance — a blood-pressure panel's systolic value must
        // never pair with its diastolic code.
        //
        // What changed is how that is enforced. The old form read one row per
        // component and used GROUP BY resource_id, composite_group + HAVING
        // MAX(CASE …) to require both within a group. The denormalized layout
        // (#279) stores one row per (resource, composite_group) carrying every
        // component's value, so a plain conjunction over ONE row enforces the
        // same thing by construction — components cannot come from different
        // groups because they cannot come from different rows.
        //
        // So this asserts the new mechanism, and deliberately asserts the
        // absence of the old one: if GROUP BY reappears here alongside the flat
        // predicate, the two layouts have been mixed and the result is wrong.
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

        // One row per composite instance: the predicate is scoped to composite
        // rows and every component is ANDed within that single row.
        assert!(
            frag.sql.contains("composite_group IS NOT NULL"),
            "the subquery must be confined to composite rows: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains(") AND (value_quantity_value"),
            "components must be ANDed within one row, not OR-prefiltered: {}",
            frag.sql
        );
        // The old aggregate form must be gone — mixing the two layouts would
        // aggregate over already-denormalized rows and match across groups.
        assert!(
            !frag.sql.contains("GROUP BY resource_id, composite_group"),
            "the denormalized layout must not re-aggregate: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("MAX(CASE WHEN"),
            "the denormalized layout must not use the HAVING form: {}",
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

    /// Builds a string `SearchParameter` with an optional modifier.
    fn string_param(name: &str, modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::String,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn folded_string_search_implies_the_partial_index_predicate() {
        // `idx_search_string_folded_pattern` is partial on `value_string IS NOT
        // NULL`, and Postgres only uses a partial index when the query implies
        // its predicate. `COALESCE(a, b) LIKE …` does not imply it — COALESCE is
        // not strict — so without this conjunct the index is unreachable and the
        // scan degrades to a filter over the whole parameter slice (measured:
        // 0 scans on the pattern index, 3,671 tuples read per scan on the other
        // one). See `migrate_v23_to_v24`.
        for modifier in [
            None,
            Some(SearchModifier::Contains),
            Some(SearchModifier::Text),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(string_param(
                "name",
                modifier.clone(),
                "Emilia",
            ));
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
            assert!(
                frag.sql.contains(
                    "value_string IS NOT NULL AND \
                     COALESCE(value_string_folded, lower(value_string)) LIKE $3 ESCAPE"
                ),
                "modifier {:?} must imply the partial index predicate: {}",
                modifier,
                frag.sql
            );
        }
    }

    #[test]
    fn exact_string_search_keeps_the_bare_column_predicate() {
        // `:exact` reads `value_string` directly, which is already strict, so it
        // must not acquire the redundant conjunct — `idx_search_string` serves it.
        let query = SearchQuery::new("Patient").with_parameter(string_param(
            "name",
            Some(SearchModifier::Exact),
            "Emilia",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");

        assert!(frag.sql.contains("value_string = $3"), "{}", frag.sql);
        assert!(!frag.sql.contains("IS NOT NULL"), "{}", frag.sql);
        assert!(!frag.sql.contains("COALESCE"), "{}", frag.sql);
    }

    /// Builds a token `SearchParameter` with an optional modifier.
    fn token_param(name: &str, modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// Builds a reference `SearchParameter` with an optional modifier.
    fn reference_param(
        name: &str,
        modifier: Option<SearchModifier>,
        value: &str,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Reference,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// v27 dropped `idx_search_string_folded`, the btree on the bare
    /// `value_string_folded` column, because no emitted predicate can seek it:
    /// the column is only ever read through `COALESCE(value_string_folded,
    /// lower(value_string))`, which an index on the bare column cannot match,
    /// and `sort_value_column` maps `String` to `value_string`.
    ///
    /// If a bare predicate on the folded column is ever added back it will get a
    /// sequential scan and nobody will notice, so pin the rule here.
    #[test]
    fn the_folded_column_is_only_ever_read_through_the_coalesce() {
        for modifier in [
            None,
            Some(SearchModifier::Contains),
            Some(SearchModifier::Text),
            Some(SearchModifier::Exact),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(string_param(
                "name",
                modifier.clone(),
                "Emilia",
            ));
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
            assert_eq!(
                frag.sql.matches("value_string_folded").count(),
                frag.sql.matches(FOLDED_STRING_EXPR).count(),
                "modifier {:?} reads the folded column outside the indexed \
                 COALESCE, which no index in the schema can serve: {}",
                modifier,
                frag.sql
            );
        }
        assert_eq!(
            sort_value_column(SearchParamType::String),
            Some("value_string")
        );
    }

    /// v27 dropped `idx_search_token_display` and `idx_search_reference_display`.
    /// Both were btrees in the default operator class, and `ILIKE` is not
    /// sargable against one at any prefix — so they were only ever scanners of
    /// their `(tenant_id, resource_type, param_name)` slice, which
    /// `idx_search_token_code` and `idx_search_reference_pattern` cover over a
    /// superset of the rows.
    ///
    /// The drop is safe exactly as long as `ILIKE` stays the only operator on
    /// these columns. An `=` or a prefix `LIKE` added later WOULD be sargable
    /// and would then want an index that no longer exists.
    #[test]
    fn display_columns_are_only_ever_matched_with_ilike() {
        let cases: Vec<(SearchQuery, &str)> = vec![
            (
                SearchQuery::new("Observation").with_parameter(token_param(
                    "code",
                    Some(SearchModifier::Text),
                    "Blood",
                )),
                "value_token_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(token_param(
                    "code",
                    Some(SearchModifier::CodeText),
                    "Blood",
                )),
                "value_token_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(reference_param(
                    "subject",
                    Some(SearchModifier::Text),
                    "Emilia",
                )),
                "value_reference_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(reference_param(
                    "subject",
                    Some(SearchModifier::CodeText),
                    "Emilia",
                )),
                "value_reference_display",
            ),
        ];

        for (query, column) in cases {
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("display shape");
            let mentions = frag.sql.matches(column).count();
            assert!(mentions > 0, "expected a {column} predicate: {}", frag.sql);
            assert_eq!(
                frag.sql.matches(&format!("{column} ILIKE")).count(),
                mentions,
                "{column} must only ever be matched with ILIKE — anything \
                 sargable needs an index this schema no longer has: {}",
                frag.sql
            );
        }
    }

    /// v27 dropped `idx_search_reference` and kept its `text_pattern_ops` twin
    /// `idx_search_reference_pattern`, which has the same key columns and the
    /// same partial predicate. That family carries `=` but NOT the ordering
    /// operators, so the drop holds only while every predicate on
    /// `value_reference` is equality or a `LIKE` — and it is the pattern index,
    /// not this one, that the prefix `LIKE`s need.
    #[test]
    fn reference_predicates_never_need_a_collation_ordered_index() {
        let queries = [
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                None,
                "Patient/p1",
            )),
            SearchQuery::new("Observation").with_parameter(reference_param("subject", None, "p1")),
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                Some(SearchModifier::Below),
                "http://h/Organization/o1",
            )),
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                Some(SearchModifier::Contains),
                "p1",
            )),
        ];

        for query in queries {
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("reference shape");
            for op in [" < ", " > ", " <= ", " >= ", "ORDER BY value_reference"] {
                assert!(
                    !frag.sql.contains(op),
                    "an ordering comparison on value_reference cannot use the \
                     text_pattern_ops index that survives v27 (`{op}`): {}",
                    frag.sql
                );
            }
        }
    }

    #[test]
    fn system_qualified_token_is_one_extractable_equality_conjunction() {
        // `Encounter?class=http://…v3-ActCode|AMB`. This is the shape v24's
        // `idx_search_token` is keyed for: every column ahead of the sort key is
        // bound by equality, so the index can return the fast path's rows already
        // ordered. If this ever became a range, an OR, or a second sublink, that
        // index could no longer stream and the LIMIT would stop terminating early.
        let param = SearchParameter {
            name: "class".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://terminology.hl7.org/CodeSystem/v3-ActCode|AMB",
            )],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(
            pred,
            "param_name = 'class' AND ((value_token_system = $3 AND value_token_code = $4))"
        );
        assert_eq!(frag.params.len(), 2);
        match (&frag.params[0], &frag.params[1]) {
            (SqlParam::Text(system), SqlParam::Text(code)) => {
                assert_eq!(system, "http://terminology.hl7.org/CodeSystem/v3-ActCode");
                assert_eq!(code, "AMB");
            }
            other => panic!("expected two text params, got {:?}", other),
        }
    }

    #[test]
    fn bare_token_predicate_never_mentions_the_system_column() {
        // The counterpart to the test above, and the reason `Encounter?status` is
        // 4x cheaper than `Encounter?class` on identical data: a bare code is not
        // strict in `value_token_system`, so it cannot reach the partial
        // `idx_search_token` at all and is served by the code-first indexes.
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "finished")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(pred, "param_name = 'status' AND (value_token_code = $3)");
        assert!(!pred.contains("value_token_system"));
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
        // Two params per type-prefixed value: the exact base and the
        // `/_history/%` pattern (the pattern is built in Rust so the value can be
        // LIKE-escaped, rather than concatenated onto the raw bind in SQL).
        assert_eq!(frag.params.len(), 4);
    }

    /// A bare logical id is the primary form of a reference search
    /// (`Observation?patient=<id>`), and must match a stored `Patient/<id>`.
    /// Postgres previously compared the raw value only, so a bare id matched
    /// nothing while `patient=Patient/<id>` worked — #490.
    #[test]
    fn reference_bare_id_matches_type_prefixed_reference() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("bare-id reference");

        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|p| match p {
                SqlParam::Text(t) => t.as_str(),
                other => panic!("expected text params, got {:?}", other),
            })
            .collect();
        assert_eq!(
            params,
            vec!["patient-1", "%/patient-1", "%/patient-1/\\_history/%"],
            "a bare id must match exactly, as a `/id` suffix, and versioned: {}",
            frag.sql
        );
        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "still a single sublink: {}",
            frag.sql
        );
    }

    /// `subject:Patient=<id>` resolves the bare id to `Patient/<id>` and matches
    /// it as a type-prefixed reference, mirroring the SQLite handler.
    #[test]
    fn reference_type_modifier_normalizes_bare_id() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Type("Patient".to_string())),
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect(":Type reference");

        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|p| match p {
                SqlParam::Text(t) => t.as_str(),
                other => panic!("expected text params, got {:?}", other),
            })
            .collect();
        assert_eq!(
            params,
            vec!["Patient/patient-1", "Patient/patient-1/\\_history/%"],
            "the type modifier pins the reference, so no `/id` suffix match: {}",
            frag.sql
        );
    }

    /// A versioned search value is stripped before the bare-id suffix match, so
    /// `subject=patient-1/_history/2` still matches a stored `Patient/patient-1`.
    #[test]
    fn reference_bare_id_is_version_stripped() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1/_history/2")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("versioned bare-id reference");

        match &frag.params[0] {
            SqlParam::Text(t) => assert_eq!(t, "patient-1"),
            other => panic!("expected a text param, got {:?}", other),
        }
        assert_eq!(frag.params.len(), 3, "version-stripped back to a bare id");
    }

    /// The suffix match makes LIKE escaping load-bearing: unescaped, `patient=%`
    /// would become `LIKE '%/%'` and match every stored reference.
    #[test]
    fn reference_bare_id_escapes_like_metacharacters() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "%")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("wildcard reference");

        match &frag.params[1] {
            SqlParam::Text(t) => assert_eq!(
                t, "%/\\%",
                "only the leading wildcard is live; the value's own '%' is escaped"
            ),
            other => panic!("expected a text param, got {:?}", other),
        }
        assert!(
            frag.sql.contains("ESCAPE '\\'"),
            "the pattern must carry its escape clause: {}",
            frag.sql
        );
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

    #[test]
    fn missing_ignores_index_rows_from_contained_resources() {
        let param = SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("false")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Patient").with_parameter(param);
        let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect(":missing must produce a condition");

        assert!(
            fragment.sql.contains("is_contained = FALSE"),
            "contained-resource index rows must not establish presence: {}",
            fragment.sql
        );
    }

    #[test]
    fn missing_metadata_uses_authoritative_resource_columns() {
        for (name, param_type, column) in [
            ("_id", SearchParamType::Token, "id"),
            ("_lastUpdated", SearchParamType::Date, "last_updated"),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: name.to_string(),
                param_type,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq("false")],
                chain: vec![],
                components: vec![],
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
                .expect(":missing must produce a condition");

            assert!(fragment.sql.contains("FROM resources"));
            assert!(fragment.sql.contains(&format!("{column} IS NOT NULL")));
            assert!(!fragment.sql.contains("FROM search_index"));
        }
    }
}
