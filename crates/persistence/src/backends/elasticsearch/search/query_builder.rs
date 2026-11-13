//! Elasticsearch Query DSL builder.
//!
//! Translates FHIR `SearchQuery` into Elasticsearch Query DSL JSON.

use serde_json::{Value, json};

use crate::error::SearchError;
use crate::types::{
    CompartmentMembership, CursorDirection, PageCursor, SearchModifier, SearchParamType,
    SearchParameter, SearchPrefix, SearchQuery, SearchValue, SortDirection, SortDirective,
    TotalMode, strip_reference_version,
};

/// Upper bound on hits counted for `_total=estimate`; beyond it Elasticsearch
/// reports a lower bound (`relation: "gte"`), so `Bundle.total` for
/// `_total=estimate` is exact up to 10000 matches and capped at 10000 above
/// that. Matches the ES default.
const ESTIMATE_TRACK_TOTAL_HITS: u32 = 10_000;

use super::fts;
use super::modifier_handlers;
use super::parameter_handlers::{composite, date, number, quantity, reference, string, token, uri};

/// The `_source` fields a resource-returning search needs from each hit.
///
/// This must stay in step with what `parse_hit_to_stored_resource`
/// (`search_impl.rs`) and `parse_stored_resource` (`storage.rs`) read. The
/// remaining indexed fields (`content_text`, `narrative_text`, `search_params`,
/// ...) are only used to match, never to build the result, so they are not
/// requested.
pub(crate) const HIT_SOURCE_FIELDS: &[&str] = &[
    "resource_type",
    "resource_id",
    "version_id",
    "content",
    "fhir_version",
    "last_updated",
];

/// A complete Elasticsearch query body ready to be sent.
#[derive(Debug, Clone)]
pub struct EsQuery {
    /// The complete query body.
    pub body: Value,
    /// The index to search.
    pub index: String,
    /// Whether `size` includes one extra hit beyond `count`. False only when
    /// `from + count + 1` would exceed `max_result_window`, in which case the
    /// results layer cannot prove a further page from an extra hit (#1079).
    pub over_fetched: bool,
}

/// Returns true if the query contains a relevance-scoring (full-text) clause:
/// the `_text` / `_content` special params, or a `:text` / `:text-advanced`
/// modifier. Such clauses make Elasticsearch's `_score` meaningful.
fn query_has_relevance(query: &SearchQuery) -> bool {
    query.parameters.iter().any(|p| {
        matches!(p.name.as_str(), "_text" | "_content")
            || matches!(
                p.modifier,
                Some(SearchModifier::Text) | Some(SearchModifier::TextAdvanced)
            )
    })
}

/// Returns the effective Elasticsearch sort order (`"asc"`/`"desc"`) for a
/// sort directive's `direction`, swapped when `paging` is
/// `CursorDirection::Previous`. Paging backward means walking the result set
/// in the opposite order, so every criterion — including `mode` and
/// `missing`, which are derived from this order — must flip too (#1015).
fn es_order(direction: SortDirection, paging: CursorDirection) -> &'static str {
    let ascending = match direction {
        SortDirection::Ascending => true,
        SortDirection::Descending => false,
    };
    let ascending = if paging == CursorDirection::Previous {
        !ascending
    } else {
        ascending
    };
    if ascending { "asc" } else { "desc" }
}

/// The refusal for a parameter [`EsQueryBuilder::build`] has values for but no
/// clause to build from (#2510): a `special`-typed parameter (`Location.near`,
/// `_type` on R5/R6, `_filter`), which no handler evaluates and nothing
/// indexes. Skipped, it would leave the query with fewer clauses than were
/// asked for — with none at all, a `bool` of filters only, which matches every
/// document of the tenant and type.
fn unevaluable_parameter(param: &SearchParameter, resource_type: &str) -> SearchError {
    SearchError::QueryParseError {
        message: format!(
            "search parameter '{}' cannot be evaluated by the Elasticsearch backend: no clause \
             can be built for it, and ignoring it would widen the search to every {resource_type}",
            param.name
        ),
    }
}

/// The refusal for one value of a parameter that no clause can be built for
/// while its siblings have one (#2510). Left out of the OR it would change the
/// question: under `:not`, `NOT (a OR b)` would become `NOT a`, a wider answer;
/// without it, `a OR b` would become `a`, a different one.
fn unevaluable_value(param: &SearchParameter, value: &str) -> SearchError {
    SearchError::QueryParseError {
        message: format!(
            "search parameter '{}' cannot be evaluated by the Elasticsearch backend: no clause \
             can be built for the value '{value}', and leaving it out would answer a different \
             search than the one asked",
            param.name
        ),
    }
}

/// Builds Elasticsearch queries from FHIR search queries.
pub struct EsQueryBuilder<'a> {
    tenant_id: &'a str,
    resource_type: &'a str,
    index: String,
    /// The index's `index.max_result_window` setting, used to clamp the
    /// over-fetched `size` so `from + size` never exceeds it (#1079).
    max_result_window: u32,
    /// The index's `index.max_terms_count`: an id list longer than this is
    /// split into several `terms` clauses.
    max_terms_count: u32,
}

impl<'a> EsQueryBuilder<'a> {
    /// Creates a new query builder.
    pub fn new(tenant_id: &'a str, resource_type: &'a str, index: String) -> Self {
        Self {
            tenant_id,
            resource_type,
            index,
            // Same default as `ElasticsearchConfig::max_result_window`
            // (`backend.rs`); overridden via `with_max_result_window` when
            // the caller knows the actual configured window.
            max_result_window: 10_000,
            max_terms_count: 65_536,
        }
    }

    /// Overrides the index's `max_result_window`, used to clamp the
    /// over-fetched `size` so `from + size` stays within the window (#1079).
    pub fn with_max_result_window(mut self, window: u32) -> Self {
        self.max_result_window = window;
        self
    }

    /// Overrides the index's `index.max_terms_count`, the most values one
    /// `terms` clause is given.
    pub fn with_max_terms_count(mut self, count: u32) -> Self {
        self.max_terms_count = count.max(1);
        self
    }

    /// Builds a complete ES query from a FHIR SearchQuery.
    ///
    /// A parameter that carries values but yields no clause is an error
    /// naming it (#2510), never a skipped term: with no `must` clause left the
    /// `bool` query matches every document of the tenant and type. That covers
    /// a parameter nothing can evaluate (a `special`-typed one) and a value of
    /// a parameter that no clause can be built for (see
    /// [`Self::build_parameter_clause`]). A parameter with no values asks for
    /// nothing and is skipped. Every caller must propagate the error: `search`,
    /// `search_ids`, `search_reference_values`, the `_contained` keys and the
    /// `_count` body all build through here.
    pub fn build(&self, query: &SearchQuery) -> Result<EsQuery, SearchError> {
        let mut must_clauses: Vec<Value> = Vec::new();
        let mut filter_clauses: Vec<Value> = vec![
            json!({ "term": { "tenant_id": self.tenant_id } }),
            json!({ "term": { "is_deleted": false } }),
        ];

        // A client-named FHIR version (#2254) narrows the match to documents
        // stored under it. `fhir_version` is a keyword field written on every
        // document (contained ones carry their container's version), so every
        // path built here — paged search, `search_after`, `search_ids`, counts,
        // reference-value aggregations and `_contained` — is filtered alike.
        // A version-blind query adds no clause.
        if let Some(version) = query.stored_fhir_version() {
            filter_clauses.push(json!({ "term": { "fhir_version": version } }));
        }

        // `_contained` mode selects between top-level and contained docs. Contained
        // docs carry `is_contained=true`; top-level docs omit the field, so
        // `must_not term is_contained=true` excludes contained docs without
        // requiring a value on every existing document.
        let mut must_not_clauses: Vec<Value> = Vec::new();
        match query.contained {
            crate::types::ContainedMode::Off => {
                must_not_clauses.push(json!({ "term": { "is_contained": true } }));
            }
            crate::types::ContainedMode::On => {
                filter_clauses.push(json!({ "term": { "is_contained": true } }));
            }
            crate::types::ContainedMode::Both => {}
        }

        // Process each search parameter
        for param in &query.parameters {
            match self.build_parameter_clause(param)? {
                Some(clause) => must_clauses.push(clause),
                // Nothing was asked for, so nothing is constrained.
                None if param.values.is_empty() => {}
                // Something was asked for and cannot be answered. Answering
                // anyway, without it, would be a wider question than the one
                // asked (#2510).
                None => return Err(unevaluable_parameter(param, self.resource_type)),
            }
        }

        // Compartment membership: a resource joins the compartment if it
        // references `comp.reference` via ANY of the membership params (OR),
        // per the FHIR CompartmentDefinition. Modeled as a single nested query
        // over the stored reference search params. A membership that matches
        // nothing still adds its (`match_none`) clause: skipping it would widen
        // the search to every resource of the type (#2509).
        if let Some(comp) = &query.compartment {
            must_clauses.push(Self::build_compartment_clause(comp));
        }

        // Build the bool query
        let mut bool_query = json!({
            "filter": filter_clauses,
        });

        if !must_clauses.is_empty() {
            bool_query["must"] = json!(must_clauses);
        }

        if !must_not_clauses.is_empty() {
            bool_query["must_not"] = json!(must_not_clauses);
        }

        let mut body = json!({
            "query": { "bool": bool_query },
        });

        // Decode the cursor once and reuse it for sort direction, size, and
        // `search_after` — an undecodable cursor is treated as absent
        // everywhere, matching the pre-existing `search_after` behavior.
        let cursor = query
            .cursor
            .as_deref()
            .and_then(|c| PageCursor::decode(c).ok());
        let paging = cursor
            .as_ref()
            .map(PageCursor::direction)
            .unwrap_or_default();

        // Add sorting
        let sort = self.build_sort(&query.sort, paging);
        body["sort"] = sort;

        // Add pagination
        let count = query.count.unwrap_or(20);
        // Both directions over-fetch by one hit so the results layer can tell
        // whether another page exists: forward it proves a next page, backward
        // (#1015) it proves a previous page. The extra hit is dropped there.
        // Elasticsearch rejects `from + size > index.max_result_window`, and
        // internal include queries already ask for exactly that window, so the
        // extra hit is only requested when it fits (#1079).
        let from = if query.cursor.is_some() {
            0
        } else {
            query.offset.unwrap_or(0)
        };
        let room = self.max_result_window.saturating_sub(from);
        let size = (count + 1).min(room.max(1));
        let over_fetched = size > count;
        body["size"] = json!(size);

        if query.cursor.is_some() {
            if let Some(cursor) = cursor.as_ref() {
                body["search_after"] = self.build_search_after(cursor);
            }
        } else if let Some(offset) = query.offset {
            body["from"] = json!(offset);
        }

        // Count hits only when `_total` asks for it: an exact count for
        // `accurate`, a bounded one for `estimate`, none otherwise. Paging does
        // not read the total (it uses the over-fetched hit and `search_after`).
        body["track_total_hits"] = match query.total {
            Some(TotalMode::Accurate) => json!(true),
            Some(TotalMode::Estimate) => json!(ESTIMATE_TRACK_TOTAL_HITS),
            Some(TotalMode::None) | None => json!(false),
        };

        // When the query contributes relevance (full-text), ask ES to compute
        // `_score` even though we sort by a field — otherwise `_score` is null
        // and we can't populate `Bundle.entry.search.score`. A `_sort=_score`
        // already scores natively, so this only matters for the field-sort case.
        if query_has_relevance(query) {
            body["track_scores"] = json!(true);
        }

        // Return only what the result parser reads, plus `is_deleted`, which
        // `search()` still checks on every hit.
        let mut source_fields: Vec<&str> = HIT_SOURCE_FIELDS.to_vec();
        source_fields.push("is_deleted");
        body["_source"] = json!(source_fields);

        Ok(EsQuery {
            body,
            index: self.index.clone(),
            over_fetched,
        })
    }

    /// Builds a nested clause matching resources that reference
    /// `comp.reference` through ANY of the compartment membership params
    /// (logical OR), which is how a resource joins a FHIR compartment.
    ///
    /// Reference matching is version-agnostic and mirrors the reference
    /// parameter handler: the stored reference must equal the base reference or
    /// carry a `/_history/<vid>` suffix. The membership params come from the
    /// bundled FHIR `CompartmentDefinition`s.
    ///
    /// A membership with no membership params or no reference matches nothing
    /// ([`CompartmentMembership::matches_nothing`]), so it yields a `match_none`
    /// clause, never an absent one: dropping the clause would match every
    /// resource of the type (#2509).
    fn build_compartment_clause(comp: &CompartmentMembership) -> Value {
        if comp.matches_nothing() {
            return date::match_none();
        }

        let base = strip_reference_version(&comp.reference);

        // Also match a reference indexed as an absolute URL, through the
        // `Type/id` the writer stores for those (#2394).
        let should = json!([
            { "term": { "search_params.reference.reference": base } },
            { "prefix": { "search_params.reference.reference": format!("{base}/_history/") } },
            { "term": { "search_params.reference.reference_target": base } }
        ]);

        json!({
            "nested": {
                "path": "search_params.reference",
                "query": {
                    "bool": {
                        "must": [
                            { "terms": { "search_params.reference.name": comp.params } },
                            {
                                "bool": {
                                    "should": should,
                                    "minimum_should_match": 1
                                }
                            }
                        ]
                    }
                }
            }
        })
    }

    /// Builds a clause for a single search parameter.
    ///
    /// `Ok(None)` means the parameter yields no clause at all — it has no
    /// values, or no value of it has a clause (a `special`-typed parameter);
    /// [`Self::build`] tells the two apart. `Err` is a parameter some of whose
    /// values have a clause and others do not: the ones without cannot be left
    /// out of the OR (#2510), see [`Self::or_of_values`].
    fn build_parameter_clause(
        &self,
        param: &SearchParameter,
    ) -> Result<Option<Value>, SearchError> {
        // Presence is independent of the parameter's ordinary value syntax.
        // Resolve it before `_id` and `_lastUpdated`, which would otherwise
        // interpret the boolean literal as an ID or date value.
        if param.modifier == Some(SearchModifier::Missing) {
            return Ok(modifier_handlers::build_missing_clause(param));
        }

        // Defence in depth behind `validate_value_presence` (#1380): an empty
        // value is a prefix of every string, so it matches nothing here rather
        // than whatever the handler below would make of it — the whole
        // parameter, since under `:not` "nothing" negates into "everything".
        // `match_none`, never `None`: the builder refuses a value with no clause
        // (#2510), and used to drop the constraint instead.
        if crate::search::has_empty_value(param) {
            return Ok(Some(date::match_none()));
        }

        // Handle special parameters. Each returns `None` only for a parameter
        // with no values.
        match param.name.as_str() {
            "_id" => return Ok(self.build_id_clause(param)),
            "_lastUpdated" => return Ok(self.build_last_updated_clause(param)),
            "_text" => return Ok(fts::build_text_clause(param)),
            "_content" => return Ok(fts::build_content_clause(param)),
            _ => {}
        }

        // Dispatch based on parameter type
        Self::or_of_values(param, |value| {
            self.build_value_clause(param, &value.value, value.prefix)
        })
    }

    /// ORs the clause `clause_for` builds for each value of `param`, negated
    /// as a whole under `:not`.
    ///
    /// A value `clause_for` builds nothing for is never dropped from the OR
    /// (#2510): left out, `:not=a,b` would be `NOT a` and the search would
    /// widen, and `a,b` would be `a` and answer a different question. When
    /// every value lacks a clause the result is `Ok(None)` — the parameter is
    /// not evaluable, which [`Self::build`] refuses by name — and when only
    /// some do it is an error naming the first of them.
    fn or_of_values(
        param: &SearchParameter,
        clause_for: impl Fn(&SearchValue) -> Option<Value>,
    ) -> Result<Option<Value>, SearchError> {
        let mut clauses: Vec<Value> = Vec::with_capacity(param.values.len());
        let mut first_without: Option<&SearchValue> = None;
        for value in &param.values {
            match clause_for(value) {
                Some(clause) => clauses.push(clause),
                None => {
                    first_without.get_or_insert(value);
                }
            }
        }

        if clauses.is_empty() {
            return Ok(None);
        }
        if let Some(value) = first_without {
            return Err(unevaluable_value(param, &value.value));
        }

        // Multiple values for the same parameter are ORed
        let combined = if clauses.len() == 1 {
            clauses.into_iter().next().unwrap()
        } else {
            json!({
                "bool": {
                    "should": clauses,
                    "minimum_should_match": 1
                }
            })
        };

        // `:not` negates HERE, around the OR of every value, not per value
        // (#473). FHIR's `:not` means "no value of the parameter matches", so
        // `:not=a,b` is `NOT (a OR b)`; negating each value first would give
        // `NOT a OR NOT b` and return every resource holding both values.
        // `must_not` over the nested query is already resource-level: a nested
        // clause matches when *some* indexed value matches, so its negation
        // covers resources with no value for the parameter at all.
        if matches!(param.modifier, Some(SearchModifier::Not)) {
            return Ok(Some(json!({ "bool": { "must_not": [combined] } })));
        }

        Ok(Some(combined))
    }

    /// Builds a clause for a single value of a parameter.
    ///
    /// `None` is "no clause can be built for this value", never "matches
    /// nothing" or "no constraint": [`Self::or_of_values`] refuses a parameter
    /// over it (#2510). A value that is to match nothing says so with
    /// [`date::match_none`].
    fn build_value_clause(
        &self,
        param: &SearchParameter,
        value: &str,
        prefix: SearchPrefix,
    ) -> Option<Value> {
        match param.param_type {
            SearchParamType::String => string::build_clause(param, value),
            SearchParamType::Token => token::build_clause(param, value),
            SearchParamType::Date => date::build_clause(&param.name, value, prefix),
            SearchParamType::Number => number::build_clause(&param.name, value, prefix),
            SearchParamType::Quantity => quantity::build_clause(&param.name, value, prefix),
            SearchParamType::Reference => reference::build_clause(param, value),
            SearchParamType::Uri => uri::build_clause(param, value),
            SearchParamType::Composite => composite::build_clause(param, value),
            SearchParamType::Special => None,
        }
    }

    /// Builds a clause for the _id special parameter.
    ///
    /// `_id` is dispatched here by name (`build_parameter_clause`, above),
    /// bypassing the generic `:not` handling that wraps every other
    /// parameter's clauses in `must_not` — so `_id:not=a` used to build the
    /// exact same `term`/`terms` clause as a bare `_id=a` and match precisely
    /// the resource the caller asked to exclude (#1092). `:missing` is
    /// resolved earlier, in `build_parameter_clause`, and any other modifier
    /// is rejected before this point by the backend's search entry point, so
    /// only `None` and `Some(SearchModifier::Not)` are handled here.
    fn build_id_clause(&self, param: &SearchParameter) -> Option<Value> {
        if param.values.is_empty() {
            return None;
        }
        let ids: Vec<&str> = param.values.iter().map(|v| v.value.as_str()).collect();
        // A `terms` query may carry at most `index.max_terms_count` values;
        // a chained search that resolved a wide terminal set (#1548) pins
        // more, so the list goes out as several `terms` clauses ORed.
        let clause = if ids.len() == 1 {
            json!({ "term": { "resource_id": ids[0] } })
        } else if ids.len() <= self.max_terms_count as usize {
            json!({ "terms": { "resource_id": ids } })
        } else {
            let chunks: Vec<Value> = ids
                .chunks(self.max_terms_count as usize)
                .map(|chunk| json!({ "terms": { "resource_id": chunk } }))
                .collect();
            json!({ "bool": { "should": chunks, "minimum_should_match": 1 } })
        };

        if matches!(param.modifier, Some(SearchModifier::Not)) {
            Some(json!({ "bool": { "must_not": [clause] } }))
        } else {
            Some(clause)
        }
    }

    /// Builds a clause for the `_lastUpdated` special parameter.
    ///
    /// Reuses the precision-aware date logic that indexed date parameters
    /// get, against the top-level `last_updated` field: `eq` at day
    /// precision means `[day, day+1)`, `ne` its complement, `sa`/`eb` mirror
    /// `gt`/`lt` on the whole period, and comma-separated values OR together
    /// (#892). Previously every value was folded into one `range` map, so
    /// `ne`/`sa`/`eb`/`ap` degraded to `eq` and a second value overwrote the
    /// first.
    fn build_last_updated_clause(&self, param: &SearchParameter) -> Option<Value> {
        let mut clauses: Vec<Value> = param
            .values
            .iter()
            .map(
                |value| match date::field_range("last_updated", &value.value, value.prefix) {
                    Some(date::DateRange::Within(range)) => range,
                    Some(date::DateRange::Outside(range)) => {
                        json!({ "bool": { "must_not": [range] } })
                    }
                    // Not a date: a clause that matches nothing. Dropping the
                    // value instead would return every resource (#1293).
                    None => date::match_none(),
                },
            )
            .collect();

        match clauses.len() {
            0 => None,
            1 => clauses.pop(),
            _ => Some(json!({
                "bool": {
                    "should": clauses,
                    "minimum_should_match": 1
                }
            })),
        }
    }

    /// Builds the sort clause.
    ///
    /// A `Previous` cursor walks the result set backward, so `paging`
    /// reverses every criterion — including the final `resource_id`
    /// tie-breaker — relative to the `Next` order. `search_after` then seeks
    /// from the cursor position in that reversed order, and the results
    /// layer restores the caller-facing order before returning the page
    /// (#1015).
    fn build_sort(&self, directives: &[SortDirective], paging: CursorDirection) -> Value {
        if directives.is_empty() {
            // Default sort: _lastUpdated descending, then _id for
            // tie-breaking; reversed when paging backward.
            return if paging == CursorDirection::Previous {
                json!([
                    { "last_updated": { "order": "asc" } },
                    { "resource_id": { "order": "desc" } }
                ])
            } else {
                json!([
                    { "last_updated": { "order": "desc" } },
                    { "resource_id": { "order": "asc" } }
                ])
            };
        }

        let mut sort_clauses: Vec<Value> = Vec::new();

        for directive in directives {
            let order = es_order(directive.direction, paging);

            match directive.parameter.as_str() {
                "_id" => {
                    sort_clauses.push(json!({ "resource_id": { "order": order } }));
                }
                "_lastUpdated" => {
                    sort_clauses.push(json!({ "last_updated": { "order": order } }));
                }
                // `_sort=_score` ranks by Elasticsearch relevance.
                "_score" => {
                    sort_clauses.push(json!({ "_score": { "order": order } }));
                }
                // For other parameters, sort on the nested search_params
                // group that actually holds the parameter's values. The sort
                // used to assume every parameter was a string — a date sort
                // like `_sort=birthdate` filtered `search_params.string` for
                // a name that only exists under `search_params.date`, matched
                // nothing, and returned the whole page in arbitrary order
                // with a 200 (#883).
                name => {
                    let (group, field) = match directive.param_type {
                        // A date is a range `[value, end)` (#1391): an
                        // ascending sort orders by where it starts, a
                        // descending one by where it ends, so a `Period` sorts
                        // by its end as it did when each end was its own
                        // entry. Chosen by the requested direction, not the
                        // paging one, so a `Previous` cursor keeps the key.
                        Some(SearchParamType::Date)
                            if matches!(directive.direction, SortDirection::Descending) =>
                        {
                            ("date", "search_params.date.end")
                        }
                        Some(SearchParamType::Date) => ("date", "search_params.date.value"),
                        Some(SearchParamType::Number) => ("number", "search_params.number.value"),
                        Some(SearchParamType::Quantity) => {
                            ("quantity", "search_params.quantity.value")
                        }
                        Some(SearchParamType::Token) => ("token", "search_params.token.code"),
                        Some(SearchParamType::Reference) => {
                            ("reference", "search_params.reference.reference")
                        }
                        Some(SearchParamType::Uri) => ("uri", "search_params.uri.value"),
                        // Strings, composites, and unresolved types sort on
                        // the string group, the pre-existing behavior.
                        _ => ("string", "search_params.string.value.keyword"),
                    };
                    // FHIR multi-value sort semantics: the smallest value
                    // orders an ascending sort, the largest a descending one
                    // (the SQL backends' MIN/MAX). Like the date field, this
                    // is the resource's sort key, so it follows the requested
                    // direction: a `Previous` cursor walks the same keys in
                    // reverse rather than switching to other ones.
                    let mode = match directive.direction {
                        SortDirection::Ascending => "min",
                        SortDirection::Descending => "max",
                    };
                    // A resource with no value sorts after those that have
                    // one, in either requested direction (#1606). Walking
                    // backward reverses that too, so a `Previous` page is the
                    // mirror of a `Next` one.
                    let missing = if paging == CursorDirection::Previous {
                        "_first"
                    } else {
                        "_last"
                    };
                    let mut clause = json!({
                        "order": order,
                        "mode": mode,
                        "nested": {
                            "path": format!("search_params.{group}"),
                            "filter": {
                                "term": { format!("search_params.{group}.name"): name }
                            }
                        },
                        "missing": missing
                    });
                    // `search_params.date.end` exists only in indices at schema
                    // version 2 (#1391). An index that has not been reconciled
                    // yet lacks the field (the startup pass may have failed or
                    // missed it, e.g. Elasticsearch was down at boot or an older
                    // instance created the index during a rolling upgrade; the
                    // first write then reconciles it), and Elasticsearch
                    // refuses to sort on an unmapped one with a 400 unless told
                    // its type.
                    if field == "search_params.date.end" {
                        clause["unmapped_type"] = json!("date");
                    }
                    sort_clauses.push(json!({ field: clause }));
                }
            }
        }

        // Always add tie-breaker, reversed when paging backward.
        let tie_breaker_order = if paging == CursorDirection::Previous {
            "desc"
        } else {
            "asc"
        };
        sort_clauses.push(json!({ "resource_id": { "order": tie_breaker_order } }));

        Value::Array(sort_clauses)
    }

    /// Builds the search_after clause from a cursor.
    fn build_search_after(&self, cursor: &PageCursor) -> Value {
        let mut values: Vec<Value> = cursor
            .sort_values()
            .iter()
            .map(|v| match v {
                crate::types::CursorValue::String(s) => json!(s),
                crate::types::CursorValue::Number(n) => json!(n),
                crate::types::CursorValue::Decimal(d) => json!(d.to_string()),
                crate::types::CursorValue::Boolean(b) => json!(b),
                crate::types::CursorValue::Null => json!(null),
            })
            .collect();

        // Append the resource_id tie-breaker
        values.push(json!(cursor.resource_id()));

        Value::Array(values)
    }
}

/// Builds the body for the `_count` API: the search's `query` clause and
/// nothing else.
///
/// `_count` accepts only `query`; every search-only field the full builder
/// sets — `sort`, `size`, `from`, `search_after`, `track_total_hits`,
/// `track_scores` — is a `parsing_exception` there, which surfaced as a 500
/// on every `_total=accurate` search served by Elasticsearch.
///
/// Known limitation: unlike `search()`, this builder is not given
/// `with_max_terms_count`, so a very large resolved id set (for example a `_has`
/// over a big reverse chain) is not chunked here and can exceed the cluster's
/// `index.max_terms_count` on `_total=accurate`.
///
/// Refuses what [`EsQueryBuilder::build`] refuses (#2510): a count over a
/// dropped parameter would be the size of the whole type.
pub fn build_count_query(
    tenant_id: &str,
    resource_type: &str,
    query: &SearchQuery,
) -> Result<Value, SearchError> {
    let builder = EsQueryBuilder::new(tenant_id, resource_type, String::new());
    let mut es_query = builder.build(query)?;

    Ok(json!({ "query": es_query.body["query"].take() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CursorValue, SortDirection};

    /// #2394: compartment membership also matches absolute references through
    /// the stored `Type/id`.
    #[test]
    fn compartment_clause_matches_absolute_reference_target() {
        let comp = CompartmentMembership {
            params: vec!["patient".to_string()],
            reference: "Patient/p1/_history/3".to_string(),
        };
        let s = serde_json::to_string(&EsQueryBuilder::build_compartment_clause(&comp)).unwrap();
        assert!(
            s.contains(r#"{"term":{"search_params.reference.reference_target":"Patient/p1"}}"#)
        );
    }

    /// #2509: a membership with no params, no reference or neither matches
    /// nothing. The clause is `match_none`, never absent: an absent clause
    /// matches every resource of the type.
    #[test]
    fn compartment_clause_of_an_empty_membership_matches_nothing() {
        for (label, comp) in [
            (
                "no params",
                CompartmentMembership {
                    params: vec![],
                    reference: "Patient/p1".to_string(),
                },
            ),
            (
                "empty reference",
                CompartmentMembership {
                    params: vec!["patient".to_string()],
                    reference: String::new(),
                },
            ),
            ("neither", CompartmentMembership::default()),
        ] {
            assert_eq!(
                EsQueryBuilder::build_compartment_clause(&comp),
                json!({ "match_none": {} }),
                "{label}"
            );
        }
    }

    /// #2509: a real membership is a nested query on the reference parameters,
    /// not `match_none`.
    #[test]
    fn compartment_clause_of_a_real_membership_is_a_nested_query() {
        let comp = CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1".to_string(),
        };
        let clause = EsQueryBuilder::build_compartment_clause(&comp);
        assert_eq!(clause["nested"]["path"], "search_params.reference");
        assert_eq!(
            clause["nested"]["query"]["bool"]["must"][0],
            json!({ "terms": { "search_params.reference.name": ["subject", "performer"] } })
        );
        assert!(clause.get("match_none").is_none());
    }

    /// #2509: the whole query, the one `search`, `search_ids`, `_contained`
    /// and the `_count` body all start from, carries the `match_none` clause of
    /// an empty membership — alone and beside an ordinary parameter, and in
    /// every `_contained` mode — and a real membership does not.
    #[test]
    fn an_empty_compartment_membership_makes_the_whole_query_match_nothing() {
        let builder =
            EsQueryBuilder::new("acme", "Observation", "hfs_acme_observation".to_string());
        let none = json!({ "match_none": {} });
        let status = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq("final")],
            ..Default::default()
        };
        for membership in [
            CompartmentMembership {
                params: vec![],
                reference: "Patient/p1".to_string(),
            },
            CompartmentMembership {
                params: vec!["subject".to_string()],
                reference: String::new(),
            },
            CompartmentMembership::default(),
        ] {
            for parameters in [vec![], vec![status.clone()]] {
                for contained in [
                    crate::types::ContainedMode::Off,
                    crate::types::ContainedMode::On,
                    crate::types::ContainedMode::Both,
                ] {
                    let mut query = SearchQuery::new("Observation");
                    query.parameters = parameters.clone();
                    query.contained = contained;
                    query.compartment = Some(membership.clone());

                    let must = builder.build(&query).unwrap().body["query"]["bool"]["must"].clone();
                    let must = must.as_array().expect("a must clause");
                    assert!(
                        must.contains(&none),
                        "{membership:?} + {} parameter(s), {contained:?}: {must:?}",
                        parameters.len()
                    );
                    let counted = build_count_query("acme", "Observation", &query).unwrap();
                    assert!(
                        counted["query"]["bool"]["must"]
                            .as_array()
                            .is_some_and(|must| must.contains(&none)),
                        "{membership:?} count: {counted}"
                    );
                }
            }
        }

        let mut real = SearchQuery::new("Observation");
        real.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string()],
            reference: "Patient/p1".to_string(),
        });
        let must = builder.build(&real).unwrap().body["query"]["bool"]["must"].clone();
        assert!(!must.as_array().unwrap().contains(&none), "{must}");
    }

    /// Encodes a `Previous` cursor at the given resource ID, used to
    /// exercise the backward-paging path of `build`.
    fn previous_cursor(id: &str) -> String {
        PageCursor::previous(vec![CursorValue::Number(1_700_000_000_000)], id).encode()
    }

    /// Encodes a `Next` cursor at the given resource ID, used as the
    /// forward-paging regression baseline.
    fn next_cursor(id: &str) -> String {
        PageCursor::new(vec![CursorValue::Number(1_700_000_000_000)], id).encode()
    }

    #[test]
    fn build_requests_only_the_fields_the_parser_reads() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&SearchQuery::new("Patient")).unwrap();

        let mut expected: Vec<&str> = HIT_SOURCE_FIELDS.to_vec();
        expected.push("is_deleted");
        assert_eq!(es_query.body["_source"], json!(expected));
        for dropped in ["content_text", "narrative_text", "search_params"] {
            assert!(!expected.contains(&dropped));
        }
    }

    #[test]
    fn count_query_carries_only_the_query() {
        let body = build_count_query("acme", "Patient", &SearchQuery::new("Patient")).unwrap();
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["query"]);
    }

    #[test]
    fn test_basic_query_build() {
        let query = SearchQuery::new("Patient");
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        // Should have tenant and is_deleted filters
        let filters = &es_query.body["query"]["bool"]["filter"];
        assert!(filters.is_array());
    }

    /// #2254: a named version is a `fhir_version` term filter on every query
    /// the builder makes, including the count body; none adds no clause.
    #[test]
    fn fhir_version_is_a_term_filter_only_when_named() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let version_terms = |body: &Value| -> Vec<Value> {
            body["query"]["bool"]["filter"]
                .as_array()
                .expect("filter array")
                .iter()
                .filter(|clause| clause["term"].get("fhir_version").is_some())
                .cloned()
                .collect()
        };

        let blind = SearchQuery::new("Patient");
        assert!(version_terms(&builder.build(&blind).unwrap().body).is_empty());
        assert!(version_terms(&build_count_query("acme", "Patient", &blind).unwrap()).is_empty());

        let r4 = SearchQuery::new("Patient").with_fhir_version(helios_fhir::FhirVersion::R4);
        let expected = vec![json!({ "term": { "fhir_version": "4.0" } })];
        assert_eq!(version_terms(&builder.build(&r4).unwrap().body), expected);
        assert_eq!(
            version_terms(&build_count_query("acme", "Patient", &r4).unwrap()),
            expected
        );
    }

    #[test]
    fn test_id_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();
        let body_str = serde_json::to_string(&es_query.body).unwrap();
        assert!(body_str.contains("resource_id"));
    }

    /// #1092: `_id` is dispatched through `build_id_clause`, bypassing the
    /// generic `:not` handling that wraps every other parameter's clauses in
    /// `must_not` — so `_id:not=a` used to build the exact same `term`
    /// clause as a bare `_id=a` and match precisely the resource the caller
    /// asked to exclude.
    #[test]
    fn id_no_modifier_control_is_a_term_clause() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();
        let clause = &es_query.body["query"]["bool"]["must"][0];

        assert_eq!(clause, &json!({ "term": { "resource_id": "a" } }));
    }

    /// A chained search whose terminal hop resolved more ids than
    /// `index.max_terms_count` used to be refused by Elasticsearch as a
    /// malformed query (#1548); the list now goes out as several `terms`
    /// clauses ORed, each within the ceiling, and `:not` negates the whole.
    #[test]
    fn id_lists_past_the_terms_ceiling_are_split_into_should_clauses() {
        let ids: Vec<SearchValue> = (0..7).map(|i| SearchValue::eq(format!("id-{i}"))).collect();
        let param = |modifier| SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: ids.clone(),
            chain: vec![],
            components: vec![],
        };
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string())
            .with_max_terms_count(3);

        let es_query = builder
            .build(&SearchQuery::new("Patient").with_parameter(param(None)))
            .unwrap();
        let clause = &es_query.body["query"]["bool"]["must"][0];
        let should = clause["bool"]["should"]
            .as_array()
            .expect("bool.should of terms");
        assert_eq!(clause["bool"]["minimum_should_match"], json!(1));
        let sizes: Vec<usize> = should
            .iter()
            .map(|c| c["terms"]["resource_id"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, [3, 3, 1]);
        assert_eq!(should[2]["terms"]["resource_id"], json!(["id-6"]));

        let within = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string())
            .with_max_terms_count(7)
            .build(&SearchQuery::new("Patient").with_parameter(param(None)))
            .unwrap();
        assert!(within.body["query"]["bool"]["must"][0]["terms"].is_object());

        let negated = builder
            .build(&SearchQuery::new("Patient").with_parameter(param(Some(SearchModifier::Not))))
            .unwrap();
        let inner = &negated.body["query"]["bool"]["must"][0]["bool"]["must_not"][0];
        assert_eq!(inner["bool"]["should"].as_array().unwrap().len(), 3);
    }

    /// Matches the SQL backends' `_id` builders, which both guard on
    /// `values.is_empty()` and contribute no condition rather than an
    /// empty `terms: []` clause (which would match nothing rather than
    /// leaving the parameter's absence unconstrained).
    #[test]
    fn id_with_no_values_produces_no_clause() {
        let param = SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![],
            chain: vec![],
            components: vec![],
        };

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        assert!(builder.build_id_clause(&param).is_none());
    }

    #[test]
    fn id_not_single_value_is_negated_with_must_not() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();
        let clause = &es_query.body["query"]["bool"]["must"][0];

        assert_eq!(
            clause,
            &json!({ "bool": { "must_not": [ { "term": { "resource_id": "a" } } ] } })
        );
    }

    #[test]
    fn id_not_two_values_is_negated_with_must_not() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a"), SearchValue::eq("b")],
            chain: vec![],
            components: vec![],
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();
        let clause = &es_query.body["query"]["bool"]["must"][0];

        assert_eq!(
            clause,
            &json!({ "bool": { "must_not": [ { "terms": { "resource_id": ["a", "b"] } } ] } })
        );
    }

    #[test]
    fn missing_precedes_id_and_last_updated_dispatch() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());

        for (name, param_type, field) in [
            ("_id", SearchParamType::Token, "resource_id"),
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
            let es_query = builder.build(&query).unwrap();
            let clause = &es_query.body["query"]["bool"]["must"][0];

            assert_eq!(clause["exists"]["field"], field);
        }
    }

    fn last_updated_query(values: Vec<SearchValue>) -> Value {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        builder.build(&query).unwrap().body["query"]["bool"]["must"][0].clone()
    }

    #[test]
    fn last_updated_eq_is_a_day_range() {
        let clause = last_updated_query(vec![SearchValue::eq("2026-09-01")]);
        assert_eq!(
            clause["range"]["last_updated"],
            json!({ "gte": "2026-09-01", "lt": "2026-09-02" })
        );
    }

    #[test]
    fn last_updated_ne_is_negated_not_eq() {
        // #892: `ne` fell into the default arm and matched exactly the day.
        let clause = last_updated_query(vec![SearchValue::new(SearchPrefix::Ne, "2026-09-01")]);
        assert_eq!(
            clause["bool"]["must_not"][0]["range"]["last_updated"],
            json!({ "gte": "2026-09-01", "lt": "2026-09-02" })
        );
    }

    #[test]
    fn last_updated_sa_and_eb_exclude_the_named_period() {
        let sa = last_updated_query(vec![SearchValue::new(SearchPrefix::Sa, "2026-09-01")]);
        assert_eq!(sa["range"]["last_updated"], json!({ "gte": "2026-09-02" }));

        let eb = last_updated_query(vec![SearchValue::new(SearchPrefix::Eb, "2026-09-01")]);
        assert_eq!(eb["range"]["last_updated"], json!({ "lt": "2026-09-01" }));
    }

    #[test]
    fn last_updated_or_list_keeps_every_value() {
        // #892: a second value overwrote the first in the single range map.
        let clause = last_updated_query(vec![
            SearchValue::eq("2026-09-01"),
            SearchValue::eq("2026-09-03"),
        ]);
        let should = clause["bool"]["should"]
            .as_array()
            .expect("OR list -> bool.should");
        assert_eq!(should.len(), 2);
        assert_eq!(clause["bool"]["minimum_should_match"], 1);
        assert_eq!(should[0]["range"]["last_updated"]["gte"], "2026-09-01");
        assert_eq!(should[1]["range"]["last_updated"]["gte"], "2026-09-03");
    }

    /// #1293: a value that is not a date used to be read as the year 2000, so
    /// `_lastUpdated=gtnot-a-date` returned every resource. The search gate
    /// rejects it before a query is built; if one is built anyway, the clause
    /// matches nothing and is never dropped from the query.
    #[test]
    fn a_date_value_that_is_not_a_date_matches_nothing() {
        let clause = last_updated_query(vec![SearchValue::new(SearchPrefix::Gt, "not-a-date")]);
        assert_eq!(clause, json!({ "match_none": {} }));

        // In an OR list the bad value contributes nothing; the good one stays.
        let clause = last_updated_query(vec![
            SearchValue::new(SearchPrefix::Ne, "2024-13-45"),
            SearchValue::eq("2026-09-03"),
        ]);
        let should = clause["bool"]["should"].as_array().expect("bool.should");
        assert_eq!(should[0], json!({ "match_none": {} }));
        assert_eq!(should[1]["range"]["last_updated"]["gte"], "2026-09-03");

        // An indexed date parameter: the handler must not return a `None` for it.
        let query = SearchQuery::new("Procedure").with_parameter(SearchParameter {
            name: "date".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "abcd")],
            chain: vec![],
            components: vec![],
        });
        let builder = EsQueryBuilder::new("acme", "Procedure", "hfs_acme_procedure".to_string());
        assert_eq!(
            builder.build(&query).unwrap().body["query"]["bool"]["must"][0],
            json!({ "match_none": {} })
        );
    }

    #[test]
    fn last_updated_second_precision_is_the_whole_second() {
        let clause = last_updated_query(vec![SearchValue::eq("2026-09-06T04:44:27-04:00")]);
        assert_eq!(
            clause["range"]["last_updated"],
            json!({ "gte": "2026-09-06T08:44:27.000Z", "lt": "2026-09-06T08:44:28.000Z" })
        );
    }

    fn not_param(values: Vec<SearchValue>) -> SearchQuery {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "language".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values,
            chain: vec![],
            components: vec![],
        })
    }

    /// `:not` wraps the token clause in a single resource-level `must_not`.
    #[test]
    fn test_not_modifier_negates_once() {
        let query = not_param(vec![SearchValue::eq("en-US")]);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        let clause = &es_query.body["query"]["bool"]["must"][0];
        let negated = &clause["bool"]["must_not"][0];
        assert!(
            negated["nested"].is_object(),
            "must_not wraps the nested query"
        );
        // The negated clause itself is positive — no double negation.
        let inner = serde_json::to_string(negated).unwrap();
        assert!(!inner.contains("must_not"));
    }

    /// `:not=a,b` is `NOT (a OR b)`, not `NOT a OR NOT b` (#473): the OR of the
    /// positive value clauses sits inside one `must_not`, so a resource holding
    /// both values is excluded rather than matching through the other value.
    #[test]
    fn test_not_modifier_multi_value_negates_the_or() {
        let query = not_param(vec![SearchValue::eq("en-US"), SearchValue::eq("es")]);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        let clause = &es_query.body["query"]["bool"]["must"][0];
        let negated = &clause["bool"]["must_not"][0];
        let should = negated["bool"]["should"].as_array().expect("OR of values");
        assert_eq!(should.len(), 2);

        let inner = serde_json::to_string(negated).unwrap();
        assert!(
            !inner.contains("must_not"),
            "values must not be negated individually"
        );
        assert!(inner.contains("en-US") && inner.contains("es"));
    }

    #[test]
    fn test_default_sort() {
        let query = SearchQuery::new("Patient");
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        let sort = &es_query.body["sort"];
        assert!(sort.is_array());
        assert!(sort[0]["last_updated"]["order"].as_str() == Some("desc"));
    }

    #[test]
    fn test_custom_sort() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "_id".to_string(),
            direction: SortDirection::Ascending,
            param_type: None,
        });

        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        let sort = &es_query.body["sort"];
        assert!(sort[0]["resource_id"]["order"].as_str() == Some("asc"));
    }

    /// #883: a parameter sort must target the nested group that holds the
    /// parameter's type — a date sort against the string group matched
    /// nothing and returned arbitrary order with a 200.
    #[test]
    fn test_parameter_sort_targets_the_type_group() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "birthdate".to_string(),
            direction: SortDirection::Ascending,
            param_type: Some(SearchParamType::Date),
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = &builder.build(&query).unwrap().body["sort"][0];

        let clause = &sort["search_params.date.value"];
        assert!(
            !clause.is_null(),
            "date parameter must sort on the date group, got {sort}"
        );
        assert_eq!(clause["order"], "asc");
        assert_eq!(clause["mode"], "min");
        assert_eq!(clause["nested"]["path"], "search_params.date");
        assert_eq!(
            clause["nested"]["filter"]["term"]["search_params.date.name"],
            "birthdate"
        );
    }

    /// #1391: a descending date sort orders by where each range ends — the
    /// end of a `Period` — and keeps that key under a `Previous` cursor.
    #[test]
    fn test_descending_date_sort_uses_the_range_end() {
        let directive = SortDirective {
            parameter: "date".to_string(),
            direction: SortDirection::Descending,
            param_type: Some(SearchParamType::Date),
        };
        let builder = EsQueryBuilder::new("acme", "Encounter", "hfs_acme_encounter".to_string());

        let query = SearchQuery::new("Encounter").with_sort(directive.clone());
        let sort = &builder.build(&query).unwrap().body["sort"][0];
        let clause = &sort["search_params.date.end"];
        assert!(!clause.is_null(), "descending sorts on the end, got {sort}");
        assert_eq!(clause["order"], "desc");
        assert_eq!(clause["mode"], "max");
        // An index not yet reconciled to schema version 2 has no `end`.
        assert_eq!(clause["unmapped_type"], "date");

        let query = SearchQuery::new("Encounter")
            .with_sort(directive)
            .with_cursor(previous_cursor("e-5"));
        let sort = &builder.build(&query).unwrap().body["sort"][0];
        assert!(
            !sort["search_params.date.end"].is_null(),
            "a Previous cursor keeps the sort key, got {sort}"
        );
    }

    #[test]
    fn test_token_sort_descending_uses_max_mode() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "gender".to_string(),
            direction: SortDirection::Descending,
            param_type: Some(SearchParamType::Token),
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = &builder.build(&query).unwrap().body["sort"][0];

        let clause = &sort["search_params.token.code"];
        assert!(!clause.is_null(), "token sorts on the code, got {sort}");
        assert_eq!(clause["order"], "desc");
        assert_eq!(clause["mode"], "max");
        assert_eq!(clause["missing"], "_last");
    }

    #[test]
    fn test_untyped_sort_falls_back_to_string_group() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "name".to_string(),
            direction: SortDirection::Ascending,
            param_type: None,
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = &builder.build(&query).unwrap().body["sort"][0];
        assert!(
            !sort["search_params.string.value.keyword"].is_null(),
            "untyped parameters keep the string-group sort, got {sort}"
        );
    }

    /// #1015: a `Previous` cursor must reverse the default sort (including
    /// the tie-breaker) and over-fetch by one hit so the results layer can
    /// tell whether an earlier page exists.
    #[test]
    fn test_previous_cursor_reverses_default_sort_and_overfetches() {
        let query = SearchQuery::new("Patient")
            .with_count(5)
            .with_cursor(previous_cursor("p-5"));
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let body = builder.build(&query).unwrap().body;

        assert_eq!(
            body["sort"],
            json!([
                { "last_updated": { "order": "asc" } },
                { "resource_id": { "order": "desc" } }
            ])
        );
        assert_eq!(body["search_after"], json!([1_700_000_000_000i64, "p-5"]));
        assert_eq!(body["size"], json!(6));
        assert!(body.get("from").is_none());
    }

    /// #1015, #1606: a `Previous` cursor over an ascending directive reverses
    /// the order and `missing` (`desc`/`_first`), but keeps the `min` sort
    /// key — it walks the same keys backward, not different ones.
    #[test]
    fn test_previous_cursor_reverses_custom_sort_order_and_missing() {
        let query = SearchQuery::new("Patient")
            .with_sort(SortDirective {
                parameter: "birthdate".to_string(),
                direction: SortDirection::Ascending,
                param_type: Some(SearchParamType::Date),
            })
            .with_cursor(previous_cursor("p-5"));
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = builder.build(&query).unwrap().body["sort"].clone();
        let sort = sort.as_array().expect("sort is an array");

        let clause = &sort[0]["search_params.date.value"];
        assert_eq!(clause["order"], "desc");
        assert_eq!(clause["mode"], "min");
        assert_eq!(clause["missing"], "_first");

        let tie_breaker = sort.last().expect("tie-breaker present");
        assert_eq!(tie_breaker, &json!({ "resource_id": { "order": "desc" } }));
    }

    /// #1015: `_sort=_score` also flips under a `Previous` cursor.
    #[test]
    fn test_previous_cursor_reverses_score_sort() {
        let query = SearchQuery::new("Patient")
            .with_sort(SortDirective {
                parameter: "_score".to_string(),
                direction: SortDirection::Descending,
                param_type: None,
            })
            .with_cursor(previous_cursor("p-5"));
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = builder.build(&query).unwrap().body["sort"].clone();

        assert_eq!(sort[0], json!({ "_score": { "order": "asc" } }));
    }

    /// #1015 regression + #1079: a Next cursor leaves the sort untouched and
    /// over-fetches by one hit like Previous does.
    #[test]
    fn test_next_cursor_keeps_sort_and_overfetches() {
        let query = SearchQuery::new("Patient")
            .with_count(5)
            .with_cursor(next_cursor("p-5"));
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let body = builder.build(&query).unwrap().body;

        assert_eq!(
            body["sort"],
            json!([
                { "last_updated": { "order": "desc" } },
                { "resource_id": { "order": "asc" } }
            ])
        );
        assert_eq!(body["size"], json!(6));
        assert_eq!(body["search_after"], json!([1_700_000_000_000i64, "p-5"]));
    }

    /// #1079: a first page with neither a cursor nor an offset over-fetches
    /// by one hit and adds neither `from` nor `search_after`.
    #[test]
    fn test_first_page_overfetches_by_one() {
        let query = SearchQuery::new("Patient").with_count(5);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let body = builder.build(&query).unwrap().body;

        assert_eq!(body["size"], json!(6));
        assert!(body.get("from").is_none());
        assert!(body.get("search_after").is_none());
    }

    /// #1079: an offset page over-fetches by one hit, keeps `from` set to the
    /// requested offset, and adds no `search_after`.
    #[test]
    fn test_offset_page_overfetches_by_one_and_keeps_from() {
        let mut query = SearchQuery::new("Patient").with_count(5);
        query.offset = Some(10);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let body = builder.build(&query).unwrap().body;

        assert_eq!(body["size"], json!(6));
        assert_eq!(body["from"], json!(10));
        assert!(body.get("search_after").is_none());
    }

    /// #1079: requesting a full window (`count == max_result_window`) leaves
    /// no room for the extra hit, so `size` is clamped to `count` and
    /// `over_fetched` is false.
    #[test]
    fn test_size_is_clamped_to_max_result_window() {
        let query = SearchQuery::new("Patient").with_count(10_000);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        assert_eq!(es_query.body["size"], json!(10_000));
        assert!(!es_query.over_fetched);
    }

    /// #1079: when the extra hit still fits under `max_result_window`, it is
    /// requested as usual.
    #[test]
    fn test_size_keeps_extra_hit_when_it_fits_the_window() {
        let query = SearchQuery::new("Patient").with_count(9_999);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        assert_eq!(es_query.body["size"], json!(10_000));
        assert!(es_query.over_fetched);
    }

    /// #1079: an offset page near the end of the window clamps `size` to the
    /// remaining room instead of over-fetching past `max_result_window`.
    #[test]
    fn test_offset_page_clamps_size_to_remaining_window() {
        let mut query = SearchQuery::new("Patient").with_count(20);
        query.offset = Some(9_990);
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query).unwrap();

        assert_eq!(es_query.body["from"], json!(9_990));
        assert_eq!(es_query.body["size"], json!(10));
        assert!(!es_query.over_fetched);
    }

    /// #1079: `with_max_result_window` overrides the default window for both
    /// the exact-fit and the room-to-spare cases.
    #[test]
    fn test_with_max_result_window_overrides_default() {
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string())
            .with_max_result_window(50);

        let full = builder
            .build(&SearchQuery::new("Patient").with_count(50))
            .unwrap();
        assert_eq!(full.body["size"], json!(50));
        assert!(!full.over_fetched);

        let room = builder
            .build(&SearchQuery::new("Patient").with_count(10))
            .unwrap();
        assert_eq!(room.body["size"], json!(11));
        assert!(room.over_fetched);
    }

    /// #1380: `family=Zzz,` reached the builder as the values `Zzz` and `""`,
    /// and a prefix match on `""` is every row. The search gate
    /// (`validate_value_presence`) rejects it before a query is built; if one
    /// is built anyway, the parameter matches nothing — the whole parameter, or
    /// `:not` would negate it into everything — and as `match_none`, since a
    /// `None` clause is dropped.
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
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
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
            assert_eq!(
                builder.build_parameter_clause(&param).unwrap(),
                Some(json!({ "match_none": {} })),
                "{context}"
            );
        }
    }

    // ------------------------------------------------------------------
    // #2510: a parameter that carries values but yields no clause is an
    // error, not a skipped term. Skipped, a query with no other clause is a
    // `bool` with only filters, which matches every document of the tenant
    // and type.
    // ------------------------------------------------------------------

    fn param_with(
        name: &str,
        param_type: SearchParamType,
        modifier: Option<SearchModifier>,
        values: &[&str],
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            ..Default::default()
        }
    }

    fn es_builder(resource_type: &'static str) -> EsQueryBuilder<'static> {
        EsQueryBuilder::new(
            "acme",
            resource_type,
            format!("hfs_acme_{}", resource_type.to_lowercase()),
        )
    }

    /// The message of the refusal `result` is, or a panic naming `context`.
    fn refusal(result: Result<EsQuery, SearchError>, context: &str) -> String {
        match result {
            Err(SearchError::QueryParseError { message }) => message,
            other => panic!("{context}: expected a QueryParseError, got {other:?}"),
        }
    }

    /// The `must` clauses of a built query; empty when it has none.
    fn must_of(es_query: &EsQuery) -> Vec<Value> {
        es_query.body["query"]["bool"]["must"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// `Location?near=...`: `near` is a `special` parameter and nothing
    /// indexes it, so there is no clause to build. The query it used to
    /// leave behind matched every Location.
    #[test]
    fn a_special_typed_parameter_alone_is_refused_not_dropped() {
        let query = SearchQuery::new("Location").with_parameter(param_with(
            "near",
            SearchParamType::Special,
            None,
            &["42.0|-71.0|10|km"],
        ));
        let message = refusal(es_builder("Location").build(&query), "near");

        assert_eq!(
            message,
            "search parameter 'near' cannot be evaluated by the Elasticsearch backend: no clause \
             can be built for it, and ignoring it would widen the search to every Location"
        );
    }

    /// `Patient?_type=Observation` on R5/R6, where `_type` is typed `special`.
    #[test]
    fn a_special_typed_type_parameter_is_refused_not_dropped() {
        for modifier in [None, Some(SearchModifier::Not)] {
            let query = SearchQuery::new("Patient").with_parameter(param_with(
                "_type",
                SearchParamType::Special,
                modifier.clone(),
                &["Observation"],
            ));
            let message = refusal(
                es_builder("Patient").build(&query),
                &format!("_type {modifier:?}"),
            );
            assert!(message.contains("'_type'"), "{message}");
            assert!(message.contains("every Patient"), "{message}");
        }
    }

    /// A parameter the backend can answer does not rescue one it cannot: a
    /// query with only the answerable clause is a narrower question than the
    /// one asked.
    #[test]
    fn a_special_typed_parameter_beside_a_valid_one_is_refused_too() {
        let id = || param_with("_id", SearchParamType::Token, None, &["a"]);
        let near = || param_with("near", SearchParamType::Special, None, &["x"]);
        let builder = es_builder("Location");

        for (label, query) in [
            (
                "valid first",
                SearchQuery::new("Location")
                    .with_parameter(id())
                    .with_parameter(near()),
            ),
            (
                "special first",
                SearchQuery::new("Location")
                    .with_parameter(near())
                    .with_parameter(id()),
            ),
        ] {
            let message = refusal(builder.build(&query), label);
            assert!(message.contains("'near'"), "{label}: {message}");
        }
    }

    /// The count body is built from the same parameters and refuses them the
    /// same way: a count over the dropped parameter would be the size of the
    /// whole type.
    #[test]
    fn the_count_body_refuses_what_the_search_body_refuses() {
        let query = SearchQuery::new("Location").with_parameter(param_with(
            "near",
            SearchParamType::Special,
            None,
            &["x"],
        ));
        match build_count_query("acme", "Location", &query) {
            Err(SearchError::QueryParseError { message }) => {
                assert!(message.contains("'near'"), "{message}")
            }
            other => panic!("expected a QueryParseError, got {other:?}"),
        }
    }

    /// A parameter with no values asks for nothing, so it constrains nothing:
    /// it is skipped, and does not stand in the way of the one beside it.
    #[test]
    fn a_parameter_with_no_values_is_skipped() {
        let builder = es_builder("Patient");
        let empty = |name: &str, param_type, modifier| param_with(name, param_type, modifier, &[]);

        for param in [
            empty("near", SearchParamType::Special, None),
            empty("family", SearchParamType::String, None),
            empty("gender", SearchParamType::Token, Some(SearchModifier::Not)),
            empty("_id", SearchParamType::Token, None),
            empty("_lastUpdated", SearchParamType::Date, None),
            empty("_text", SearchParamType::Special, None),
            empty("_content", SearchParamType::Special, None),
        ] {
            let name = param.name.clone();
            let alone = builder
                .build(&SearchQuery::new("Patient").with_parameter(param.clone()))
                .unwrap_or_else(|e| panic!("{name}: no values is not an error: {e:?}"));
            assert!(must_of(&alone).is_empty(), "{name}: {alone:?}");

            let beside = builder
                .build(
                    &SearchQuery::new("Patient")
                        .with_parameter(param)
                        .with_parameter(param_with("_id", SearchParamType::Token, None, &["a"])),
                )
                .unwrap_or_else(|e| panic!("{name}: no values is not an error: {e:?}"));
            assert_eq!(
                must_of(&beside),
                vec![json!({ "term": { "resource_id": "a" } })],
                "{name}: only the parameter that asked for something is a clause"
            );
        }
    }

    /// What `_id`, `_lastUpdated`, `_text`, `_content` and `:missing` built
    /// before they build still, and none is mistaken for a parameter that has
    /// no clause (`_text` and `_content` are `special`-typed).
    #[test]
    fn the_name_dispatched_parameters_still_build() {
        let builder = es_builder("Patient");
        let must = |param: SearchParameter| {
            let name = param.name.clone();
            let built = builder
                .build(&SearchQuery::new("Patient").with_parameter(param))
                .unwrap_or_else(|e| panic!("{name} must build: {e:?}"));
            let mut must = must_of(&built);
            assert_eq!(must.len(), 1, "{name}: {built:?}");
            must.remove(0)
        };

        assert_eq!(
            must(param_with("_id", SearchParamType::Token, None, &["a"])),
            json!({ "term": { "resource_id": "a" } })
        );
        assert_eq!(
            must(param_with(
                "_lastUpdated",
                SearchParamType::Date,
                None,
                &["2026-09-01"]
            ))["range"]["last_updated"],
            json!({ "gte": "2026-09-01", "lt": "2026-09-02" })
        );
        assert_eq!(
            must(param_with(
                "_text",
                SearchParamType::Special,
                None,
                &["headache"]
            )),
            json!({ "match": { "narrative_text": { "query": "headache", "operator": "and" } } })
        );
        assert_eq!(
            must(param_with(
                "_content",
                SearchParamType::Special,
                None,
                &["aspirin"]
            )),
            json!({ "match": { "content_text": { "query": "aspirin", "operator": "and" } } })
        );

        // `:missing` is a presence test on the parameter's nested group.
        let missing = must(param_with(
            "email",
            SearchParamType::String,
            Some(SearchModifier::Missing),
            &["true"],
        ));
        assert_eq!(
            missing,
            json!({ "bool": { "must_not": [ { "nested": {
                "path": "search_params.string",
                "query": { "term": { "search_params.string.name": "email" } }
            } } ] } })
        );
        assert_eq!(
            must(param_with(
                "_id",
                SearchParamType::Token,
                Some(SearchModifier::Missing),
                &["false"]
            )),
            json!({ "exists": { "field": "resource_id" } })
        );
    }

    /// D3 characterization: `:missing` on a `special`-typed parameter is
    /// resolved before the parameter has to have a clause, against the string
    /// group (`modifier_handlers::nested_path_for_type`). Special parameters
    /// write no index entries, so that group never holds one of this name:
    /// `:missing=true` is every resource and `:missing=false` none. Left as it
    /// is by #2510; this pins what it builds today.
    #[test]
    fn missing_on_a_special_typed_parameter_is_a_presence_test_on_the_string_group() {
        let builder = es_builder("Location");
        let built = |value: &str| {
            let query = SearchQuery::new("Location").with_parameter(param_with(
                "near",
                SearchParamType::Special,
                Some(SearchModifier::Missing),
                &[value],
            ));
            must_of(&builder.build(&query).expect("`:missing` builds a clause"))
        };

        let nested = json!({ "nested": {
            "path": "search_params.string",
            "query": { "term": { "search_params.string.name": "near" } }
        } });
        assert_eq!(
            built("true"),
            vec![json!({ "bool": { "must_not": [nested.clone()] } })]
        );
        assert_eq!(built("false"), vec![nested]);
    }

    /// The shared `:of-type` expectation (`modifier_parity_suite`) is that a
    /// value with fewer than three parts "names nothing". The handler used to
    /// return no clause for one, so a lone short value left the query without
    /// a constraint (every Patient) and, in an OR-list, the value vanished.
    #[test]
    fn a_short_of_type_value_matches_nothing_instead_of_vanishing() {
        let builder = es_builder("Patient");
        let of_type = |values: &[&str]| {
            let query = SearchQuery::new("Patient").with_parameter(param_with(
                "identifier",
                SearchParamType::Token,
                Some(SearchModifier::OfType),
                values,
            ));
            must_of(&builder.build(&query).expect("`:of-type` builds a clause"))
        };

        for short in ["MR|12345", "12345", "a|b"] {
            let clauses = of_type(&[short]);
            assert_eq!(clauses, vec![json!({ "match_none": {} })], "{short}");
        }

        // Beside a full value the short one is a `match_none` alternative: the
        // OR keeps both, it does not collapse to the first.
        let clauses = of_type(&[
            "http://terminology.hl7.org/CodeSystem/v2-0203|MR|12345",
            "MR|12345",
        ]);
        let should = clauses[0]["bool"]["should"]
            .as_array()
            .expect("two values are an OR");
        assert_eq!(should.len(), 2);
        assert_eq!(should[1], json!({ "match_none": {} }));
        assert!(should[0]["nested"].is_object());
    }

    /// A clause for every value but `bad`: no real handler leaves a value
    /// without one any more, so this drives the per-value rule directly.
    fn all_but_bad(value: &SearchValue) -> Option<Value> {
        (value.value != "bad").then(|| json!({ "term": { "v": value.value } }))
    }

    /// A value `clause_for` builds nothing for is never dropped from the OR,
    /// plain or under `:not`. Dropped under `:not`, `NOT (a OR b)` is `NOT a`
    /// and the search widens; dropped without it, `a OR b` is `a`.
    #[test]
    fn a_value_without_a_clause_is_refused_not_dropped_from_the_or() {
        for modifier in [None, Some(SearchModifier::Not)] {
            let param = param_with(
                "code",
                SearchParamType::Token,
                modifier.clone(),
                &["a", "bad", "b"],
            );
            match EsQueryBuilder::or_of_values(&param, all_but_bad) {
                Err(SearchError::QueryParseError { message }) => assert_eq!(
                    message,
                    "search parameter 'code' cannot be evaluated by the Elasticsearch backend: \
                     no clause can be built for the value 'bad', and leaving it out would \
                     answer a different search than the one asked",
                    "{modifier:?}"
                ),
                other => panic!("{modifier:?}: expected a refusal, got {other:?}"),
            }
        }
    }

    /// When no value has a clause the result is `Ok(None)`, which `build`
    /// turns into the parameter-level refusal (or a skip, for no values), and
    /// when every value has one the OR is built as before, `:not` around it.
    #[test]
    fn the_or_of_values_is_unchanged_when_every_value_has_a_clause() {
        let build = |modifier: Option<SearchModifier>, values: &[&str]| {
            let param = param_with("code", SearchParamType::Token, modifier, values);
            EsQueryBuilder::or_of_values(&param, all_but_bad).unwrap()
        };
        let term = |v: &str| json!({ "term": { "v": v } });

        for modifier in [None, Some(SearchModifier::Not)] {
            assert_eq!(build(modifier.clone(), &["bad"]), None, "{modifier:?}");
            assert_eq!(
                build(modifier, &["bad", "bad"]),
                None,
                "every value without a clause leaves the refusal to `build`"
            );
        }
        assert_eq!(build(None, &[]), None);

        assert_eq!(build(None, &["a"]), Some(term("a")));
        assert_eq!(
            build(None, &["a", "b"]),
            Some(json!({ "bool": {
                "should": [term("a"), term("b")],
                "minimum_should_match": 1
            } }))
        );
        assert_eq!(
            build(Some(SearchModifier::Not), &["a", "b"]),
            Some(json!({ "bool": { "must_not": [ { "bool": {
                "should": [term("a"), term("b")],
                "minimum_should_match": 1
            } } ] } }))
        );
    }
}
