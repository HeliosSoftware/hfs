//! SQL Query Builder for FHIR Search.
//!
//! Translates FHIR search queries into SQL statements that can be executed
//! against the SQLite search_index table.

use std::collections::HashSet;

use crate::error::SearchError;
use crate::types::{
    CompartmentMembership, ContainedMode, SearchModifier, SearchParamType, SearchParameter,
    SearchQuery, SearchValue, strip_reference_version,
};

use super::modifier_handlers::{build_missing_condition, get_missing_value, is_missing_modifier};
use super::parameter_handlers::{
    CompositeHandler, DateHandler, NumberHandler, QuantityHandler, ReferenceHandler, StringHandler,
    TokenHandler, UriHandler,
};

// Keep generated SQL and bind counts small well before SQLite's 32,766-variable limit.
const LARGE_ID_SET_THRESHOLD: usize = 1_000;

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
        // Projects `resource_key` (the integer surrogate), not `resource_id`:
        // with the v31 composite index `(tenant_id, resource_type, resource_key,
        // …)`, selecting and filtering on `resource_key` keeps this wrapper a
        // covering-index seek. The outer query compares it to `resources.rowid`.
        let mut base = SqlFragment::new(
            "SELECT DISTINCT resource_key FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2",
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

        // Compartment membership: a resource is in the compartment if it
        // references the compartment via ANY of the membership params (OR),
        // per the FHIR CompartmentDefinition. Applied as a single subquery.
        if let Some(comp) = &query.compartment {
            // Last condition appended, so no need to advance `current_offset`.
            if let Some(condition) = self.build_compartment_condition(comp, current_offset) {
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

    /// Builds the `_contained` match subquery.
    ///
    /// Returns SQL selecting `(resource_type, resource_id, contained_local_id)`
    /// from `search_index` for contained resources (`is_contained = 1`) of the
    /// searched type (`contained_type = ?2`) that match every search
    /// parameter. Matching is keyed on the contained entity
    /// `(resource_id, contained_local_id)` via `GROUP BY ... HAVING
    /// COUNT(DISTINCT param_name) >= <n>`, so a container only matches when a
    /// single contained resource satisfies all parameters.
    ///
    /// Each occurrence of a parameter is its own AND-ed branch (values within
    /// an occurrence are ORed). Counting distinct names only proves every
    /// branch matched while the names are distinct, so once a name repeats
    /// (`date=ge2020&date=le2020`) the `HAVING` instead requires each branch
    /// with `MAX(CASE WHEN <branch> THEN 1 ELSE 0 END) = 1`, on the same
    /// contained entity (#1362). `:not` is the same aggregate required to be
    /// `0` — "no row of this entity matches" — which needs every row of the
    /// entity, so the row filter in `WHERE` is left out when one is present
    /// (#1363).
    ///
    /// `_id` is the contained resource's local id, a column of every row, and
    /// narrows the rows directly. `_tag`, `_profile`, `_security`, `_source`
    /// and `_language` are indexed from the contained resource's own `meta`
    /// like any other parameter. What the contained rows cannot answer is
    /// refused by [`Self::reject_unsupported_contained`], which every caller
    /// runs first; this function skips those parameters.
    ///
    /// Param layout: `?1` = tenant, `?2` = contained type, then value params.
    /// The `HAVING` aggregates repeat the `WHERE` branches verbatim, reusing
    /// their numbered placeholders rather than binding again.
    ///
    /// `query.compartment` is one more branch, on the contained resource's
    /// own references. With no criterion at all the result is every contained
    /// resource of the type (#1383), so this always returns `Some`. The rows
    /// are unordered; the caller sorts them before paging.
    pub fn build_contained(&self, query: &SearchQuery) -> Option<SqlFragment> {
        // (branch, negated)
        let mut branches: Vec<(String, bool)> = Vec::new();
        let mut entity_filters: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        let mut distinct_names: HashSet<String> = HashSet::new();
        // ?1 = tenant, ?2 = contained_type already consumed by the caller.
        let mut offset = 2;

        for param in &query.parameters {
            if Self::contained_unsupported_reason(param).is_some() || param.values.is_empty() {
                continue;
            }

            if param.name == "_id" {
                let placeholders: Vec<String> = param
                    .values
                    .iter()
                    .map(|value| {
                        params.push(SqlParam::string(&value.value));
                        offset += 1;
                        format!("?{offset}")
                    })
                    .collect();
                entity_filters.push(format!(
                    "contained_local_id IN ({})",
                    placeholders.join(", ")
                ));
                continue;
            }

            // A composite is decided per `composite_group` of one contained
            // entity — every component satisfied by a row of the same group —
            // which is not a predicate on one row, so it narrows the entities
            // like `_id` does rather than joining the branches (#1407). Same
            // pairing as `build_composite_parameter_condition`, keyed on the
            // contained entity instead of `resource_key`.
            if param.param_type == SearchParamType::Composite {
                let mut alternatives = Vec::new();
                for value in &param.values {
                    match CompositeHandler::build_component_fragments(
                        value,
                        &param.components,
                        offset,
                    ) {
                        Some(fragments) if !fragments.is_empty() => {
                            let havings: Vec<String> = fragments
                                .iter()
                                .map(|f| format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", f.sql))
                                .collect();
                            for f in fragments {
                                offset += f.params.len();
                                params.extend(f.params);
                            }
                            alternatives.push(format!(
                                "(resource_type, resource_id, contained_local_id) IN \
                                 (SELECT resource_type, resource_id, contained_local_id \
                                 FROM search_index WHERE tenant_id = ?1 AND is_contained = 1 \
                                 AND contained_type = ?2 AND param_name = '{}' \
                                 GROUP BY resource_type, resource_id, contained_local_id, \
                                 composite_group HAVING {})",
                                param.name,
                                havings.join(" AND ")
                            ));
                        }
                        // Unparseable: fail closed, as the top-level builder does.
                        _ => alternatives.push("0 = 1".to_string()),
                    }
                }
                entity_filters.push(format!("({})", alternatives.join(" OR ")));
                continue;
            }

            let mut or_conditions = Vec::new();
            let mut local_offset = offset;
            for value in &param.values {
                if let Some(cond) = self.build_value_condition(param, value, local_offset) {
                    local_offset += cond.params.len();
                    or_conditions.push(cond);
                }
            }
            if or_conditions.is_empty() {
                continue;
            }
            let mut combined = or_conditions.remove(0);
            for cond in or_conditions {
                combined = combined.or(cond);
            }
            offset += combined.params.len();
            branches.push((
                format!("(param_name = '{}' AND ({}))", param.name, combined.sql),
                matches!(param.modifier, Some(SearchModifier::Not)),
            ));
            params.extend(combined.params);
            distinct_names.insert(param.name.clone());
        }

        // Compartment membership is a criterion on the contained resource like
        // any other: it references the compartment through ANY of the
        // membership parameters (#1383). That one branch spans several
        // parameter names, so the names an entity matched no longer prove
        // every branch did.
        let mut names_prove_branches = true;
        if let Some(comp) = &query.compartment {
            if !comp.params.is_empty() && !comp.reference.is_empty() {
                let in_list = comp
                    .params
                    .iter()
                    .map(|p| format!("'{}'", p.replace('\'', "''")))
                    .collect::<Vec<_>>()
                    .join(", ");
                let base = strip_reference_version(&comp.reference);
                branches.push((
                    format!(
                        "(param_name IN ({in_list}) AND value_reference IS NOT NULL \
                         AND (value_reference = ?{} OR value_reference LIKE ?{} || '/_history/%'))",
                        offset + 1,
                        offset + 2
                    ),
                    false,
                ));
                params.push(SqlParam::string(base));
                params.push(SqlParam::string(base));
                names_prove_branches = false;
            }
        }

        // With no criterion at all, every contained resource of the type
        // matches (#1383): the grouping below lists each of them once.
        let any_negated = branches.iter().any(|(_, negated)| *negated);
        let mut sql = String::from(
            "SELECT resource_type, resource_id, contained_local_id FROM search_index \
             WHERE tenant_id = ?1 AND is_contained = 1 AND contained_type = ?2",
        );
        for filter in &entity_filters {
            sql.push_str(&format!(" AND {filter}"));
        }
        if !branches.is_empty() && !any_negated {
            let positive: Vec<&str> = branches.iter().map(|(b, _)| b.as_str()).collect();
            sql.push_str(&format!(" AND ({})", positive.join(" OR ")));
        }
        sql.push_str(" GROUP BY resource_type, resource_id, contained_local_id");
        if !branches.is_empty() {
            let having =
                if !any_negated && names_prove_branches && distinct_names.len() == branches.len() {
                    format!("COUNT(DISTINCT param_name) >= {}", distinct_names.len())
                } else {
                    // A repeated name or a negation: one row can satisfy only some
                    // of the branches, so state each. The placeholders are reused,
                    // not rebound.
                    branches
                        .iter()
                        .map(|(branch, negated)| {
                            format!(
                                "MAX(CASE WHEN {branch} THEN 1 ELSE 0 END) = {}",
                                if *negated { 0 } else { 1 }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" AND ")
                };
            sql.push_str(&format!(" HAVING {having}"));
        }
        Some(SqlFragment::with_params(sql, params))
    }

    /// Why `_contained` matching cannot apply `param`, if it cannot.
    ///
    /// The contained rows of `search_index` hold what the extractor found in
    /// the contained resource itself, one row per value. That answers every
    /// ordinary parameter, the `meta`-derived `_`-parameters and — through
    /// `contained_local_id` — `_id`. It does not answer:
    ///
    /// - `_lastUpdated`: a contained resource has no `meta.lastUpdated` of its
    ///   own, and the container's is not on these rows;
    /// - `_text`, `_content`, `_filter` and the other `_`-parameters that are
    ///   resolved against `resources` or the FTS tables, which only know the
    ///   container;
    /// - chains, which it does not follow (composites are paired per
    ///   `composite_group` of the contained entity, #1407);
    /// - `:missing`, refused here since the modifier was introduced and pinned
    ///   as a 400 by the REST and PostgreSQL suites, and the modifiers that are
    ///   not a predicate on one index row.
    fn contained_unsupported_reason(param: &SearchParameter) -> Option<String> {
        if !param.chain.is_empty() {
            return Some("chained parameters are".to_string());
        }
        if param.name == "_id" {
            return param
                .modifier
                .as_ref()
                .map(|m| format!("the ':{m}' modifier on _id is"));
        }
        if param.name.starts_with('_')
            && !matches!(
                param.name.as_str(),
                "_tag" | "_profile" | "_security" | "_source" | "_language"
            )
        {
            return Some("this parameter is".to_string());
        }
        let row_level = match (&param.modifier, param.param_type) {
            // Paired per `composite_group` by `build_contained`. Without its
            // components (the REST layer resolves them) there is nothing to
            // pair, and no modifier applies to a composite.
            (None, SearchParamType::Composite) if !param.components.is_empty() => true,
            (_, SearchParamType::Composite) => {
                return Some(
                    "composite parameters with a modifier or no components are".to_string(),
                );
            }
            (_, SearchParamType::Special) => return Some("special parameters are".to_string()),
            (None | Some(SearchModifier::Not), _) => true,
            (
                Some(SearchModifier::Exact | SearchModifier::Contains | SearchModifier::Text),
                SearchParamType::String,
            ) => true,
            (
                Some(SearchModifier::Text | SearchModifier::CodeText | SearchModifier::OfType),
                SearchParamType::Token,
            ) => true,
            (
                Some(SearchModifier::Contains | SearchModifier::Below | SearchModifier::Above),
                SearchParamType::Uri,
            ) => true,
            (
                Some(SearchModifier::Identifier | SearchModifier::Type(_)),
                SearchParamType::Reference,
            ) => true,
            _ => false,
        };
        if row_level {
            return None;
        }
        param
            .modifier
            .as_ref()
            .map(|m| format!("the ':{m}' modifier is"))
    }

    /// Refuses a `_contained=true|both` search carrying a criterion
    /// [`Self::build_contained`] cannot apply, naming it (#1363). Such
    /// criteria used to be skipped, so the search answered a wider question
    /// than the one asked. A no-op for `_contained=false`.
    ///
    /// `_has` and `_list` live outside `query.parameters` and select
    /// *top-level* resources, which a contained resource never is: nothing
    /// outside its container can reference it. They are refused too (#1383),
    /// and so is `_sort`, which this path would otherwise ignore (#1407).
    pub fn reject_unsupported_contained(query: &SearchQuery) -> Result<(), SearchError> {
        if query.contained == ContainedMode::Off {
            return Ok(());
        }
        for (present, name) in [
            (!query.reverse_chains.is_empty(), "_has"),
            (!query.list.is_empty(), "_list"),
        ] {
            if present {
                return Err(SearchError::QueryParseError {
                    message: format!(
                        "'{name}' cannot be combined with _contained=true or both: it selects \
                         top-level resources, which a contained resource is not"
                    ),
                });
            }
        }
        // `_sort` orders by the *contained* resource's values, which live on
        // index rows this path only groups — it lists matches by container
        // type, id and local id, and `_contained=both` appends them to the
        // top-level page. Returning that order for a `_sort` the client asked
        // for is the silent ignore #1363 rules out, so it is refused (#1407).
        if !query.sort.is_empty() {
            return Err(SearchError::QueryParseError {
                message: "'_sort' cannot be combined with _contained=true or both: sorting \
                          contained matches is not supported on SQLite"
                    .to_string(),
            });
        }
        for param in &query.parameters {
            if let Some(reason) = Self::contained_unsupported_reason(param) {
                let message = format!(
                    "search parameter '{}' cannot be combined with _contained=true or both: \
                     {reason} not supported for contained resources on SQLite",
                    param.name
                );
                return Err(match param.param_type {
                    SearchParamType::Composite => SearchError::InvalidComposite { message },
                    _ => SearchError::QueryParseError { message },
                });
            }
        }
        Ok(())
    }

    /// Builds the compartment-membership subquery: matches resources that
    /// reference `comp.reference` via ANY of `comp.params` (logical OR), which
    /// is how a resource joins a FHIR compartment. Reference matching mirrors the
    /// standard reference handler (version-agnostic). Returns `None` if there are
    /// no membership params or no reference.
    fn build_compartment_condition(
        &self,
        comp: &CompartmentMembership,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return None;
        }

        // Membership params come from the bundled FHIR CompartmentDefinitions
        // (trusted, e.g. "patient"), but bind nothing user-controlled here: build
        // a safe `IN (...)` list by escaping any single quotes defensively.
        let in_list = comp
            .params
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");

        let base = strip_reference_version(&comp.reference);
        let p1 = param_offset + 1;
        let p2 = param_offset + 2;

        Some(SqlFragment::with_params(
            format!(
                "resource_key IN (SELECT resource_key FROM search_index \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name IN ({in_list}) \
                 AND value_reference IS NOT NULL \
                 AND (value_reference = ?{p1} OR value_reference LIKE ?{p2} || '/_history/%'))"
            ),
            vec![SqlParam::string(base), SqlParam::string(base)],
        ))
    }

    /// Builds a condition for a single search parameter.
    fn build_parameter_condition(
        &self,
        param: &SearchParameter,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if param.values.is_empty() {
            return None;
        }

        // `:missing` is a resource-level presence test. Resolve it before the
        // composite and ordinary value paths so it is not wrapped in a subquery
        // that simultaneously requires an index row for the same parameter.
        if is_missing_modifier(&param.modifier) {
            let is_missing = get_missing_value(&param.values[0].value);
            return Some(build_missing_condition(param, is_missing));
        }

        // Defence in depth behind `validate_value_presence` (#1380): an empty
        // value is a prefix of every string, so it matches nothing here rather
        // than whatever the handler below would make of it — the whole
        // parameter, since under `:not` "nothing" negates into "everything".
        if crate::search::has_empty_value(param) {
            return Some(SqlFragment::new("1 = 0"));
        }

        // Handle special parameters. `_tag`/`_profile`/`_security`/`_source`/
        // `_language` are NOT special on the query side: the extractor indexes
        // them from `meta` (and, for `_language`, from `Resource.language`)
        // like any typed parameter — rows in search_index under their own
        // param_name — so they take the regular token/uri path below. Routing
        // them into the special handler silently dropped the condition and
        // returned the unfiltered result set (#474). `_language` is indexed
        // only on R5/R6, where the spec first defines it; on R4/R4B it is not
        // a registered parameter and never reaches here.
        //
        // Anything added to this list must have a working regular path, and
        // anything left off it must have an arm in
        // `build_special_parameter_condition` — the `_ => None` fallback there
        // drops the filter rather than narrowing it.
        if param.name.starts_with('_')
            && !matches!(
                param.name.as_str(),
                "_tag" | "_profile" | "_security" | "_source" | "_language"
            )
        {
            return self.build_special_parameter_condition(param, param_offset);
        }

        // Composite parameters need a group-aware subquery (see below).
        if matches!(param.param_type, SearchParamType::Composite) {
            return self.build_composite_parameter_condition(param, param_offset);
        }

        // A plain multi-value reference search (all `Type/id`, no modifier)
        // collapses to a single index-friendly `value_reference IN (...)`
        // instead of ORing one `(= OR range)` branch per value (#1052). The
        // OR-of-branches did two `idx_search_reference` probes per value and
        // nested one level per term — ~18 ms/value (250 refs ≈ 4.6 s) and the
        // parse-depth blow-up behind #943; a flat `IN` is a single indexed
        // lookup of any length at ~1 node. Version-stripped bases are matched
        // exactly, which covers every unversioned stored reference (references
        // are stored unversioned in practice); the single-value path below
        // keeps the full `_history` range match for the rare versioned case.
        let is_plain_multi_reference = matches!(param.param_type, SearchParamType::Reference)
            && param.modifier.is_none()
            && param.values.len() >= 2
            && param
                .values
                .iter()
                .all(|v| strip_reference_version(&v.value).contains('/'));

        let combined = if is_plain_multi_reference {
            let mut params = Vec::with_capacity(param.values.len());
            let placeholders: Vec<String> = param
                .values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    params.push(SqlParam::string(strip_reference_version(&v.value)));
                    format!("?{}", param_offset + i + 1)
                })
                .collect();
            SqlFragment::with_params(
                format!("value_reference IN ({})", placeholders.join(", ")),
                params,
            )
        } else {
            // Multiple values are ORed together
            let mut or_conditions = Vec::new();
            let mut total_params = 0usize;

            for value in &param.values {
                let condition =
                    self.build_value_condition(param, value, param_offset + total_params);
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
            combined
        };

        // Wrap in subquery to ensure proper AND/OR semantics. `:not` negates
        // HERE, at the resource level, not inside the row predicate (#473):
        // FHIR's :not means "no value of the parameter matches", so a
        // multi-valued resource must not slip through via its other rows, and
        // resources with no rows for the parameter count as matches — both of
        // which NOT IN gives and a row-level NOT cannot.
        let membership = if matches!(param.modifier, Some(SearchModifier::Not)) {
            "NOT IN"
        } else {
            "IN"
        };
        Some(SqlFragment::with_params(
            format!(
                "resource_key {} (SELECT resource_key FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}' AND ({}))",
                membership, param.name, combined.sql
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
                        "resource_key IN (SELECT resource_key FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = '{}' GROUP BY resource_key, composite_group HAVING {})",
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
                // _id searches directly on the resources table. Build a flat
                // `id IN (?, ?, ...)` rather than a chain of `id = ? OR id = ?`:
                // a nested OR of N terms parses to a tree of depth N, and SQLite
                // refuses to prepare past depth 1000, so a chained/`_has`
                // resolution that injects thousands of ids as an `_id` filter
                // used to 500 (#943). An `IN` predicate avoids that depth
                // limit; wide lists still need to respect the bind limit.
                if param.values.is_empty() {
                    return None;
                }
                // SQLite limits the number of bound variables in a statement.
                // A chain can resolve to more ids than that limit, so pass wide
                // sets as one JSON value and expand it inside SQLite.
                let (id_set, params) = if param.values.len() > LARGE_ID_SET_THRESHOLD {
                    let ids: Vec<&str> = param
                        .values
                        .iter()
                        .map(|value| value.value.as_str())
                        .collect();
                    let json = serde_json::to_string(&ids).expect("string ids serialize to JSON");
                    (
                        format!("SELECT value FROM json_each(?{})", param_offset + 1),
                        vec![SqlParam::String(json)],
                    )
                } else {
                    let params = param
                        .values
                        .iter()
                        .map(|value| SqlParam::string(&value.value))
                        .collect();
                    let placeholders = (1..=param.values.len())
                        .map(|i| format!("?{}", param_offset + i))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (placeholders, params)
                };

                // `_id` is dispatched here rather than through the generic
                // `:not` handling in `build_parameter_condition` (the
                // `membership`/`NOT IN` wrapping above), so that handling
                // never sees it and `_id:not` used to build the exact same
                // `IN (...)` as a bare `_id`, returning precisely the
                // resource the caller asked to exclude (#1092). The outer
                // query enumerates `search_index` rows, so negating the
                // membership test here yields every indexed resource of the
                // type except the listed ids — the same semantics the
                // generic path uses for every other parameter. Any modifier
                // other than `:not` (`:missing` is resolved earlier, in
                // `build_parameter_condition`) is rejected before this point
                // by each backend's search entry point, so it is not handled
                // here.
                let not_kw = if matches!(param.modifier, Some(SearchModifier::Not)) {
                    "NOT "
                } else {
                    ""
                };

                Some(SqlFragment::with_params(
                    format!(
                        "resource_key {not_kw}IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN ({}))",
                        id_set
                    ),
                    params,
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
                // Not "fall through to regular handling", as this comment used
                // to claim: the caller returns whatever this yields, so `None`
                // *drops* the parameter and the search answers with every
                // resource of the type. That is how `_tag`/`_profile`/
                // `_security`/`_source` came to be unfiltered (#474). Anything
                // that must actually filter belongs in an arm above, or in the
                // caller's exclusion list so it never reaches here.
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

        // Numbered off the running count of *accepted* terms rather than the
        // loop index: escaping can empty a term (`_text=*` escapes to nothing),
        // and the caller binds this fragment's params consecutively from
        // `param_offset`, so a skipped value must not leave a placeholder gap.
        // `_text=*,fracture` used to emit `?4` while binding only one param,
        // which SQLite rejects at prepare time.
        let mut param_num = param_offset;

        for value in values {
            // Escape and prepare the search term
            let search_term = Fts5Search::escape_fts_query(&value.value);
            if search_term.is_empty() {
                continue;
            }

            // Build the FTS match query
            // Use the column prefix to search only the specified column
            param_num += 1;
            // `tenant_id = ?1` is not optional. Without it this sub-select
            // matches *every* tenant's `resource_fts` rows and yields a bare
            // `resource_id` set, which the outer query then intersects with this
            // tenant's resources: tenant A's Patient/123 comes back because
            // tenant B's Patient/123 contained the search term. That is a
            // cross-tenant match oracle, and it is exactly the discriminator
            // that `BackendCapability::SharedSchema` promises every query
            // carries. `?1` is the tenant in every param layout this builder
            // produces (see `with_param_offset`), so reusing it adds no binding.
            conditions.push(SqlFragment::with_params(
                format!(
                    "resource_key IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN (SELECT resource_id FROM resource_fts WHERE tenant_id = ?1 AND {} MATCH ?{}))",
                    column, param_num
                ),
                vec![SqlParam::string(&search_term)],
            ));
        }

        if conditions.is_empty() {
            // Every term escaped to nothing. Returning `None` would drop the
            // parameter and answer a full-text query with every resource of the
            // type; a term that cannot match must match nothing instead.
            return Some(SqlFragment::new("0"));
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

        // Advanced by the binds actually made: the match-nothing fragment for
        // a value that is not a date binds none.
        let mut offset = param_offset;
        for value in values {
            // `_lastUpdated` is an instant on the resource row: a point
            // comparison, not the range a date parameter's index row holds.
            let cond = DateHandler::build_point_sql("last_updated", value, offset);
            if !cond.is_empty() {
                offset += cond.params.len();
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
                "resource_key IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND ({}))",
                combined.sql
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
                    // A malformed `_filter` must NOT be silently dropped — that
                    // would return an unfiltered superset (everything matching
                    // the other params). Fail closed: emit a match-nothing
                    // condition so the client gets zero results, not wrong ones.
                    tracing::warn!(
                        "Failed to parse _filter expression '{}': {} — failing closed (no matches)",
                        value.value,
                        e
                    );
                    conditions.push(SqlFragment::new("1 = 0"));
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
                let (agg, col) = match directive.direction {
                    crate::types::SortDirection::Ascending => ("MIN", col),
                    // A date row is a range (#1391): descending sorts on where
                    // the latest one ends, not where it starts.
                    crate::types::SortDirection::Descending if col == "value_date" => {
                        ("MAX", "value_date_end")
                    }
                    crate::types::SortDirection::Descending => ("MAX", col),
                };
                format!(
                    "(SELECT {}({}) FROM search_index si WHERE si.tenant_id = ?1 AND si.resource_type = ?2 AND si.resource_key = resources.rowid AND si.param_name = '{}')",
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
    fn fts_placeholders_are_gap_free() {
        // `*` escapes to nothing and binds no parameter, so the term after it
        // must still take the next consecutive placeholder. The old index-based
        // numbering emitted `?4` while binding a single param, and SQLite
        // rejects the prepared statement.
        let builder = QueryBuilder::new("tenant1", "Patient");
        let values = vec![SearchValue::eq("*"), SearchValue::eq("fracture")];

        let frag = builder
            .build_fts_condition(&values, "narrative_text", 2)
            .expect("a matchable term must produce a condition");

        assert_eq!(frag.params.len(), 1);
        assert!(frag.sql.contains("?3"), "{}", frag.sql);
        assert!(!frag.sql.contains("?4"), "{}", frag.sql);
    }

    #[test]
    fn fts_unmatchable_term_fails_closed() {
        // Nothing survives escaping. Dropping the parameter would answer a
        // full-text query with every resource of the type.
        let builder = QueryBuilder::new("tenant1", "Patient");
        let values = vec![SearchValue::eq("*")];

        let frag = builder
            .build_fts_condition(&values, "narrative_text", 2)
            .expect("an unmatchable term must still constrain the query");

        assert_eq!(frag.sql, "0");
        assert!(frag.params.is_empty());
    }

    /// Every indexed `_`-prefixed parameter must produce a real condition.
    ///
    /// `build_special_parameter_condition`'s `_ => None` arm *drops* the
    /// parameter rather than narrowing it, so a search whose only filter is a
    /// missing arm answers with every resource of the type — the #474 failure
    /// mode. `_language` (R5/R6) is indexed by the extractor exactly like the
    /// `meta` set and so belongs on the regular token path with them; before
    /// this it fell through to the `None` arm.
    #[test]
    fn indexed_meta_parameters_are_not_dropped() {
        let builder = QueryBuilder::new("tenant1", "Patient");

        for (name, param_type, value) in [
            ("_language", SearchParamType::Token, "en-US"),
            ("_source", SearchParamType::Uri, "http://example.org/src"),
            ("_tag", SearchParamType::Token, "http://sys|t1"),
            ("_profile", SearchParamType::Uri, "http://example.org/sd"),
            ("_security", SearchParamType::Token, "http://sys|R"),
        ] {
            let param = SearchParameter {
                name: name.to_string(),
                param_type,
                modifier: None,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![],
            };

            let fragment = builder
                .build_parameter_condition(&param, 2)
                .unwrap_or_else(|| {
                    panic!("{name} produced no condition — the filter would be dropped")
                });
            assert!(
                fragment.sql.contains(&format!("param_name = '{name}'")),
                "{name} should read its own search_index rows, got: {}",
                fragment.sql
            );
        }
    }

    /// `_in` never reaches the index — nothing writes rows for it, and the
    /// SQLite special-parameter path has no arm for it, so a query naming it
    /// would silently return the whole type. `helios-rest` rejects it before
    /// the backend is asked; this pins the backend half of that contract so a
    /// future "just let it through" change has to confront the drop. Update it
    /// when membership resolution lands (#638).
    #[test]
    fn membership_parameter_has_no_backend_condition() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let param = SearchParameter {
            name: "_in".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("List/42")],
            chain: vec![],
            components: vec![],
        };

        assert!(
            builder.build_parameter_condition(&param, 2).is_none(),
            "_in must not be answered by the index; it is rejected at the REST layer"
        );
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
    fn missing_parameter_bypasses_generic_value_wrapper() {
        let builder = QueryBuilder::new("tenant1", "Patient");

        for (value, membership) in [("true", "NOT IN"), ("false", "IN")] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![],
            });

            let fragment = builder.build(&query);

            assert!(
                fragment.sql.contains(&format!("resource_key {membership}")),
                "{value}: {}",
                fragment.sql
            );
            assert_eq!(
                fragment.sql.matches("param_name = 'birthdate'").count(),
                1,
                ":missing must not be wrapped in a second birthdate membership query: {}",
                fragment.sql
            );
            assert!(
                fragment.sql.contains("is_contained = 0"),
                "contained rows must not establish top-level presence: {}",
                fragment.sql
            );
        }
    }

    /// #1092: `_id` is dispatched through `build_special_parameter_condition`,
    /// which bypassed the generic `:not` handling in `build_parameter_condition`
    /// (the `membership`/`NOT IN` wrapping just above) entirely, so `_id:not=a`
    /// built the same `IN (...)` as a plain `_id=a` and returned exactly the
    /// resource the caller asked to exclude.
    fn id_param_string(value: &SqlParam) -> &str {
        match value {
            SqlParam::String(s) => s.as_str(),
            other => panic!("expected a string param, got {:?}", other),
        }
    }

    #[test]
    fn id_no_modifier_control_is_a_positive_match() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let param = SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        };

        let fragment = builder
            .build_parameter_condition(&param, 2)
            .expect("_id must produce a condition");

        assert_eq!(
            fragment.sql,
            "resource_key IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN (?3))"
        );
        assert_eq!(fragment.params.len(), 1);
        assert_eq!(id_param_string(&fragment.params[0]), "a");
    }

    #[test]
    fn id_not_single_value_excludes_instead_of_matching() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let param = SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        };

        let fragment = builder
            .build_parameter_condition(&param, 2)
            .expect("_id:not must produce a condition");

        assert_eq!(
            fragment.sql,
            "resource_key NOT IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN (?3))"
        );
        assert_eq!(fragment.params.len(), 1);
        assert_eq!(id_param_string(&fragment.params[0]), "a");
    }

    #[test]
    fn id_not_two_values_excludes_both() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let param = SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a"), SearchValue::eq("b")],
            chain: vec![],
            components: vec![],
        };

        let fragment = builder
            .build_parameter_condition(&param, 2)
            .expect("_id:not must produce a condition");

        assert_eq!(
            fragment.sql,
            "resource_key NOT IN (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id IN (?3, ?4))"
        );
        assert_eq!(fragment.params.len(), 2);
        assert_eq!(id_param_string(&fragment.params[0]), "a");
        assert_eq!(id_param_string(&fragment.params[1]), "b");
    }

    #[test]
    fn large_id_set_uses_one_json_bind() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let mut values: Vec<SearchValue> = (0..32_767)
            .map(|i| SearchValue::eq(format!("id-{i}")))
            .collect();
        values.push(SearchValue::eq("a\"b"));
        let param = SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values,
            ..Default::default()
        };

        let fragment = builder.build_parameter_condition(&param, 2).unwrap();
        assert!(
            fragment
                .sql
                .contains("id IN (SELECT value FROM json_each(?3))")
        );
        assert!(fragment.sql.starts_with("resource_key NOT IN"));
        assert_eq!(fragment.params.len(), 1);
        let ids: Vec<String> = serde_json::from_str(id_param_string(&fragment.params[0])).unwrap();
        assert_eq!(ids.len(), 32_768);
        assert_eq!(ids.last().unwrap(), "a\"b");
    }

    #[test]
    fn missing_parameter_without_values_is_ignored() {
        let builder = QueryBuilder::new("tenant1", "Patient");
        let param = SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Missing),
            values: vec![],
            chain: vec![],
            components: vec![],
        };

        assert!(builder.build_parameter_condition(&param, 2).is_none());
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

    #[test]
    fn test_multi_value_typed_reference_uses_flat_in() {
        // A plain multi-value `Type/id` reference collapses to one flat
        // `value_reference IN (...)` — a single indexed lookup, not an OR of
        // per-value range branches (#1052).
        let builder = QueryBuilder::new("default", "Observation");
        let mut query = SearchQuery::new("Observation");
        query.parameters.push(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::eq("Patient/a"),
                SearchValue::eq("Patient/b"),
                SearchValue::eq("Patient/c"),
            ],
            chain: vec![],
            components: vec![],
        });

        let fragment = builder.build(&query);
        assert!(
            fragment.sql.contains("value_reference IN (?3, ?4, ?5)"),
            "expected a flat IN list, got: {}",
            fragment.sql
        );
        // No per-value OR branches, and no `_history` range scan.
        assert!(
            !fragment.sql.contains(" OR "),
            "should not OR: {}",
            fragment.sql
        );
        assert!(
            !fragment.sql.contains("_history"),
            "multi-value IN path is exact-base only: {}",
            fragment.sql
        );
        // tenant, resource_type, + one base per value
        assert_eq!(fragment.params.len(), 5);
    }

    #[test]
    fn test_multi_value_typed_reference_strips_versions() {
        // Each search value is version-stripped to its base before going into
        // the IN list, so a versioned search value still matches the base.
        let builder = QueryBuilder::new("default", "Observation");
        let mut query = SearchQuery::new("Observation");
        query.parameters.push(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::eq("Patient/a/_history/2"),
                SearchValue::eq("Patient/b"),
            ],
            chain: vec![],
            components: vec![],
        });

        let fragment = builder.build(&query);
        assert!(
            fragment.sql.contains("value_reference IN (?3, ?4)"),
            "{}",
            fragment.sql
        );
        let bound: Vec<String> = fragment
            .params
            .iter()
            .filter_map(|p| match p {
                SqlParam::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(
            bound.contains(&"Patient/a".to_string()),
            "version not stripped: {bound:?}"
        );
        assert!(bound.contains(&"Patient/b".to_string()), "bound: {bound:?}");
    }

    #[test]
    fn test_multi_value_bare_id_reference_keeps_or_path() {
        // Bare ids (no `Type/`) cannot use the base-equality IN — they still
        // need the `%/id` suffix match — so they keep the generic OR path.
        let builder = QueryBuilder::new("default", "Immunization");
        let mut query = SearchQuery::new("Immunization");
        query.parameters.push(SearchParameter {
            name: "patient".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("p1"), SearchValue::eq("p2")],
            chain: vec![],
            components: vec![],
        });
        let fragment = builder.build(&query);
        assert!(
            !fragment.sql.contains("value_reference IN (?3, ?4)"),
            "bare ids must not take the flat-IN path: {}",
            fragment.sql
        );
        assert!(
            fragment.sql.contains(" OR "),
            "bare-id path still ORs: {}",
            fragment.sql
        );
    }

    fn contained_param(
        name: &str,
        ty: SearchParamType,
        modifier: Option<SearchModifier>,
        values: &[&str],
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: ty,
            modifier,
            values: values.iter().map(|v| SearchValue::parse(v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    fn contained_query(parameters: Vec<SearchParameter>) -> SearchQuery {
        let mut query = SearchQuery::new("Observation");
        query.contained = ContainedMode::On;
        query.parameters = parameters;
        query
    }

    /// The `?N` numbers in `sql`, in order of appearance.
    fn numbered_placeholders(sql: &str) -> Vec<usize> {
        sql.split('?')
            .skip(1)
            .filter_map(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            })
            .collect()
    }

    #[test]
    fn contained_distinct_names_count_names() {
        // Distinct names: the name count proves every branch matched, and the
        // SQL is what it was before occurrences were told apart (#1362).
        let query = contained_query(vec![
            contained_param("code", SearchParamType::Token, None, &["X"]),
            contained_param("date", SearchParamType::Date, None, &["ge2020-01-01"]),
        ]);
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&query)
            .unwrap();

        assert!(
            frag.sql.starts_with(
                "SELECT resource_type, resource_id, contained_local_id FROM search_index \
                 WHERE tenant_id = ?1 AND is_contained = 1 AND contained_type = ?2 AND \
                 ((param_name = 'code' AND ("
            ),
            "{}",
            frag.sql
        );
        assert!(
            frag.sql.ends_with(
                " GROUP BY resource_type, resource_id, contained_local_id \
                 HAVING COUNT(DISTINCT param_name) >= 2"
            ),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("MAX("), "{}", frag.sql);
    }

    #[test]
    fn contained_repeated_name_requires_every_occurrence() {
        // `code=X&date=ge2020-01-01&date=le2020-12-31,2019`: a name count of 2
        // is met by a contained resource matching only one date bound (#1362).
        let query = contained_query(vec![
            contained_param("code", SearchParamType::Token, None, &["X"]),
            contained_param("date", SearchParamType::Date, None, &["ge2020-01-01"]),
            contained_param(
                "date",
                SearchParamType::Date,
                None,
                &["le2020-12-31", "2019"],
            ),
        ]);
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&query)
            .unwrap();

        let (filter, having) = frag.sql.split_once(" HAVING ").expect("a HAVING clause");
        assert!(!having.contains("COUNT("), "{having}");
        let required: Vec<&str> = having.split(" AND MAX(CASE WHEN ").collect();
        assert_eq!(required.len(), 3, "one aggregate per occurrence: {having}");
        assert!(required[0].starts_with("MAX(CASE WHEN (param_name = 'code'"));
        assert!(required[1].starts_with("(param_name = 'date'"), "{having}");
        assert!(required[2].starts_with("(param_name = 'date'"), "{having}");
        assert!(
            required
                .iter()
                .all(|r| r.ends_with("THEN 1 ELSE 0 END) = 1")),
            "{having}"
        );

        // HAVING re-reads the WHERE placeholders: gap-free from ?3, none new.
        // (A year-precision date names its one placeholder twice.)
        let in_filter = numbered_placeholders(filter);
        let mut distinct = in_filter.clone();
        distinct.dedup();
        let mut expected: Vec<usize> = vec![1, 2];
        expected.extend(3..3 + frag.params.len());
        assert_eq!(distinct, expected, "{}", frag.sql);
        assert_eq!(
            numbered_placeholders(having),
            in_filter[2..],
            "{}",
            frag.sql
        );
    }

    #[test]
    fn contained_negation_is_decided_over_every_row_of_the_entity() {
        // `:not` means "no row of this contained resource matches", which a
        // WHERE row filter would make unanswerable (#1363).
        let query = contained_query(vec![
            contained_param("code", SearchParamType::Token, None, &["X"]),
            contained_param(
                "category",
                SearchParamType::Token,
                Some(SearchModifier::Not),
                &["cat1"],
            ),
        ]);
        assert!(QueryBuilder::reject_unsupported_contained(&query).is_ok());
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&query)
            .unwrap();

        let (filter, having) = frag.sql.split_once(" HAVING ").expect("a HAVING clause");
        assert!(!filter.contains("param_name"), "{filter}");
        let required: Vec<&str> = having.split(" AND MAX(").collect();
        assert_eq!(required.len(), 2, "{having}");
        assert!(
            required[0].contains("'code'") && required[0].ends_with("END) = 1"),
            "{having}"
        );
        assert!(
            required[1].contains("'category'") && required[1].ends_with("END) = 0"),
            "{having}"
        );
        assert_eq!(numbered_placeholders(&frag.sql), vec![1, 2, 3, 4]);
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn contained_id_is_the_local_id_and_needs_no_other_criterion() {
        let id = contained_param("_id", SearchParamType::Token, None, &["a", "b"]);
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&contained_query(vec![id.clone()]))
            .unwrap();
        assert!(
            frag.sql.ends_with(
                "contained_type = ?2 AND contained_local_id IN (?3, ?4) \
                 GROUP BY resource_type, resource_id, contained_local_id"
            ),
            "{}",
            frag.sql
        );

        // Beside a value parameter the numbering carries on after the ids.
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&contained_query(vec![
                id,
                contained_param("code", SearchParamType::Token, None, &["X"]),
            ]))
            .unwrap();
        assert_eq!(numbered_placeholders(&frag.sql), vec![1, 2, 3, 4, 5]);
        assert_eq!(frag.params.len(), 3);
        assert!(
            frag.sql.contains("HAVING COUNT(DISTINCT param_name) >= 1"),
            "{}",
            frag.sql
        );
    }

    /// #1383: no criterion is every contained resource of the type, and
    /// compartment membership is one explicit branch over several names.
    #[test]
    fn contained_without_criteria_and_with_a_compartment() {
        let builder = QueryBuilder::new("t", "Observation");
        let frag = builder.build_contained(&contained_query(vec![])).unwrap();
        assert!(
            frag.sql.ends_with(
                "contained_type = ?2 GROUP BY resource_type, resource_id, contained_local_id"
            ),
            "{}",
            frag.sql
        );
        assert!(frag.params.is_empty());

        let mut query = contained_query(vec![contained_param(
            "code",
            SearchParamType::Token,
            None,
            &["X"],
        )]);
        query.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1/_history/2".to_string(),
        });
        let frag = builder.build_contained(&query).unwrap();
        // The `HAVING` reuses the `WHERE` placeholders; none is bound twice.
        let filter = frag.sql.split(" HAVING ").next().unwrap();
        assert_eq!(numbered_placeholders(filter), vec![1, 2, 3, 4, 5]);
        assert_eq!(frag.params.len(), 3);
        let having = frag.sql.split(" HAVING ").nth(1).expect(&frag.sql);
        // Counting names would let `subject` stand in for `code`.
        assert!(!having.contains("COUNT(DISTINCT"), "{having}");
        assert!(
            having.contains(
                "MAX(CASE WHEN (param_name IN ('subject', 'performer') AND value_reference IS NOT NULL"
            ),
            "{having}"
        );
        assert!(
            matches!(&frag.params[1], SqlParam::String(s) if s == "Patient/p1"),
            "{:?}",
            frag.params
        );
    }

    /// #1383: `_has` and `_list` select top-level resources.
    #[test]
    fn contained_refuses_has_and_list_by_name() {
        let mut has = contained_query(vec![]);
        has.reverse_chains
            .push(crate::types::ReverseChainedParameter::terminal(
                "Provenance",
                "target",
                "agent",
                SearchValue::eq("Practitioner/x"),
            ));
        let mut list = contained_query(vec![]);
        list.list.push("l1".to_string());
        // `_sort` would be ignored by the contained path, so it is refused too
        // (#1407).
        let mut sorted = contained_query(vec![]);
        sorted
            .sort
            .push(crate::types::SortDirective::parse("-date"));
        for (query, name) in [(has, "'_has'"), (list, "'_list'"), (sorted, "'_sort'")] {
            let message = QueryBuilder::reject_unsupported_contained(&query)
                .unwrap_err()
                .to_string();
            assert!(message.contains(name), "{message}");
            let mut off = query.clone();
            off.contained = ContainedMode::Off;
            assert!(QueryBuilder::reject_unsupported_contained(&off).is_ok());
        }
    }

    #[test]
    fn contained_meta_parameters_are_ordinary_branches() {
        let query = contained_query(vec![
            contained_param("_tag", SearchParamType::Token, None, &["foo"]),
            contained_param("_profile", SearchParamType::Uri, None, &["http://p"]),
        ]);
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&query)
            .unwrap();
        assert!(frag.sql.contains("param_name = '_tag'"), "{}", frag.sql);
        assert!(frag.sql.contains("param_name = '_profile'"), "{}", frag.sql);
        assert!(QueryBuilder::reject_unsupported_contained(&query).is_ok());
    }

    /// A composite narrows the contained entities by a per-`composite_group`
    /// pairing (#1407); its placeholders continue the numbering.
    #[test]
    fn contained_composite_is_paired_per_group_of_the_entity() {
        let mut composite = contained_param(
            "code-value-quantity",
            SearchParamType::Composite,
            None,
            &["X$gt5"],
        );
        composite.components = vec![
            crate::types::CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "code".to_string(),
            },
            crate::types::CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".to_string(),
            },
        ];
        let query = contained_query(vec![
            contained_param("status", SearchParamType::Token, None, &["final"]),
            composite,
        ]);
        assert!(QueryBuilder::reject_unsupported_contained(&query).is_ok());
        let frag = QueryBuilder::new("t", "Observation")
            .build_contained(&query)
            .unwrap();
        assert!(
            frag.sql.contains(
                "AND ((resource_type, resource_id, contained_local_id) IN (SELECT resource_type, \
                 resource_id, contained_local_id FROM search_index WHERE tenant_id = ?1 AND \
                 is_contained = 1 AND contained_type = ?2 AND param_name = 'code-value-quantity' \
                 GROUP BY resource_type, resource_id, contained_local_id, composite_group HAVING "
            ),
            "{}",
            frag.sql
        );
        // `status` binds ?3; the composite's components follow, gap-free.
        let highest = (3..=frag.params.len() + 2).all(|n| frag.sql.contains(&format!("?{n}")));
        assert!(highest, "{} / {} params", frag.sql, frag.params.len());
        assert!(!frag.sql.contains(&format!("?{}", frag.params.len() + 3)));
    }

    #[test]
    fn contained_refuses_what_it_cannot_apply_by_name() {
        let mut chained = contained_param("subject", SearchParamType::Reference, None, &["x"]);
        chained.chain = vec![crate::types::ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }];
        let refused = [
            contained_param("_lastUpdated", SearchParamType::Date, None, &["gt2020"]),
            contained_param("_text", SearchParamType::Special, None, &["x"]),
            contained_param(
                "_id",
                SearchParamType::Token,
                Some(SearchModifier::Not),
                &["a"],
            ),
            contained_param(
                "code-value-quantity",
                SearchParamType::Composite,
                None,
                &["X$5"],
            ),
            contained_param(
                "code",
                SearchParamType::Token,
                Some(SearchModifier::In),
                &["vs"],
            ),
            contained_param(
                "date",
                SearchParamType::Date,
                Some(SearchModifier::Missing),
                &["true"],
            ),
            contained_param(
                "code",
                SearchParamType::Token,
                Some(SearchModifier::TextAdvanced),
                &["x"],
            ),
            chained,
        ];
        for param in refused {
            let name = param.name.clone();
            let mut query = contained_query(vec![
                contained_param("status", SearchParamType::Token, None, &["final"]),
                param,
            ]);
            let error = QueryBuilder::reject_unsupported_contained(&query)
                .expect_err(&name)
                .to_string();
            assert!(error.contains(&format!("'{name}'")), "{error}");

            // The same query without `_contained` is none of this gate's business.
            query.contained = ContainedMode::Off;
            assert!(QueryBuilder::reject_unsupported_contained(&query).is_ok());
        }
    }

    /// #1380: `family=Zzz,` reached the builder as the values `Zzz` and `""`,
    /// and a prefix match on `""` is every row. The search gate
    /// (`validate_value_presence`) rejects it before a query is built; if one
    /// is built anyway, the parameter matches nothing — the whole parameter, or
    /// `:not` would negate it into everything.
    #[test]
    fn a_parameter_with_an_empty_value_matches_nothing() {
        use SearchModifier as M;
        use SearchParamType as T;
        let cases: Vec<(&str, SearchParamType, Option<SearchModifier>, Vec<&str>)> = vec![
            ("family", T::String, None, vec!["Zzz", ""]),
            ("family", T::String, None, vec![""]),
            ("family", T::String, Some(M::Contains), vec!["", "Zzz"]),
            ("family", T::String, Some(M::Text), vec![" "]),
            ("gender", T::Token, None, vec![""]),
            ("gender", T::Token, Some(M::Not), vec!["female", ""]),
            ("gender", T::Token, Some(M::Text), vec![""]),
            ("identifier", T::Token, Some(M::OfType), vec![""]),
            ("_id", T::Token, None, vec!["a", ""]),
            ("_tag", T::Token, None, vec![""]),
            ("general-practitioner", T::Reference, None, vec![""]),
            ("url", T::Uri, Some(M::Below), vec![""]),
            ("url", T::Uri, Some(M::Contains), vec!["", "x"]),
        ];
        for (name, param_type, modifier, values) in cases {
            let context = format!("{name} {modifier:?} {values:?}");
            let param = SearchParameter {
                name: name.to_string(),
                param_type,
                modifier,
                values: values.into_iter().map(SearchValue::eq).collect(),
                chain: vec![],
                components: vec![],
            };
            let fragment = QueryBuilder::new("tenant1", "Patient")
                .build_parameter_condition(&param, 2)
                .unwrap_or_else(|| panic!("{context}: a dropped condition matches everything"));
            assert_eq!(fragment.sql, "1 = 0", "{context}");
            assert!(fragment.params.is_empty(), "{context}");
        }
    }
}
