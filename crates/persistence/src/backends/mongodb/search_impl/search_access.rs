//! Search-index access shared by ID collectors and batched search plans.
//!
//! Callers choose candidates, ordering and totals. This module chooses indexes,
//! bounds membership to those candidates and applies operation-specific retries.

use super::super::search_deadline::{self, SearchCommand};
use super::super::search_index_catalog::{SEARCH_COMPOSITE_INDEX, SEARCH_DATE_RANGE_INDEX};
use super::internal_error;
use crate::error::{QueryErrorExt, StorageResult};
use crate::types::{SearchModifier, SearchParamType, SearchParameter, SearchPrefix};
use mongodb::{
    bson::{Bson, Document, doc},
    options::Hint,
};
use std::collections::HashSet;

pub(super) const SEARCH_TOKEN_MEMBERSHIP_INDEX: &str = "idx_search_token_v2";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum IndexAccess {
    Automatic,
    DateRange,
    DateEndRange,
}

#[derive(Clone, Copy)]
pub(super) struct PredicateAccess {
    pub(super) discovery: IndexAccess,
    membership: MembershipAccess,
}

impl Default for PredicateAccess {
    fn default() -> Self {
        Self {
            discovery: IndexAccess::Automatic,
            membership: MembershipAccess::CandidateIds,
        }
    }
}

impl PredicateAccess {
    pub(super) fn new(filter: &Document, parameter: &SearchParameter, indexes_ready: bool) -> Self {
        if !indexes_ready {
            return Self::default();
        }
        let discovery = if parameter.param_type != SearchParamType::Date
            || matches!(parameter.modifier, Some(SearchModifier::Missing))
        {
            IndexAccess::Automatic
        } else if parameter.values.iter().any(|value| {
            matches!(
                value.prefix,
                SearchPrefix::Ge | SearchPrefix::Gt | SearchPrefix::Ne
            )
        }) {
            IndexAccess::DateEndRange
        } else {
            IndexAccess::DateRange
        };
        Self {
            discovery,
            membership: token_membership_access(filter, parameter),
        }
    }

    pub(super) fn discovery_hint(self) -> Option<Hint> {
        match self.discovery {
            IndexAccess::Automatic => None,
            IndexAccess::DateRange | IndexAccess::DateEndRange => {
                Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned()))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MembershipAccess {
    CandidateIds,
    Automatic,
    TokenEquality,
}

/// A code-only token leaves the system key unbounded, so MongoDB chooses
/// between covered value reads and candidate-ID seeks. Constrained systems
/// allow the value index to seek through every key preceding resource_id.
fn token_membership_access(filter: &Document, parameter: &SearchParameter) -> MembershipAccess {
    if parameter.param_type != SearchParamType::Token
        || parameter.modifier.is_some()
        || !parameter.chain.is_empty()
        || parameter.values.len() != 1
        || !parameter
            .values
            .iter()
            .all(|value| value.prefix == SearchPrefix::Eq)
        || ![
            "tenant_id",
            "resource_type",
            "param_name",
            "value_token_code",
        ]
        .iter()
        .all(|field| filter.get_str(*field).is_ok_and(|value| !value.is_empty()))
        || !filter.keys().all(|field| {
            matches!(
                field.as_str(),
                "tenant_id"
                    | "resource_type"
                    | "param_name"
                    | "value_token_code"
                    | "value_token_system"
            )
        })
    {
        return MembershipAccess::CandidateIds;
    }
    match filter.get("value_token_system") {
        None => MembershipAccess::Automatic,
        Some(Bson::String(system)) if !system.is_empty() => MembershipAccess::TokenEquality,
        Some(Bson::Document(condition))
            if condition.len() == 1
                && condition.get_array("$in").is_ok_and(|systems| {
                    // One named system expands to itself and the implicit-system marker.
                    matches!(systems.len(), 1 | 2)
                        && systems
                            .iter()
                            .all(|system| system.as_str().is_some_and(|system| !system.is_empty()))
                }) =>
        {
            MembershipAccess::TokenEquality
        }
        _ => MembershipAccess::CandidateIds,
    }
}

#[derive(Clone, Copy)]
pub(super) enum CandidateSource {
    Unfiltered,
    IndexedPredicate,
}

impl CandidateSource {
    pub(super) fn membership_hint(self, access: PredicateAccess, position: usize) -> Option<Hint> {
        // Resource-order candidates and explicit ID seeds have not passed an
        // indexed predicate. Keep their first check on candidate IDs.
        let access = if matches!(self, Self::IndexedPredicate) || position > 0 {
            access.membership
        } else {
            MembershipAccess::CandidateIds
        };
        match access {
            MembershipAccess::CandidateIds => Some(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned())),
            MembershipAccess::Automatic => None,
            MembershipAccess::TokenEquality => {
                Some(Hint::Name(SEARCH_TOKEN_MEMBERSHIP_INDEX.to_owned()))
            }
        }
    }
}

/// Preserve the entire scoped predicate, including date alternatives and
/// cursor bounds. Each parameter occurrence is checked separately, because
/// repeated bounds may match different values on the same resource.
pub(super) fn membership_pipeline(predicate: &Document, ids: &[String]) -> Vec<Document> {
    vec![
        doc! { "$match": { "$and": [
            predicate.clone(), { "resource_id": { "$in": ids } },
        ] } },
        doc! { "$group": { "_id": "$resource_id" } },
    ]
}

pub(super) fn date_hint_rejected(hint: &Option<Hint>, error: &mongodb::error::Error) -> bool {
    *hint == Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned())) && hint_rejected(error)
}

/// Runs `operation` with `hint`. When the hint names the date range index and
/// the server rejects it because the index is gone, which happens on a node
/// still hinting a superseded value index that a newer node has dropped
/// (rolling upgrade), runs the operation once more without it. The identity
/// index is required, so a rejection of that hint propagates.
pub(super) async fn with_hint_fallback<T, F>(
    mut hint: Option<Hint>,
    operation: impl Fn(Option<Hint>) -> F,
) -> StorageResult<mongodb::error::Result<T>>
where
    F: SearchCommand + IntoFuture<Output = mongodb::error::Result<T>>,
{
    loop {
        // Build a fresh command budget for the unhinted retry as well.
        let result = operation(hint.clone()).search_deadline()?.await;
        if let Err(error) = &result
            && date_hint_rejected(&hint, error)
        {
            search_deadline::check()?;
            tracing::warn!(error = %error, ?hint, "Hinted index is missing; running unhinted");
            hint = None;
            continue;
        }
        return Ok(result);
    }
}

/// MongoDB answers a hint naming a missing index with `BadValue` (code 2).
pub(super) fn hint_rejected(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => {
            command.code == 2
                && command
                    .message
                    .contains("hint provided does not correspond to an existing index")
        }
        _ => false,
    }
}

/// Read distinct membership only within the current bounded candidate set.
pub(super) async fn matching_index_ids(
    index: &mongodb::Collection<Document>,
    candidates: &HashSet<String>,
    predicate: &Document,
    hint: Option<Hint>,
) -> StorageResult<HashSet<String>> {
    let ids: Vec<_> = candidates.iter().cloned().collect();
    let mut cursor = aggregate_membership(
        index,
        membership_pipeline(predicate, &ids),
        hint,
        "Failed to filter search candidates",
    )
    .await?;
    let mut matched = HashSet::new();
    while cursor
        .advance()
        .await
        .or_query_error("Failed to read filtered candidate")?
    {
        let row = cursor
            .deserialize_current()
            .or_query_error("Invalid filtered candidate")?;
        let id = row
            .get_str("_id")
            .map_err(|_| internal_error("Search candidate is missing resource_id".to_owned()))?;
        if candidates.contains(id) {
            matched.insert(id.to_owned());
        }
    }
    Ok(matched)
}

/// Membership of a composite parameter: per comma-separated value, every
/// component's scoped filter, in declaration order. A resource matches when one
/// `composite_group` has a row for every component of some value. The rows of
/// different components are paired by group, so no single filter document can
/// express this and the check always reads one find per component.
#[derive(Debug)]
pub(super) struct CompositeMembership {
    pub(super) values: Vec<Vec<Document>>,
}

impl CompositeMembership {
    /// The members of `candidates`, read through `idx_search_composite`, like
    /// every other bounded pair check. A missing index fails the search.
    pub(super) async fn matching_ids(
        &self,
        index: &mongodb::Collection<Document>,
        candidates: &HashSet<String>,
    ) -> StorageResult<HashSet<String>> {
        super::composite_pair_matches(
            index,
            &self.values,
            candidates,
            Some(SEARCH_COMPOSITE_INDEX),
            None,
        )
        .await
    }
}

fn membership_hint_rejected(hint: &Option<Hint>, error: &mongodb::error::Error) -> bool {
    matches!(hint, Some(Hint::Name(name)) if name == SEARCH_TOKEN_MEMBERSHIP_INDEX)
        && hint_rejected(error)
}

/// A background index can disappear after readiness. Retry only that rejected
/// hint through the existing identity-led index, preserving every predicate.
pub(super) async fn aggregate_membership(
    index: &mongodb::Collection<Document>,
    pipeline: Vec<Document>,
    hint: Option<Hint>,
    context: &str,
) -> StorageResult<mongodb::Cursor<Document>> {
    if !matches!(&hint, Some(Hint::Name(name)) if name == SEARCH_TOKEN_MEMBERSHIP_INDEX) {
        let mut command = index.aggregate(pipeline).search_deadline()?;
        if let Some(hint) = hint {
            command = command.hint(hint);
        }
        return command.await.or_query_error(context);
    }
    let opened = index
        .aggregate(pipeline.clone())
        .search_deadline()?
        .hint(Hint::Name(SEARCH_TOKEN_MEMBERSHIP_INDEX.to_owned()))
        .await;
    match opened {
        Err(error) if membership_hint_rejected(&hint, &error) => {
            search_deadline::check()?;
            tracing::warn!(
                error = %error,
                ?hint,
                "Hinted index is missing; retrying through the composite index"
            );
            index
                .aggregate(pipeline)
                .search_deadline()?
                .hint(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()))
                .await
                .or_query_error(context)
        }
        result => result.or_query_error(context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use crate::types::SearchValue;
    use MembershipAccess::{Automatic, CandidateIds, TokenEquality};
    #[test]
    fn token_membership_access_requires_bounded_equality_shapes() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        for (name, modifier, values, access) in [
            ("code only", None, vec!["final"], Automatic),
            (
                "named system",
                None,
                vec!["urn:status|final"],
                TokenEquality,
            ),
            ("no system", None, vec!["|final"], CandidateIds),
            ("system only", None, vec!["urn:status|"], CandidateIds),
            (
                "alternatives",
                None,
                vec!["final", "preliminary"],
                CandidateIds,
            ),
            ("empty code", None, vec![""], CandidateIds),
            (
                "display regex",
                Some(SearchModifier::Text),
                vec!["final"],
                CandidateIds,
            ),
            (
                "presence",
                Some(SearchModifier::Missing),
                vec!["false"],
                CandidateIds,
            ),
        ] {
            let parameter = SearchParameter {
                name: "status".to_owned(),
                param_type: SearchParamType::Token,
                modifier,
                values: values.into_iter().map(SearchValue::eq).collect(),
                chain: vec![],
                components: vec![],
            };
            let filter = backend
                .build_search_index_filter("tenant", "Observation", &parameter)
                .unwrap();
            assert_eq!(
                token_membership_access(&filter, &parameter),
                access,
                "{name}: {filter:?}"
            );
        }
    }

    #[test]
    fn membership_hints_preserve_driver_first_predicate_and_not_ready_paths() {
        let parameter = SearchParameter {
            name: "status".to_owned(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("final")],
            chain: vec![],
            components: vec![],
        };
        let filter = doc! { "tenant_id": "tenant", "resource_type": "Observation",
        "param_name": "status", "value_token_code": "final" };
        let mut empty = parameter.clone();
        empty.values.clear();
        assert_eq!(
            token_membership_access(&filter, &empty),
            MembershipAccess::CandidateIds
        );
        let ready = PredicateAccess::new(&filter, &parameter, true);
        let unready = PredicateAccess::new(&filter, &parameter, false);
        let old_hint = Some(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()));
        assert_eq!(
            ready.discovery_hint(),
            None,
            "driver and probes must remain unchanged"
        );
        assert_eq!(
            CandidateSource::Unfiltered.membership_hint(ready, 0),
            old_hint
        );
        for position in [1, 2, 3] {
            assert_eq!(
                CandidateSource::Unfiltered.membership_hint(ready, position),
                None
            );
        }
        for position in [0, 1, 2] {
            assert_eq!(
                CandidateSource::IndexedPredicate.membership_hint(ready, position),
                None
            );
            assert_eq!(
                CandidateSource::IndexedPredicate.membership_hint(unready, position),
                old_hint
            );
        }
        for (extra, expected) in [
            (
                doc! {"value_token_system": "urn:status"},
                MembershipAccess::TokenEquality,
            ),
            (
                doc! {"value_token_system": {"$in": ["urn:status", "urn:other"]}},
                MembershipAccess::TokenEquality,
            ),
            (
                doc! {"value_token_system": Bson::Null},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_system": ""},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_system": {"$in": []}},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_system": {"$in": ["urn:a", "urn:b", "urn:c"]}},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_system": {"$in": ["urn:status", Bson::Null]}},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_system": {"$in": ["urn:status"], "$ne": "urn:other"}},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"value_token_code": {"$regex": "final"}},
                MembershipAccess::CandidateIds,
            ),
            (
                doc! {"resource_id": {"$gt": "cursor"}},
                MembershipAccess::CandidateIds,
            ),
            (doc! {"composite_group": 1}, MembershipAccess::CandidateIds),
        ] {
            let mut constrained = filter.clone();
            constrained.extend(extra.clone());
            let predicate = PredicateAccess::new(&constrained, &parameter, true);
            assert_eq!(predicate.membership, expected, "{extra:?}");
            assert_eq!(
                CandidateSource::Unfiltered.membership_hint(predicate, 0),
                old_hint
            );
            assert_eq!(
                CandidateSource::IndexedPredicate.membership_hint(predicate, 0),
                Some(Hint::Name(
                    if expected == MembershipAccess::TokenEquality {
                        SEARCH_TOKEN_MEMBERSHIP_INDEX
                    } else {
                        SEARCH_COMPOSITE_INDEX
                    }
                    .to_owned()
                ))
            );
        }
    }

    #[test]
    fn membership_fallback_requires_its_own_missing_value_index() {
        let command_error = |code, message| {
            let command: mongodb::error::CommandError =
                mongodb::bson::from_document(doc! {"code": code, "errmsg": message}).unwrap();
            mongodb::error::Error::from(mongodb::error::ErrorKind::Command(command))
        };
        let missing = "hint provided does not correspond to an existing index";
        let covered = Some(Hint::Name(SEARCH_TOKEN_MEMBERSHIP_INDEX.to_owned()));
        assert!(membership_hint_rejected(
            &covered,
            &command_error(2, missing)
        ));
        assert!(!membership_hint_rejected(&None, &command_error(2, missing)));
        for hint in [
            SEARCH_COMPOSITE_INDEX,
            SEARCH_DATE_RANGE_INDEX,
            "another_index",
        ] {
            assert!(!membership_hint_rejected(
                &Some(Hint::Name(hint.to_owned())),
                &command_error(2, missing)
            ));
        }
        for (code, message) in [
            (2, "invalid query"),
            (13, missing),
            (50, missing),
            (262, missing),
            (91, missing),
        ] {
            assert!(!membership_hint_rejected(
                &covered,
                &command_error(code, message)
            ));
        }
        let network =
            mongodb::error::Error::from(std::io::Error::new(std::io::ErrorKind::TimedOut, missing));
        assert!(!membership_hint_rejected(&covered, &network));
    }
}
