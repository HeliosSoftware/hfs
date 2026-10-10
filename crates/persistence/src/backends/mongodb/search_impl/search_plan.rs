//! MongoDB search planning: bounded index batches and resource existence checks.
//!
//! FHIR filter translation stays in `search_impl`. Plans preserve each predicate
//! occurrence. Pages and exact totals validate live resources; estimates count
//! indexed matches without live-resource validation.

use super::super::probe_cache::{OffsetPageDecision, ProbeCache, ProbeEstimate, ProbeKey};
use super::super::search_index_catalog::SEARCH_DATE_RANGE_INDEX;
use super::collect_documents;
use super::{
    DEFAULT_PAGE_SIZE, MongoBackend, SEARCH_COMPOSITE_INDEX, internal_error,
    missing_presence_filter,
};
use crate::error::{QueryErrorExt, StorageResult};
use crate::types::{SearchModifier, SearchParamType, SearchParameter, SearchPrefix, SearchQuery};
use futures::{TryStreamExt, stream::FuturesUnordered};
use mongodb::{
    bson::{Bson, Document, doc},
    options::Hint,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::Duration,
};

// Limits bound candidate memory and BSON command size, not returned page size.
const MAX_CANDIDATE_ROWS: u64 = 10_000;
// Pages stop early; counts consume every candidate and amortize round trips.
const PAGE_CANDIDATE_BATCH_ROWS: usize = 256;
const COUNT_CANDIDATE_BATCH_ROWS: usize = 4096;
// Exact counts need no ordering. Overlap two bounded validations without
// creating a task per batch or retaining the complete candidate set.
const COUNT_BATCH_CONCURRENCY: usize = 2;
// Deep offsets must not grow the global top-k buffer without bound. They keep
// the streaming page path plus a separate exact count.
const MAX_COMBINED_PAGE_IDS: usize = 10_000;

pub(super) fn combined_page_fits(offset: u32, limit: usize) -> bool {
    (offset as usize)
        .checked_add(limit)
        .is_some_and(|rows| rows <= MAX_COMBINED_PAGE_IDS)
}

fn retained_page_size(offset: u32, limit: usize) -> StorageResult<usize> {
    (offset as usize)
        .checked_add(limit)
        .ok_or_else(|| internal_error("Combined page offset exceeds memory bounds".to_owned()))
}

fn retained_page_limit(offset: u32, limit: usize) -> StorageResult<i64> {
    i64::try_from(retained_page_size(offset, limit)?)
        .map_err(|_| internal_error("Search page limit exceeds BSON range".to_owned()))
}

pub(super) fn parse_total(document: &Document, field: &str) -> StorageResult<u64> {
    document
        .get_i64(field)
        .or_else(|_| document.get_i32(field).map(i64::from))
        .ok()
        .and_then(|total| u64::try_from(total).ok())
        .ok_or_else(|| internal_error(format!("Invalid search total in {field}")))
}

fn facet_total(document: &Document) -> StorageResult<u64> {
    let total = document
        .get_array("total")
        .map_err(|_| internal_error("Missing candidate total".to_owned()))?;
    match total.as_slice() {
        [] => Ok(0),
        [Bson::Document(total)] => parse_total(total, "n"),
        _ => Err(internal_error("Invalid candidate total".to_owned())),
    }
}

#[derive(Default)]
struct IdBounds<'a> {
    lower: Option<&'a str>,
    upper: Option<&'a str>,
}

impl<'a> IdBounds<'a> {
    fn from_boundary(boundary: Option<&'a Document>) -> StorageResult<Self> {
        let Some(boundary) = boundary else {
            return Ok(Self::default());
        };
        let comparisons = boundary
            .get_document("id")
            .map_err(|_| internal_error("Invalid combined ID boundary".to_owned()))?;
        Ok(Self {
            lower: comparisons.get_str("$gt").ok(),
            upper: comparisons.get_str("$lt").ok(),
        })
    }

    fn contains(&self, id: &str) -> bool {
        self.lower.is_none_or(|bound| id > bound) && self.upper.is_none_or(|bound| id < bound)
    }
}

const MAX_CANDIDATE_BYTES: usize = 8 * 1024 * 1024;
// A complete candidate sample with at most this many live matches (1 in 16) selects
// bounded offset execution. Starting value; retune from the 8M/10M results.
const SPARSE_SAMPLE_MAX_LIVE: u64 = 16;
// Cardinality discovery is optional planning work, not part of matching. Bound
// server execution per probe; maxTimeMS does not include all network latency.
pub(crate) const DEFAULT_PROBE_TIMEOUT_MS: u64 = 10;
const SELECTIVE_SEARCH_ROW_LIMIT: u64 = 64;

/// Index access for an unbounded predicate. End comparisons benefit most from
/// driving the outer aggregation: a hint cannot be applied inside `$unionWith`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IndexAccess {
    Automatic,
    DateRange,
    DateEndRange,
}

#[derive(Clone)]
pub(super) struct IndexPredicate {
    filter: Document,
    access: IndexAccess,
    probe_key: Option<ProbeKey>,
    ordered_page_ready: bool,
}

impl From<Document> for IndexPredicate {
    fn from(filter: Document) -> Self {
        Self {
            filter,
            access: IndexAccess::Automatic,
            probe_key: None,
            ordered_page_ready: false,
        }
    }
}

impl IndexPredicate {
    fn new(filter: Document, parameter: &SearchParameter) -> Self {
        let access = if parameter.param_type != SearchParamType::Date {
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
            filter,
            access,
            probe_key: None,
            ordered_page_ready: true,
        }
    }

    fn hint(&self) -> Option<Hint> {
        match self.access {
            IndexAccess::Automatic => None,
            IndexAccess::DateRange | IndexAccess::DateEndRange => {
                Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned()))
            }
        }
    }
}

pub(super) struct SearchPipeline {
    pub(super) collection: &'static str,
    pub(super) stages: Vec<Document>,
    pub(super) hint: Option<Hint>,
}

/// Page retrieval can stop early; an exact count must consume all matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchFilterPurpose {
    Page,
    Count,
}

/// Observations last for one request, including an inconclusive optional probe.
/// They select execution order only; matching always rereads the candidates.
#[derive(Default)]
pub(super) struct ProbeFacts {
    counts: HashMap<Vec<u8>, Option<u64>>,
    pub(super) requests: u32,
    pub(super) reuses: u32,
}

impl ProbeFacts {
    /// `None` means the probe is not reusable within this request. A probe
    /// without a `probe_timeout` runs untimed.
    fn key(
        db: &mongodb::Database,
        filter: &Document,
        hint: Option<Hint>,
        limit: u64,
        probe_timeout: Option<Duration>,
    ) -> Option<Vec<u8>> {
        let hint = match hint {
            Some(Hint::Name(name)) => Bson::String(name),
            _ => Bson::Null,
        };
        let timeout_ms = match probe_timeout {
            Some(timeout) => Bson::Int64(timeout.as_millis() as i64),
            None => Bson::Null,
        };
        mongodb::bson::to_vec(&doc! {
            "database": db.name(), "filter": filter.clone(),
            "hint": hint, "limit": limit as i64, "timeout_ms": timeout_ms,
        })
        .ok()
    }

    /// Remember an observation independently of whether it required a probe.
    fn remember(&mut self, key: Vec<u8>, count: Option<u64>) {
        self.counts.insert(key, count);
    }

    fn reuse(&mut self, key: &[u8]) -> Option<Option<u64>> {
        let count = self.peek(key);
        if count.is_some() {
            self.reuses += 1;
        }
        count
    }

    pub(super) fn peek(&self, key: &[u8]) -> Option<Option<u64>> {
        self.counts.get(key).copied()
    }

    pub(super) fn record(&self) {
        tracing::Span::current().record("probe_requests", self.requests);
        tracing::Span::current().record("probe_reuses", self.reuses);
        tracing::debug!(target: "helios_mongodb_plan", probe_requests = self.requests,
            probe_reuses = self.reuses, "Completed request planning probes");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PageTotalExecution {
    Separate,
    Accurate,
    Estimated,
    EstimateOnly,
}

#[derive(Clone, Copy)]
pub(super) struct ExecutionFacts {
    pub(super) index_estimate_eligible: bool,
    pub(super) adaptive_offset_enabled: bool,
}

impl ExecutionFacts {
    pub(super) fn count_selection(&self) -> BatchSelection {
        if self.index_estimate_eligible {
            BatchSelection::EstimatedCount
        } else {
            BatchSelection::Count
        }
    }
}

pub(super) struct ExecutionPlan {
    pub(super) page_total: PageTotalExecution,
    pub(super) filter_purpose: SearchFilterPurpose,
    pub(super) page_policy: NoTotalPagePolicy,
}

/// Central query routing. Runtime probes may change the driver or trigger the
/// existing ordered/adaptive fallbacks, but never change total semantics.
pub(super) fn plan_execution(query: &SearchQuery, facts: ExecutionFacts) -> ExecutionPlan {
    use crate::types::{SummaryMode, TotalMode};
    let offset = if query.cursor.is_none() {
        query.offset.unwrap_or(0)
    } else {
        0
    };
    let fits = combined_page_fits(
        offset,
        query.count.unwrap_or(DEFAULT_PAGE_SIZE).max(1) as usize + 1,
    ) && matches!(query.sort.as_slice(), [sort] if sort.parameter == "_id");
    let page_total = if facts.index_estimate_eligible
        && query.summary == Some(SummaryMode::Count)
        && query.includes.is_empty()
    {
        PageTotalExecution::EstimateOnly
    } else if query.total == Some(TotalMode::Accurate) && fits {
        PageTotalExecution::Accurate
    } else if facts.index_estimate_eligible && query.parameters.len() > 1 && fits {
        PageTotalExecution::Estimated
    } else {
        PageTotalExecution::Separate
    };
    let filter_purpose = match page_total {
        PageTotalExecution::Separate => SearchFilterPurpose::Page,
        _ => SearchFilterPurpose::Count,
    };
    let indexed_predicates = query
        .parameters
        .iter()
        .filter(|param| !matches!(param.name.as_str(), "_id" | "_lastUpdated"))
        .count();
    let page_policy = if facts.adaptive_offset_enabled {
        NoTotalPagePolicy::for_query(query, filter_purpose, indexed_predicates)
    } else {
        NoTotalPagePolicy::Streaming
    };
    ExecutionPlan {
        page_total,
        filter_purpose,
        page_policy,
    }
}

impl ExecutionPlan {
    pub(super) fn combines_total(&self) -> bool {
        matches!(
            self.page_total,
            PageTotalExecution::Accurate | PageTotalExecution::Estimated
        )
    }

    pub(super) fn page_selection(
        &self,
        direction: i32,
        offset: u32,
        limit: usize,
        boundary: Option<Document>,
    ) -> BatchSelection {
        match self.page_total {
            PageTotalExecution::Accurate => BatchSelection::PageAndCount {
                direction,
                offset,
                limit,
                boundary,
            },
            PageTotalExecution::Estimated => BatchSelection::EstimatedPageAndCount {
                direction,
                offset,
                limit,
                boundary,
            },
            _ => BatchSelection::Page {
                direction,
                offset,
                limit,
            },
        }
    }

    pub(super) fn record(&self) {
        let total_plan = match self.page_total {
            PageTotalExecution::Separate => "separate",
            PageTotalExecution::Accurate => "accurate",
            PageTotalExecution::Estimated => "estimated",
            PageTotalExecution::EstimateOnly => "estimate_only",
        };
        tracing::Span::current().record("total_plan", total_plan);
        tracing::debug!(target: "helios_mongodb_plan", total_plan,
            page_policy = ?self.page_policy, "Selected search execution plan");
    }
}

pub(super) fn record_total_strategy(strategy: &'static str) {
    tracing::Span::current().record("total_strategy", strategy);
    tracing::debug!(target: "helios_mongodb_plan", total_strategy = strategy, "Executed total strategy");
}

/// Reports which filter plan a page or a separate total selected.
pub(super) fn record_filter_strategy(
    span_field: &'static str,
    plan: Option<&SearchFilterPlan>,
    purpose: SearchFilterPurpose,
) {
    let strategy = match plan {
        None => "resource_filter",
        Some(SearchFilterPlan::CandidateIds(_)) => "bounded_candidates",
        Some(SearchFilterPlan::BatchedIndex(_)) => "batched_index",
        Some(SearchFilterPlan::IndexIntersection(_)) => "index_intersection",
        Some(SearchFilterPlan::ResourceLookups { .. }) => "resource_lookups",
    };
    tracing::Span::current().record(span_field, strategy);
    tracing::debug!(target: "helios_mongodb_plan", span_field, filter_strategy = strategy, ?purpose,
        "Selected filter execution plan");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoTotalPagePolicy {
    Streaming,
    SampleSelectivity,
}

#[derive(Clone, Copy)]
struct PageSample {
    candidates: usize,
    live_matches: u64,
}

impl NoTotalPagePolicy {
    fn for_query(
        query: &SearchQuery,
        purpose: SearchFilterPurpose,
        indexed_predicates: usize,
    ) -> Self {
        let offset = query.offset.unwrap_or(0);
        let Some(limit) = (query.count.unwrap_or(DEFAULT_PAGE_SIZE).max(1) as usize).checked_add(1)
        else {
            return Self::Streaming;
        };
        if matches!(purpose, SearchFilterPurpose::Page)
            && indexed_predicates > 1
            && !query.wants_total()
            && query.cursor.is_none()
            && offset > PAGE_CANDIDATE_BATCH_ROWS as u32
            && combined_page_fits(offset, limit)
            && matches!(query.sort.as_slice(), [sort] if sort.parameter == "_id")
        {
            Self::SampleSelectivity
        } else {
            Self::Streaming
        }
    }

    fn use_bounded(self, sample: Option<PageSample>) -> bool {
        let Some(sample) = sample else {
            return false;
        };
        self == Self::SampleSelectivity
            && sample.candidates == PAGE_CANDIDATE_BATCH_ROWS
            && sample.live_matches <= SPARSE_SAMPLE_MAX_LIVE
    }
}

/// A shared matching strategy for pages and exact counts.
/// Positive counts and ID pages stream bounded index batches. Timestamp pages
/// retain their database sort; existence checks preserve absent complements.
pub(super) enum SearchFilterPlan {
    /// Bounded index matches, still validated against live resources.
    CandidateIds(Vec<String>),
    /// Stream one index predicate and intersect bounded batches with the others.
    BatchedIndex(BatchedIndexPlan),
    /// Intersect positive predicates and live resource IDs before fetching a page.
    IndexIntersection(Vec<IndexPredicate>),
    /// Fetch candidates from one index, or from resources for complement filters.
    ResourceLookups {
        index_filter: Option<IndexPredicate>,
        stages: Vec<Document>,
    },
}

impl SearchFilterPlan {
    pub(super) fn pipeline(
        self,
        resource_filter: Document,
        sort: Option<Document>,
        index_cursor_filter: Option<Document>,
    ) -> StorageResult<SearchPipeline> {
        let id_ordered = sort
            .as_ref()
            .is_some_and(|sort| sort.len() == 1 && sort.contains_key("id"));
        let (collection, stages, mut hint) =
            self.pipeline_parts(resource_filter, sort, index_cursor_filter)?;
        if hint.is_none() && collection == MongoBackend::RESOURCES_COLLECTION && id_ordered {
            hint = Some(Hint::Name(
                super::super::schema::RESOURCES_IDENTITY_INDEX.to_owned(),
            ));
        }
        Ok(SearchPipeline {
            collection,
            stages,
            hint,
        })
    }

    fn pipeline_parts(
        self,
        resource_filter: Document,
        sort: Option<Document>,
        index_cursor_filter: Option<Document>,
    ) -> StorageResult<(&'static str, Vec<Document>, Option<Hint>)> {
        let (index_filter, stages) = match self {
            Self::BatchedIndex(_) => {
                return Err(internal_error(
                    "Batched search requires streaming execution".to_owned(),
                ));
            }
            Self::CandidateIds(ids) => {
                let mut pipeline = vec![doc! { "$match": { "$and": [
                    resource_filter, { "id": { "$in": ids } },
                ] } }];
                if let Some(sort) = sort {
                    pipeline.push(doc! { "$sort": sort });
                }
                return Ok((MongoBackend::RESOURCES_COLLECTION, pipeline, None));
            }
            Self::IndexIntersection(filters) => {
                return Ok(Self::intersection_pipeline(filters, resource_filter, sort));
            }
            Self::ResourceLookups {
                index_filter,
                stages,
            } => (index_filter, stages),
        };
        let hint = index_filter.as_ref().and_then(IndexPredicate::hint);
        let (collection, mut pipeline) = if let Some(predicate) = index_filter {
            let filter = predicate.filter;
            // Reject IDs outside the page before sorting and fetching resources.
            // In particular, previous-page scans must not look up every ID on
            // the far side of the cursor before reaching the requested page.
            let filter = match index_cursor_filter {
                Some(boundary) => doc! { "$and": [filter, boundary] },
                None => filter,
            };
            let mut pipeline = vec![
                doc! { "$match": filter },
                doc! { "$group": { "_id": "$resource_id" } },
            ];
            // IDs can be ordered before loading resources. Other sort fields
            // live on resources, and therefore require sorting after the join.
            let id_sort = sort
                .as_ref()
                .filter(|sort| sort.len() == 1)
                .and_then(|sort| sort.get("id"));
            if let Some(direction) = id_sort {
                // Rename the field without reinterpreting the sort value
                // produced by build_sort_document.
                pipeline.push(doc! { "$sort": { "_id": direction.clone() } });
            }
            pipeline.extend([
                doc! { "$lookup": {
                    "from": MongoBackend::RESOURCES_COLLECTION,
                    "let": { "candidate_id": "$_id" },
                    "pipeline": [
                        { "$match": { "$and": [
                            resource_filter,
                            { "$expr": { "$eq": ["$id", "$$candidate_id"] } },
                        ] } },
                        { "$limit": 1 },
                    ],
                    "as": "_hfs_resource",
                } },
                doc! { "$unwind": "$_hfs_resource" },
                doc! { "$replaceWith": "$_hfs_resource" },
            ]);
            if id_sort.is_none() {
                if let Some(sort) = sort {
                    pipeline.push(doc! { "$sort": sort });
                }
            }
            (MongoBackend::SEARCH_INDEX_COLLECTION, pipeline)
        } else {
            let mut pipeline = vec![doc! { "$match": resource_filter }];
            if let Some(sort) = sort {
                pipeline.push(doc! { "$sort": sort });
            }
            (MongoBackend::RESOURCES_COLLECTION, pipeline)
        };
        pipeline.extend(stages);
        Ok((collection, pipeline, hint))
    }

    fn intersection_pipeline(
        filters: Vec<IndexPredicate>,
        resource_filter: Document,
        sort: Option<Document>,
    ) -> (&'static str, Vec<Document>, Option<Hint>) {
        // A resource branch validates tenant/type, deletion state and cursor or
        // resource-local predicates without one cross-shard lookup per candidate.
        // Only IDs and the resource sort timestamp enter the grouping stage.
        let resource_position = filters.len() as i64;
        let driver = filters
            .iter()
            .enumerate()
            .filter(|(_, predicate)| predicate.access != IndexAccess::Automatic)
            .max_by_key(|(_, predicate)| predicate.access)
            .map(|(position, _)| position + 1)
            .unwrap_or(0);
        let mut branches = vec![(
            MongoBackend::RESOURCES_COLLECTION,
            vec![
                doc! { "$match": resource_filter },
                doc! { "$project": {
                    "_id": 0, "resource_id": "$id", "last_updated": 1,
                    "_hfs_predicate": { "$literal": resource_position },
                } },
            ],
            None,
        )];
        for (position, predicate) in filters.into_iter().enumerate() {
            let hint = predicate.hint();
            branches.push((
                MongoBackend::SEARCH_INDEX_COLLECTION,
                vec![
                    doc! { "$match": predicate.filter },
                    doc! { "$project": {
                        "_id": 0, "resource_id": 1,
                        "_hfs_predicate": { "$literal": position as i64 },
                    } },
                ],
                hint,
            ));
        }
        let (collection, mut pipeline, hint) = branches.remove(driver);
        for (collection, stages, _) in branches {
            pipeline.push(doc! { "$unionWith": {
                "coll": collection, "pipeline": stages,
            } });
        }
        // Tag by occurrence, not parameter name: ge/lt date bounds must both
        // match. Duplicate rows for a repeating element satisfy a predicate once.
        let required: Vec<Bson> = (0..=resource_position).map(Bson::Int64).collect();
        pipeline.extend([
            doc! { "$group": {
                "_id": "$resource_id",
                "_hfs_predicates": { "$addToSet": "$_hfs_predicate" },
                "last_updated": { "$max": "$last_updated" },
            } },
            doc! { "$match": { "_hfs_predicates": { "$all": required } } },
            doc! { "$project": { "_id": 0, "id": "$_id", "last_updated": 1 } },
        ]);
        if let Some(sort) = sort {
            pipeline.push(doc! { "$sort": sort });
        }
        (collection, pipeline, hint)
    }

    pub(super) fn fetch_page(tenant_id: &str, resource_type: &str) -> [Document; 3] {
        [
            doc! { "$lookup": {
                "from": MongoBackend::RESOURCES_COLLECTION,
                "localField": "id", "foreignField": "id",
                "pipeline": [{ "$match": {
                    "tenant_id": tenant_id, "resource_type": resource_type,
                    "is_deleted": false,
                } }],
                "as": "_hfs_resource",
            } },
            doc! { "$unwind": "$_hfs_resource" },
            doc! { "$replaceWith": "$_hfs_resource" },
        ]
    }
}

impl MongoBackend {
    pub(super) fn execution_facts(&self, query: &SearchQuery) -> ExecutionFacts {
        ExecutionFacts {
            index_estimate_eligible: self.can_estimate_index_total(query),
            adaptive_offset_enabled: self.config().adaptive_offset_paging,
        }
    }

    /// Only explicit estimates with entirely index-backed positive conditions
    /// may omit the final live-resource check. Every other query stays exact.
    pub(super) fn can_estimate_index_total(&self, query: &SearchQuery) -> bool {
        query.total == Some(crate::types::TotalMode::Estimate)
            && self.search_indexes_ready()
            && query.contained == crate::types::ContainedMode::Off
            && query.compartment.is_none()
            && query.list.is_empty()
            && query.reverse_chains.is_empty()
            && !query.parameters.is_empty()
            && !query.parameters.iter().any(crate::search::has_empty_value)
            && query
                .sort
                .iter()
                .all(|sort| matches!(sort.parameter.as_str(), "_id" | "_lastUpdated"))
            && query.parameters.iter().all(|parameter| {
                !matches!(parameter.name.as_str(), "_id" | "_lastUpdated" | "_filter")
                    && parameter.chain.is_empty()
                    && !parameter.values.is_empty()
                    && parameter
                        .values
                        .iter()
                        .all(|value| value.prefix != SearchPrefix::Ne)
                    && !matches!(
                        parameter.param_type,
                        SearchParamType::Composite | SearchParamType::Special
                    )
                    && !matches!(
                        parameter.modifier,
                        Some(SearchModifier::Missing | SearchModifier::Not | SearchModifier::NotIn)
                    )
                    && !(parameter.param_type == SearchParamType::Reference
                        && matches!(parameter.modifier, Some(SearchModifier::Identifier)))
            })
    }

    /// Capture the exact definition and index access used by this predicate.
    /// Serialization failure or oversized scope simply bypasses this cache.
    fn planning_predicate(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        parameter: &SearchParameter,
        filter: Document,
        indexes_ready: bool,
    ) -> IndexPredicate {
        let mut predicate = if indexes_ready {
            IndexPredicate::new(filter, parameter)
        } else {
            filter.into()
        };
        let registry = self.tenant_registry(tenant_id);
        let definition = registry.read().get_param(resource_type, &parameter.name);
        let definition = definition
            .as_ref()
            .map(|definition| mongodb::bson::to_bson(definition.as_ref()))
            .transpose();
        if let Ok(definition) = definition {
            let definition_scope = Bson::Document(doc! {
                "fhir_version": self.config().fhir_version.as_str(),
                "parameter": definition.unwrap_or(Bson::Null),
            });
            predicate.probe_key = ProbeKey::new(
                db.name(),
                MongoBackend::SEARCH_INDEX_COLLECTION,
                tenant_id,
                resource_type,
                &predicate.filter,
                predicate.hint().as_ref(),
                indexes_ready,
                MAX_CANDIDATE_ROWS + 1,
                &definition_scope,
            );
        }
        predicate
    }

    /// Select an index-driven plan for positive counts and ID pages. Timestamp
    /// pages retain bounded candidates or database intersections for their sort.
    /// Complements use per-resource existence checks.
    /// Composite pair matching and reference-identifier resolution still use the
    /// candidate-selection path, as do compartments and search-parameter sorts.
    pub(super) async fn resource_filter_plan(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        query: &SearchQuery,
        purpose: SearchFilterPurpose,
        page_policy: NoTotalPagePolicy,
        facts: &mut ProbeFacts,
    ) -> StorageResult<Option<SearchFilterPlan>> {
        // Upstream's candidate executor bounds all sibling predicates to a
        // positive ID seed, including IDs resolved by chains and lists.
        if super::positive_id_seed(query).is_some() {
            return Ok(None);
        }
        if query.parameters.iter().any(crate::search::has_empty_value)
            || query.compartment.as_ref().is_some_and(|compartment| {
                !compartment.params.is_empty() && !compartment.reference.is_empty()
            })
            || query
                .sort
                .iter()
                .any(|sort| !matches!(sort.parameter.as_str(), "_id" | "_lastUpdated"))
        {
            return Ok(None);
        }
        let indexed: Vec<_> = query
            .parameters
            .iter()
            .filter(|param| !matches!(param.name.as_str(), "_id" | "_lastUpdated"))
            .collect();
        if indexed.is_empty()
            || indexed.iter().any(|param| {
                (param.param_type == SearchParamType::Composite
                    && !matches!(param.modifier, Some(SearchModifier::Missing)))
                    || (param.param_type == SearchParamType::Reference
                        && matches!(param.modifier, Some(SearchModifier::Identifier)))
            })
        {
            return Ok(None);
        }

        // Positive searches share candidate selection for pages and totals.
        // Complements retain existence checks so absent values remain eligible.
        let has_complement = indexed.iter().any(|param| {
            matches!(
                param.modifier,
                Some(SearchModifier::Missing | SearchModifier::Not)
            )
        });
        if !has_complement {
            let probe_timeout = self.probe_timeout();
            let indexes_ready = self.search_indexes_ready();
            let filters = indexed
                .iter()
                .map(|param| {
                    self.build_search_index_filter(tenant_id, &query.resource_type, param)
                        .map(|filter| {
                            self.planning_predicate(
                                db,
                                tenant_id,
                                &query.resource_type,
                                param,
                                filter,
                                indexes_ready,
                            )
                        })
                })
                .collect::<StorageResult<Vec<_>>>()?;
            // ID pages validate a bounded batch with one query instead of a
            // cross-shard resource lookup for every candidate. The seed still
            // applies cursor bounds before sorting, and pages stop early.
            if matches!(purpose, SearchFilterPurpose::Count)
                || matches!(query.sort.as_slice(), [directive] if directive.parameter == "_id")
            {
                return Ok(Some(SearchFilterPlan::BatchedIndex(
                    BatchedIndexPlan::new(
                        db,
                        filters,
                        &self.probe_cache,
                        page_policy,
                        facts,
                        probe_timeout,
                    )
                    .await?,
                )));
            }
            if let Some(ids) =
                bounded_candidates(db, &filters, &self.probe_cache, facts, probe_timeout).await?
            {
                return Ok(Some(SearchFilterPlan::CandidateIds(ids)));
            }
            return Ok(Some(SearchFilterPlan::IndexIntersection(filters)));
        }

        // Keep selective positive predicates on the index-first path. A
        // bounded count transfers only a scalar; broad searches never collect
        // their full candidate set just to choose a plan.
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut driver = None;
        for (position, param) in indexed.iter().enumerate() {
            if matches!(param.modifier, Some(SearchModifier::Missing)) {
                // Presence is a positive predicate: drive from its typed partial
                // index, rather than enumerating resources with no value.
                if driver.is_none()
                    && param
                        .values
                        .first()
                        .is_some_and(|value| value.value == "false")
                {
                    driver = Some((
                        position,
                        missing_presence_filter(tenant_id, &query.resource_type, param),
                    ));
                }
                continue;
            }
            if matches!(param.modifier, Some(SearchModifier::Not)) {
                continue;
            }
            let filter = self.build_search_index_filter(tenant_id, &query.resource_type, param)?;
            let key = ProbeFacts::key(db, &filter, None, SELECTIVE_SEARCH_ROW_LIMIT + 1, None);
            let reused = key.as_deref().and_then(|key| facts.reuse(key));
            let count = if let Some(Some(count)) = reused {
                count
            } else {
                facts.requests += 1;
                let count = search_index
                    .count_documents(filter.clone())
                    .limit(SELECTIVE_SEARCH_ROW_LIMIT + 1)
                    .await
                    .or_query_error("Failed to probe resource-filtered search")?;
                if let Some(key) = key {
                    facts.remember(key, Some(count));
                }
                count
            };
            if count <= SELECTIVE_SEARCH_ROW_LIMIT {
                return Ok(None);
            }
            if driver.is_none() {
                driver = Some((position, filter));
            }
        }
        let mut stages = Vec::new();
        for (position, param) in indexed.into_iter().enumerate() {
            if driver
                .as_ref()
                .is_some_and(|(driver_position, _)| *driver_position == position)
            {
                continue;
            }
            let (filter, exclude) = if matches!(param.modifier, Some(SearchModifier::Missing)) {
                (
                    missing_presence_filter(tenant_id, &query.resource_type, param),
                    param
                        .values
                        .first()
                        .is_some_and(|value| value.value == "true"),
                )
            } else if matches!(param.modifier, Some(SearchModifier::Not)) {
                let mut positive = param.clone();
                positive.modifier = None;
                (
                    self.build_search_index_filter(tenant_id, &query.resource_type, &positive)?,
                    true,
                )
            } else {
                (
                    self.build_search_index_filter(tenant_id, &query.resource_type, param)?,
                    false,
                )
            };
            stages.push(doc! { "$lookup": {
                "from": MongoBackend::SEARCH_INDEX_COLLECTION,
                "let": { "candidate_id": "$id" },
                "pipeline": [
                    { "$match": { "$and": [
                        filter,
                        { "$expr": { "$eq": ["$resource_id", "$$candidate_id"] } },
                    ] } },
                    { "$limit": 1 },
                    { "$project": { "_id": 1 } },
                ],
                "as": "_hfs_search_match",
            } });
            stages.push(doc! { "$match": {
                "_hfs_search_match.0": { "$exists": !exclude },
            } });
            stages.push(doc! { "$unset": "_hfs_search_match" });
        }
        Ok(Some(SearchFilterPlan::ResourceLookups {
            index_filter: driver.map(|(_, filter)| filter.into()),
            stages,
        }))
    }

    /// Server execution budget for each optional planning probe.
    pub(super) fn probe_timeout(&self) -> Duration {
        Duration::from_millis(self.config().probe_timeout_ms)
    }
}

/// Choose by observed index rows, never by parameter name. Counts at the probe
/// cap are lower bounds; they are not full cardinalities or resource totals.
fn candidate_driver(counts: &[Option<u64>]) -> Option<usize> {
    counts
        .iter()
        .enumerate()
        .filter_map(|(position, count)| count.map(|count| (position, count)))
        .filter(|(_, count)| *count <= MAX_CANDIDATE_ROWS)
        .min_by_key(|(_, count)| *count)
        .map(|(position, _)| position)
}

fn remaining_filter_order(counts: &[Option<u64>], driver: usize, informative: bool) -> Vec<usize> {
    let mut remaining: Vec<_> = (0..counts.len())
        .filter(|position| *position != driver)
        .collect();
    if informative {
        // Stable sorting preserves each occurrence and tie order.
        remaining.sort_by_key(|position| counts[*position].unwrap_or(u64::MAX));
    }
    remaining
}

/// Only optional planning work may recover from an execution or network timeout.
/// The logged error tells a server budget expiry from a network timeout.
fn optional_probe_timed_out(error: &mongodb::error::Error) -> bool {
    let timed_out = match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => matches!(command.code, 50 | 89 | 262),
        mongodb::error::ErrorKind::Io(error) => error.kind() == std::io::ErrorKind::TimedOut,
        _ => false,
    };
    if timed_out {
        tracing::debug!(error = %error, "Optional probe timed out");
    }
    timed_out
}

fn probe_count(result: mongodb::error::Result<u64>) -> StorageResult<Option<u64>> {
    match result {
        Err(error) if optional_probe_timed_out(&error) => Ok(None),
        result => result
            .map(Some)
            .or_query_error("Failed to probe search candidates"),
    }
}

async fn probe_candidates(
    db: &mongodb::Database,
    filters: &[IndexPredicate],
    cache: &ProbeCache,
    facts: &mut ProbeFacts,
    probe_timeout: Duration,
) -> StorageResult<Vec<Option<u64>>> {
    let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
    let mut counts = Vec::with_capacity(filters.len());
    for filter in filters {
        let key = ProbeFacts::key(
            db,
            &filter.filter,
            filter.hint(),
            MAX_CANDIDATE_ROWS + 1,
            Some(probe_timeout),
        );
        if let Some(count) = key.as_deref().and_then(|key| facts.reuse(key)) {
            counts.push(count);
            continue;
        }
        if let Some(estimate) = filter.probe_key.as_ref().and_then(|key| cache.get(key)) {
            // AtLeast stays at the probe cap and cannot qualify as selective.
            let count = estimate.observed_rows();
            if let Some(key) = key {
                facts.remember(key, count);
            }
            counts.push(count);
            continue;
        }
        let mut probe = index
            .count_documents(filter.filter.clone())
            .limit(MAX_CANDIDATE_ROWS + 1)
            .max_time(probe_timeout);
        if let Some(hint) = filter.hint() {
            probe = probe.hint(hint);
        }
        facts.requests += 1;
        let count = probe_count(probe.await)?;
        if let Some(key) = key {
            facts.remember(key, count);
        }
        if let (Some(key), Some(estimate)) = (
            filter.probe_key.as_ref(),
            match count {
                Some(count) => ProbeEstimate::from_bounded(count, MAX_CANDIDATE_ROWS + 1),
                None => Some(ProbeEstimate::Unknown),
            },
        ) {
            cache.insert(key.clone(), estimate);
        }
        counts.push(count);
    }
    Ok(counts)
}

#[derive(Default)]
struct CandidateIds {
    ids: HashSet<String>,
    bson_bytes: usize,
}

impl CandidateIds {
    fn insert(&mut self, id: &str) -> bool {
        if self.ids.contains(id) {
            return true;
        }
        // Include the BSON string length, terminator and array element key.
        let bytes = self.bson_bytes.saturating_add(id.len().saturating_add(16));
        if self.ids.len() >= MAX_CANDIDATE_ROWS as usize || bytes > MAX_CANDIDATE_BYTES {
            return false;
        }
        self.bson_bytes = bytes;
        self.ids.insert(id.to_owned());
        true
    }
}

/// `None` means use the broad plan; an empty set means no index matches.
/// Probes and the subsequent read are not a snapshot. Recheck both limits
/// during the read so concurrent inserts cannot silently truncate candidates.
async fn bounded_candidates(
    db: &mongodb::Database,
    filters: &[IndexPredicate],
    cache: &ProbeCache,
    facts: &mut ProbeFacts,
    probe_timeout: Duration,
) -> StorageResult<Option<Vec<String>>> {
    let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
    let counts = probe_candidates(db, filters, cache, facts, probe_timeout).await?;
    let Some(driver) = candidate_driver(&counts) else {
        return Ok(None);
    };
    let mut seed = index
        .find(filters[driver].filter.clone())
        .projection(doc! { "_id": 0, "resource_id": 1 })
        .limit((MAX_CANDIDATE_ROWS + 1) as i64);
    if let Some(hint) = filters[driver].hint() {
        seed = seed.hint(hint);
    }
    let mut cursor = seed
        .await
        .or_query_error("Failed to read search candidates")?;
    let mut candidates = CandidateIds::default();
    let mut rows = 0;
    while cursor
        .advance()
        .await
        .or_query_error("Failed to read search candidate row")?
    {
        rows += 1;
        let row = cursor
            .deserialize_current()
            .or_query_error("Invalid search candidate row")?;
        let id = row
            .get_str("resource_id")
            .map_err(|_| internal_error("Search candidate is missing resource_id".to_owned()))?;
        if rows > MAX_CANDIDATE_ROWS || !candidates.insert(id) {
            return Ok(None);
        }
    }

    // Keep each occurrence separate: repeated date bounds can match different
    // indexed values of the same resource, as required by FHIR search semantics.
    let mut remaining: Vec<_> = (0..filters.len())
        .filter(|position| *position != driver)
        .collect();
    remaining.sort_by_key(|position| counts[*position].unwrap_or(u64::MAX));
    let remaining: Vec<_> = remaining
        .into_iter()
        .map(|position| &filters[position])
        .collect();
    filter_candidates(db, &mut candidates.ids, &remaining).await?;
    let mut ids: Vec<_> = candidates.ids.into_iter().collect();
    ids.sort_unstable();
    Ok(Some(ids))
}

/// Membership checks retain each parameter occurrence, including repeated date
/// bounds that may match different values on the same resource.
async fn filter_candidates(
    db: &mongodb::Database,
    candidates: &mut HashSet<String>,
    filters: &[&IndexPredicate],
) -> StorageResult<()> {
    let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
    for predicate in filters {
        if candidates.is_empty() {
            break;
        }
        let ids: Vec<_> = candidates.iter().cloned().collect();
        let filter = doc! { "$and": [
            predicate.filter.clone(), { "resource_id": { "$in": ids } },
        ] };
        // This index has tenant/type/resource_id as its leading fields. Use the
        // bounded IDs as seeks rather than scan a broad date range then filter.
        // Repeated indexed values contribute membership once. The `$in`
        // bounds group cardinality as well as the returned ID list.
        let mut cursor = index
            .aggregate(vec![
                doc! { "$match": filter },
                doc! { "$group": { "_id": "$resource_id" } },
            ])
            .hint(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()))
            .await
            .or_query_error("Failed to filter search candidates")?;
        let mut matched = HashSet::new();
        while cursor
            .advance()
            .await
            .or_query_error("Failed to read filtered candidate")?
        {
            let row = cursor
                .deserialize_current()
                .or_query_error("Invalid filtered candidate")?;
            let id = row.get_str("_id").map_err(|_| {
                internal_error("Search candidate is missing resource_id".to_owned())
            })?;
            if candidates.contains(id) {
                matched.insert(id.to_owned());
            }
        }
        *candidates = matched;
    }
    Ok(())
}

/// Probe estimates choose a driver; they never change matching or its memory
/// bounds. If every probe is inconclusive, retain the preferred date access.
pub(super) struct BatchedIndexPlan {
    driver: IndexPredicate,
    remaining: Vec<IndexPredicate>,
    page_policy: NoTotalPagePolicy,
    page_cache: Option<(ProbeCache, ProbeKey)>,
    probe_timeout: Duration,
}

#[derive(Clone)]
pub(super) enum BatchSelection {
    Count,
    EstimatedCount,
    BoundedPage {
        direction: i32,
        offset: u32,
        limit: usize,
        boundary: Option<Document>,
    },
    EstimatedPageAndCount {
        direction: i32,
        offset: u32,
        limit: usize,
        boundary: Option<Document>,
    },
    PageAndCount {
        direction: i32,
        offset: u32,
        limit: usize,
        // Applied only to page selection; the total always covers all matches.
        boundary: Option<Document>,
    },
    Page {
        direction: i32,
        offset: u32,
        limit: usize,
    },
}

pub(super) struct BatchMatches {
    pub(super) count: u64,
    pub(super) ids: Vec<String>,
}

struct OrderedPageWindow {
    ids: Vec<String>,
    raw_rows: usize,
    may_continue: bool,
}

// A bounded top-k selection over unordered batches. Counts do not need a
// global sort; only the requested page IDs are retained across batches.
struct PageIds {
    ids: BTreeSet<String>,
    direction: i32,
    limit: usize,
}

impl PageIds {
    // Once a full live top-k is known, worse IDs cannot enter the page.
    // This bound may lag concurrent batches; an older bound only does more work.
    fn constrain(&self, selection: &BatchSelection) -> StorageResult<BatchSelection> {
        let mut selection = selection.clone();
        if self.ids.len() < self.limit {
            return Ok(selection);
        }
        if let BatchSelection::PageAndCount { boundary, .. }
        | BatchSelection::BoundedPage { boundary, .. }
        | BatchSelection::EstimatedPageAndCount { boundary, .. } = &mut selection
        {
            let worst = if self.direction == 1 {
                self.ids.last()
            } else {
                self.ids.first()
            }
            .ok_or_else(|| internal_error("Missing full page boundary".to_owned()))?;
            let operator = if self.direction == 1 { "$lt" } else { "$gt" };
            let mut comparisons = match boundary.take() {
                Some(boundary) => boundary
                    .get_document("id")
                    .map_err(|_| internal_error("Invalid combined ID boundary".to_owned()))?
                    .clone(),
                None => Document::new(),
            };
            // Retained IDs already satisfy the cursor, so this always tightens
            // the page bound while leaving the opposing cursor bound intact.
            comparisons.insert(operator, worst.clone());
            *boundary = Some(doc! { "id": comparisons });
        }
        Ok(selection)
    }

    fn extend(&mut self, ids: Vec<String>) {
        for id in ids {
            self.ids.insert(id);
            if self.ids.len() > self.limit {
                if self.direction == 1 {
                    self.ids.pop_last();
                } else {
                    self.ids.pop_first();
                }
            }
        }
    }

    fn into_ids(self) -> Vec<String> {
        if self.direction == 1 {
            self.ids.into_iter().collect()
        } else {
            self.ids.into_iter().rev().collect()
        }
    }
}

impl BatchedIndexPlan {
    async fn new(
        db: &mongodb::Database,
        mut filters: Vec<IndexPredicate>,
        cache: &ProbeCache,
        page_policy: NoTotalPagePolicy,
        facts: &mut ProbeFacts,
        probe_timeout: Duration,
    ) -> StorageResult<Self> {
        if filters.is_empty() {
            return Err(internal_error(
                "Batched search requires an index predicate".to_owned(),
            ));
        }
        // A single predicate has no driver choice, so probing only adds work.
        let counts = if filters.len() == 1 {
            vec![None]
        } else {
            probe_candidates(db, &filters, cache, facts, probe_timeout).await?
        };
        let informative_driver = candidate_driver(&counts);
        let position = informative_driver.unwrap_or_else(|| {
            filters
                .iter()
                .enumerate()
                .max_by_key(|(_, filter)| filter.access)
                .map(|(position, _)| position)
                .unwrap_or(0)
        });
        let remaining: Vec<_> =
            remaining_filter_order(&counts, position, informative_driver.is_some())
                .into_iter()
                .map(|position| filters[position].clone())
                .collect();
        let driver = filters.remove(position);
        let page_cache = if page_policy == NoTotalPagePolicy::SampleSelectivity {
            driver.probe_key.as_ref().and_then(|driver_key| {
                let remaining_keys = remaining
                    .iter()
                    .map(|predicate| predicate.probe_key.clone())
                    .collect::<Option<Vec<_>>>()?;
                ProbeKey::offset_page(driver_key, &remaining_keys).map(|key| (cache.clone(), key))
            })
        } else {
            None
        };
        Ok(Self {
            driver,
            remaining,
            page_policy,
            page_cache,
            probe_timeout,
        })
    }

    /// Count an already-filtered single-index driver without transferring or
    /// rereading its IDs. Preserve the shared stream's index-ID shape check.
    async fn count_estimated_seed(&self, db: &mongodb::Database) -> StorageResult<u64> {
        let mut stages = self.seed_pipeline(None, None);
        stages.push(doc! { "$group": {
            "_id": Bson::Null,
            "total": { "$sum": 1 },
            "invalid_id": { "$max": { "$ne": [{ "$type": "$_id" }, "string"] } },
        } });
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut aggregate = index.aggregate(stages).allow_disk_use(true);
        if let Some(hint) = self.driver.hint() {
            aggregate = aggregate.hint(hint);
        }
        let mut cursor = aggregate
            .await
            .or_query_error("Failed to count estimated index driver")?;
        let Some(document) = cursor
            .try_next()
            .await
            .or_query_error("Failed to read estimated driver count")?
        else {
            return Ok(0);
        };
        if !matches!(document.get_bool("invalid_id"), Ok(false)) {
            return Err(internal_error(
                "Search candidate is missing resource_id".to_owned(),
            ));
        }
        parse_total(&document, "total")
    }

    pub(super) async fn execute(
        &self,
        db: &mongodb::Database,
        resource_filter: Document,
        index_cursor_filter: Option<Document>,
        selection: BatchSelection,
    ) -> StorageResult<BatchMatches> {
        if matches!(selection, BatchSelection::EstimatedCount) && self.remaining.is_empty() {
            return Ok(BatchMatches {
                count: self.count_estimated_seed(db).await?,
                ids: Vec::new(),
            });
        }
        if let BatchSelection::Page {
            direction,
            offset,
            limit,
        } = selection
        {
            return self
                .execute_page(
                    db,
                    resource_filter,
                    index_cursor_filter,
                    BatchSelection::Page {
                        direction,
                        offset,
                        limit,
                    },
                )
                .await;
        }
        self.execute_batches(db, resource_filter, index_cursor_filter, selection)
            .await
    }

    async fn execute_batches(
        &self,
        db: &mongodb::Database,
        resource_filter: Document,
        index_cursor_filter: Option<Document>,
        selection: BatchSelection,
    ) -> StorageResult<BatchMatches> {
        if matches!(selection, BatchSelection::Page { .. }) {
            return Err(internal_error(
                "Streaming pages require execute_page".to_owned(),
            ));
        }
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut aggregate = index
            .aggregate(self.seed_pipeline(index_cursor_filter, None))
            .allow_disk_use(true)
            .batch_size(COUNT_CANDIDATE_BATCH_ROWS as u32);
        if let Some(hint) = self.driver.hint() {
            aggregate = aggregate.hint(hint);
        }
        let cursor = aggregate
            .await
            .or_query_error("Failed to stream search candidates")?;
        self.consume_batches(db, &resource_filter, cursor, selection, Vec::new())
            .await
    }

    /// Continue the same bounded stream, including an adaptive sample already
    /// validated by the caller. Only the page buffer is seeded; totals start at zero.
    async fn consume_batches(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        mut cursor: mongodb::Cursor<Document>,
        selection: BatchSelection,
        validated_ids: Vec<String>,
    ) -> StorageResult<BatchMatches> {
        let batch_rows = COUNT_CANDIDATE_BATCH_ROWS;
        let mut result = BatchMatches {
            count: 0,
            ids: Vec::new(),
        };
        let mut page_ids = match &selection {
            BatchSelection::PageAndCount {
                direction,
                offset,
                limit,
                ..
            }
            | BatchSelection::BoundedPage {
                direction,
                offset,
                limit,
                ..
            }
            | BatchSelection::EstimatedPageAndCount {
                direction,
                offset,
                limit,
                ..
            } => Some(PageIds {
                ids: BTreeSet::new(),
                direction: *direction,
                limit: retained_page_size(*offset, *limit)?,
            }),
            _ => None,
        };
        if let Some(page) = &mut page_ids {
            page.extend(validated_ids);
        }
        let mut pending_counts = FuturesUnordered::new();
        let mut batch = Vec::with_capacity(batch_rows);
        let mut bytes = 0usize;
        while cursor
            .advance()
            .await
            .or_query_error("Failed to read candidate batch")?
        {
            let document = cursor
                .deserialize_current()
                .or_query_error("Invalid candidate row")?;
            let id = document.get_str("_id").map_err(|_| {
                internal_error("Search candidate is missing resource_id".to_owned())
            })?;
            let size = id.len().saturating_add(16);
            if size > MAX_CANDIDATE_BYTES {
                return Err(internal_error(
                    "Search candidate exceeds the BSON batch budget".to_owned(),
                ));
            }
            if !batch.is_empty()
                && (batch.len() == batch_rows || bytes.saturating_add(size) > MAX_CANDIDATE_BYTES)
            {
                pending_counts.push(self.count_batch(
                    db,
                    resource_filter,
                    match &page_ids {
                        Some(page) => page.constrain(&selection)?,
                        None => selection.clone(),
                    },
                    std::mem::replace(&mut batch, Vec::with_capacity(batch_rows)),
                ));
                if pending_counts.len() == COUNT_BATCH_CONCURRENCY
                    && let Some(matches) = pending_counts.try_next().await?
                {
                    result.count += matches.count;
                    if let Some(page) = &mut page_ids {
                        page.extend(matches.ids);
                    }
                }
                bytes = 0;
            }
            batch.push(id.to_owned());
            bytes += size;
        }
        if !batch.is_empty() {
            pending_counts.push(self.count_batch(
                db,
                resource_filter,
                match &page_ids {
                    Some(page) => page.constrain(&selection)?,
                    None => selection.clone(),
                },
                batch,
            ));
        }
        while let Some(matches) = pending_counts.try_next().await? {
            result.count += matches.count;
            if let Some(page) = &mut page_ids {
                page.extend(matches.ids);
            }
        }
        if let Some(page) = page_ids {
            let offset = match selection {
                BatchSelection::PageAndCount { offset, .. }
                | BatchSelection::BoundedPage { offset, .. }
                | BatchSelection::EstimatedPageAndCount { offset, .. } => offset as usize,
                _ => 0,
            };
            result.ids = page.into_ids().into_iter().skip(offset).collect();
        }
        Ok(result)
    }

    fn record_page_decision(
        &self,
        decision: OffsetPageDecision,
        sample: Option<PageSample>,
        cached: bool,
    ) {
        let page_strategy = match decision {
            OffsetPageDecision::Bounded => "bounded",
            OffsetPageDecision::Streaming => "streaming",
        };
        tracing::Span::current().record("page_strategy", page_strategy);
        tracing::debug!(target: "helios_mongodb_page", page_strategy,
            cached, sample_complete = sample.is_some(),
            candidates = sample.map_or(0, |sample| sample.candidates),
            live_matches = sample.map_or(0, |sample| sample.live_matches),
            "Selected no-total offset execution");
    }

    async fn execute_adaptive_page(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        direction: i32,
        offset: u32,
        limit: usize,
    ) -> StorageResult<Option<BatchMatches>> {
        if !combined_page_fits(offset, limit) {
            return Err(internal_error(
                "Adaptive page exceeds memory bounds".to_owned(),
            ));
        }
        let selection = BatchSelection::BoundedPage {
            direction,
            offset,
            limit,
            boundary: None,
        };
        if let Some(decision) = self
            .page_cache
            .as_ref()
            .and_then(|(cache, key)| cache.get_page_decision(key))
        {
            self.record_page_decision(decision, None, true);
            return match decision {
                OffsetPageDecision::Streaming => Ok(None),
                OffsetPageDecision::Bounded => self
                    .execute_batches(db, resource_filter.clone(), None, selection)
                    .await
                    .map(Some),
            };
        }
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut aggregate = index
            .aggregate(self.seed_pipeline(None, None))
            .allow_disk_use(true)
            .batch_size(COUNT_CANDIDATE_BATCH_ROWS as u32);
        if let Some(hint) = self.driver.hint() {
            aggregate = aggregate.hint(hint);
        }
        let mut cursor = aggregate
            .await
            .or_query_error("Failed to open adaptive page candidates")?;
        let mut batch = Vec::with_capacity(PAGE_CANDIDATE_BATCH_ROWS);
        let mut bytes = 0usize;
        while batch.len() < PAGE_CANDIDATE_BATCH_ROWS {
            let Some(row) = cursor
                .try_next()
                .await
                .or_query_error("Failed to read adaptive page sample")?
            else {
                break;
            };
            let id = row
                .get_str("_id")
                .map_err(|_| internal_error("Page sample is missing resource_id".to_owned()))?;
            let size = id.len().saturating_add(16);
            if size > MAX_CANDIDATE_BYTES {
                return Err(internal_error(
                    "Search candidate exceeds the BSON batch budget".to_owned(),
                ));
            }
            bytes = bytes.saturating_add(size);
            if bytes > MAX_CANDIDATE_BYTES {
                self.record_page_decision(OffsetPageDecision::Streaming, None, false);
                return Ok(None);
            }
            batch.push(id.to_owned());
        }
        let mut validated = BatchMatches {
            count: 0,
            ids: Vec::new(),
        };
        self.match_batch(
            db,
            resource_filter,
            &BatchSelection::Page {
                direction,
                offset: 0,
                limit: PAGE_CANDIDATE_BATCH_ROWS + 1,
            },
            &batch,
            &mut validated,
        )
        .await?;
        let sample = PageSample {
            candidates: batch.len(),
            live_matches: validated.count,
        };
        // EOF makes this the complete result, not an estimate of later matches.
        if batch.len() < PAGE_CANDIDATE_BATCH_ROWS {
            self.record_page_decision(OffsetPageDecision::Bounded, Some(sample), false);
            let mut page = PageIds {
                ids: BTreeSet::new(),
                direction,
                limit: retained_page_size(offset, limit)?,
            };
            page.extend(validated.ids);
            return Ok(Some(BatchMatches {
                count: 0,
                ids: page.into_ids().into_iter().skip(offset as usize).collect(),
            }));
        }
        let decision = if self.page_policy.use_bounded(Some(sample)) {
            OffsetPageDecision::Bounded
        } else {
            OffsetPageDecision::Streaming
        };
        if let Some((cache, key)) = &self.page_cache {
            cache.remember_page_decision(key.clone(), decision);
        }
        self.record_page_decision(decision, Some(sample), false);
        if decision == OffsetPageDecision::Streaming {
            // Unordered sample IDs cannot establish an offset or cursor boundary.
            return Ok(None);
        }
        self.consume_batches(db, resource_filter, cursor, selection, validated.ids)
            .await
            .map(Some)
    }

    #[tracing::instrument(skip_all, fields(page_strategy = tracing::field::Empty))]
    async fn execute_page(
        &self,
        db: &mongodb::Database,
        resource_filter: Document,
        mut boundary: Option<Document>,
        selection: BatchSelection,
    ) -> StorageResult<BatchMatches> {
        let BatchSelection::Page {
            direction,
            offset,
            limit,
        } = selection
        else {
            return Err(internal_error(
                "Expected streaming page selection".to_owned(),
            ));
        };
        tracing::Span::current().record("page_strategy", "streaming");
        // A dense single predicate can stream in ID order without grouping
        // every match. An optional bounded read chooses the plan; sparse or
        // timed-out reads retain the original indexed page execution.
        if self.driver.ordered_page_ready
            && self.remaining.is_empty()
            && let Some(result) = self
                .try_ordered_page(db, &resource_filter, boundary.as_ref(), &selection)
                .await?
        {
            tracing::Span::current().record("page_strategy", "ordered_index");
            return Ok(result);
        }
        if self.page_policy == NoTotalPagePolicy::SampleSelectivity
            && let Some(result) = self
                .execute_adaptive_page(db, &resource_filter, direction, offset, limit)
                .await?
        {
            return Ok(result);
        }
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut result = BatchMatches {
            count: 0,
            ids: Vec::new(),
        };
        // A one-batch page benefits from a small top-k sort. Larger offsets or
        // pages use one sorted stream immediately: limiting to their raw ID
        // count can force a second seed scan when even a few IDs fail validation.
        let mut bounded =
            u64::from(offset).saturating_add(limit as u64) <= PAGE_CANDIDATE_BATCH_ROWS as u64;
        loop {
            // Offset applies only after liveness and every remaining predicate
            // have been checked. The small first window has a full validation
            // batch of headroom; sparse results continue in one remaining pass.
            let window = PAGE_CANDIDATE_BATCH_ROWS as u64;
            let mut pipeline = self.seed_pipeline(boundary, Some(direction));
            if bounded {
                pipeline.push(doc! { "$limit": window as i64 });
            }
            let mut aggregate = index
                .aggregate(pipeline)
                .allow_disk_use(true)
                .batch_size(PAGE_CANDIDATE_BATCH_ROWS as u32);
            if let Some(hint) = self.driver.hint() {
                aggregate = aggregate.hint(hint);
            }
            let mut cursor = aggregate
                .await
                .or_query_error("Failed to open candidate page window")?;
            let mut batch = Vec::with_capacity(PAGE_CANDIDATE_BATCH_ROWS);
            let mut bytes = 0usize;
            let mut scanned = 0u64;
            let mut last_id = None;
            while cursor
                .advance()
                .await
                .or_query_error("Failed to read candidate page window")?
            {
                let document = cursor
                    .deserialize_current()
                    .or_query_error("Invalid page candidate")?;
                let id = document.get_str("_id").map_err(|_| {
                    internal_error("Search candidate is missing resource_id".to_owned())
                })?;
                let size = id.len().saturating_add(16);
                if size > MAX_CANDIDATE_BYTES {
                    return Err(internal_error(
                        "Search candidate exceeds the BSON batch budget".to_owned(),
                    ));
                }
                if !batch.is_empty()
                    && (batch.len() == PAGE_CANDIDATE_BATCH_ROWS
                        || bytes.saturating_add(size) > MAX_CANDIDATE_BYTES)
                {
                    if self
                        .match_batch(db, &resource_filter, &selection, &batch, &mut result)
                        .await?
                    {
                        return Ok(result);
                    }
                    batch.clear();
                    bytes = 0;
                }
                batch.push(id.to_owned());
                last_id = Some(id.to_owned());
                bytes += size;
                scanned += 1;
            }
            if self
                .match_batch(db, &resource_filter, &selection, &batch, &mut result)
                .await?
                || !bounded
                || scanned < window
            {
                return Ok(result);
            }
            // All scanned IDs have now been checked. Continue strictly past
            // the last seed ID, including when an entire window was stale or
            // failed another predicate. Grouping deduplicates within a window;
            // strict progression prevents duplicates between windows.
            let last_id =
                last_id.ok_or_else(|| internal_error("Missing page window boundary".to_owned()))?;
            let operator = if direction == 1 { "$gt" } else { "$lt" };
            boundary = Some(doc! { "resource_id": { operator: last_id } });
            // A sparse or stale index can exhaust the first window. Stream the
            // remaining range in one pass instead of repeatedly regrouping and
            // sorting that range for every small window.
            bounded = false;
        }
    }

    async fn read_ordered_page_window(
        &self,
        db: &mongodb::Database,
        boundary: Option<&Document>,
        direction: i32,
    ) -> StorageResult<Option<OrderedPageWindow>> {
        let filter = match boundary {
            Some(boundary) => doc! { "$and": [self.driver.filter.clone(), boundary.clone()] },
            None => self.driver.filter.clone(),
        };
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let opened = index
            .find(filter)
            .projection(doc! { "_id": 0, "resource_id": 1 })
            .sort(doc! { "resource_id": direction })
            .hint(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()))
            .limit(PAGE_CANDIDATE_BATCH_ROWS as i64)
            .batch_size(PAGE_CANDIDATE_BATCH_ROWS as u32)
            .max_time(self.probe_timeout)
            .await;
        let mut cursor = match opened {
            Err(error) if optional_probe_timed_out(&error) => {
                return Ok(None);
            }
            result => result.or_query_error("Failed to open ordered page probe")?,
        };
        let mut window = OrderedPageWindow {
            ids: Vec::new(),
            raw_rows: 0,
            may_continue: false,
        };
        let mut bytes = 0usize;
        loop {
            let row = match cursor.try_next().await {
                Err(error) if optional_probe_timed_out(&error) => {
                    return Ok(None);
                }
                result => result.or_query_error("Failed to read ordered page probe")?,
            };
            let Some(row) = row else { break };
            window.raw_rows += 1;
            let id = row.get_str("resource_id").map_err(|_| {
                internal_error("Search candidate is missing resource_id".to_owned())
            })?;
            // Equal IDs are adjacent in MongoDB's globally merged ID order.
            if window.ids.last().is_some_and(|last| last == id) {
                continue;
            }
            let size = id.len().saturating_add(16);
            if size > MAX_CANDIDATE_BYTES {
                return Err(internal_error(
                    "Search candidate exceeds the BSON batch budget".to_owned(),
                ));
            }
            if bytes.saturating_add(size) > MAX_CANDIDATE_BYTES {
                window.may_continue = true;
                return Ok(Some(window));
            }
            bytes += size;
            window.ids.push(id.to_owned());
        }
        window.may_continue = window.raw_rows == PAGE_CANDIDATE_BATCH_ROWS;
        Ok(Some(window))
    }

    async fn try_ordered_page(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        initial_boundary: Option<&Document>,
        selection: &BatchSelection,
    ) -> StorageResult<Option<BatchMatches>> {
        let BatchSelection::Page { direction, .. } = selection else {
            return Err(internal_error("Expected ordered page selection".to_owned()));
        };
        let Some(mut window) = self
            .read_ordered_page_window(db, initial_boundary, *direction)
            .await?
        else {
            return Ok(None);
        };
        // No observed cardinality proves emptiness. Only a full distinct window
        // selects this plan; all other probes run the existing page path.
        if window.ids.len() != PAGE_CANDIDATE_BATCH_ROWS {
            return Ok(None);
        }
        let mut result = BatchMatches {
            count: 0,
            ids: Vec::new(),
        };
        loop {
            let live_before = result.count;
            if self
                .match_batch(db, resource_filter, selection, &window.ids, &mut result)
                .await?
            {
                return Ok(Some(result));
            }
            // A dense index may contain stale IDs. Keep the existing fallback
            // when live matches become sparse; this observation chooses work,
            // never an empty result or a count.
            if window.may_continue
                && result.count.saturating_sub(live_before).saturating_mul(2)
                    < window.ids.len() as u64
            {
                return Ok(None);
            }
            if !window.may_continue {
                return Ok(Some(result));
            }
            let last = window
                .ids
                .last()
                .ok_or_else(|| internal_error("Missing ordered page boundary".to_owned()))?;
            let operator = if *direction == 1 { "$gt" } else { "$lt" };
            let boundary = doc! { "resource_id": { operator: last.clone() } };
            let Some(next) = self
                .read_ordered_page_window(db, Some(&boundary), *direction)
                .await?
            else {
                // Discard any partial page and replay the original query. A
                // timeout or density change never truncates or invents results.
                return Ok(None);
            };
            window = next;
        }
    }

    async fn count_batch(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        selection: BatchSelection,
        batch: Vec<String>,
    ) -> StorageResult<BatchMatches> {
        let mut result = BatchMatches {
            count: 0,
            ids: Vec::new(),
        };
        self.match_batch(db, resource_filter, &selection, &batch, &mut result)
            .await?;
        Ok(result)
    }

    fn seed_pipeline(&self, boundary: Option<Document>, direction: Option<i32>) -> Vec<Document> {
        let filter = match boundary {
            Some(boundary) => doc! { "$and": [self.driver.filter.clone(), boundary] },
            None => self.driver.filter.clone(),
        };
        // Group before streaming so duplicate index rows cannot cross batches.
        // Only one predicate's IDs are grouped; resources and other predicates
        // no longer enter a whole-type union or shared blocking group.
        let mut pipeline = vec![
            doc! { "$match": filter },
            doc! { "$group": { "_id": "$resource_id" } },
        ];
        // Pages need a deterministic stream; a total does not. Avoid another
        // blocking stage and its sort memory for exact counts.
        if let Some(direction) = direction {
            pipeline.push(doc! { "$sort": { "_id": direction } });
        }
        pipeline
    }

    async fn remaining_candidates(
        db: &mongodb::Database,
        batch: &[String],
        predicates: &[IndexPredicate],
    ) -> StorageResult<Vec<String>> {
        let mut candidates: HashSet<_> = batch.iter().cloned().collect();
        let predicates: Vec<_> = predicates.iter().collect();
        filter_candidates(db, &mut candidates, &predicates).await?;
        Ok(candidates.into_iter().collect())
    }

    /// Leave the last indexed predicate in MongoDB so estimates can return a
    /// scalar count or a combined count/ID facet without a second matching pass.
    async fn estimated_batch_pipeline(
        &self,
        db: &mongodb::Database,
        batch: &[String],
    ) -> StorageResult<Option<Vec<Document>>> {
        // A single-predicate estimated page has no remaining predicate; its
        // batches rematch the driver so the facet can still count and page.
        let (last, preceding) = self.remaining.split_last().unwrap_or((&self.driver, &[]));
        let ids = Self::remaining_candidates(db, batch, preceding).await?;
        if ids.is_empty() {
            return Ok(None);
        }
        Ok(Some(vec![
            doc! { "$match": { "$and": [
                last.filter.clone(), { "resource_id": { "$in": ids } },
            ] } },
            doc! { "$group": { "_id": "$resource_id" } },
        ]))
    }

    /// Count a bounded batch using the same predicate order and membership
    /// checks as exact counts. The final indexed predicate returns a scalar
    /// instead of IDs; there is no live-resource check or per-ID lookup.
    async fn count_estimated_batch(
        &self,
        db: &mongodb::Database,
        batch: &[String],
    ) -> StorageResult<u64> {
        let Some(mut pipeline) = self.estimated_batch_pipeline(db, batch).await? else {
            return Ok(0);
        };
        pipeline.push(doc! { "$count": "total" });
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut cursor = index
            .aggregate(pipeline)
            .hint(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()))
            .await
            .or_query_error("Failed to count indexed candidate batch")?;
        match cursor
            .try_next()
            .await
            .or_query_error("Failed to read indexed batch count")?
        {
            None => Ok(0),
            Some(document) => parse_total(&document, "total"),
        }
    }

    /// Match once for an estimated total and a live page. The index facet
    /// counts the complete batch; cursor/top-k bounds restrict only page IDs.
    async fn match_estimated_page_batch(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        selection: &BatchSelection,
        batch: &[String],
        result: &mut BatchMatches,
    ) -> StorageResult<bool> {
        let BatchSelection::EstimatedPageAndCount {
            direction,
            offset,
            limit,
            boundary,
        } = selection
        else {
            return Err(internal_error(
                "Expected estimated page/count selection".to_owned(),
            ));
        };
        let Some(mut pipeline) = self.estimated_batch_pipeline(db, batch).await? else {
            return Ok(false);
        };
        pipeline.push(doc! { "$facet": {
            "total": [{ "$count": "n" }],
            "ids": [{ "$project": { "_id": 1 } }],
        } });
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut cursor = index
            .aggregate(pipeline)
            .hint(Hint::Name(SEARCH_COMPOSITE_INDEX.to_owned()))
            .await
            .or_query_error("Failed to count and match indexed page candidates")?;
        let facet = cursor
            .try_next()
            .await
            .or_query_error("Failed to read indexed page/count batch")?
            .ok_or_else(|| internal_error("Missing indexed page/count batch".to_owned()))?;
        result.count = facet_total(&facet)?;
        let mut ids = facet
            .get_array("ids")
            .map_err(|_| internal_error("Missing indexed candidate IDs".to_owned()))?
            .iter()
            .map(|row| {
                row.as_document()
                    .and_then(|doc| doc.get_str("_id").ok())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        internal_error("Search candidate is missing resource_id".to_owned())
                    })
            })
            .collect::<StorageResult<Vec<_>>>()?;
        let bounds = IdBounds::from_boundary(boundary.as_ref())?;
        ids.retain(|id| bounds.contains(id));
        result.ids =
            Self::select_live_page(db, resource_filter, ids, *direction, *offset, *limit, None)
                .await?;
        Ok(false)
    }

    /// Select a live top-k after indexed matching. Bounds restrict pages only.
    async fn select_live_page(
        db: &mongodb::Database,
        resource_filter: &Document,
        ids: Vec<String>,
        direction: i32,
        offset: u32,
        limit: usize,
        boundary: Option<&Document>,
    ) -> StorageResult<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut filters = vec![resource_filter.clone(), doc! { "id": { "$in": ids } }];
        if let Some(boundary) = boundary {
            filters.push(boundary.clone());
        }
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        collect_documents(
            resources
                .find(doc! { "$and": filters })
                .projection(doc! { "_id": 0, "id": 1 })
                .sort(doc! { "id": direction })
                .limit(retained_page_limit(offset, limit)?)
                .await
                .or_query_error("Failed to select live page candidates")?,
        )
        .await?
        .into_iter()
        .map(|document| {
            document
                .get_str("id")
                .map(str::to_owned)
                .map_err(|_| internal_error("Live candidate is missing id".to_owned()))
        })
        .collect()
    }

    async fn match_batch(
        &self,
        db: &mongodb::Database,
        resource_filter: &Document,
        selection: &BatchSelection,
        batch: &[String],
        result: &mut BatchMatches,
    ) -> StorageResult<bool> {
        if batch.is_empty() {
            return Ok(false);
        }
        if matches!(selection, BatchSelection::EstimatedCount) {
            result.count += self.count_estimated_batch(db, batch).await?;
            return Ok(false);
        }
        if matches!(selection, BatchSelection::EstimatedPageAndCount { .. }) {
            return self
                .match_estimated_page_batch(db, resource_filter, selection, batch, result)
                .await;
        }
        let ids = Self::remaining_candidates(db, batch, &self.remaining).await?;
        if ids.is_empty() {
            return Ok(false);
        }
        if let BatchSelection::BoundedPage {
            direction,
            offset,
            limit,
            boundary,
        } = selection
        {
            result.ids = Self::select_live_page(
                db,
                resource_filter,
                ids,
                *direction,
                *offset,
                *limit,
                boundary.as_ref(),
            )
            .await?;
            return Ok(false);
        }
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let count_only = match selection {
            BatchSelection::Count => true,
            BatchSelection::PageAndCount {
                boundary: Some(boundary),
                ..
            } => {
                // A batch outside the page bounds still contributes its full live count.
                let bounds = IdBounds::from_boundary(Some(boundary))?;
                !ids.iter().any(|id| bounds.contains(id))
            }
            _ => false,
        };
        let live_filter = doc! { "$and": [resource_filter.clone(), { "id": { "$in": ids } }] };
        if count_only {
            // Seed IDs are globally deduplicated and resource identity is
            // unique within tenant/type. Count live matches server-side;
            // returning their IDs would add transfer without new information.
            // Leave index selection to MongoDB: the type-scan index places
            // last_updated before id, so forcing it can repeatedly scan many
            // timestamp partitions for an otherwise bounded ID lookup.
            result.count += resources
                .count_documents(live_filter)
                .await
                .or_query_error("Failed to count live candidates")?;
            return Ok(false);
        }
        if let BatchSelection::PageAndCount {
            direction,
            offset,
            limit,
            boundary,
        } = selection
        {
            // Count every live match, but transfer only the batch's best page
            // IDs. An ID outside a batch's top k cannot enter the global top k.
            // Project before the facet so it never buffers resource bodies;
            // its input is bounded by the candidate row and BSON budgets.
            let limit = retained_page_limit(*offset, *limit)?;
            let mut page_stages = Vec::new();
            if let Some(boundary) = boundary {
                page_stages.push(doc! { "$match": boundary.clone() });
            }
            page_stages.extend([
                doc! { "$sort": { "id": *direction } },
                doc! { "$limit": limit },
            ]);
            let mut cursor = resources
                .aggregate(vec![
                    doc! { "$match": live_filter },
                    doc! { "$project": { "_id": 0, "id": 1 } },
                    doc! { "$facet": {
                        "total": [{ "$count": "n" }],
                        "page": page_stages,
                    } },
                ])
                .await
                .or_query_error("Failed to count and select live candidates")?;
            let facet = cursor
                .try_next()
                .await
                .or_query_error("Failed to read counted candidate page")?
                .ok_or_else(|| internal_error("Missing counted candidate page".to_owned()))?;
            result.count = facet_total(&facet)?;
            result.ids = facet
                .get_array("page")
                .map_err(|_| internal_error("Missing candidate page".to_owned()))?
                .iter()
                .map(|row| {
                    row.as_document()
                        .and_then(|doc| doc.get_str("id").ok())
                        .map(str::to_owned)
                        .ok_or_else(|| internal_error("Live candidate is missing id".to_owned()))
                })
                .collect::<StorageResult<Vec<_>>>()?;
            return Ok(false);
        }
        let mut cursor = resources
            .find(live_filter)
            .projection(doc! { "_id": 0, "id": 1 })
            .await
            .or_query_error("Failed to validate live candidates")?;
        let mut live = HashSet::new();
        while cursor
            .advance()
            .await
            .or_query_error("Failed to read live candidate")?
        {
            let document = cursor
                .deserialize_current()
                .or_query_error("Invalid live candidate")?;
            let id = document
                .get_str("id")
                .map_err(|_| internal_error("Live candidate is missing id".to_owned()))?;
            live.insert(id.to_owned());
        }
        // Finds need not preserve order. Follow the deduplicated seed stream.
        for id in batch.iter().filter(|id| live.contains(id.as_str())) {
            result.count += 1;
            if let BatchSelection::Page { offset, limit, .. } = selection {
                if result.count > u64::from(*offset) {
                    result.ids.push(id.clone());
                }
                if result.ids.len() >= *limit {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod candidate_tests {
    use super::*;

    #[test]
    fn totals_accept_nonnegative_bson_integers_and_reject_malformed_results() {
        for value in [Bson::Int32(0), Bson::Int32(17), Bson::Int64(i64::MAX)] {
            let expected = match value {
                Bson::Int32(value) => value as u64,
                Bson::Int64(value) => value as u64,
                _ => unreachable!(),
            };
            assert_eq!(
                parse_total(&doc! { "n": value.clone() }, "n").unwrap(),
                expected
            );
            assert_eq!(
                facet_total(&doc! { "total": [{ "n": value }] }).unwrap(),
                expected
            );
        }
        assert_eq!(facet_total(&doc! { "total": [] }).unwrap(), 0);
        for value in [
            Bson::Int32(-1),
            Bson::Int64(-1),
            Bson::Double(1.0),
            Bson::String("1".into()),
            Bson::Null,
        ] {
            assert!(parse_total(&doc! { "n": value.clone() }, "n").is_err());
            assert!(facet_total(&doc! { "total": [{ "n": value }] }).is_err());
        }
        assert!(parse_total(&doc! {}, "n").is_err());
        for document in [
            doc! {},
            doc! { "total": 0 },
            doc! { "total": [0] },
            doc! { "total": [{}] },
            doc! { "total": [{"n": 1}, {"n": 2}] },
        ] {
            assert!(facet_total(&document).is_err(), "{document:?}");
        }
    }

    #[test]
    fn retained_page_limits_check_arithmetic_and_bson_range() {
        assert_eq!(retained_page_size(4097, 21).unwrap(), 4118);
        assert_eq!(retained_page_limit(4097, 21).unwrap(), 4118);
        assert!(retained_page_size(1, usize::MAX).is_err());
        assert!(retained_page_limit(1, usize::MAX).is_err());
        if usize::BITS == 64 {
            assert!(retained_page_limit(0, usize::MAX).is_err());
        }
    }

    #[test]
    fn page_bounds_keep_both_strict_cursor_limits() {
        let boundary = doc! { "id": { "$gt": "b", "$lt": "d" } };
        let bounds = IdBounds::from_boundary(Some(&boundary)).unwrap();
        for (id, expected) in [
            ("a", false),
            ("b", false),
            ("c", true),
            ("d", false),
            ("e", false),
        ] {
            assert_eq!(bounds.contains(id), expected, "{id}");
        }
        assert!(IdBounds::from_boundary(None).unwrap().contains("anything"));
        assert!(IdBounds::from_boundary(Some(&doc! { "id": "invalid" })).is_err());
    }

    #[test]
    fn execution_plan_total_decision_table() {
        use crate::types::{SearchValue, SortDirective, TotalMode};
        use PageTotalExecution::{Accurate, Estimated, Separate};
        let cases = [
            (
                "exact first",
                TotalMode::Accurate,
                false,
                "_id",
                0,
                20,
                false,
                Accurate,
            ),
            (
                "exact low offset",
                TotalMode::Accurate,
                false,
                "_id",
                20,
                20,
                false,
                Accurate,
            ),
            (
                "exact memory edge",
                TotalMode::Accurate,
                false,
                "_id",
                9979,
                20,
                false,
                Accurate,
            ),
            (
                "exact past edge",
                TotalMode::Accurate,
                false,
                "_id",
                9980,
                20,
                false,
                Separate,
            ),
            (
                "cursor overrides deep offset",
                TotalMode::Accurate,
                false,
                "-_id",
                40097,
                20,
                true,
                Accurate,
            ),
            (
                "default sort",
                TotalMode::Accurate,
                false,
                "",
                0,
                20,
                false,
                Separate,
            ),
            (
                "timestamp sort",
                TotalMode::Accurate,
                false,
                "_lastUpdated",
                0,
                20,
                false,
                Separate,
            ),
            (
                "parameter sort",
                TotalMode::Accurate,
                false,
                "date",
                0,
                20,
                false,
                Separate,
            ),
            (
                "estimate combined",
                TotalMode::Estimate,
                true,
                "_id",
                4097,
                20,
                false,
                Estimated,
            ),
            (
                "estimate deep",
                TotalMode::Estimate,
                true,
                "_id",
                10000,
                20,
                false,
                Separate,
            ),
            (
                "estimate fallback",
                TotalMode::Estimate,
                false,
                "_id",
                0,
                20,
                false,
                Separate,
            ),
            (
                "none",
                TotalMode::None,
                false,
                "_id",
                0,
                20,
                false,
                Separate,
            ),
            (
                "zero count",
                TotalMode::Accurate,
                false,
                "_id",
                9998,
                0,
                false,
                Accurate,
            ),
            (
                "large count",
                TotalMode::Accurate,
                false,
                "_id",
                0,
                u32::MAX,
                false,
                Separate,
            ),
        ];
        for (name, total, eligible, sort, offset, count, cursor, expected) in cases {
            let mut query = SearchQuery::new("Observation").with_count(count);
            query.total = Some(total);
            query.offset = Some(offset);
            if !sort.is_empty() {
                query.sort.push(SortDirective::parse(sort));
            }
            if cursor {
                query.cursor = Some("validated-cursor".into());
            }
            query.parameters = ["code", "status"]
                .into_iter()
                .map(|name| SearchParameter {
                    name: name.into(),
                    param_type: SearchParamType::Token,
                    modifier: None,
                    values: vec![SearchValue::eq("value")],
                    chain: vec![],
                    components: vec![],
                })
                .collect();
            let facts = ExecutionFacts {
                index_estimate_eligible: eligible,
                adaptive_offset_enabled: true,
            };
            let plan = plan_execution(&query, facts);
            assert_eq!(plan.page_total, expected, "{name}");
            assert_eq!(
                plan.filter_purpose,
                if expected == Separate {
                    SearchFilterPurpose::Page
                } else {
                    SearchFilterPurpose::Count
                },
                "{name}"
            );
            assert!(
                matches!(facts.count_selection(), BatchSelection::EstimatedCount) == eligible,
                "{name}"
            );
            let boundary = Some(doc! {"id":{"$gt":"m"}});
            match (
                expected,
                plan.page_selection(-1, offset, 21, boundary.clone()),
            ) {
                (
                    Accurate,
                    BatchSelection::PageAndCount {
                        direction,
                        offset: got_offset,
                        limit,
                        boundary: got_boundary,
                    },
                )
                | (
                    Estimated,
                    BatchSelection::EstimatedPageAndCount {
                        direction,
                        offset: got_offset,
                        limit,
                        boundary: got_boundary,
                    },
                ) => {
                    assert_eq!(
                        (direction, got_offset, limit, got_boundary),
                        (-1, offset, 21, boundary),
                        "{name}"
                    );
                }
                (
                    Separate,
                    BatchSelection::Page {
                        direction,
                        offset: got_offset,
                        limit,
                    },
                ) => {
                    assert_eq!((direction, got_offset, limit), (-1, offset, 21), "{name}");
                }
                _ => panic!("{name}: wrong batch selection"),
            }
        }
    }

    #[test]
    fn estimate_summary_and_single_filter_keep_their_existing_execution() {
        use crate::types::{IncludeDirective, SearchValue, SortDirective, SummaryMode, TotalMode};
        let mut query = SearchQuery::new("Observation")
            .with_sort(SortDirective::parse("_id"))
            .with_parameter(SearchParameter {
                name: "status".into(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("final")],
                chain: vec![],
                components: vec![],
            });
        query.total = Some(TotalMode::Estimate);
        let facts = || ExecutionFacts {
            index_estimate_eligible: true,
            adaptive_offset_enabled: true,
        };
        assert_eq!(
            plan_execution(&query, facts()).page_total,
            PageTotalExecution::Separate
        );
        query.summary = Some(SummaryMode::Count);
        assert_eq!(
            plan_execution(&query, facts()).page_total,
            PageTotalExecution::EstimateOnly
        );
        query.includes.push(IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Observation".into(),
            search_param: "subject".into(),
            target_type: None,
            iterate: false,
        });
        assert_eq!(
            plan_execution(&query, facts()).page_total,
            PageTotalExecution::Separate
        );
        query.includes.clear();
        assert_eq!(
            plan_execution(
                &query,
                ExecutionFacts {
                    index_estimate_eligible: false,
                    adaptive_offset_enabled: true
                }
            )
            .page_total,
            PageTotalExecution::Separate
        );
    }

    #[test]
    fn no_total_page_policy_has_explicit_query_boundaries() {
        use crate::types::{SortDirective, TotalMode};
        use NoTotalPagePolicy::{SampleSelectivity, Streaming};

        #[derive(Clone, Copy)]
        struct Case {
            name: &'static str,
            offset: u32,
            count: Option<u32>,
            sort: &'static str,
            total: Option<TotalMode>,
            cursor: bool,
            expected: NoTotalPagePolicy,
        }
        let base = Case {
            name: "regression offset",
            offset: 4097,
            count: Some(20),
            sort: "_id",
            total: None,
            cursor: false,
            expected: SampleSelectivity,
        };
        let cases = [
            base,
            Case {
                name: "first",
                offset: 0,
                expected: Streaming,
                ..base
            },
            Case {
                name: "low offset",
                offset: 20,
                expected: Streaming,
                ..base
            },
            Case {
                name: "window edge",
                offset: 256,
                expected: Streaming,
                ..base
            },
            Case {
                name: "eligible offset",
                offset: 257,
                ..base
            },
            Case {
                name: "descending",
                sort: "-_id",
                ..base
            },
            Case {
                name: "explicit no total",
                total: Some(TotalMode::None),
                ..base
            },
            Case {
                name: "exact",
                total: Some(TotalMode::Accurate),
                expected: Streaming,
                ..base
            },
            Case {
                name: "estimate",
                total: Some(TotalMode::Estimate),
                expected: Streaming,
                ..base
            },
            Case {
                name: "cursor overrides offset",
                cursor: true,
                expected: Streaming,
                ..base
            },
            Case {
                name: "default sort",
                sort: "",
                expected: Streaming,
                ..base
            },
            Case {
                name: "timestamp sort",
                sort: "_lastUpdated",
                expected: Streaming,
                ..base
            },
            Case {
                name: "parameter sort",
                sort: "date",
                expected: Streaming,
                ..base
            },
            Case {
                name: "memory edge",
                offset: 9979,
                ..base
            },
            Case {
                name: "over memory edge",
                offset: 9980,
                expected: Streaming,
                ..base
            },
            Case {
                name: "deep offset",
                offset: 10000,
                expected: Streaming,
                ..base
            },
            Case {
                name: "maximum offset",
                offset: u32::MAX,
                expected: Streaming,
                ..base
            },
            Case {
                name: "maximum count",
                count: Some(u32::MAX),
                expected: Streaming,
                ..base
            },
            Case {
                name: "default count fits",
                offset: 9899,
                count: None,
                ..base
            },
            Case {
                name: "default count exceeds",
                offset: 9900,
                count: None,
                expected: Streaming,
                ..base
            },
            Case {
                name: "zero count normalizes",
                offset: 9998,
                count: Some(0),
                ..base
            },
        ];
        for case in cases {
            let mut query = SearchQuery::new("Observation");
            query.offset = Some(case.offset);
            query.count = case.count;
            query.total = case.total;
            if !case.sort.is_empty() {
                query.sort.push(SortDirective::parse(case.sort));
            }
            if case.cursor {
                query.cursor = Some("fixture-cursor".to_string());
            }
            assert_eq!(
                NoTotalPagePolicy::for_query(&query, SearchFilterPurpose::Page, 2),
                case.expected,
                "{}",
                case.name
            );
            assert_eq!(
                NoTotalPagePolicy::for_query(&query, SearchFilterPurpose::Count, 2),
                Streaming,
                "count execution must stay unchanged: {}",
                case.name
            );
        }
        let mut query = SearchQuery::new("Observation").with_count(20);
        query.offset = Some(4097);
        query.sort.push(SortDirective::parse("_id"));
        for indexed_predicates in [0, 1] {
            assert_eq!(
                NoTotalPagePolicy::for_query(&query, SearchFilterPurpose::Page, indexed_predicates),
                Streaming,
                "single-filter ordered reads must stay unchanged"
            );
        }
        query.sort.push(SortDirective::parse("_lastUpdated"));
        assert_eq!(
            NoTotalPagePolicy::for_query(&query, SearchFilterPurpose::Page, 2),
            Streaming,
            "compound sorting must stay unchanged"
        );
    }

    #[test]
    fn no_total_page_policy_requires_a_complete_sparse_live_sample() {
        for (name, sample, bounded) in [
            ("unknown", None, false),
            (
                "dense date range",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 256,
                }),
                false,
            ),
            (
                "dense status and date",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 240,
                }),
                false,
            ),
            (
                "moderate intersection",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 64,
                }),
                false,
            ),
            (
                "sparse boundary",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 16,
                }),
                true,
            ),
            (
                "above sparse boundary",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 17,
                }),
                false,
            ),
            (
                "empty sample is only a planning fact",
                Some(PageSample {
                    candidates: 256,
                    live_matches: 0,
                }),
                true,
            ),
            (
                "incomplete sample",
                Some(PageSample {
                    candidates: 255,
                    live_matches: 0,
                }),
                false,
            ),
            (
                "invalid oversized sample",
                Some(PageSample {
                    candidates: 257,
                    live_matches: 0,
                }),
                false,
            ),
            (
                "invalid match count",
                Some(PageSample {
                    candidates: 256,
                    live_matches: u64::MAX,
                }),
                false,
            ),
        ] {
            assert_eq!(
                NoTotalPagePolicy::SampleSelectivity.use_bounded(sample),
                bounded,
                "{name}"
            );
            assert!(
                !NoTotalPagePolicy::Streaming.use_bounded(sample),
                "ineligible query: {name}"
            );
        }
    }

    #[tokio::test]
    async fn misleading_samples_and_cached_decisions_preserve_live_offset_pages() {
        use crate::backends::mongodb::MongoBackendConfig;
        use testcontainers::runners::AsyncRunner;
        // Standalone is enough: this test performs no transactions.
        let (uri, _container) = match std::env::var("HFS_TEST_MONGODB_URL") {
            Ok(uri) => (uri, None),
            Err(_) => match testcontainers_modules::mongo::Mongo::default()
                .start()
                .await
            {
                Ok(container) => {
                    let host = container.get_host().await.unwrap();
                    let port = container.get_host_port_ipv4(27017).await.unwrap();
                    (format!("mongodb://{host}:{port}"), Some(container))
                }
                Err(error) => {
                    super::execution_tests::skip_or_fail(
                        "sample-bias test",
                        &format!("no HFS_TEST_MONGODB_URL or Docker ({error})"),
                    );
                    return;
                }
            },
        };
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: uri,
            database_name: format!("hfs_adaptive_bias_{}", uuid::Uuid::new_v4().simple()),
            ..Default::default()
        })
        .unwrap();
        backend.init_schema().await.unwrap();
        backend.wait_for_search_index_build().await;
        let db = backend.get_database().await.unwrap();
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        const ROWS: usize = 12_000;
        const DELETED_PREFIX: usize = 256;
        let index_row = |id: &str, parameter: &str, value: &str| {
            doc! {
                "tenant_id":"bias", "resource_type":"Observation", "resource_id":id,
                "param_name":parameter,"value_token_code":value,
            }
        };
        for start in (0..ROWS).step_by(500) {
            let mut bodies = Vec::new();
            let mut indexes = Vec::new();
            for number in start..(start + 500).min(ROWS) {
                let id = format!("r{number:05}");
                bodies.push(doc! {"tenant_id":"bias", "resource_type":"Observation",
                "id":&id, "is_deleted":number<DELETED_PREFIX});
                indexes.push(index_row(&id, "code", "base"));
                indexes.push(index_row(&id, "status", "final"));
            }
            resources.insert_many(bodies).await.unwrap();
            index.insert_many(indexes).await.unwrap();
        }
        resources.insert_many([
            doc! {"tenant_id":"other", "resource_type":"Observation", "id":"foreign", "is_deleted":false},
            doc! {"tenant_id":"bias", "resource_type":"Patient", "id":"wrongtype", "is_deleted":false},
        ]).await.unwrap();
        for id in ["foreign", "missing", "wrongtype", "r00000"] {
            index
                .insert_many([
                    index_row(id, "code", "base"),
                    index_row(id, "status", "final"),
                ])
                .await
                .unwrap();
        }
        let driver = doc! {"tenant_id":"bias","resource_type":"Observation","param_name":"code","value_token_code":"base"};
        let remaining = doc! {"tenant_id":"bias","resource_type":"Observation","param_name":"status","value_token_code":"final"};
        let resource_filter =
            doc! {"tenant_id":"bias","resource_type":"Observation","is_deleted":false};
        let key_for = |filter: &Document| {
            ProbeKey::new(
                db.name(),
                "search_index",
                "bias",
                "Observation",
                filter,
                None,
                true,
                10_001,
                &Bson::Null,
            )
            .unwrap()
        };
        let cache_key = ProbeKey::offset_page(&key_for(&driver), &[key_for(&remaining)]).unwrap();
        let cache = ProbeCache::default();
        let plan = BatchedIndexPlan {
            driver: driver.into(),
            remaining: vec![remaining.into()],
            page_policy: NoTotalPagePolicy::SampleSelectivity,
            page_cache: Some((cache.clone(), cache_key.clone())),
            probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
        };
        for direction in [1, -1] {
            let mut ordered: Vec<_> = (DELETED_PREFIX..ROWS).map(|n| format!("r{n:05}")).collect();
            if direction == -1 {
                ordered.reverse();
            }
            let expected: Vec<_> = ordered.iter().skip(4097).take(21).cloned().collect();
            // Only the fixture is sorted, to guarantee a misleading first sample.
            // Production sampling uses the unsorted grouped cursor.
            let mut stages = plan.seed_pipeline(None, None);
            stages.push(doc! {"$sort":{"_id":1}});
            let mut cursor = index.aggregate(stages).batch_size(4096).await.unwrap();
            let mut sample = Vec::new();
            for _ in 0..PAGE_CANDIDATE_BATCH_ROWS {
                let row = cursor.try_next().await.unwrap().unwrap();
                sample.push(row.get_str("_id").unwrap().to_string());
            }
            let mut validated = BatchMatches {
                count: 0,
                ids: Vec::new(),
            };
            plan.match_batch(
                &db,
                &resource_filter,
                &BatchSelection::Page {
                    direction,
                    offset: 0,
                    limit: 257,
                },
                &sample,
                &mut validated,
            )
            .await
            .unwrap();
            assert_eq!(validated.count, 0, "the sample must actually be misleading");
            assert!(plan.page_policy.use_bounded(Some(PageSample {
                candidates: 256,
                live_matches: 0
            })));
            let result = plan
                .consume_batches(
                    &db,
                    &resource_filter,
                    cursor,
                    BatchSelection::BoundedPage {
                        direction,
                        offset: 4097,
                        limit: 21,
                        boundary: None,
                    },
                    validated.ids,
                )
                .await
                .unwrap();
            assert_eq!(
                result.ids, expected,
                "zero sample must continue the cursor, direction={direction}"
            );
            for decision in [OffsetPageDecision::Bounded, OffsetPageDecision::Streaming] {
                cache.remember_page_decision(cache_key.clone(), decision);
                let result = plan
                    .execute(
                        &db,
                        resource_filter.clone(),
                        None,
                        BatchSelection::Page {
                            direction,
                            offset: 4097,
                            limit: 21,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    result.ids, expected,
                    "cached {decision:?}, direction={direction}"
                );
            }
        }
        let total = plan
            .execute(&db, resource_filter, None, BatchSelection::Count)
            .await
            .unwrap();
        assert_eq!(total.count, (ROWS - DELETED_PREFIX) as u64);
        db.drop().await.unwrap();
    }

    #[test]
    fn remaining_order_uses_informative_probes_and_keeps_inconclusive_fallback() {
        let counts = [None, Some(37), Some(10_001), Some(3), Some(37)];
        assert_eq!(candidate_driver(&counts), Some(3));
        assert_eq!(remaining_filter_order(&counts, 3, true), vec![1, 4, 2, 0]);
        let counts = [Some(10_001), None, Some(10_001), None];
        assert_eq!(candidate_driver(&counts), None);
        assert_eq!(remaining_filter_order(&counts, 2, false), vec![0, 1, 3]);
        assert_eq!(
            remaining_filter_order(&[None], 0, false),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn tightening_page_bounds_preserves_global_cursor_pages() {
        let all: Vec<_> = (0..1400)
            .filter(|n| n % 7 != 0)
            .map(|n| format!("r{n:04}"))
            .collect();
        for direction in [1, -1] {
            for cursor_operator in ["$gt", "$lt"] {
                for retained in [1, 21, 301, 1500] {
                    let original = BatchSelection::PageAndCount {
                        direction,
                        offset: 0,
                        limit: retained,
                        boundary: Some(doc! { "id": { cursor_operator: "r0700" } }),
                    };
                    let mut page = PageIds {
                        ids: BTreeSet::new(),
                        direction,
                        limit: retained,
                    };
                    for batch in all.chunks(37).rev() {
                        let constrained = page.constrain(&original).unwrap();
                        let BatchSelection::PageAndCount {
                            boundary: Some(boundary),
                            ..
                        } = constrained
                        else {
                            panic!("combined cursor boundary must be preserved");
                        };
                        let bounds = boundary.get_document("id").unwrap();
                        let mut eligible: Vec<_> = batch
                            .iter()
                            .filter(|id| {
                                bounds
                                    .get_str("$gt")
                                    .ok()
                                    .is_none_or(|bound| id.as_str() > bound)
                                    && bounds
                                        .get_str("$lt")
                                        .ok()
                                        .is_none_or(|bound| id.as_str() < bound)
                            })
                            .cloned()
                            .collect();
                        eligible.sort();
                        if direction == -1 {
                            eligible.reverse();
                        }
                        eligible.truncate(retained);
                        page.extend(eligible);
                    }
                    let mut expected: Vec<_> = all
                        .iter()
                        .filter(|id| {
                            if cursor_operator == "$gt" {
                                id.as_str() > "r0700"
                            } else {
                                id.as_str() < "r0700"
                            }
                        })
                        .cloned()
                        .collect();
                    if direction == -1 {
                        expected.reverse();
                    }
                    expected.truncate(retained);
                    assert_eq!(page.into_ids(), expected);
                }
            }
        }
    }

    #[test]
    fn combined_page_retains_only_the_requested_extreme_ids() {
        for direction in [1, -1] {
            let mut page = PageIds {
                ids: BTreeSet::new(),
                direction,
                limit: 3,
            };
            for ids in [vec!["d", "a"], vec!["g", "b", "f"], vec!["c", "e", "a"]] {
                page.extend(ids.into_iter().map(str::to_owned).collect());
                assert!(page.ids.len() <= 3);
            }
            assert_eq!(
                page.into_ids(),
                if direction == 1 {
                    vec!["a", "b", "c"]
                } else {
                    vec!["g", "f", "e"]
                }
            );
        }
    }

    #[test]
    fn combined_pages_bound_offset_memory() {
        assert!(combined_page_fits(0, 21));
        assert!(combined_page_fits(257, 38));
        assert!(combined_page_fits(9_979, 21));
        assert!(!combined_page_fits(9_980, 21));
        assert!(!combined_page_fits(u32::MAX, 21));
        assert!(!combined_page_fits(1, usize::MAX));
    }

    #[test]
    fn combined_page_is_bounded_across_unordered_batches() {
        let count = COUNT_CANDIDATE_BATCH_ROWS * 3 + 1;
        let ids: Vec<_> = (0..count)
            .step_by(2)
            .chain((1..count).step_by(2))
            .map(|number| format!("r{number:05}"))
            .collect();
        for direction in [1, -1] {
            for limit in [1, 20, 100, 1000, count + 1] {
                let mut page = PageIds {
                    ids: BTreeSet::new(),
                    direction,
                    limit,
                };
                for batch in ids.chunks(COUNT_CANDIDATE_BATCH_ROWS).rev() {
                    // The server returns only each batch's top k IDs.
                    let mut batch_page = batch.to_vec();
                    batch_page.sort();
                    if direction == -1 {
                        batch_page.reverse();
                    }
                    batch_page.truncate(limit);
                    page.extend(batch_page);
                    assert!(page.ids.len() <= limit);
                }
                let mut expected = ids.clone();
                expected.sort();
                if direction == -1 {
                    expected.reverse();
                }
                expected.truncate(limit);
                assert_eq!(page.into_ids(), expected);
            }
        }
    }

    #[test]
    fn batched_plan_cannot_silently_execute_as_a_database_pipeline() {
        let plan = SearchFilterPlan::BatchedIndex(BatchedIndexPlan {
            driver: doc! { "param_name": "code" }.into(),
            remaining: Vec::new(),
            page_policy: NoTotalPagePolicy::Streaming,
            page_cache: None,
            probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
        });
        assert!(
            plan.pipeline(doc! { "is_deleted": false }, None, None)
                .is_err()
        );
    }

    #[test]
    fn batch_seed_deduplicates_before_streaming_in_cursor_order() {
        let predicate =
            doc! { "tenant_id": "tenant", "resource_type": "Observation", "param_name": "code" };
        let plan = BatchedIndexPlan {
            driver: predicate.clone().into(),
            remaining: Vec::new(),
            page_policy: NoTotalPagePolicy::Streaming,
            page_cache: None,
            probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
        };
        for (direction, operator) in [(1, "$gt"), (-1, "$lt")] {
            let boundary = doc! { "resource_id": { operator: "o0256" } };
            let stages = plan.seed_pipeline(Some(boundary.clone()), Some(direction));
            assert_eq!(
                stages,
                vec![
                    doc! { "$match": { "$and": [predicate.clone(), boundary] } },
                    doc! { "$group": { "_id": "$resource_id" } },
                    doc! { "$sort": { "_id": direction } },
                ]
            );
        }
        assert_eq!(
            plan.seed_pipeline(None, Some(1))[0],
            doc! { "$match": predicate }
        );
    }

    #[test]
    fn count_seed_deduplicates_without_sorting() {
        let predicate = doc! { "param_name": "status" };
        let plan = BatchedIndexPlan {
            driver: predicate.clone().into(),
            remaining: Vec::new(),
            page_policy: NoTotalPagePolicy::Streaming,
            page_cache: None,
            probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
        };
        assert_eq!(
            plan.seed_pipeline(None, None),
            vec![
                doc! { "$match": predicate },
                doc! { "$group": { "_id": "$resource_id" } },
            ]
        );
    }

    #[test]
    fn driver_uses_smallest_bounded_probe_regardless_of_parameter_order() {
        assert_eq!(
            candidate_driver(&[Some(10_001), Some(3_260), Some(8_000)]),
            Some(1)
        );
        assert_eq!(
            candidate_driver(&[Some(3_260), Some(100), Some(10_001)]),
            Some(1)
        );
        assert_eq!(candidate_driver(&[Some(10_001), Some(10_001)]), None);
        assert_eq!(candidate_driver(&[Some(10_000)]), Some(0));
        assert_eq!(candidate_driver(&[]), None);
    }

    #[test]
    fn unknown_probes_do_not_hide_a_completed_selective_probe() {
        assert_eq!(candidate_driver(&[None, Some(3260), None]), Some(1));
        assert_eq!(candidate_driver(&[Some(3260), None]), Some(0));
        assert_eq!(candidate_driver(&[None, Some(10001)]), None);
        assert_eq!(candidate_driver(&[None, None]), None);
    }

    #[test]
    fn only_optional_probe_timeouts_are_treated_as_unknown() {
        let command_error = |code| {
            let command: mongodb::error::CommandError =
                mongodb::bson::from_document(doc! { "code": code, "errmsg": "test" }).unwrap();
            mongodb::error::Error::from(mongodb::error::ErrorKind::Command(command))
        };
        for code in [50, 89, 262] {
            assert_eq!(probe_count(Err(command_error(code))).unwrap(), None);
        }
        assert_eq!(probe_count(Ok(0)).unwrap(), Some(0));
        assert_eq!(probe_count(Ok(3260)).unwrap(), Some(3260));
        assert!(probe_count(Err(command_error(13))).is_err());
        assert!(probe_count(Err(command_error(91))).is_err());
        assert_eq!(
            probe_count(Err(mongodb::error::Error::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "network timeout"
            ))))
            .unwrap(),
            None
        );
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            assert!(
                probe_count(Err(mongodb::error::Error::from(std::io::Error::new(
                    kind,
                    "not a timeout"
                ))))
                .is_err()
            );
        }
    }

    #[test]
    fn candidate_budget_deduplicates_and_refuses_excess_without_truncation() {
        let mut candidates = CandidateIds::default();
        assert!(candidates.insert("same"));
        let bytes = candidates.bson_bytes;
        assert!(candidates.insert("same"));
        assert_eq!(candidates.bson_bytes, bytes);
        for i in 1..MAX_CANDIDATE_ROWS {
            assert!(candidates.insert(&i.to_string()));
        }
        assert!(!candidates.insert("overflow"));
        assert_eq!(candidates.ids.len(), MAX_CANDIDATE_ROWS as usize);
    }

    #[test]
    fn candidate_budget_also_bounds_bson_size() {
        let mut candidates = CandidateIds::default();
        assert!(candidates.insert(&"x".repeat(MAX_CANDIDATE_BYTES - 16)));
        assert!(!candidates.insert("y"));
    }

    #[test]
    fn candidate_pages_and_counts_preserve_resource_and_cursor_constraints() {
        let live = doc! { "tenant_id": "t", "resource_type": "Observation",
        "is_deleted": false, "id": { "$gt": "p0" } };
        for ids in [vec!["p1".to_owned()], vec![]] {
            for sort in [None, Some(doc! { "id": -1 })] {
                let SearchPipeline {
                    collection, stages, ..
                } = SearchFilterPlan::CandidateIds(ids.clone())
                    .pipeline(live.clone(), sort.clone(), None)
                    .unwrap();
                assert_eq!(collection, MongoBackend::RESOURCES_COLLECTION);
                assert_eq!(
                    stages[0],
                    doc! { "$match": { "$and": [
                        live.clone(), { "id": { "$in": &ids } },
                    ] } }
                );
                assert_eq!(stages.len(), if sort.is_some() { 2 } else { 1 });
                if let Some(sort) = sort {
                    assert_eq!(stages[1], doc! { "$sort": sort });
                }
            }
        }
    }
}

#[cfg(test)]
mod index_intersection_tests {
    use super::*;

    fn date_predicate(raw: &str) -> IndexPredicate {
        let parameter = SearchParameter {
            name: "date".to_owned(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![crate::types::SearchValue::parse(raw)],
            chain: vec![],
            components: vec![],
        };
        let mut filter =
            doc! { "tenant_id": "t", "resource_type": "Observation", "param_name": "date" };
        filter.extend(
            super::super::build_date_range_filter_doc(&parameter.values[0], "date").unwrap(),
        );
        IndexPredicate::new(filter, &parameter)
    }

    #[test]
    fn end_bound_drives_date_intersection_without_dropping_other_occurrences_or_live_checks() {
        let ge = date_predicate("ge2024-11-01");
        let lt = date_predicate("lt2026-09-05");
        let patient: IndexPredicate =
            doc! { "param_name": "patient", "value_reference": "Patient/p0" }.into();
        let live = doc! { "tenant_id": "t", "resource_type": "Observation", "is_deleted": false, "id": { "$gt": "previous" } };
        for predicates in [
            vec![patient.clone(), ge.clone(), lt.clone()],
            vec![lt.clone(), patient, ge.clone()],
        ] {
            let ge_position = predicates
                .iter()
                .position(|p| p.access == IndexAccess::DateEndRange)
                .unwrap();
            let plan = SearchFilterPlan::IndexIntersection(predicates)
                .pipeline(live.clone(), Some(doc! {"id": 1}), None)
                .unwrap();
            assert_eq!(plan.collection, MongoBackend::SEARCH_INDEX_COLLECTION);
            assert_eq!(
                plan.hint,
                Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned()))
            );
            assert_eq!(plan.stages[0], doc! { "$match": ge.filter.clone() });
            assert_eq!(
                plan.stages[1]
                    .get_document("$project")
                    .unwrap()
                    .get_document("_hfs_predicate")
                    .unwrap()
                    .get_i64("$literal")
                    .unwrap(),
                ge_position as i64
            );
            let branches: Vec<_> = plan
                .stages
                .iter()
                .filter_map(|s| s.get_document("$unionWith").ok())
                .collect();
            assert_eq!(branches.len(), 3);
            let resources = branches
                .iter()
                .find(|b| b.get_str("coll").unwrap() == MongoBackend::RESOURCES_COLLECTION)
                .unwrap();
            assert_eq!(
                resources.get_array("pipeline").unwrap()[0],
                mongodb::bson::to_bson(&doc! { "$match": live.clone() }).unwrap()
            );
            assert!(plan.stages.contains(
                &doc! { "$match": { "_hfs_predicates": { "$all": [0_i64, 1_i64, 2_i64, 3_i64] } } }
            ));
            assert!(branches.iter().any(|b| b.get_array("pipeline").unwrap()[0]
                == mongodb::bson::to_bson(&doc! { "$match": lt.filter.clone() }).unwrap()));
        }
    }

    #[test]
    fn resource_lookup_keeps_its_date_hint_and_cursor_bound() {
        let predicate = date_predicate("ge2024-11-01");
        let boundary = doc! { "resource_id": { "$lt": "next" } };
        let plan = SearchFilterPlan::ResourceLookups {
            index_filter: Some(predicate.clone()),
            stages: Vec::new(),
        }
        .pipeline(
            doc! { "is_deleted": false },
            Some(doc! { "id": -1 }),
            Some(boundary.clone()),
        )
        .unwrap();
        assert_eq!(
            plan.hint,
            Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned()))
        );
        assert_eq!(
            plan.stages[0],
            doc! { "$match": { "$and": [predicate.filter, boundary] } }
        );
    }

    #[test]
    fn counting_intersects_predicate_occurrences_and_live_resources_without_lookups() {
        let filters = vec![
            doc! {"param_name": "patient", "value_reference": "Patient/p0"},
            doc! {"param_name": "date", "value_date": {"$gte": 1}},
            doc! {"param_name": "date", "value_date": {"$lt": 2}},
        ];
        let live = doc! {"tenant_id": "t", "resource_type": "Observation", "is_deleted": false};
        let SearchPipeline {
            collection,
            stages: pipeline,
            ..
        } = SearchFilterPlan::IndexIntersection(filters.into_iter().map(Into::into).collect())
            .pipeline(live.clone(), None, None)
            .unwrap();
        assert_eq!(collection, MongoBackend::RESOURCES_COLLECTION);
        assert_eq!(pipeline[0], doc! {"$match": live});
        assert!(!pipeline.iter().any(|stage| stage.contains_key("$lookup")));
        let tags: Vec<_> = pipeline
            .iter()
            .filter_map(|stage| stage.get_document("$unionWith").ok())
            .map(|branch| {
                branch.get_array("pipeline").unwrap()[1]
                    .as_document()
                    .unwrap()
                    .get_document("$project")
                    .unwrap()
                    .get_document("_hfs_predicate")
                    .unwrap()
                    .get_i64("$literal")
                    .unwrap()
            })
            .collect();
        assert_eq!(tags, vec![0, 1, 2]);
        assert!(pipeline.contains(
            &doc! {"$match": {"_hfs_predicates": {"$all": [0_i64, 1_i64, 2_i64, 3_i64]}}}
        ));
    }
}

#[cfg(test)]
mod execution_tests;
