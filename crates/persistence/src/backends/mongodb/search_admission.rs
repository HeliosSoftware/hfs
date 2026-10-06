//! Admission limit for potentially broad standard searches (#1748).
//!
//! A broad search keeps issuing database operations for seconds while a read
//! by id finishes in about a millisecond, and both draw on the same driver
//! pool. When `HFS_MONGODB_BROAD_SEARCH_CONCURRENCY` is set, `MongoBackend`
//! holds a semaphore with that many permits (checked by
//! [`check_broad_search_limit`]); a standard search that
//! [`is_potentially_broad`] classifies as broad takes a permit before its
//! first database operation and keeps it until its response is built. Unset,
//! there is no semaphore and no search waits.
//!
//! The classification is a heuristic decided from the query alone, before any
//! database work. It can be wrong in both directions; it aims to catch the
//! common expensive shapes cheaply.

use crate::types::{SearchParamType, SearchParameter, SearchPrefix, SearchQuery};

/// Environment variable that sets the number of permits.
pub(crate) const BROAD_SEARCH_CONCURRENCY_ENV: &str = "HFS_MONGODB_BROAD_SEARCH_CONCURRENCY";

/// Whether a standard (non-`_contained`) search should take a permit.
///
/// A search is narrow when at least one of its predicates usually bounds the
/// result (see [`is_narrowing_predicate`]), or when it is a list or
/// compartment search. A search with no predicates at all is narrow only as
/// a plain paged read: computing a total, sorting on a search parameter, or
/// skipping rows makes it count or order the whole type. Everything else is
/// potentially broad.
pub(crate) fn is_potentially_broad(query: &SearchQuery) -> bool {
    is_potentially_broad_operation(query, false)
}

/// A direct count always computes a total, even when the query has no `_total` option.
pub(crate) fn is_potentially_broad_count(query: &SearchQuery) -> bool {
    is_potentially_broad_operation(query, true)
}

fn is_potentially_broad_operation(query: &SearchQuery, count_only: bool) -> bool {
    if !query.list.is_empty() || query.compartment.is_some() {
        return false;
    }
    if query.parameters.iter().any(is_narrowing_predicate) {
        return false;
    }
    if !query.parameters.is_empty() || !query.reverse_chains.is_empty() {
        return true;
    }
    count_only || query.wants_total() || sorts_on_search_parameter(query) || skips_rows(query)
}

/// An unmodified, unchained predicate that usually bounds the result: a
/// positive `_id`, a reference or uri parameter, a date whose values all use
/// `eq` or `ap`, or `identifier` with both a system and a code. A reference
/// to a large organization or a busy day can still match many resources;
/// these are treated as narrow because they usually do not.
fn is_narrowing_predicate(param: &SearchParameter) -> bool {
    if param.modifier.is_some() || !param.chain.is_empty() || param.values.is_empty() {
        return false;
    }
    if param.name == "_id" {
        return true;
    }
    match param.param_type {
        SearchParamType::Reference | SearchParamType::Uri => true,
        SearchParamType::Date => param
            .values
            .iter()
            .all(|value| matches!(value.prefix, SearchPrefix::Eq | SearchPrefix::Ap)),
        SearchParamType::Token if param.name == "identifier" => param.values.iter().all(|value| {
            value
                .value
                .split_once('|')
                .is_some_and(|(system, code)| !system.is_empty() && !code.is_empty())
        }),
        _ => false,
    }
}

/// `_id`, `_lastUpdated` and `_score` follow the resources collection's own
/// order; any other key is sorted from `search_index` over every candidate.
fn sorts_on_search_parameter(query: &SearchQuery) -> bool {
    query
        .sort
        .iter()
        .any(|d| !matches!(d.parameter.as_str(), "_id" | "_lastUpdated" | "_score"))
}

/// A cursor replaces the offset, and `_offset=0` skips nothing.
fn skips_rows(query: &SearchQuery) -> bool {
    query.cursor.is_none() && query.offset.unwrap_or(0) > 0
}

/// Checks a configured number of permits: zero, or more than a semaphore can
/// hold, is an `Err`.
pub(crate) fn check_broad_search_limit(limit: usize) -> Result<usize, String> {
    if limit == 0 {
        return Err(format!(
            "{BROAD_SEARCH_CONCURRENCY_ENV} must be a positive integer; got 0"
        ));
    }
    if limit > tokio::sync::Semaphore::MAX_PERMITS {
        return Err(format!(
            "{BROAD_SEARCH_CONCURRENCY_ENV} must be at most {}; got {limit}",
            tokio::sync::Semaphore::MAX_PERMITS
        ));
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        CompartmentMembership, ReverseChainedParameter, SearchModifier, SearchValue, SortDirective,
        TotalMode,
    };

    fn param(name: &str, param_type: SearchParamType, values: &[SearchValue]) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: None,
            values: values.to_vec(),
            chain: vec![],
            components: vec![],
        }
    }

    fn query_with(params: Vec<SearchParameter>) -> SearchQuery {
        let mut query = SearchQuery::new("Observation");
        query.parameters = params;
        query
    }

    #[test]
    fn narrowing_predicates_make_a_search_narrow() {
        let narrow = [
            param("_id", SearchParamType::Token, &[SearchValue::eq("o-1")]),
            param(
                "subject",
                SearchParamType::Reference,
                &[SearchValue::eq("Patient/p-1")],
            ),
            param(
                "url",
                SearchParamType::Uri,
                &[SearchValue::eq("http://example.org/vs")],
            ),
            param(
                "date",
                SearchParamType::Date,
                &[
                    SearchValue::eq("2026-10-02"),
                    SearchValue::new(SearchPrefix::Ap, "2026-10-03"),
                ],
            ),
            param(
                "identifier",
                SearchParamType::Token,
                &[SearchValue::eq("http://example.org/mrn|123")],
            ),
        ];
        for p in narrow {
            let name = p.name.clone();
            let mut query = query_with(vec![
                param("code", SearchParamType::Token, &[SearchValue::eq("1234-5")]),
                p,
            ]);
            assert!(!is_potentially_broad(&query), "{name} narrows");
            query.total = Some(TotalMode::Accurate);
            query.sort = vec![SortDirective::parse("date")];
            query.offset = Some(50);
            assert!(
                !is_potentially_broad(&query),
                "{name} narrows a total, a sort and an offset too"
            );
        }
    }

    #[test]
    fn other_predicates_are_broad_whatever_their_count() {
        let token = |name: &str, value: &str| {
            param(name, SearchParamType::Token, &[SearchValue::eq(value)])
        };
        assert!(is_potentially_broad(&query_with(vec![token(
            "code", "1234-5"
        )])));
        assert!(is_potentially_broad(&query_with(vec![
            token("code", "1234-5"),
            token("status", "final"),
            token("category", "laboratory"),
        ])));
        assert!(is_potentially_broad(&query_with(vec![param(
            "value-quantity",
            SearchParamType::Quantity,
            &[SearchValue::new(SearchPrefix::Gt, "5")],
        )])));
    }

    #[test]
    fn a_weakened_narrowing_predicate_is_broad() {
        let date_range = param(
            "date",
            SearchParamType::Date,
            &[SearchValue::new(SearchPrefix::Ge, "2026-01-01")],
        );
        let mixed_dates = param(
            "date",
            SearchParamType::Date,
            &[
                SearchValue::eq("2026-10-02"),
                SearchValue::new(SearchPrefix::Lt, "2026-10-03"),
            ],
        );
        let code_only = param(
            "identifier",
            SearchParamType::Token,
            &[SearchValue::eq("|123")],
        );
        let system_only = param(
            "identifier",
            SearchParamType::Token,
            &[SearchValue::eq("http://example.org/mrn|")],
        );
        let mut negated_id = param("_id", SearchParamType::Token, &[SearchValue::eq("o-1")]);
        negated_id.modifier = Some(SearchModifier::Not);
        let mut missing_subject = param(
            "subject",
            SearchParamType::Reference,
            &[SearchValue::eq("true")],
        );
        missing_subject.modifier = Some(SearchModifier::Missing);
        let mut chained = param(
            "subject",
            SearchParamType::Reference,
            &[SearchValue::eq("Smith")],
        );
        chained.chain = vec![crate::types::ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }];
        let no_values = param("subject", SearchParamType::Reference, &[]);
        for p in [
            date_range,
            mixed_dates,
            code_only,
            system_only,
            negated_id,
            missing_subject,
            chained,
            no_values,
        ] {
            let shown = format!("{p:?}");
            assert!(is_potentially_broad(&query_with(vec![p])), "{shown}");
        }
    }

    #[test]
    fn a_search_without_predicates_is_narrow_only_as_a_plain_paged_read() {
        let plain = SearchQuery::new("Observation");
        assert!(!is_potentially_broad(&plain));

        for (label, query) in [
            ("_total=accurate", {
                let mut q = plain.clone();
                q.total = Some(TotalMode::Accurate);
                q
            }),
            ("_total=estimate", {
                let mut q = plain.clone();
                q.total = Some(TotalMode::Estimate);
                q
            }),
            ("_sort=date", {
                let mut q = plain.clone();
                q.sort = vec![SortDirective::parse("-date")];
                q
            }),
            ("_offset=20", {
                let mut q = plain.clone();
                q.offset = Some(20);
                q
            }),
        ] {
            assert!(is_potentially_broad(&query), "{label} is broad");
        }

        for (label, query) in [
            ("_total=none", {
                let mut q = plain.clone();
                q.total = Some(TotalMode::None);
                q
            }),
            ("_offset=0", {
                let mut q = plain.clone();
                q.offset = Some(0);
                q
            }),
            ("_sort=-_lastUpdated,_id", {
                let mut q = plain.clone();
                q.sort = vec![
                    SortDirective::parse("-_lastUpdated"),
                    SortDirective::parse("_id"),
                ];
                q
            }),
            ("a cursor with a stale offset", {
                let mut q = plain.clone();
                q.offset = Some(20);
                q.cursor = Some("cursor".to_string());
                q
            }),
        ] {
            assert!(
                !is_potentially_broad(&query),
                "{label} is a plain paged read"
            );
        }
    }

    #[test]
    fn reverse_chains_are_broad_and_list_or_compartment_searches_are_narrow() {
        let mut has = SearchQuery::new("Patient");
        has.reverse_chains = vec![ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "code",
            SearchValue::eq("1234-5"),
        )];
        assert!(is_potentially_broad(&has));

        let mut listed = query_with(vec![param(
            "code",
            SearchParamType::Token,
            &[SearchValue::eq("1234-5")],
        )]);
        listed.list = vec!["list-1".to_string()];
        assert!(!is_potentially_broad(&listed));

        let mut compartment = SearchQuery::new("Observation");
        compartment.total = Some(TotalMode::Accurate);
        compartment.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string()],
            reference: "Patient/p-1".to_string(),
        });
        assert!(!is_potentially_broad(&compartment));
    }

    #[test]
    fn direct_counts_do_not_depend_on_the_total_option() {
        for total in [None, Some(TotalMode::None), Some(TotalMode::Accurate)] {
            let mut query = SearchQuery::new("Observation");
            query.total = total;
            assert!(is_potentially_broad_count(&query));
            query.parameters.push(param(
                "_id",
                SearchParamType::Token,
                &[SearchValue::eq("o-1")],
            ));
            assert!(!is_potentially_broad_count(&query));
        }
    }

    #[test]
    fn a_configured_limit_must_be_positive() {
        assert_eq!(check_broad_search_limit(1), Ok(1));
        assert_eq!(check_broad_search_limit(40), Ok(40));
        let zero = check_broad_search_limit(0).unwrap_err();
        assert!(zero.contains(BROAD_SEARCH_CONCURRENCY_ENV), "{zero}");
        assert!(check_broad_search_limit(tokio::sync::Semaphore::MAX_PERMITS + 1).is_err());
    }
}
