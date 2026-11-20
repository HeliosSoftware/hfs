//! #2285: the `:not` / `:missing` complement and the unfiltered parameter
//! `_sort` are bounded by the id cap and the page, not by the size of the type.
//! The cap is lowered with `with_result_id_cap` so a few hundred rows cross it.
//! #2588: a filtered single-key parameter `_sort` over the cap walks the sort
//! index and gives the page the collected path gives.

use super::super::super::{
    CANDIDATE_BATCH_SIZE, ParamSortWindow, SEARCH_DATE_END_INDEX, with_result_id_cap,
};
use super::*;
use crate::error::StorageError;
use chrono::Datelike;

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

// #2588: a filtered parameter sort whose matches exceed the cap walks the
// sort parameter's rows in order. Each page must be the one the collected
// path gives when the cap covers the matches.

const WIDE: usize = 1_000;
const DAY_MS: i64 = 86_400_000;
const BASE_MS: i64 = 1_577_836_800_000; // 2020-01-01T00:00:00Z

fn at(ms: i64) -> DateTime {
    DateTime::from_millis(ms)
}

/// The first instant of the month `ms` falls in, and of the next one.
fn month_of(ms: i64) -> (DateTime, DateTime) {
    let day = chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .date_naive();
    let first = day.with_day(1).unwrap();
    let next = first.checked_add_months(chrono::Months::new(1)).unwrap();
    let millis = |date: chrono::NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    };
    (at(millis(first)), at(millis(next)))
}

/// `date` rows of mixed precision (instants, days, months, some written
/// before #1391 without an end), many equal keys, a second value on some
/// resources and none on others; `value-quantity` rows on half of them.
async fn add_sort_rows(fixture: &Fixture) {
    let mut rows = Vec::new();
    for (number, row) in fixture.rows.iter().enumerate() {
        let number = number as i64;
        let day = BASE_MS + (number / 3) * DAY_MS;
        if number % 7 != 3 {
            let (start, end) = match number % 3 {
                0 => (at(day + 36_000_000), at(day + 36_000_001)),
                1 => (at(day), at(day + DAY_MS)),
                _ => month_of(day),
            };
            let mut date = doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation",
            "resource_id":&row.id, "param_name":"date", "value_date":start };
            if number % 10 != 5 {
                date.insert("value_date_end", end);
            }
            rows.push(date);
        }
        if number % 4 == 1 {
            rows.push(
                doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation",
                "resource_id":&row.id, "param_name":"date",
                "value_date":at(day + 40 * DAY_MS), "value_date_end":at(day + 41 * DAY_MS) },
            );
        }
        if number % 2 == 0 {
            let value = if number % 3 == 0 {
                Bson::Int32((number % 13) as i32)
            } else {
                Bson::Double((number % 13) as f64 + 0.5)
            };
            rows.push(
                doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation",
                "resource_id":&row.id, "param_name":"value-quantity",
                "value_quantity_value":value },
            );
        }
    }
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_many(rows)
        .await
        .unwrap();
}

/// Rows returned by the cursors hinted onto `idx_search_date_end`, their
/// `getMore`s included, as the profiler recorded them.
async fn sort_key_rows_returned(db: &mongodb::Database) -> i64 {
    let rows = collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! {})
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    let returned = |row: &Document| {
        row.get_i64("nreturned")
            .or_else(|_| row.get_i32("nreturned").map(i64::from))
            .unwrap_or(0)
    };
    let walks = |row: &Document| {
        row.get_document("command")
            .is_ok_and(|command| command.get_str("hint") == Ok(SEARCH_DATE_END_INDEX))
    };
    let cursors: HashSet<i64> = rows
        .iter()
        .filter(|row| walks(row))
        .filter_map(|row| row.get_i64("cursorid").ok())
        .collect();
    rows.iter()
        .filter(|row| {
            walks(row)
                || (row.get_str("op") == Ok("getmore")
                    && row
                        .get_i64("cursorid")
                        .is_ok_and(|cursor| cursors.contains(&cursor)))
        })
        .map(returned)
        .sum()
}

fn sort_by(spec: &str, param_type: SearchParamType) -> SortDirective {
    SortDirective::parse(spec).with_param_type(Some(param_type))
}

fn filtered(params: Vec<SearchParameter>, sort: SortDirective, offset: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Observation")
        .with_sort(sort)
        .with_count(10);
    query.parameters = params;
    query.offset = Some(offset);
    query.total = Some(TotalMode::Accurate);
    query
}

/// The query served with the cap covering its matches and with the cap
/// lowered below them: pages, `has_next` and totals must agree.
async fn assert_same_page(fixture: &Fixture, query: &SearchQuery, label: &str) {
    assert_same_page_on(&fixture.backend, &fixture.tenant, query, label).await;
}

async fn assert_same_page_on(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
    label: &str,
) {
    let collected = with_result_id_cap(WIDE, backend.search(tenant, query))
        .await
        .unwrap_or_else(|error| panic!("{label}: collected path failed: {error:?}"));
    let walked = with_result_id_cap(CAP, backend.search(tenant, query))
        .await
        .unwrap_or_else(|error| panic!("{label}: walked path failed: {error:?}"));
    assert_eq!(page_ids(&walked), page_ids(&collected), "{label}: page");
    assert_eq!(
        walked.resources.page_info.has_next, collected.resources.page_info.has_next,
        "{label}: has_next"
    );
    assert_eq!(walked.total, collected.total, "{label}: total");
}

fn not_final() -> SearchParameter {
    not_status("final")
}

#[tokio::test]
async fn over_cap_param_sort_pages_match_the_collected_order() {
    let Some(mut fixture) = Fixture::new("sort-walk-parity", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    // Deleted resources keep their search rows; some carry the first keys.
    fixture.mark_deleted(1, 12).await;
    fixture.mark_deleted(150, 160).await;
    let filters: Vec<(&str, Vec<SearchParameter>)> = vec![
        ("code", vec![parameter("code", "base")]),
        ("status", vec![parameter("status", "preliminary")]),
        ("not", vec![parameter("code", "base"), not_final()]),
        (
            "missing",
            vec![parameter("code", "base"), missing_method("true")],
        ),
    ];
    let sorts = [
        ("date", SearchParamType::Date),
        ("-date", SearchParamType::Date),
        ("status", SearchParamType::Token),
        ("-status", SearchParamType::Token),
        ("value-quantity", SearchParamType::Quantity),
        ("-value-quantity", SearchParamType::Quantity),
    ];
    for (filter_label, params) in &filters {
        for (spec, param_type) in sorts {
            for offset in [0u32, 10, 40] {
                let query = filtered(params.clone(), sort_by(spec, param_type), offset);
                let label = format!("{filter_label} _sort={spec} offset {offset}");
                assert_same_page(&fixture, &query, &label).await;
            }
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn over_cap_param_sort_reaches_the_keyless_tail_in_id_order() {
    let Some(fixture) = Fixture::new("sort-walk-tail", 120).await else {
        return;
    };
    // Only 10 resources carry `date`: every page past them is keyless.
    let rows: Vec<Document> = fixture.rows[..10]
        .iter()
        .enumerate()
        .map(|(number, row)| {
            doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation",
            "resource_id":&row.id, "param_name":"date",
            "value_date":at(BASE_MS + number as i64 * DAY_MS),
            "value_date_end":at(BASE_MS + (number as i64 + 1) * DAY_MS) }
        })
        .collect();
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_many(rows)
        .await
        .unwrap();
    for spec in ["date", "-date"] {
        for offset in [0u32, 5, 30, 40] {
            let query = filtered(
                vec![parameter("code", "base")],
                sort_by(spec, SearchParamType::Date),
                offset,
            );
            assert_same_page(&fixture, &query, &format!("_sort={spec} offset {offset}")).await;
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn over_cap_param_sort_with_only_resource_level_filters_walks() {
    let Some(fixture) = Fixture::new("sort-walk-resource-level", 200).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let mut updated = parameter("_lastUpdated", "gt2000-01-01");
    updated.param_type = SearchParamType::Date;
    updated.values = vec![SearchValue::parse("gt2000-01-01")];
    for spec in ["date", "-date"] {
        let query = filtered(
            vec![updated.clone()],
            sort_by(spec, SearchParamType::Date),
            10,
        );
        assert_same_page(&fixture, &query, &format!("_lastUpdated _sort={spec}")).await;
    }
    fixture.close().await;
}

#[tokio::test]
async fn over_cap_param_sort_first_page_reads_a_bounded_window() {
    let rows = CANDIDATE_BATCH_SIZE * 4;
    let Some(fixture) = Fixture::new("sort-walk-bounded-read", rows).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let mut query = filtered(
        vec![parameter("code", "base")],
        sort_by("-date", SearchParamType::Date),
        0,
    );
    query.total = Some(TotalMode::None);
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let walked = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(walked.resources.items.len(), 10);
    assert!(walked.resources.page_info.has_next);
    // The failed collection before the walk is not part of it, so only the
    // rows the sort-key cursors returned are counted.
    let returned = sort_key_rows_returned(&fixture.db).await;
    assert!(
        returned < rows as i64,
        "the walk read {returned} sort-key rows for a page of 10 over {rows} matches"
    );
    let collected = with_result_id_cap(WIDE * 4, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    assert_eq!(page_ids(&walked), page_ids(&collected));
    fixture.close().await;
}

/// A second backend on the fixture's database after `idx_search_date_end` is
/// dropped, booted with automatic builds off, as a node upgraded before the
/// index is pre-built: the existing indexes are verified, date-end is not.
async fn boot_without_date_end(fixture: &Fixture) -> MongoBackend {
    fixture
        .db
        .collection::<Document>("search_index")
        .drop_index(SEARCH_DATE_END_INDEX)
        .await
        .unwrap();
    let mut config = fixture.backend.config().clone();
    config.index_build = IndexBuildMode::Off;
    let backend = MongoBackend::new(config).unwrap();
    backend.init_schema().await.unwrap();
    assert_eq!(
        backend.wait_for_search_index_build().await,
        Some(crate::backends::mongodb::BuildOutcome::Skipped {
            missing: vec![SEARCH_DATE_END_INDEX.into()],
        })
    );
    assert!(backend.search_indexes_ready());
    assert!(!backend.date_end_index_ready());
    backend
}

/// Whether any profiled command hinted `index`.
async fn hinted(db: &mongodb::Database, index: &str) -> bool {
    !collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "command.hint": index })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
    .is_empty()
}

#[tokio::test]
async fn over_cap_descending_date_sort_waits_for_the_date_end_index() {
    let Some(fixture) = Fixture::new("sort-walk-date-end-missing", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let backend = boot_without_date_end(&fixture).await;
    let mut descending = filtered(
        vec![parameter("code", "base")],
        sort_by("-date", SearchParamType::Date),
        0,
    );
    descending.total = Some(TotalMode::None);
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    let outcome = with_result_id_cap(CAP, backend.search(&fixture.tenant, &descending)).await;
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let error = outcome.expect_err("-date over the cap must wait for idx_search_date_end");
    assert!(too_many(&error), "unexpected error: {error:?}");
    assert!(!hinted(&fixture.db, SEARCH_DATE_END_INDEX).await);
    // The verified existing indexes still serve the other walks: answering
    // past the cap at all means the walk served the page.
    for (spec, param_type) in [
        ("date", SearchParamType::Date),
        ("status", SearchParamType::Token),
        ("-status", SearchParamType::Token),
    ] {
        for offset in [0u32, 40] {
            let query = filtered(
                vec![parameter("code", "base")],
                sort_by(spec, param_type),
                offset,
            );
            let label = format!("date-end missing _sort={spec} offset {offset}");
            assert_same_page_on(&backend, &fixture.tenant, &query, &label).await;
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn over_cap_param_sort_beyond_the_cap_or_on_two_keys_answers_too_many_results() {
    let Some(fixture) = Fixture::new("sort-walk-refusals", 120).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let deep = filtered(
        vec![parameter("code", "base")],
        sort_by("date", SearchParamType::Date),
        41,
    );
    let error = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &deep))
        .await
        .expect_err("offset + count over the cap must be refused");
    assert!(too_many(&error), "unexpected error: {error:?}");
    let mut two_keys = filtered(
        vec![parameter("code", "base")],
        sort_by("date", SearchParamType::Date),
        0,
    );
    two_keys
        .sort
        .push(sort_by("status", SearchParamType::Token));
    let error = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &two_keys))
        .await
        .expect_err("a two-key sort over the cap keeps the collected path");
    assert!(too_many(&error), "unexpected error: {error:?}");
    fixture.close().await;
}

#[test]
fn param_sort_window_bounds_large_ties_in_both_id_directions() {
    use crate::types::SortDirection;

    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let wanted = 11; // The page prefix includes its extra has-next match.
        let mut window = ParamSortWindow::new(wanted, direction);
        let mut expected = Vec::new();
        for number in (0..4096).map(|number| number * 7919 % 4096) {
            let id = format!("{number:064}");
            let matches = number % 3 != 0;
            if matches {
                expected.push(id.clone());
            }
            assert!(!window.push(id, Bson::String("same".into()), matches));
            assert!(window.ordered.len() + window.tied.len() <= wanted);
        }
        expected.sort();
        if direction == SortDirection::Descending {
            expected.reverse();
        }
        expected.truncate(wanted);
        // End of input must emit the final tie even without a following key.
        assert_eq!(window.finish(), expected);
    }
}

#[test]
fn param_sort_window_bounds_completed_keys_and_closes_ties_on_rejected_rows() {
    use crate::types::SortDirection;

    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let wanted = 7;
        let mut window = ParamSortWindow::new(wanted, direction);
        for id in ["b", "c", "a"] {
            assert!(!window.push(id.into(), Bson::Int32(1), true));
        }
        let mut tied = Vec::new();
        for number in (0..2048).rev() {
            let id = format!("tie-{number:04}");
            let matches = number % 2 == 1;
            if matches {
                tied.push(id.clone());
            }
            assert!(!window.push(id, Bson::Int32(2), matches));
            assert_eq!(window.ordered.len(), 3);
            assert!(window.ordered.len() + window.tied.len() <= wanted);
        }
        // The next key completes the window even though its resource fails
        // membership. It cannot enter the result or keep the prior tie open.
        assert!(window.push("rejected".into(), Bson::Int32(3), false));
        assert_eq!(window.ordered.len(), wanted);
        assert!(window.tied.is_empty());
        let mut expected: Vec<String> = ["a", "b", "c"].map(str::to_owned).into();
        tied.sort();
        if direction == SortDirection::Descending {
            expected.reverse();
            tied.reverse();
        }
        expected.extend(tied.into_iter().take(wanted - expected.len()));
        assert_eq!(window.finish(), expected);
    }
}

#[test]
fn param_sort_window_returns_a_short_final_window() {
    use crate::types::SortDirection;

    let mut window = ParamSortWindow::new(4, SortDirection::Ascending);
    assert!(!window.push("rejected".into(), Bson::Int32(0), false));
    assert!(!window.push("b".into(), Bson::Int32(1), true));
    assert!(!window.push("a".into(), Bson::Int32(1), true));
    assert!(!window.push("c".into(), Bson::Int32(2), true));
    assert_eq!(window.finish(), ["a", "b", "c"]);
}

/// Inspect every nested `$in`, including membership predicates inside an
/// aggregation's `$and`. Looking only at returned rows misses oversized
/// commands that a small requested page can issue.
fn largest_membership(value: &Bson) -> usize {
    match value {
        Bson::Document(document) => document
            .iter()
            .map(|(key, value)| {
                let own = if key == "$in" {
                    value.as_array().map_or(0, Vec::len)
                } else {
                    0
                };
                own.max(largest_membership(value))
            })
            .max()
            .unwrap_or(0),
        Bson::Array(values) => values.iter().map(largest_membership).max().unwrap_or(0),
        _ => 0,
    }
}

async fn assert_sort_membership_commands_are_bounded(db: &mongodb::Database) {
    let operations = collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! {})
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    let mut largest = 0;
    for operation in operations {
        let Ok(command) = operation.get_document("command") else {
            continue;
        };
        let count = largest_membership(&Bson::Document(command.clone()));
        largest = largest.max(count);
        assert!(
            count <= CANDIDATE_BATCH_SIZE,
            "oversized membership: {command:?}"
        );
        assert!(
            mongodb::bson::to_vec(command).unwrap().len() < 128 * 1024,
            "a bounded membership command with 64-character ids should fit in 128 KiB"
        );
    }
    assert_eq!(
        largest, CANDIDATE_BATCH_SIZE,
        "must observe a full membership batch"
    );
}

#[tokio::test]
async fn over_cap_param_sort_bounds_large_ties_and_matches_collected_pages() {
    let rows = CANDIDATE_BATCH_SIZE * 3;
    let Some(mut fixture) = Fixture::new("sort-walk-large-tie", rows).await else {
        return;
    };
    // Use the full FHIR id length in actual membership commands. Keep ids in
    // resources, bodies and search rows consistent with the fixture oracle.
    let suffix = "x".repeat(58);
    fixture
        .db
        .collection::<Document>("resources")
        .update_many(
            doc! {},
            vec![doc! { "$set": {
                "id": { "$concat": ["$id", &suffix] },
                "data.id": { "$concat": ["$id", &suffix] },
            } }],
        )
        .await
        .unwrap();
    fixture
        .db
        .collection::<Document>("search_index")
        .update_many(
            doc! {},
            vec![doc! { "$set": { "resource_id": { "$concat": ["$resource_id", &suffix] } } }],
        )
        .await
        .unwrap();
    for row in &mut fixture.rows {
        row.id.push_str(&suffix);
    }
    fixture.mark_deleted(1, 12).await;
    fixture.mark_deleted(700, 710).await;
    let mut sort_rows = Vec::new();
    for (number, row) in fixture.rows.iter().enumerate() {
        sort_rows.push(index_row(&row.id, "method", "same"));
        if number % 17 == 0 {
            sort_rows.push(index_row(&row.id, "method", "same"));
        }
        // A second key on matching resources checks first-key deduplication
        // in both directions, with the large tie still needed for each page.
        if number == 17 || number == 21 {
            let value = if number == 17 { "a" } else { "z" };
            sort_rows.push(index_row(&row.id, "method", value));
        }
    }
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_many(sort_rows)
        .await
        .unwrap();

    for spec in ["method", "-method"] {
        for id_spec in ["_id", "-_id"] {
            let mut expected: Vec<_> = fixture
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| !row.deleted && row.status != "final")
                .map(|(number, row)| {
                    let key = match (spec, number) {
                        ("method", 17) => "a",
                        ("-method", 21) => "z",
                        _ => "same",
                    };
                    (key, row.id.clone())
                })
                .collect();
            expected.sort_by(|(a_key, a_id), (b_key, b_id)| {
                let order = if spec == "method" {
                    a_key.cmp(b_key)
                } else {
                    b_key.cmp(a_key)
                };
                order.then_with(|| {
                    if id_spec == "_id" {
                        a_id.cmp(b_id)
                    } else {
                        b_id.cmp(a_id)
                    }
                })
            });
            for offset in [0, 40] {
                let mut query = filtered(
                    vec![parameter("code", "base"), not_final()],
                    sort_by(spec, SearchParamType::Token),
                    offset,
                );
                query.sort.push(SortDirective::parse(id_spec));
                query.total = Some(TotalMode::None);
                let collected =
                    with_result_id_cap(rows * 2, fixture.backend.search(&fixture.tenant, &query))
                        .await
                        .unwrap();
                restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
                let walked =
                    with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query))
                        .await
                        .unwrap();
                fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
                let label = format!("{spec},{id_spec} offset {offset}");
                let expected_page: Vec<_> = expected
                    .iter()
                    .skip(offset as usize)
                    .take(10)
                    .map(|(_, id)| id.clone())
                    .collect();
                assert_eq!(
                    page_ids(&walked),
                    expected_page,
                    "{label}: independent order"
                );
                assert_eq!(page_ids(&walked), page_ids(&collected), "{label}: parity");
                assert!(walked.resources.page_info.has_next, "{label}: extra match");
                assert_eq!(
                    walked.resources.page_info.has_next, collected.resources.page_info.has_next,
                    "{label}: has_next parity"
                );
                assert_sort_membership_commands_are_bounded(&fixture.db).await;
            }
        }
    }
    fixture.close().await;
}

#[cfg(feature = "R4")]
#[tokio::test]
async fn over_cap_version_filtered_sort_keeps_the_documented_total_boundary() {
    let Some(fixture) = Fixture::new("sort-walk-version-total", 200).await else {
        return;
    };
    let other_version: Vec<_> = fixture.rows[..30]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    fixture
        .db
        .collection::<Document>("resources")
        .update_many(
            doc! { "id": { "$in": &other_version } },
            doc! { "$set": { "fhir_version": "5.0" } },
        )
        .await
        .unwrap();
    let mut query = filtered(vec![parameter("code", "base")], status_sort(""), 0)
        .with_fhir_version(helios_fhir::FhirVersion::R4);
    let collected = with_result_id_cap(WIDE, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    assert_eq!(collected.total, Some(170));
    let error = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query))
        .await
        .expect_err("version-restricted accurate totals retain the collected-path cap");
    assert!(too_many(&error), "unexpected error: {error:?}");

    query.total = Some(TotalMode::None);
    let walked = with_result_id_cap(CAP, fixture.backend.search(&fixture.tenant, &query))
        .await
        .unwrap();
    assert_eq!(page_ids(&walked), page_ids(&collected));
    assert!(
        page_ids(&walked)
            .iter()
            .all(|id| !other_version.contains(id))
    );
    assert!(walked.resources.page_info.has_next);
    assert_eq!(walked.total, None);
    fixture.close().await;
}

// #2601: an unfiltered descending date sort walks `idx_search_date_end`
// instead of grouping every row of the parameter.

fn unfiltered_desc_date(offset: u32) -> SearchQuery {
    let mut query = paged(sort_by("-date", SearchParamType::Date), 10, offset);
    query.total = Some(TotalMode::Accurate);
    query
}

#[tokio::test]
async fn unfiltered_descending_date_sort_walk_matches_the_grouped_pages() {
    let Some(mut fixture) = Fixture::new("sort-unfiltered-date-walk", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    // Stale index rows of deleted resources must not take page slots.
    fixture.mark_deleted(1, 20).await;
    let mut walked = Vec::new();
    for offset in [0u32, 10, 40, 200] {
        let result = with_result_id_cap(
            WIDE,
            fixture
                .backend
                .search(&fixture.tenant, &unfiltered_desc_date(offset)),
        )
        .await
        .unwrap();
        walked.push((
            page_ids(&result),
            result.resources.page_info.has_next,
            result.total,
        ));
    }
    // Same pages without the index: the grouped aggregate answers.
    let grouped_backend = boot_without_date_end(&fixture).await;
    for (offset, walked) in [0u32, 10, 40, 200].into_iter().zip(walked) {
        let result = with_result_id_cap(
            WIDE,
            grouped_backend.search(&fixture.tenant, &unfiltered_desc_date(offset)),
        )
        .await
        .unwrap();
        assert_eq!(walked.0, page_ids(&result), "offset {offset}: page");
        assert_eq!(
            walked.1, result.resources.page_info.has_next,
            "offset {offset}: has_next"
        );
        assert_eq!(walked.2, result.total, "offset {offset}: total");
    }
    fixture.close().await;
}

#[tokio::test]
async fn unfiltered_descending_date_sort_does_not_group_the_parameter() {
    let Some(fixture) = Fixture::new("sort-unfiltered-date-no-group", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    with_result_id_cap(
        WIDE,
        fixture
            .backend
            .search(&fixture.tenant, &unfiltered_desc_date(0)),
    )
    .await
    .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert!(hinted(&fixture.db, SEARCH_DATE_END_INDEX).await);
    let grouped = collect_documents(
        fixture
            .db
            .collection::<Document>("system.profile")
            .find(doc! { "command.pipeline.$group.key0": { "$exists": true } })
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(
        grouped.is_empty(),
        "the sort-key $group ran for the unfiltered walk: {grouped:?}"
    );
    fixture.close().await;
}

/// The unfiltered `-date` answer with the walk (`idx_search_date_end` present)
/// and with the grouped aggregate (it is missing) agree page by page at the
/// edges of the keyed prefix and of the live set, with and without a total.
#[tokio::test]
async fn unfiltered_descending_date_sort_matches_at_the_keyed_keyless_boundary() {
    let Some(mut fixture) = Fixture::new("sort-unfiltered-date-boundary", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    fixture.mark_deleted(1, 20).await;
    // Resources with a date row come first; the rest follow in id order.
    let live = fixture.rows.iter().filter(|row| !row.deleted).count();
    let keyed = fixture
        .rows
        .iter()
        .enumerate()
        .filter(|(number, row)| !row.deleted && (number % 7 != 3 || number % 4 == 1))
        .count();
    assert!(keyed < live, "the fixture needs a keyless tail");
    let mut offsets = vec![
        keyed - 10,
        keyed - 9,
        keyed - 1,
        keyed,
        keyed + 1,
        live - 10,
        live - 9,
        live,
        live + 1,
    ];
    offsets.sort_unstable();
    offsets.dedup();
    let query = |offset: usize, total: TotalMode| {
        let mut query = paged(sort_by("-date", SearchParamType::Date), 10, offset as u32);
        query.total = Some(total);
        query
    };
    let mut walked = Vec::new();
    for total in [TotalMode::None, TotalMode::Accurate] {
        for &offset in &offsets {
            let result = with_result_id_cap(
                WIDE,
                fixture
                    .backend
                    .search(&fixture.tenant, &query(offset, total)),
            )
            .await
            .unwrap();
            walked.push((
                (offset, total),
                page_ids(&result),
                result.resources.page_info.has_next,
                result.total,
            ));
        }
    }
    let grouped_backend = boot_without_date_end(&fixture).await;
    for ((offset, total), ids, has_next, count) in walked {
        let result = with_result_id_cap(
            WIDE,
            grouped_backend.search(&fixture.tenant, &query(offset, total)),
        )
        .await
        .unwrap();
        let label = format!("offset {offset} total {total:?}");
        assert_eq!(ids, page_ids(&result), "{label}: page");
        assert_eq!(
            has_next, result.resources.page_info.has_next,
            "{label}: has_next"
        );
        assert_eq!(count, result.total, "{label}: total");
        assert_eq!(
            has_next,
            offset + ids.len() < live,
            "{label}: has_next against the live count"
        );
    }
    fixture.close().await;
}

/// With `idx_search_date_end` missing the unfiltered `-date` sort never
/// hints it and the grouped aggregate (the old path) runs: the positive control for the
/// `$group.key0` check of the walk.
#[tokio::test]
async fn unfiltered_descending_date_sort_groups_the_parameter_without_the_end_index() {
    let Some(fixture) = Fixture::new("sort-unfiltered-date-grouped", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let backend = boot_without_date_end(&fixture).await;
    restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
    with_result_id_cap(
        WIDE,
        backend.search(&fixture.tenant, &unfiltered_desc_date(0)),
    )
    .await
    .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert!(!hinted(&fixture.db, SEARCH_DATE_END_INDEX).await);
    let grouped = collect_documents(
        fixture
            .db
            .collection::<Document>("system.profile")
            .find(doc! { "command.pipeline.$group.key0": { "$exists": true } })
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(
        !grouped.is_empty(),
        "the sort-key $group must run on the grouped path"
    );
    fixture.close().await;
}

#[tokio::test]
async fn unfiltered_descending_date_sort_falls_back_when_the_index_is_dropped_hot() {
    let Some(fixture) = Fixture::new("sort-unfiltered-date-hot-drop", 300).await else {
        return;
    };
    add_sort_rows(&fixture).await;
    let expected = with_result_id_cap(
        WIDE,
        fixture
            .backend
            .search(&fixture.tenant, &unfiltered_desc_date(0)),
    )
    .await
    .unwrap();
    // The running backend still believes the index is built.
    fixture
        .db
        .collection::<Document>("search_index")
        .drop_index(SEARCH_DATE_END_INDEX)
        .await
        .unwrap();
    assert!(fixture.backend.date_end_index_ready());
    let result = with_result_id_cap(
        WIDE,
        fixture
            .backend
            .search(&fixture.tenant, &unfiltered_desc_date(0)),
    )
    .await
    .expect("a missing date end index must not fail the sort");
    assert_eq!(page_ids(&result), page_ids(&expected));
    assert_eq!(result.total, expected.total);
    fixture.close().await;
}
