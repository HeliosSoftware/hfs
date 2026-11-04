//! #2285: the `:not` / `:missing` complement and the unfiltered parameter
//! `_sort` are bounded by the id cap and the page, not by the size of the type.
//! The cap is lowered with `with_result_id_cap` so a few hundred rows cross it.

use super::super::super::{CANDIDATE_BATCH_SIZE, with_result_id_cap};
use super::*;
use crate::error::StorageError;

const CAP: usize = 50;

fn not_status(value: &str) -> SearchParameter {
    let mut param = parameter("status", value);
    param.modifier = Some(SearchModifier::Not);
    param
}

fn missing_method(value: &str) -> SearchParameter {
    let mut param = parameter("method", value);
    param.modifier = Some(SearchModifier::Missing);
    param
}

fn page_ids(result: &crate::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_owned())
        .collect()
}

fn too_many(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Search(crate::error::SearchError::TooManyResults { .. })
    )
}

async fn matching(
    fixture: &Fixture,
    query: &SearchQuery,
) -> StorageResult<Option<HashSet<String>>> {
    let support = fixture
        .backend
        .classify_missing(PLAN_TENANT, &query.resource_type, query);
    fixture
        .backend
        .matching_resource_ids(
            &fixture.db,
            PLAN_TENANT,
            &query.resource_type,
            query,
            &support,
        )
        .await
}

/// Documents returned by every command on `resources` and `search_index`,
/// as the profiler recorded them: what the server handed to the process.
async fn documents_returned(db: &mongodb::Database) -> i64 {
    let rows = collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "ns": { "$in": [
                format!("{}.resources", db.name()),
                format!("{}.search_index", db.name()),
            ] } })
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    rows.iter()
        .map(|row| {
            row.get_i64("nreturned")
                .or_else(|_| row.get_i32("nreturned").map(i64::from))
                .unwrap_or(0)
        })
        .sum()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[tokio::test]
async fn not_only_complement_stops_reading_at_the_cap() {
    let rows = CANDIDATE_BATCH_SIZE * 3;
    let Some(fixture) = Fixture::new("complement-cap-read", rows).await else {
        return;
    };
    // 3 in 4 rows are `preliminary`, so the complement of `final` is far over
    // the cap, and the whole type is 3 batches.
    let query = SearchQuery::new("Observation").with_parameter(not_status("final"));
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let outcome = with_result_id_cap(CAP, matching(&fixture, &query)).await;
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let error = outcome.expect_err("a complement over the cap must be refused");
    assert!(too_many(&error), "unexpected error: {error:?}");
    let returned = documents_returned(&fixture.db).await;
    assert!(
        returned <= 2 * CANDIDATE_BATCH_SIZE as i64,
        "read {returned} documents for a type of {rows}; the scan must stop after the batch that crosses the cap"
    );
    fixture.close().await;
}

#[tokio::test]
async fn not_only_complement_within_the_cap_excludes_deleted_ids() {
    let Some(mut fixture) = Fixture::new("complement-parity-not", 200).await else {
        return;
    };
    fixture.mark_deleted(1, 9).await;
    // `final` rows are every 4th; row 0 is deleted from the start.
    let query = SearchQuery::new("Observation").with_parameter(not_status("preliminary"));
    let ids = with_result_id_cap(CAP, matching(&fixture, &query))
        .await
        .unwrap()
        .unwrap();
    let expected: HashSet<String> = fixture
        .rows
        .iter()
        .filter(|row| !row.deleted && row.status == "final")
        .map(|row| row.id.clone())
        .collect();
    assert!(!expected.is_empty() && expected.len() <= CAP);
    assert_eq!(ids, expected);
    fixture.close().await;
}

#[tokio::test]
async fn missing_true_complement_matches_live_ids_and_honours_the_cap() {
    let Some(mut fixture) = Fixture::new("complement-parity-missing", 120).await else {
        return;
    };
    fixture.mark_deleted(1, 5).await;
    // No row carries `method`, so every live resource is missing it.
    let query = SearchQuery::new("Observation").with_parameter(missing_method("true"));
    let live: HashSet<String> = fixture
        .rows
        .iter()
        .filter(|row| !row.deleted)
        .map(|row| row.id.clone())
        .collect();
    let ids = with_result_id_cap(1_000, matching(&fixture, &query))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ids, live);
    let error = with_result_id_cap(CAP, matching(&fixture, &query))
        .await
        .expect_err("115 live ids are over the cap");
    assert!(too_many(&error), "unexpected error: {error:?}");
    fixture.close().await;
}

#[tokio::test]
async fn not_only_page_answers_or_refuses_without_changing_results() {
    let Some(fixture) = Fixture::new("complement-page", 300).await else {
        return;
    };
    let mut query = SearchQuery::new("Observation")
        .with_parameter(not_status("final"))
        .with_count(10);
    query.total = Some(TotalMode::None);
    // With a cap that covers the complement the page is the one from before.
    let wide = with_result_id_cap(1_000, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    // Every row shares one `last_updated`, so the default order is id descending.
    let mut expected: Vec<String> = sorted(
        fixture
            .rows
            .iter()
            .filter(|row| !row.deleted && row.status != "final")
            .map(|row| row.id.clone())
            .collect(),
    );
    expected.reverse();
    expected.truncate(10);
    assert_eq!(page_ids(&wide), expected);
    // Lowering the cap may move the page to another executor, never change it.
    match with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query)).await {
        Ok(narrow) => assert_eq!(page_ids(&narrow), expected),
        Err(error) => assert!(too_many(&error), "unexpected error: {error:?}"),
    }
    fixture.close().await;
}

fn status_sort(direction: &str) -> SortDirective {
    SortDirective::parse(&format!("{direction}status"))
        .with_param_type(Some(SearchParamType::Token))
}

fn method_sort() -> SortDirective {
    SortDirective::parse("method").with_param_type(Some(SearchParamType::Token))
}

fn paged(sort: SortDirective, count: u32, offset: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Observation")
        .with_sort(sort)
        .with_count(count);
    query.offset = Some(offset);
    query
}

#[tokio::test]
async fn unfiltered_param_sort_first_page_reads_a_bounded_window() {
    let rows = CANDIDATE_BATCH_SIZE * 2;
    let Some(fixture) = Fixture::new("sort-bounded-read", rows).await else {
        return;
    };
    let query = paged(status_sort(""), 10, 0);
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let result = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    // `final` sorts before `preliminary`; ties break by id.
    let expected: Vec<String> = sorted(
        fixture
            .rows
            .iter()
            .filter(|row| !row.deleted && row.status == "final")
            .map(|row| row.id.clone())
            .collect(),
    )
    .into_iter()
    .take(10)
    .collect();
    assert_eq!(page_ids(&result), expected);
    assert!(result.resources.page_info.has_next);
    let returned = documents_returned(&fixture.db).await;
    assert!(
        returned < (rows / 2) as i64,
        "read {returned} documents for a page of 10 in a type of {rows}"
    );
    fixture.close().await;
}

#[tokio::test]
async fn unfiltered_param_sort_refills_the_page_past_stale_index_rows() {
    let Some(mut fixture) = Fixture::new("sort-stale-refill", 300).await else {
        return;
    };
    // Deleted resources keep their index rows (#1056); the 10 first `final`
    // ids are among them, so the first window of keys is mostly dead.
    fixture.mark_deleted(1, 41).await;
    let result = with_result_id_cap(
        CAP,
        fixture
            .backend
            .search(&fixture.tenant, &paged(status_sort(""), 10, 0)),
    )
    .await
    .unwrap();
    let expected: Vec<String> = sorted(
        fixture
            .rows
            .iter()
            .filter(|row| !row.deleted && row.status == "final")
            .map(|row| row.id.clone())
            .collect(),
    )
    .into_iter()
    .take(10)
    .collect();
    assert_eq!(expected.len(), 10);
    assert_eq!(page_ids(&result), expected);
    assert!(result.resources.page_info.has_next);
    fixture.close().await;
}

#[tokio::test]
async fn unfiltered_param_sort_orders_the_keyless_tail_by_id() {
    let Some(mut fixture) = Fixture::new("sort-keyless-tail", 60).await else {
        return;
    };
    fixture.mark_deleted(1, 4).await;
    let live = sorted(
        fixture
            .rows
            .iter()
            .filter(|row| !row.deleted)
            .map(|row| row.id.clone())
            .collect(),
    );
    // No row carries `method`: every resource is keyless and the order is id.
    for (offset, count) in [(0u32, 10u32), (10, 10), (40, 10)] {
        let result = with_result_id_cap(
            CAP,
            fixture
                .backend
                .search(&fixture.tenant, &paged(method_sort(), count, offset)),
        )
        .await
        .unwrap();
        let expected: Vec<String> = live
            .iter()
            .skip(offset as usize)
            .take(count as usize)
            .cloned()
            .collect();
        assert_eq!(page_ids(&result), expected, "offset {offset}");
        assert_eq!(
            result.resources.page_info.has_next,
            offset as usize + (count as usize) < live.len(),
            "offset {offset}"
        );
    }
    // The keyless tail follows the id tie-break direction, not the direction of
    // the parameter: `-method` alone still lists it in ascending id order.
    let result = with_result_id_cap(
        CAP,
        fixture.backend.search(
            &fixture.tenant,
            &paged(
                SortDirective::parse("-method").with_param_type(Some(SearchParamType::Token)),
                5,
                0,
            ),
        ),
    )
    .await
    .unwrap();
    let expected: Vec<String> = live.iter().take(5).cloned().collect();
    assert_eq!(page_ids(&result), expected);
    fixture.close().await;
}

#[tokio::test]
async fn unfiltered_param_sort_beyond_the_cap_answers_too_many_results() {
    let Some(fixture) = Fixture::new("sort-over-cap", 120).await else {
        return;
    };
    let within = with_result_id_cap(
        CAP,
        fixture
            .backend
            .search(&fixture.tenant, &paged(status_sort(""), 10, 40)),
    )
    .await
    .unwrap();
    assert_eq!(within.resources.items.len(), 10);
    let error = with_result_id_cap(
        CAP,
        fixture
            .backend
            .search(&fixture.tenant, &paged(status_sort(""), 10, 41)),
    )
    .await
    .expect_err("offset + count over the cap must be refused");
    assert!(too_many(&error), "unexpected error: {error:?}");
    fixture.close().await;
}
