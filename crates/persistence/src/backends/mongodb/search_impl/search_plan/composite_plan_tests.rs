//! Composite parameters, positive reference `:identifier` and compartment
//! membership planned as predicates (#1896). The corpus is raw rows with
//! independent expectations; the legacy collector (`matching_resource_ids`)
//! serves as the oracle for results, order, flags and totals.

use super::super::super::{CursorKeyset, MissingSupport};
use super::*;
use crate::error::{SearchError, StorageError};
use crate::types::{CompartmentMembership, CompositeSearchComponent};

const KINDS: usize = 1200;
const BASE_SECONDS: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z
const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";

fn at(seconds: i64) -> DateTime {
    DateTime::from_millis((BASE_SECONDS + seconds) * 1000)
}

fn resource_doc(tenant: &str, kind: &str, id: &str, seconds: i64, deleted: bool) -> Document {
    doc! {
        "tenant_id": tenant, "resource_type": kind, "id": id,
        "version_id": "1", "created_at": at(seconds), "last_updated": at(seconds),
        "is_deleted": deleted, "fhir_version": "4.0",
        "data": { "resourceType": kind, "id": id, "status": "final" },
    }
}

fn status_row(tenant: &str, kind: &str, id: &str, status: &str) -> Document {
    doc! { "tenant_id": tenant, "resource_type": kind, "resource_id": id,
    "param_name": "status", "value_token_code": status }
}

fn composite_token(
    tenant: &str,
    kind: &str,
    id: &str,
    name: &str,
    group: i32,
    code: &str,
) -> Document {
    doc! { "tenant_id": tenant, "resource_type": kind, "resource_id": id,
    "param_name": name, "value_token_system": LOINC, "value_token_code": code,
    "composite_group": group, "composite_slot": 1_i32 }
}

fn composite_quantity(
    tenant: &str,
    kind: &str,
    id: &str,
    name: &str,
    group: i32,
    value: f64,
) -> Document {
    doc! { "tenant_id": tenant, "resource_type": kind, "resource_id": id,
    "param_name": name, "value_quantity_value": value, "value_quantity_unit": "cm",
    "value_quantity_system": UCUM, "composite_group": group, "composite_slot": 1_i32 }
}

/// Rows of one `code-value-quantity` panel: `(group, code, value)` per entry.
fn panel_rows(tenant: &str, kind: &str, id: &str, panel: &[(i32, &str, f64)]) -> Vec<Document> {
    let mut rows = Vec::new();
    for (group, code, value) in panel {
        rows.push(composite_token(
            tenant,
            kind,
            id,
            "code-value-quantity",
            *group,
            code,
        ));
        rows.push(composite_quantity(
            tenant,
            kind,
            id,
            "code-value-quantity",
            *group,
            *value,
        ));
    }
    rows
}

/// What an Observation of the corpus holds, by `n % 6`.
fn panel_of(n: usize) -> Vec<(i32, &'static str, f64)> {
    match n % 6 {
        // 8302-2 at 170 in one group: matches `8302-2$gt160`.
        0..=2 => vec![(0, "8302-2", 170.0)],
        // The code and a value above 160 exist, but in different groups.
        3 => vec![(0, "8302-2", 100.0), (1, "29463-7", 170.0)],
        // The pair fails: 150 is not above 160.
        4 => vec![(0, "8302-2", 150.0)],
        // Another code; below 100 for every other block of six.
        _ => {
            let value = if (n / 6).is_multiple_of(2) {
                60.0
            } else {
                170.0
            };
            vec![(0, "29463-7", value)]
        }
    }
}

fn id_of(n: usize) -> String {
    format!("c{n:05}")
}

fn is_final(n: usize) -> bool {
    n.is_multiple_of(2)
}

fn ids_where(predicate: impl Fn(usize) -> bool) -> Vec<String> {
    (0..KINDS).filter(|n| predicate(*n)).map(id_of).collect()
}

/// Seeds the corpus: 1,200 live Observations in six interleaved kinds, 10
/// deleted ones with matching rows, 5 index rows of resources that do not exist,
/// and copies of 20 matching rows for another tenant and another type.
async fn seed_corpus(fixture: &Fixture) {
    let mut resources = Vec::new();
    let mut index = Vec::new();
    for n in 0..KINDS {
        let id = id_of(n);
        // Groups of three share a timestamp, so every order has ties.
        resources.push(resource_doc(
            PLAN_TENANT,
            "Observation",
            &id,
            (n / 3) as i64,
            false,
        ));
        index.push(status_row(
            PLAN_TENANT,
            "Observation",
            &id,
            if is_final(n) { "final" } else { "preliminary" },
        ));
        index.extend(panel_rows(PLAN_TENANT, "Observation", &id, &panel_of(n)));
        if n.is_multiple_of(2) {
            index.push(composite_token(
                PLAN_TENANT,
                "Observation",
                &id,
                "component-code-value-quantity",
                0,
                "8480-6",
            ));
            index.push(composite_quantity(
                PLAN_TENANT,
                "Observation",
                &id,
                "component-code-value-quantity",
                0,
                130.0,
            ));
        }
        if n < 20 {
            for (tenant, kind) in [("foreign", "Observation"), (PLAN_TENANT, "Patient")] {
                resources.push(resource_doc(tenant, kind, &id, (n / 3) as i64, false));
                index.extend(panel_rows(tenant, kind, &id, &[(0, "8302-2", 170.0)]));
            }
        }
    }
    for number in 0..10 {
        let id = format!("d{number:04}");
        resources.push(resource_doc(PLAN_TENANT, "Observation", &id, 0, true));
        index.extend(panel_rows(
            PLAN_TENANT,
            "Observation",
            &id,
            &[(0, "8302-2", 170.0)],
        ));
    }
    for number in 0..5 {
        let id = format!("s{number:04}");
        index.extend(panel_rows(
            PLAN_TENANT,
            "Observation",
            &id,
            &[(0, "8302-2", 170.0)],
        ));
    }
    for chunk in resources.chunks(1000) {
        fixture
            .db
            .collection::<Document>("resources")
            .insert_many(chunk.to_vec())
            .await
            .unwrap();
    }
    for chunk in index.chunks(1000) {
        fixture
            .db
            .collection::<Document>("search_index")
            .insert_many(chunk.to_vec())
            .await
            .unwrap();
    }
}

fn composite(
    name: &str,
    token_name: &str,
    quantity_name: &str,
    values: &[&str],
) -> SearchParameter {
    SearchParameter {
        name: name.into(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: values.iter().map(|value| SearchValue::eq(*value)).collect(),
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: token_name.into(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: quantity_name.into(),
            },
        ],
    }
}

fn code_value_quantity(values: &[&str]) -> SearchParameter {
    composite("code-value-quantity", "code", "value-quantity", values)
}

fn component_code_value_quantity(values: &[&str]) -> SearchParameter {
    composite(
        "component-code-value-quantity",
        "component-code",
        "component-value-quantity",
        values,
    )
}

fn cvq_query(values: &[&str]) -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(code_value_quantity(values))
}

fn status_final() -> SearchParameter {
    parameter("status", "final")
}

/// `_lastUpdated` at or after the 200th second of the corpus: from `c00600` on.
fn last_updated_from_middle() -> SearchParameter {
    SearchParameter {
        name: "_lastUpdated".into(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Ge, "2024-01-01T00:03:20Z")],
        chain: vec![],
        components: vec![],
    }
}

fn with_sort(mut query: SearchQuery, sort: &[&str]) -> SearchQuery {
    query.sort = sort.iter().map(|sort| SortDirective::parse(sort)).collect();
    query
}

fn with_total(mut query: SearchQuery, total: TotalMode) -> SearchQuery {
    query.total = Some(total);
    query
}

fn kind_of(plan: &Option<SearchFilterPlan>) -> &'static str {
    match plan {
        None => "none",
        Some(SearchFilterPlan::CandidateIds(_)) => "candidate_ids",
        Some(SearchFilterPlan::BatchedIndex(_)) => "batched_index",
        Some(SearchFilterPlan::ResourceBatches(_)) => "resource_batches",
        Some(SearchFilterPlan::IndexIntersection(_)) => "index_intersection",
        Some(SearchFilterPlan::ResourceLookups { .. }) => "resource_lookups",
    }
}

async fn plan_for(
    fixture: &Fixture,
    query: &SearchQuery,
    purpose: SearchFilterPurpose,
) -> StorageResult<Option<SearchFilterPlan>> {
    fixture
        .backend
        .resource_filter_plan(
            &fixture.db,
            PLAN_TENANT,
            query,
            &fixture
                .backend
                .classify_missing(PLAN_TENANT, &query.resource_type, query),
            purpose,
            NoTotalPagePolicy::Streaming,
            None,
            &mut ProbeFacts::default(),
        )
        .await
}

fn scope_filter(fixture: &Fixture, query: &SearchQuery) -> Document {
    fixture
        .backend
        .build_resource_filter(
            PLAN_TENANT,
            "Observation",
            query,
            &MissingSupport::none(),
            None,
            None,
        )
        .unwrap()
}

/// Every id the legacy collector matches, in the order the legacy path pages
/// them: the collected set becomes `id: {$in}` on `resources`, sorted by the
/// query's sort document. Nothing here consults the planner.
async fn legacy_oracle(fixture: &Fixture, query: &SearchQuery) -> Vec<String> {
    let support = MissingSupport::none();
    let set = fixture
        .backend
        .matching_resource_ids(&fixture.db, PLAN_TENANT, "Observation", query, &support)
        .await
        .unwrap()
        .expect("a query with parameters collects ids");
    let filter = fixture
        .backend
        .build_resource_filter(
            PLAN_TENANT,
            "Observation",
            query,
            &support,
            Some(&set),
            None,
        )
        .unwrap();
    let sort = fixture.backend.build_sort_document(query, false).unwrap();
    collect_documents(
        fixture
            .db
            .collection::<Document>("resources")
            .find(filter)
            .projection(doc! { "_id": 0, "id": 1 })
            .sort(sort)
            .await
            .unwrap(),
    )
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.get_str("id").unwrap().to_owned())
    .collect()
}

struct Fetched {
    ids: Vec<String>,
    has_next: bool,
    has_previous: bool,
    next: Option<String>,
    previous: Option<String>,
    total: Option<u64>,
}

async fn page(fixture: &Fixture, query: &SearchQuery) -> Fetched {
    let result = fixture
        .backend
        .search(&fixture.tenant, query)
        .await
        .unwrap_or_else(|error| panic!("search failed: {error}"));
    Fetched {
        ids: result
            .resources
            .items
            .iter()
            .map(|resource| resource.id().to_owned())
            .collect(),
        has_next: result.resources.page_info.has_next,
        has_previous: result.resources.page_info.has_previous,
        next: result.resources.page_info.next_cursor.clone(),
        previous: result.resources.page_info.previous_cursor.clone(),
        total: result.total,
    }
}

/// Offset pages from the front, a deep one and the last, and (for a sort with a
/// keyset) cursor pages forward and back, all against the oracle order.
async fn assert_pages_match(
    fixture: &Fixture,
    query: &SearchQuery,
    oracle: &[String],
    count: usize,
    context: &str,
) {
    let length = oracle.len();
    let last_offset = (length - 1) / count * count;
    let mut offsets: Vec<usize> = (0..4).map(|page| page * count).collect();
    offsets.extend([length / 2 / count * count, last_offset]);
    offsets.retain(|offset| *offset < length);
    offsets.sort_unstable();
    offsets.dedup();
    for offset in offsets {
        let mut paged = with_total(query.clone(), TotalMode::None);
        paged.count = Some(count as u32);
        paged.offset = Some(offset as u32);
        let got = page(fixture, &paged).await;
        let end = (offset + count).min(length);
        assert_eq!(
            got.ids,
            oracle[offset..end].to_vec(),
            "{context}: count={count} offset={offset}"
        );
        assert_eq!(
            got.has_next,
            end < length,
            "{context}: has_next count={count} offset={offset}"
        );
        assert_eq!(
            got.has_previous,
            offset > 0,
            "{context}: has_previous count={count} offset={offset}"
        );
        assert_eq!(got.total, None, "{context}: no total was requested");
    }
    if CursorKeyset::for_query(query).is_none() {
        return;
    }
    let mut paged = with_total(query.clone(), TotalMode::None);
    paged.count = Some(count as u32);
    let mut forward = Vec::new();
    let mut current = page(fixture, &paged).await;
    let mut previous_cursors = Vec::new();
    for position in 0..4 {
        let start = position * count;
        if start >= length {
            break;
        }
        let end = (start + count).min(length);
        assert_eq!(
            current.ids,
            oracle[start..end].to_vec(),
            "{context}: cursor page {position}"
        );
        assert_eq!(
            current.has_next,
            end < length,
            "{context}: cursor {position}"
        );
        assert_eq!(
            current.has_previous,
            position > 0,
            "{context}: cursor {position}"
        );
        forward.push(current.ids.clone());
        previous_cursors.push(current.previous.clone());
        let Some(next) = current.next.clone() else {
            break;
        };
        current = page(fixture, &paged.clone().with_cursor(next)).await;
    }
    // Walk back from the last page read: each hop returns the page before it.
    for position in (1..forward.len()).rev() {
        let cursor = previous_cursors[position]
            .clone()
            .unwrap_or_else(|| panic!("{context}: page {position} needs a previous cursor"));
        let back = page(fixture, &paged.clone().with_cursor(cursor)).await;
        assert_eq!(
            back.ids,
            forward[position - 1],
            "{context}: back to {}",
            position - 1
        );
        assert!(back.has_next, "{context}: a backward hop has a next page");
        assert_eq!(
            back.has_previous,
            position > 1,
            "{context}: back to {}",
            position - 1
        );
    }
}

/// Totals: a combined or separate accurate total and the standalone count.
async fn assert_totals_match(
    fixture: &Fixture,
    query: &SearchQuery,
    oracle: &[String],
    context: &str,
) {
    for sort in [&[][..], &["_id"][..], &["-_id"][..]] {
        let mut counted = with_total(with_sort(query.clone(), sort), TotalMode::Accurate);
        counted.count = Some(7);
        let got = page(fixture, &counted).await;
        assert_eq!(
            got.total,
            Some(oracle.len() as u64),
            "{context}: _total=accurate sort={sort:?}"
        );
    }
    assert_eq!(
        fixture
            .backend
            .search_count(&fixture.tenant, query)
            .await
            .unwrap(),
        oracle.len() as u64,
        "{context}: search_count"
    );
}

/// Pins every probe of the query's predicates at the cap, so no predicate is
/// selective and the plans that stream by resource order or by id are chosen.
async fn force_broad_probes(fixture: &Fixture, query: &SearchQuery) {
    let broad = ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1);
    for parameter in query
        .parameters
        .iter()
        .filter(|parameter| !matches!(parameter.name.as_str(), "_id" | "_lastUpdated"))
    {
        let key = if parameter.param_type == SearchParamType::Composite {
            let (arms, _) = fixture
                .backend
                .composite_arms(&fixture.db, PLAN_TENANT, "Observation", parameter, true)
                .unwrap();
            for arm in arms.iter().flatten() {
                fixture
                    .backend
                    .probe_cache
                    .insert(arm.probe_key.clone().unwrap(), broad);
            }
            fixture
                .backend
                .composite_predicate(
                    &fixture.db,
                    PLAN_TENANT,
                    "Observation",
                    parameter,
                    false,
                    &mut ProbeFacts::default(),
                )
                .await
                .unwrap()
                .probe_key
        } else {
            fixture
                .backend
                .planning_predicate(
                    &fixture.db,
                    PLAN_TENANT,
                    "Observation",
                    parameter,
                    fixture
                        .backend
                        .build_search_index_filter(PLAN_TENANT, "Observation", parameter)
                        .unwrap(),
                    true,
                )
                .probe_key
        };
        fixture
            .backend
            .probe_cache
            .insert(key.expect("predicate has a probe key"), broad);
    }
}

#[tokio::test]
async fn composite_routes_use_bounded_plans_and_never_type_scans() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-routes", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    let query = cvq_query(&["8302-2$gt160"]);
    let matching = ids_where(|n| n % 6 < 3);
    assert_eq!(matching.len(), 600);

    // A count and an id-ordered page stream the discovery arm in batches.
    let count = plan_for(&fixture, &query, SearchFilterPurpose::Count)
        .await
        .unwrap();
    assert_eq!(kind_of(&count), "batched_index");
    let (selected, ids) = fixture
        .backend
        .plan_or_matching_ids(
            &fixture.db,
            PLAN_TENANT,
            &query,
            &MissingSupport::none(),
            count,
        )
        .await
        .unwrap();
    assert_eq!(kind_of(&selected), "batched_index");
    assert!(ids.is_none(), "a bounded plan never collects the id set");
    for sort in ["_id", "-_id"] {
        let plan = plan_for(
            &fixture,
            &with_sort(query.clone(), &[sort]),
            SearchFilterPurpose::Page,
        )
        .await
        .unwrap();
        assert_eq!(kind_of(&plan), "batched_index", "sort {sort}");
    }

    // A selective default page reads its candidates once and pair-checks them.
    let plan = plan_for(&fixture, &query, SearchFilterPurpose::Page)
        .await
        .unwrap();
    let Some(SearchFilterPlan::CandidateIds(ids)) = &plan else {
        panic!("a selective composite page must reach CandidateIds");
    };
    // Candidates keep deleted and stale ids (liveness is checked when the page
    // is fetched) but none of the 400 cross-group and failed-pair decoys.
    let mut expected: Vec<String> = matching.clone();
    expected.extend((0..10).map(|number| format!("d{number:04}")));
    expected.extend((0..5).map(|number| format!("s{number:04}")));
    expected.sort();
    assert_eq!(ids, &expected);
    let (selected, collected) = fixture
        .backend
        .plan_or_matching_ids(
            &fixture.db,
            PLAN_TENANT,
            &query,
            &MissingSupport::none(),
            plan,
        )
        .await
        .unwrap();
    assert_eq!(kind_of(&selected), "candidate_ids");
    assert!(collected.is_none());

    // With every arm broad, a default page streams by resource order.
    force_broad_probes(&fixture, &query).await;
    let plan = plan_for(&fixture, &query, SearchFilterPurpose::Page)
        .await
        .unwrap();
    assert_eq!(kind_of(&plan), "resource_batches");
    let (selected, collected) = fixture
        .backend
        .plan_or_matching_ids(
            &fixture.db,
            PLAN_TENANT,
            &query,
            &MissingSupport::none(),
            plan,
        )
        .await
        .unwrap();
    assert_eq!(kind_of(&selected), "resource_batches");
    assert!(collected.is_none());
    // The type-scan plans cannot pair components, so the collector serves
    // a sort that no resource-order plan supports.
    let plan = plan_for(
        &fixture,
        &with_sort(query.clone(), &["_id", "_lastUpdated"]),
        SearchFilterPurpose::Page,
    )
    .await
    .unwrap();
    assert_eq!(kind_of(&plan), "none");
    fixture.backend.probe_cache.clear();

    // Shapes that keep the candidate collector.
    let mut excluded = status_final();
    excluded.modifier = Some(SearchModifier::Not);
    let mut sort_parameter = SortDirective::parse("date");
    sort_parameter.param_type = Some(SearchParamType::Date);
    let mut by_parameter = query.clone();
    by_parameter.sort = vec![sort_parameter];
    let seeded = query.clone().with_parameter(SearchParameter {
        name: "_id".into(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("c00000")],
        chain: vec![],
        components: vec![],
    });
    for (label, query) in [
        (
            "composite with :not",
            query.clone().with_parameter(excluded),
        ),
        ("composite sorted by a parameter", by_parameter),
        ("composite with a positive _id", seeded),
    ] {
        for purpose in [SearchFilterPurpose::Page, SearchFilterPurpose::Count] {
            let plan = plan_for(&fixture, &query, purpose).await.unwrap();
            assert_eq!(kind_of(&plan), "none", "{label} {purpose:?}");
        }
    }
    fixture.close().await;
}

/// The discovery arm proves one component. Every executor must still check
/// all of them, or a decoy that shares the arm's code is returned.
#[tokio::test]
async fn composite_driver_is_rechecked_by_every_executor() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-recheck", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    let query = cvq_query(&["8302-2$gt160"]);
    let matching = ids_where(|n| n % 6 < 3);
    let decoys: HashSet<String> = ids_where(|n| n % 6 == 3 || n % 6 == 4)
        .into_iter()
        .collect();
    assert_eq!(decoys.len(), 400);
    let scope = scope_filter(&fixture, &query);
    let not_decoys = |ids: &[String], what: &str| {
        assert!(
            ids.iter().all(|id| !decoys.contains(id)),
            "{what} returned a decoy"
        );
    };

    // CandidateIds: the list handed to the page fetch.
    let Some(SearchFilterPlan::CandidateIds(ids)) =
        plan_for(&fixture, &query, SearchFilterPurpose::Page)
            .await
            .unwrap()
    else {
        panic!("expected CandidateIds");
    };
    not_decoys(&ids, "CandidateIds");
    assert!(matching.iter().all(|id| ids.contains(id)));

    // BatchedIndex: an id page, a combined page and total, a count.
    let Some(SearchFilterPlan::BatchedIndex(plan)) =
        plan_for(&fixture, &query, SearchFilterPurpose::Count)
            .await
            .unwrap()
    else {
        panic!("expected BatchedIndex");
    };
    let all = plan
        .execute(
            &fixture.db,
            scope.clone(),
            None,
            BatchSelection::Page {
                direction: 1,
                offset: 0,
                limit: 2000,
            },
        )
        .await
        .unwrap();
    assert_eq!(all.ids, matching, "an id page lists exactly the matches");
    let combined = plan
        .execute(
            &fixture.db,
            scope.clone(),
            None,
            BatchSelection::PageAndCount {
                direction: -1,
                offset: 0,
                limit: 10,
                boundary: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(combined.count, 600, "a combined total counts no decoy");
    let mut newest = matching.clone();
    newest.reverse();
    assert_eq!(combined.ids, newest[..10].to_vec());
    let counted = plan
        .execute(&fixture.db, scope.clone(), None, BatchSelection::Count)
        .await
        .unwrap();
    assert_eq!(counted.count, 600, "an exact count has no decoy");

    // ResourceBatches: pages in resource order with every arm broad.
    force_broad_probes(&fixture, &query).await;
    let Some(SearchFilterPlan::ResourceBatches(plan)) =
        plan_for(&fixture, &query, SearchFilterPurpose::Page)
            .await
            .unwrap()
    else {
        panic!("expected ResourceBatches");
    };
    let sort = fixture.backend.build_sort_document(&query, false).unwrap();
    let ordered = plan
        .execute_page(&fixture.db, scope, None, sort, 0, 2000)
        .await
        .unwrap();
    not_decoys(&ordered, "ResourceBatches");
    assert_eq!(ordered, legacy_oracle(&fixture, &query).await);
    assert_eq!(ordered.len(), 600);
    fixture.close().await;
}

#[tokio::test]
async fn composite_plans_match_the_legacy_collector() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-oracle", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    let shapes: Vec<(&str, SearchQuery, Vec<String>)> = vec![
        (
            "one value",
            cvq_query(&["8302-2$gt160"]),
            ids_where(|n| n % 6 < 3),
        ),
        (
            "system qualified",
            cvq_query(&["http://loinc.org|8302-2$gt160"]),
            ids_where(|n| n % 6 < 3),
        ),
        (
            "two values with different arms",
            cvq_query(&["8302-2$gt160", "29463-7$lt100"]),
            ids_where(|n| n % 6 < 3 || (n % 6 == 5 && (n / 6).is_multiple_of(2))),
        ),
        (
            "an ne component",
            cvq_query(&["8302-2$ne150"]),
            ids_where(|n| n % 6 < 4),
        ),
        (
            "with status",
            cvq_query(&["8302-2$gt160"]).with_parameter(status_final()),
            ids_where(|n| n % 6 < 3 && is_final(n)),
        ),
        (
            "with _lastUpdated",
            cvq_query(&["8302-2$gt160"]).with_parameter(last_updated_from_middle()),
            ids_where(|n| n % 6 < 3 && n >= 600),
        ),
        (
            "two composites",
            cvq_query(&["8302-2$gt160"])
                .with_parameter(component_code_value_quantity(&["8480-6$gt100"])),
            ids_where(|n| n % 6 < 3 && n.is_multiple_of(2)),
        ),
    ];
    let sorts: [&[&str]; 6] = [
        &[],
        &["_lastUpdated"],
        &["-_lastUpdated"],
        &["_lastUpdated", "_id"],
        &["_id"],
        &["-_id"],
    ];
    for (label, query, expected) in &shapes {
        let mut want = expected.clone();
        want.sort();
        for sort in sorts {
            let sorted = with_sort(query.clone(), sort);
            let oracle = legacy_oracle(&fixture, &sorted).await;
            let mut found = oracle.clone();
            found.sort();
            assert_eq!(
                found, want,
                "{label}: the oracle must agree with the fixture"
            );
            assert!(!oracle.is_empty(), "{label}");
            let context = format!("{label} sort={sort:?}");
            let counts: &[usize] = if *label == "one value" { &[1, 7] } else { &[7] };
            for count in counts {
                assert_pages_match(&fixture, &sorted, &oracle, *count, &context).await;
            }
            assert_totals_match(&fixture, &sorted, &oracle, &context).await;
        }
        // The same shapes when no predicate is selective: resource-order pages.
        force_broad_probes(&fixture, query).await;
        for sort in [
            &[][..],
            &["_lastUpdated"][..],
            &["-_lastUpdated"][..],
            &["_lastUpdated", "_id"][..],
        ] {
            let sorted = with_sort(query.clone(), sort);
            let oracle = legacy_oracle(&fixture, &sorted).await;
            let context = format!("{label} forced sort={sort:?}");
            assert_pages_match(&fixture, &sorted, &oracle, 7, &context).await;
        }
        fixture.backend.probe_cache.clear();
    }

    // A deep offset without a total takes the adaptive route.
    let query = with_sort(
        cvq_query(&["8302-2$gt160"]).with_parameter(status_final()),
        &["_id"],
    );
    let oracle = legacy_oracle(&fixture, &query).await;
    let mut deep = with_total(query.clone(), TotalMode::None);
    deep.count = Some(20);
    deep.offset = Some(300);
    let got = page(&fixture, &deep).await;
    assert_eq!(got.ids, oracle[300..320].to_vec());
    assert!(got.has_next);
    fixture.close().await;
}

#[tokio::test]
async fn composite_planning_preserves_legacy_errors() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-errors", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;

    // Every component with `ne`: the message of the collector, before any read.
    let all_ne = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code-value-quantity".into(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: vec![SearchValue::eq("ne1$ne2")],
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".into(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity-2".into(),
            },
        ],
    });
    let legacy = fixture
        .backend
        .matching_resource_ids(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            &all_ne,
            &MissingSupport::none(),
        )
        .await
        .unwrap_err();
    let StorageError::Search(SearchError::InvalidComposite { message }) = &legacy else {
        panic!("expected InvalidComposite, got {legacy:?}");
    };
    assert!(message.contains("every component using the 'ne' prefix"));
    for purpose in [SearchFilterPurpose::Page, SearchFilterPurpose::Count] {
        let error = plan_for(&fixture, &all_ne, purpose).await.err().unwrap();
        assert_eq!(error.to_string(), legacy.to_string(), "{purpose:?}");
    }
    let page_error = fixture
        .backend
        .search(&fixture.tenant, &all_ne)
        .await
        .unwrap_err();
    assert_eq!(page_error.to_string(), legacy.to_string());
    let count_error = fixture
        .backend
        .search_count(&fixture.tenant, &all_ne)
        .await
        .unwrap_err();
    assert_eq!(count_error.to_string(), legacy.to_string());

    // Repeated-type rows written before slots existed fail closed.
    let rows: Vec<_> = ["A", "B"]
        .into_iter()
        .map(|code| {
            doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "resource_id": "legacy", "param_name": "code-value-concept",
            "value_token_code": code, "composite_group": 0_i32 }
        })
        .collect();
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_many(rows)
        .await
        .unwrap();
    let legacy_rows = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code-value-concept".into(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: vec![SearchValue::eq("A$B")],
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "code".into(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "value-concept".into(),
            },
        ],
    });
    for purpose in [SearchFilterPurpose::Page, SearchFilterPurpose::Count] {
        let error = plan_for(&fixture, &legacy_rows, purpose)
            .await
            .err()
            .expect("the planner must preflight legacy rows");
        assert!(error.to_string().contains("$reindex"), "{error}");
    }
    let error = fixture
        .backend
        .search(&fixture.tenant, &legacy_rows)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("$reindex"), "{error}");
    let error = fixture
        .backend
        .search_count(&fixture.tenant, &legacy_rows)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("$reindex"), "{error}");
    fixture.close().await;
}

/// A composite's discovery filter finds more rows than it matches, so an
/// estimate over it would count the decoys. Planning never asks for one; an
/// executor handed one anyway counts exactly.
/// Both arms over `MAX_CANDIDATE_ROWS`: the code arm has 30,000 rows and the
/// value arm 12,000. The widened probe must still pick the value arm, as the
/// collector's 100,000-row probe does, rather than fall back to the first
/// component. Exact totals and `_id` pages read every row of the arm they
/// discover from, so the code arm cost them 2.5 times the rows here.
#[tokio::test]
async fn composite_arm_choice_survives_saturated_probes() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-arm-choice", 0, 5_000).await else {
        return;
    };
    const ROWS: usize = 30_000;
    const MATCHING: usize = 12_000;
    let rows: Vec<_> = (0..ROWS)
        .flat_map(|n| {
            let value = if n < MATCHING { 170.0 } else { 100.0 };
            panel_rows(
                PLAN_TENANT,
                "Observation",
                &format!("arm{n:05}"),
                &[(0, "8302-2", value)],
            )
        })
        .collect();
    for chunk in rows.chunks(5000) {
        fixture
            .db
            .collection::<Document>("search_index")
            .insert_many(chunk.to_vec())
            .await
            .unwrap();
    }
    let parameter = code_value_quantity(&["8302-2$gt160"]);
    let (arms, _) = fixture
        .backend
        .composite_arms(&fixture.db, PLAN_TENANT, "Observation", &parameter, true)
        .unwrap();
    let [value_arms] = arms.as_slice() else {
        panic!("one comma value");
    };
    let [code_arm, quantity_arm] = value_arms.as_slice() else {
        panic!("two non-ne components");
    };
    let mut facts = ProbeFacts::default();
    let predicate = fixture
        .backend
        .composite_predicate(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            &parameter,
            true,
            &mut facts,
        )
        .await
        .unwrap();
    assert_eq!(
        predicate.filter, quantity_arm.filter,
        "the 12,000-row arm discovers"
    );
    assert_ne!(predicate.filter, code_arm.filter);
    assert!(predicate.pairs_components());

    // The planner widens for counts and `_id` pages, which stream the arm.
    let query = SearchQuery::new("Observation").with_parameter(parameter.clone());
    for (purpose, query) in [
        (SearchFilterPurpose::Count, query.clone()),
        (SearchFilterPurpose::Page, with_sort(query, &["_id"])),
    ] {
        let Some(SearchFilterPlan::BatchedIndex(plan)) =
            plan_for(&fixture, &query, purpose).await.unwrap()
        else {
            panic!("expected BatchedIndex for {purpose:?}");
        };
        assert_eq!(plan.driver.filter, quantity_arm.filter, "{purpose:?}");
    }
}

#[tokio::test]
async fn composite_estimates_are_counted_exactly() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-estimates", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    let query = cvq_query(&["8302-2$gt160"]);
    assert!(
        !fixture
            .backend
            .can_estimate_index_total(&with_total(query.clone(), TotalMode::Estimate)),
        "a composite is never eligible for an index estimate"
    );
    let Some(SearchFilterPlan::BatchedIndex(plan)) =
        plan_for(&fixture, &query, SearchFilterPurpose::Count)
            .await
            .unwrap()
    else {
        panic!("expected BatchedIndex");
    };
    let scope = scope_filter(&fixture, &query);
    let estimated = plan
        .execute(
            &fixture.db,
            scope.clone(),
            None,
            BatchSelection::EstimatedCount,
        )
        .await
        .unwrap();
    assert_eq!(estimated.count, 600);
    let estimated = plan
        .execute(
            &fixture.db,
            scope,
            None,
            BatchSelection::EstimatedPageAndCount {
                direction: 1,
                offset: 0,
                limit: 5,
                boundary: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(estimated.count, 600);
    assert_eq!(estimated.ids, ids_where(|n| n % 6 < 3)[..5].to_vec());

    let estimate = with_total(query, TotalMode::Estimate);
    let got = page(&fixture, &estimate).await;
    assert_eq!(got.total, Some(600));
    fixture.close().await;
}

fn in_list_length(value: &Bson) -> Option<usize> {
    match value {
        Bson::Document(document) => document
            .get_document("resource_id")
            .ok()
            .and_then(|ids| ids.get_array("$in").ok())
            .map(Vec::len)
            .or_else(|| document.values().find_map(in_list_length)),
        Bson::Array(values) => values.iter().find_map(in_list_length),
        _ => None,
    }
}

/// Every profiled aggregate on `search_index`; the profile cannot be queried by
/// the `$`-named stage keys, so callers filter in Rust.
async fn profiled_aggregates(db: &mongodb::Database) -> Vec<Document> {
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "command.aggregate": "search_index" })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
}

fn mentions_param(value: &Bson, name: &str) -> bool {
    match value {
        Bson::Document(document) => {
            document.get_str("param_name") == Ok(name)
                || document.values().any(|value| mentions_param(value, name))
        }
        Bson::Array(values) => values.iter().any(|value| mentions_param(value, name)),
        _ => false,
    }
}

fn pipeline_of(operation: &Document) -> Bson {
    Bson::Array(
        operation
            .get_document("command")
            .unwrap()
            .get_array("pipeline")
            .unwrap()
            .clone(),
    )
}

fn limits_rows_to(operation: &Document, rows: i64) -> bool {
    operation
        .get_document("command")
        .unwrap()
        .get_array("pipeline")
        .unwrap()
        .iter()
        .filter_map(Bson::as_document)
        .any(|stage| {
            stage
                .get_i64("$limit")
                .or_else(|_| stage.get_i32("$limit").map(i64::from))
                == Ok(rows)
        })
}

async fn composite_pair_finds(db: &mongodb::Database) -> Vec<Document> {
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! {
                "command.find": "search_index",
                "command.projection.composite_group": { "$exists": true },
            })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn composite_membership_is_hinted_and_bounded() {
    let Some(fixture) = Fixture::with_probe_timeout("composite-hint", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    let query = cvq_query(&["8302-2$gt160"]);
    let wide = with_total(with_sort(query.clone(), &["_id"]), TotalMode::Accurate);
    for (label, query) in [
        ("default page", query.clone()),
        ("id page with total", wide),
    ] {
        restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
        let got = page(&fixture, &query).await;
        fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
        assert!(!got.ids.is_empty(), "{label}");
        let finds = composite_pair_finds(&fixture.db).await;
        assert!(
            finds.len() >= 2,
            "{label}: one find per component, per batch: {}",
            finds.len()
        );
        for find in &finds {
            let command = find.get_document("command").unwrap();
            assert_eq!(
                command.get_str("hint"),
                Ok(SEARCH_COMPOSITE_INDEX),
                "{label}"
            );
            let summary = find.get_str("planSummary").unwrap_or_default();
            assert!(
                summary.contains("resource_id: 1, param_name: 1, composite_group: 1"),
                "{label}: {summary}"
            );
            let bound = in_list_length(&Bson::Document(command.clone()))
                .expect("every pair check is bounded by candidate ids");
            assert!(
                bound > 0 && bound <= MAX_CANDIDATE_ROWS as usize,
                "{label}: {bound}"
            );
        }
        let aggregates = profiled_aggregates(&fixture.db).await;
        assert!(
            !aggregates.is_empty(),
            "{label}: the probes must be profiled"
        );
        assert!(
            aggregates
                .iter()
                .all(|operation| !limits_rows_to(operation, 100_000)),
            "{label}: the planner probes arms with the 10,001-row cap, not the collector's 100,000"
        );
    }

    // The index is required: a rejected hint fails the search, once, unhinted
    // never.
    fixture
        .db
        .collection::<Document>("search_index")
        .drop_index(SEARCH_COMPOSITE_INDEX)
        .await
        .unwrap();
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let error = fixture
        .backend
        .search(&fixture.tenant, &query)
        .await
        .unwrap_err();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert!(
        error
            .to_string()
            .contains("hint provided does not correspond to an existing index"),
        "{error}"
    );
    let finds = composite_pair_finds(&fixture.db).await;
    assert_eq!(finds.len(), 1, "a rejected hint is not retried");
    assert!(
        finds
            .iter()
            .all(|find| find.get_document("command").unwrap().get_str("hint")
                == Ok(SEARCH_COMPOSITE_INDEX)),
        "no pair check may run unhinted"
    );
    fixture.close().await;
}

fn identifier_parameter(value: &str) -> SearchParameter {
    SearchParameter {
        name: "subject".into(),
        param_type: SearchParamType::Reference,
        modifier: Some(SearchModifier::Identifier),
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn compartment(reference: &str) -> CompartmentMembership {
    CompartmentMembership {
        params: vec!["subject".into(), "performer".into()],
        reference: reference.into(),
    }
}

/// Patients with identifiers and the Observations that reference them: `c`
/// ids with `n % 40 == 0` reference `Patient/p1` (30 of them), `n % 40 == 1`
/// reference `Patient/p2`, and every Observation with `n % 40 == 5` performs
/// for `Patient/p1/_history/3`. The 15 Observations with `n % 80 == 0` also
/// name `Patient/p1` as performer, so they reference it through two
/// parameters: the compartment holds two index rows for each of them, and
/// a page or a count must still list each one once.
async fn seed_references(fixture: &Fixture) {
    let mut index = Vec::new();
    for (patient, value) in [("p1", "A1"), ("p2", "B2")] {
        index.push(doc! { "tenant_id": PLAN_TENANT, "resource_type": "Patient",
        "resource_id": patient, "param_name": "identifier",
        "value_token_system": "urn:mrn", "value_token_code": value });
    }
    for n in 0..KINDS {
        let id = id_of(n);
        let reference = match n % 40 {
            0 => Some("Patient/p1"),
            1 => Some("Patient/p2"),
            _ => None,
        };
        if let Some(reference) = reference {
            index.push(
                doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
                "resource_id": &id, "param_name": "subject", "value_reference": reference },
            );
        }
        if n % 40 == 5 {
            index.push(
                doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
                "resource_id": &id, "param_name": "performer",
                "value_reference": "Patient/p1/_history/3" },
            );
        }
        if n % 80 == 0 {
            index.push(
                doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
                "resource_id": &id, "param_name": "performer",
                "value_reference": "Patient/p1" },
            );
        }
    }
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_many(index)
        .await
        .unwrap();
}

/// Pages of `_sort=_id`, `-_id` and `-_lastUpdated` against the legacy oracle,
/// whose members the fixture names independently, and a `_total=estimate`
/// search, which these shapes never answer from the index estimate.
async fn assert_sorts_and_estimates(
    fixture: &Fixture,
    query: &SearchQuery,
    expected: &[String],
    count: usize,
    label: &str,
) {
    let mut want = expected.to_vec();
    want.sort();
    for sort in [&["_id"][..], &["-_id"][..], &["-_lastUpdated"][..]] {
        let sorted = with_sort(query.clone(), sort);
        let oracle = legacy_oracle(fixture, &sorted).await;
        assert!(!oracle.is_empty(), "{label} sort={sort:?}");
        let mut found = oracle.clone();
        found.sort();
        assert_eq!(
            found, want,
            "{label} sort={sort:?}: the oracle must agree with the fixture"
        );
        let context = format!("{label} sort={sort:?}");
        assert_pages_match(fixture, &sorted, &oracle, count, &context).await;
    }
    let estimate = with_total(query.clone(), TotalMode::Estimate);
    assert!(
        !fixture.backend.can_estimate_index_total(&estimate),
        "{label}: never eligible for an index estimate"
    );
    assert_eq!(
        page(fixture, &estimate).await.total,
        Some(expected.len() as u64),
        "{label}: _total=estimate stays exact"
    );
}

#[tokio::test]
async fn identifier_and_compartment_predicates_plan_like_plain_filters() {
    let Some(fixture) = Fixture::with_probe_timeout("identifier-compartment", 0, 5_000).await
    else {
        return;
    };
    seed_corpus(&fixture).await;
    seed_references(&fixture).await;
    let p1 = ids_where(|n| n.is_multiple_of(40) || n % 40 == 5);

    // Positive `:identifier`: resolved by the planner, then a plain filter.
    let known = SearchQuery::new("Observation").with_parameter(identifier_parameter("urn:mrn|A1"));
    let plan = plan_for(&fixture, &known, SearchFilterPurpose::Count)
        .await
        .unwrap();
    assert_eq!(kind_of(&plan), "batched_index");
    let plan = plan_for(&fixture, &known, SearchFilterPurpose::Page)
        .await
        .unwrap();
    let Some(SearchFilterPlan::CandidateIds(ids)) = &plan else {
        panic!(
            "a small identifier page must reach CandidateIds, got {}",
            kind_of(&plan)
        );
    };
    assert_eq!(ids, &ids_where(|n| n.is_multiple_of(40)));
    let (selected, collected) = fixture
        .backend
        .plan_or_matching_ids(
            &fixture.db,
            PLAN_TENANT,
            &known,
            &MissingSupport::none(),
            plan,
        )
        .await
        .unwrap();
    assert_eq!(kind_of(&selected), "candidate_ids");
    assert!(collected.is_none());
    let oracle = legacy_oracle(&fixture, &known).await;
    assert_eq!(oracle.len(), 30);
    assert_pages_match(&fixture, &known, &oracle, 7, "identifier").await;
    assert_totals_match(&fixture, &known, &oracle, "identifier").await;
    assert_sorts_and_estimates(
        &fixture,
        &known,
        &ids_where(|n| n.is_multiple_of(40)),
        7,
        "identifier",
    )
    .await;

    // No target has the identifier: an empty search, a zero count.
    let unknown =
        SearchQuery::new("Observation").with_parameter(identifier_parameter("urn:mrn|none"));
    for purpose in [SearchFilterPurpose::Page, SearchFilterPurpose::Count] {
        let plan = plan_for(&fixture, &unknown, purpose).await.unwrap();
        let Some(SearchFilterPlan::CandidateIds(ids)) = &plan else {
            panic!(
                "an unresolved identifier gives CandidateIds, got {}",
                kind_of(&plan)
            );
        };
        assert!(ids.is_empty());
    }
    let none = page(&fixture, &with_total(unknown.clone(), TotalMode::Accurate)).await;
    assert!(none.ids.is_empty());
    assert_eq!(none.total, Some(0));
    assert_eq!(
        fixture
            .backend
            .search_count(&fixture.tenant, &unknown)
            .await
            .unwrap(),
        0
    );

    // Identifier with a complement keeps the collector.
    let mut excluded = status_final();
    excluded.modifier = Some(SearchModifier::Not);
    let mixed = known.clone().with_parameter(excluded);
    assert_eq!(
        kind_of(
            &plan_for(&fixture, &mixed, SearchFilterPurpose::Count)
                .await
                .unwrap()
        ),
        "none"
    );

    // Compartment membership alone and with a broad sibling.
    let mut member = SearchQuery::new("Observation");
    member.compartment = Some(compartment("Patient/p1"));
    let mut broad = member.clone().with_parameter(status_final());
    broad.count = Some(5);
    for (label, query, expected) in [
        ("compartment", member.clone(), p1.clone()),
        (
            "compartment with a broad sibling",
            broad.clone(),
            ids_where(|n| (n.is_multiple_of(40) || n % 40 == 5) && is_final(n)),
        ),
    ] {
        let plan = plan_for(&fixture, &query, SearchFilterPurpose::Page)
            .await
            .unwrap();
        let Some(SearchFilterPlan::CandidateIds(ids)) = &plan else {
            panic!("{label}: expected CandidateIds, got {}", kind_of(&plan));
        };
        let mut candidates = ids.clone();
        candidates.sort();
        let mut members = expected.clone();
        members.sort();
        assert!(
            members.iter().all(|id| candidates.contains(id)),
            "{label}: every member is a candidate"
        );
        assert!(
            candidates.len() <= p1.len(),
            "{label}: the compartment drives, so no candidate is outside it"
        );
        let oracle = legacy_oracle(&fixture, &query).await;
        let mut found = oracle.clone();
        found.sort();
        assert_eq!(
            found, members,
            "{label}: the oracle agrees with the fixture"
        );
        assert_pages_match(&fixture, &query, &oracle, 5, label).await;
        assert_totals_match(&fixture, &query, &oracle, label).await;
        assert_sorts_and_estimates(&fixture, &query, &expected, 5, label).await;
        assert_eq!(
            kind_of(
                &plan_for(&fixture, &query, SearchFilterPurpose::Count)
                    .await
                    .unwrap()
            ),
            "batched_index",
            "{label}"
        );
    }

    // Fifteen members reference the compartment through two parameters, so it
    // holds more index rows than members. A page that holds the whole
    // compartment and both counts list and count each member once.
    let direct_rows = fixture
        .db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "param_name": { "$in": ["subject", "performer"] },
            "value_reference": "Patient/p1",
        })
        .await
        .unwrap();
    assert_eq!(direct_rows, 45, "30 subject rows and 15 performer rows");
    let mut whole = with_total(member.clone(), TotalMode::Accurate);
    whole.count = Some(200);
    let got = page(&fixture, &whole).await;
    let mut unique = got.ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        got.ids.len(),
        "a member that references the compartment twice is listed once"
    );
    assert_eq!(unique, p1);
    assert_eq!(got.total, Some(p1.len() as u64));
    assert_eq!(
        fixture
            .backend
            .search_count(&fixture.tenant, &member)
            .await
            .unwrap(),
        p1.len() as u64,
        "search_count lists a double reference once"
    );

    // The sibling is checked on the compartment's ids only.
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let got = page(&fixture, &broad).await;
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(got.ids.len(), 5);
    let status_checks: Vec<_> = profiled_aggregates(&fixture.db)
        .await
        .into_iter()
        .filter(|operation| {
            let pipeline = pipeline_of(operation);
            mentions_param(&pipeline, "status") && in_list_length(&pipeline).is_some()
        })
        .collect();
    assert!(
        !status_checks.is_empty(),
        "the status check must be profiled"
    );
    for check in &status_checks {
        let bound = in_list_length(&pipeline_of(check)).unwrap();
        assert!(bound <= p1.len(), "status was checked against {bound} ids");
    }

    // A compartment with a complement keeps the collector.
    let complement = member.clone().with_parameter({
        let mut excluded = status_final();
        excluded.modifier = Some(SearchModifier::Not);
        excluded
    });
    assert_eq!(
        kind_of(
            &plan_for(&fixture, &complement, SearchFilterPurpose::Page)
                .await
                .unwrap()
        ),
        "none"
    );
    fixture.close().await;
}

/// Whether `value` holds `value_reference: <reference>` anywhere.
fn mentions_reference(value: &Bson, reference: &str) -> bool {
    match value {
        Bson::Document(document) => {
            document.get_str("value_reference") == Ok(reference)
                || document
                    .values()
                    .any(|value| mentions_reference(value, reference))
        }
        Bson::Array(values) => values
            .iter()
            .any(|value| mentions_reference(value, reference)),
        _ => false,
    }
}

/// The profiled reads of `search_index` that name `reference` as a member:
/// `(count probes, other reads)`. A probe counts at most
/// `MAX_CANDIDATE_ROWS + 1` rows.
async fn compartment_reads(
    db: &mongodb::Database,
    reference: &str,
) -> (Vec<Document>, Vec<Document>) {
    let operations = collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "ns": format!("{}.search_index", db.name()) })
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    operations
        .into_iter()
        .filter(|operation| {
            mentions_reference(
                &Bson::Document(operation.get_document("command").unwrap().clone()),
                reference,
            )
        })
        .partition(|operation| {
            operation
                .get_document("command")
                .unwrap()
                .contains_key("aggregate")
                && limits_rows_to(operation, (MAX_CANDIDATE_ROWS + 1) as i64)
        })
}

/// The key fields of a profiled index scan, in order, from its `planSummary`
/// (`IXSCAN { tenant_id: 1, … }`); empty for any other plan.
fn scanned_fields(operation: &Document) -> Vec<String> {
    let summary = operation.get_str("planSummary").unwrap_or_default();
    let Some(keys) = summary
        .strip_prefix("IXSCAN { ")
        .and_then(|rest| rest.split(" }").next())
    else {
        return Vec::new();
    };
    keys.split(", ")
        .filter_map(|key| key.split(':').next())
        .map(str::to_owned)
        .collect()
}

/// The key fields of the index `name` on `search_index`, in order.
async fn index_fields(db: &mongodb::Database, name: &str) -> Vec<String> {
    let indexes: Vec<mongodb::IndexModel> = futures::TryStreamExt::try_collect(
        db.collection::<Document>("search_index")
            .list_indexes()
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    indexes
        .iter()
        .find(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some(name)
        })
        .map(|index| index.keys.keys().cloned().collect())
        .unwrap_or_else(|| panic!("index {name} exists"))
}

/// A compartment's count probe is cached under a key of its own, like a
/// parameter's: planning the same compartment again reuses the estimate,
/// while another compartment probes. The probe and the candidate read both
/// scan the reference value index.
#[tokio::test]
async fn compartment_probe_is_cached_and_reads_the_reference_index() {
    let Some(fixture) = Fixture::with_probe_timeout("compartment-probe", 0, 5_000).await else {
        return;
    };
    seed_corpus(&fixture).await;
    seed_references(&fixture).await;
    let p1 = ids_where(|n| n.is_multiple_of(40) || n % 40 == 5);
    let mut member = SearchQuery::new("Observation");
    member.compartment = Some(compartment("Patient/p1"));
    let reference_index = index_fields(&fixture.db, "idx_search_reference_v2").await;

    // First plan: one bounded probe, then one candidate read.
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let plan = plan_for(&fixture, &member, SearchFilterPurpose::Page)
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let Some(SearchFilterPlan::CandidateIds(ids)) = &plan else {
        panic!("a small compartment drives, got {}", kind_of(&plan));
    };
    let mut candidates = ids.clone();
    candidates.sort();
    assert_eq!(candidates, p1, "every member and nothing else");
    let (probes, seeds) = compartment_reads(&fixture.db, "Patient/p1").await;
    assert_eq!(probes.len(), 1, "one count probe");
    assert_eq!(seeds.len(), 1, "one candidate read");
    for (what, operation) in [("probe", &probes[0]), ("seed", &seeds[0])] {
        assert_eq!(
            scanned_fields(operation),
            reference_index,
            "the {what} scans idx_search_reference_v2: {operation}"
        );
    }

    // Planned again, by another request: the cached estimate serves it.
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let again = plan_for(&fixture, &member, SearchFilterPurpose::Page)
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(kind_of(&again), "candidate_ids");
    let (probes, seeds) = compartment_reads(&fixture.db, "Patient/p1").await;
    assert!(
        probes.is_empty(),
        "a cached compartment is not probed again"
    );
    assert_eq!(seeds.len(), 1);

    // The estimate is cached under the compartment's own key: 30 subject rows,
    // 15 performer rows and 30 versioned performer rows.
    let key = fixture
        .backend
        .compartment_predicate(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            member.compartment.as_ref().unwrap(),
            true,
        )
        .probe_key
        .expect("a compartment predicate has a probe key");
    assert_eq!(
        fixture.backend.probe_cache.get(&key),
        Some(ProbeEstimate::Completed(75))
    );

    // Another compartment has its own key and its own probe.
    let mut other = SearchQuery::new("Observation");
    other.compartment = Some(compartment("Patient/p2"));
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    plan_for(&fixture, &other, SearchFilterPurpose::Page)
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let (probes, _) = compartment_reads(&fixture.db, "Patient/p2").await;
    assert_eq!(probes.len(), 1, "another compartment is probed");
    fixture.close().await;
}
