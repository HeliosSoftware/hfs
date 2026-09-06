//! Elasticsearch Query DSL builder.
//!
//! Translates FHIR `SearchQuery` into Elasticsearch Query DSL JSON.

use serde_json::{Value, json};

use crate::types::{
    CompartmentMembership, PageCursor, SearchModifier, SearchParamType, SearchParameter,
    SearchPrefix, SearchQuery, SortDirection, SortDirective, strip_reference_version,
};

use super::fts;
use super::modifier_handlers;
use super::parameter_handlers::{composite, date, number, quantity, reference, string, token, uri};

/// A complete Elasticsearch query body ready to be sent.
#[derive(Debug, Clone)]
pub struct EsQuery {
    /// The complete query body.
    pub body: Value,
    /// The index to search.
    pub index: String,
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

/// Builds Elasticsearch queries from FHIR search queries.
pub struct EsQueryBuilder<'a> {
    tenant_id: &'a str,
    #[allow(dead_code)]
    resource_type: &'a str,
    index: String,
}

impl<'a> EsQueryBuilder<'a> {
    /// Creates a new query builder.
    pub fn new(tenant_id: &'a str, resource_type: &'a str, index: String) -> Self {
        Self {
            tenant_id,
            resource_type,
            index,
        }
    }

    /// Builds a complete ES query from a FHIR SearchQuery.
    pub fn build(&self, query: &SearchQuery) -> EsQuery {
        let mut must_clauses: Vec<Value> = Vec::new();
        let mut filter_clauses: Vec<Value> = vec![
            json!({ "term": { "tenant_id": self.tenant_id } }),
            json!({ "term": { "is_deleted": false } }),
        ];

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
            if let Some(clause) = self.build_parameter_clause(param) {
                must_clauses.push(clause);
            }
        }

        // Compartment membership: a resource joins the compartment if it
        // references `comp.reference` via ANY of the membership params (OR),
        // per the FHIR CompartmentDefinition. Modeled as a single nested query
        // over the stored reference search params.
        if let Some(comp) = &query.compartment {
            if let Some(clause) = Self::build_compartment_clause(comp) {
                must_clauses.push(clause);
            }
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

        // Add sorting
        let sort = self.build_sort(&query.sort);
        body["sort"] = sort;

        // Add pagination
        let count = query.count.unwrap_or(20);
        body["size"] = json!(count);

        if let Some(ref cursor_str) = query.cursor {
            if let Ok(cursor) = PageCursor::decode(cursor_str) {
                let search_after = self.build_search_after(&cursor);
                body["search_after"] = search_after;
            }
        } else if let Some(offset) = query.offset {
            body["from"] = json!(offset);
        }

        // Track total hits
        body["track_total_hits"] = json!(true);

        // When the query contributes relevance (full-text), ask ES to compute
        // `_score` even though we sort by a field — otherwise `_score` is null
        // and we can't populate `Bundle.entry.search.score`. A `_sort=_score`
        // already scores natively, so this only matters for the field-sort case.
        if query_has_relevance(query) {
            body["track_scores"] = json!(true);
        }

        EsQuery {
            body,
            index: self.index.clone(),
        }
    }

    /// Builds a nested clause matching resources that reference
    /// `comp.reference` through ANY of the compartment membership params
    /// (logical OR), which is how a resource joins a FHIR compartment.
    ///
    /// Reference matching is version-agnostic and mirrors the reference
    /// parameter handler: the stored reference must equal the base reference or
    /// carry a `/_history/<vid>` suffix. The membership params come from the
    /// bundled FHIR `CompartmentDefinition`s. Returns `None` if there are no
    /// membership params or no reference.
    fn build_compartment_clause(comp: &CompartmentMembership) -> Option<Value> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return None;
        }

        let base = strip_reference_version(&comp.reference);

        Some(json!({
            "nested": {
                "path": "search_params.reference",
                "query": {
                    "bool": {
                        "must": [
                            { "terms": { "search_params.reference.name": comp.params } },
                            {
                                "bool": {
                                    "should": [
                                        { "term": { "search_params.reference.reference": base } },
                                        { "prefix": { "search_params.reference.reference": format!("{base}/_history/") } }
                                    ],
                                    "minimum_should_match": 1
                                }
                            }
                        ]
                    }
                }
            }
        }))
    }

    /// Builds a clause for a single search parameter.
    fn build_parameter_clause(&self, param: &SearchParameter) -> Option<Value> {
        // Presence is independent of the parameter's ordinary value syntax.
        // Resolve it before `_id` and `_lastUpdated`, which would otherwise
        // interpret the boolean literal as an ID or date value.
        if param.modifier == Some(SearchModifier::Missing) {
            return modifier_handlers::build_missing_clause(param);
        }

        // Handle special parameters
        match param.name.as_str() {
            "_id" => return self.build_id_clause(param),
            "_lastUpdated" => return self.build_last_updated_clause(param),
            "_text" => return fts::build_text_clause(param),
            "_content" => return fts::build_content_clause(param),
            _ => {}
        }

        // Dispatch based on parameter type
        let clauses: Vec<Value> = param
            .values
            .iter()
            .filter_map(|value| self.build_value_clause(param, &value.value, value.prefix))
            .collect();

        if clauses.is_empty() {
            return None;
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
            return Some(json!({ "bool": { "must_not": [combined] } }));
        }

        Some(combined)
    }

    /// Builds a clause for a single value of a parameter.
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
    fn build_id_clause(&self, param: &SearchParameter) -> Option<Value> {
        let ids: Vec<&str> = param.values.iter().map(|v| v.value.as_str()).collect();
        if ids.len() == 1 {
            Some(json!({ "term": { "resource_id": ids[0] } }))
        } else {
            Some(json!({ "terms": { "resource_id": ids } }))
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
                    date::DateRange::Within(range) => range,
                    date::DateRange::Outside(range) => {
                        json!({ "bool": { "must_not": [range] } })
                    }
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
    fn build_sort(&self, directives: &[SortDirective]) -> Value {
        if directives.is_empty() {
            // Default sort: _lastUpdated descending, then _id for tie-breaking
            return json!([
                { "last_updated": { "order": "desc" } },
                { "resource_id": { "order": "asc" } }
            ]);
        }

        let mut sort_clauses: Vec<Value> = Vec::new();

        for directive in directives {
            let order = match directive.direction {
                SortDirection::Ascending => "asc",
                SortDirection::Descending => "desc",
            };

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
                    // (the SQL backends' MIN/MAX).
                    let mode = if order == "asc" { "min" } else { "max" };
                    sort_clauses.push(json!({
                        field: {
                            "order": order,
                            "mode": mode,
                            "nested": {
                                "path": format!("search_params.{group}"),
                                "filter": {
                                    "term": { format!("search_params.{group}.name"): name }
                                }
                            },
                            "missing": if order == "asc" { "_last" } else { "_first" }
                        }
                    }));
                }
            }
        }

        // Always add tie-breaker
        sort_clauses.push(json!({ "resource_id": { "order": "asc" } }));

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
pub fn build_count_query(tenant_id: &str, resource_type: &str, query: &SearchQuery) -> Value {
    let builder = EsQueryBuilder::new(tenant_id, resource_type, String::new());
    let mut es_query = builder.build(query);

    json!({ "query": es_query.body["query"].take() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchValue, SortDirection};

    #[test]
    fn test_basic_query_build() {
        let query = SearchQuery::new("Patient");
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let es_query = builder.build(&query);

        // Should have tenant and is_deleted filters
        let filters = &es_query.body["query"]["bool"]["filter"];
        assert!(filters.is_array());
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
        let es_query = builder.build(&query);
        let body_str = serde_json::to_string(&es_query.body).unwrap();
        assert!(body_str.contains("resource_id"));
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
            let es_query = builder.build(&query);
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
        builder.build(&query).body["query"]["bool"]["must"][0].clone()
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
        let es_query = builder.build(&query);

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
        let es_query = builder.build(&query);

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
        let es_query = builder.build(&query);

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
        let es_query = builder.build(&query);

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
        let sort = &builder.build(&query).body["sort"][0];

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

    #[test]
    fn test_token_sort_descending_uses_max_mode() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "gender".to_string(),
            direction: SortDirection::Descending,
            param_type: Some(SearchParamType::Token),
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = &builder.build(&query).body["sort"][0];

        let clause = &sort["search_params.token.code"];
        assert!(!clause.is_null(), "token sorts on the code, got {sort}");
        assert_eq!(clause["order"], "desc");
        assert_eq!(clause["mode"], "max");
        assert_eq!(clause["missing"], "_first");
    }

    #[test]
    fn test_untyped_sort_falls_back_to_string_group() {
        let query = SearchQuery::new("Patient").with_sort(SortDirective {
            parameter: "name".to_string(),
            direction: SortDirection::Ascending,
            param_type: None,
        });
        let builder = EsQueryBuilder::new("acme", "Patient", "hfs_acme_patient".to_string());
        let sort = &builder.build(&query).body["sort"][0];
        assert!(
            !sort["search_params.string.value.keyword"].is_null(),
            "untyped parameters keep the string-group sort, got {sort}"
        );
    }
}
