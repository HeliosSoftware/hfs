//! Timestamp-page contracts with independent fixture expectations and work counts.

use super::*;
use crate::backends::mongodb::schema::RESOURCES_TYPE_SCAN_INDEX;
use std::collections::BTreeMap;

#[test]
fn resource_batches_support_timestamp_sorts_without_other_sort_fields() {
    for (sorts, expected) in [
        (vec![], true),
        (vec!["-_lastUpdated"], true),
        (vec!["_lastUpdated", "_id"], true),
        (vec!["-_lastUpdated", "-_id"], true),
        (vec!["_lastUpdated"], true),
        (vec!["_lastUpdated", "-_id"], true),
        (vec!["-_lastUpdated", "_id"], true),
        (vec!["_id"], false),
        (vec!["_id", "_lastUpdated"], false),
        (vec!["date"], false),
    ] {
        let mut query = SearchQuery::new("Observation");
        for sort in &sorts {
            query = query.with_sort(SortDirective::parse(sort));
        }
        assert_eq!(
            ResourceBatchPlan::supports_query(&query),
            expected,
            "{sorts:?}"
        );
    }
}

struct TimestampFixture {
    fixture: Fixture,
    ordered: Vec<String>,
    timestamps: BTreeMap<String, i64>,
    markers: BTreeMap<String, Vec<String>>,
}

impl TimestampFixture {
    async fn new(label: &str) -> Option<Self> {
        // Late matches lie beyond the first 256 + 512 + 1,024 candidates.
        let mut fixture = Fixture::new(label, 2048).await?;
        fixture.mark_deleted(1, 130).await;
        let resources = fixture.db.collection::<Document>("resources");
        let index = fixture.db.collection::<Document>("search_index");
        let mut timestamps = BTreeMap::new();
        // Timestamp groups deliberately disagree with ID order and contain ties.
        for (position, chunk) in fixture.rows.chunks(128).enumerate() {
            let timestamp =
                DateTime::from_millis(1_700_000_000_000 + ((position * 5) % 12) as i64 * 1000);
            let ids: Vec<_> = chunk.iter().map(|row| row.id.clone()).collect();
            resources
                .update_many(
                    doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &ids } },
                    doc! { "$set": { "last_updated": timestamp } },
                )
                .await
                .unwrap();
            timestamps.extend(ids.into_iter().map(|id| (id, timestamp.timestamp_millis())));
        }
        let mut ordered: Vec<_> = fixture
            .rows
            .iter()
            .filter(|row| !row.deleted)
            .map(|row| row.id.clone())
            .collect();
        ordered.sort_by(|a, b| (timestamps[b], b).cmp(&(timestamps[a], a)));
        let mut markers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        markers.insert("frequent".into(), ordered.clone());
        markers.insert(
            "sparse".into(),
            ordered.iter().step_by(83).cloned().collect::<Vec<_>>(),
        );
        markers.insert(
            "late".into(),
            ordered.iter().rev().take(40).cloned().collect::<Vec<_>>(),
        );
        markers.insert("empty".into(), Vec::new());
        // Over sixteen matches in the first 256 candidates but fewer than a
        // page, so the scan keeps going instead of falling back.
        markers.insert(
            "medium".into(),
            ordered.iter().step_by(14).cloned().collect::<Vec<_>>(),
        );
        // Both conditions are broad individually, but together have only three matches.
        markers.insert("left".into(), ordered[..ordered.len() / 2 + 3].to_vec());
        markers.insert("right".into(), ordered[ordered.len() / 2..].to_vec());
        for (marker, ids) in &markers {
            if !ids.is_empty() {
                let rows: Vec<_> = ids
                    .iter()
                    .flat_map(|id| [index_row(id, "code", marker), index_row(id, "code", marker)])
                    .collect();
                index.insert_many(rows).await.unwrap();
            }
        }
        let template = resources
            .find_one(doc! { "id": &ordered[0] })
            .await
            .unwrap()
            .unwrap();
        for (id, tenant, resource_type) in [
            ("foreign", "another-tenant", "Observation"),
            ("wrong-type", PLAN_TENANT, "Patient"),
        ] {
            let mut resource = template.clone();
            resource.remove("_id");
            resource.insert("id", id);
            resource.insert("tenant_id", tenant);
            resource.insert("resource_type", resource_type);
            resources.insert_one(resource).await.unwrap();
        }
        for id in ["foreign", "wrong-type", "orphan", "r00000"] {
            index
                .insert_many([
                    index_row(id, "code", "base"),
                    index_row(id, "code", "frequent"),
                ])
                .await
                .unwrap();
            markers.get_mut("frequent").unwrap().push(id.to_owned());
        }
        Some(Self {
            fixture,
            ordered,
            timestamps,
            markers,
        })
    }

    fn expected(&self, conditions: &[&str]) -> Vec<String> {
        self.ordered
            .iter()
            .filter(|id| {
                conditions
                    .iter()
                    .all(|condition| self.markers[*condition].contains(id))
            })
            .cloned()
            .collect()
    }

    fn indexed_total(&self, conditions: &[&str]) -> u64 {
        self.markers[conditions[0]]
            .iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|id| {
                conditions
                    .iter()
                    .all(|condition| self.markers[*condition].contains(id))
            })
            .count() as u64
    }

    fn query(&self, conditions: &[&str]) -> SearchQuery {
        let mut query = SearchQuery::new("Observation")
            .with_count(20)
            .with_parameter(parameter("code", "base"));
        for condition in conditions {
            query = query.with_parameter(parameter("code", condition));
        }
        query.total = Some(TotalMode::None);
        query
    }

    fn force_broad_observations(&self, query: &SearchQuery) {
        for parameter in &query.parameters {
            let filter = self
                .fixture
                .backend
                .build_search_index_filter(PLAN_TENANT, "Observation", parameter)
                .unwrap();
            let predicate = self.fixture.backend.planning_predicate(
                &self.fixture.db,
                PLAN_TENANT,
                "Observation",
                parameter,
                filter,
                true,
            );
            self.fixture.backend.probe_cache.insert(
                predicate.probe_key.unwrap(),
                ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1),
            );
        }
    }

    async fn search_ids(&self, query: &SearchQuery) -> crate::core::SearchResult {
        self.force_broad_observations(query);
        self.fixture
            .backend
            .search(&self.fixture.tenant, query)
            .await
            .unwrap()
    }
}

fn result_ids(result: &crate::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_owned())
        .collect()
}

async fn start_profiling(db: &mongodb::Database) {
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    db.collection::<Document>("system.profile")
        .drop()
        .await
        .unwrap();
    db.run_command(doc! { "profile": 2 }).await.unwrap();
}

async fn profiled_operations(db: &mongodb::Database, filter: Document) -> Vec<Document> {
    let operations = collect_documents(
        db.collection::<Document>("system.profile")
            .find(filter)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(!operations.is_empty());
    operations
}

/// Candidate streams carry the type-scan hint on their find or getMore; the
/// final page fetch does not.
async fn candidate_stream_operations(db: &mongodb::Database) -> Vec<Document> {
    profiled_operations(
        db,
        doc! { "$or": [
            { "command.hint": RESOURCES_TYPE_SCAN_INDEX },
            { "originatingCommand.hint": RESOURCES_TYPE_SCAN_INDEX },
        ] },
    )
    .await
}

fn profile_count(operation: &Document, field: &str) -> i64 {
    operation
        .get_i64(field)
        .or_else(|_| operation.get_i32(field).map(i64::from))
        .unwrap_or_else(|_| panic!("Missing numeric {field} in {operation:?}"))
}

fn profile_sum(operations: &[Document], field: &str) -> i64 {
    operations
        .iter()
        .map(|operation| profile_count(operation, field))
        .sum()
}

/// The hinted index must supply the order and every projected field, so no
/// operation sorts in memory or reads a resource document.
fn assert_covered_index_order(operations: &[Document], context: &str) {
    for operation in operations {
        assert!(
            !operation.get_bool("hasSortStage").unwrap_or(false),
            "{context}: index must supply ordering"
        );
        assert_eq!(
            profile_count(operation, "docsExamined"),
            0,
            "{context}: candidate scan must be covered: {operation:?}"
        );
        assert!(
            operation
                .get_str("planSummary")
                .is_ok_and(|summary| summary.contains("IXSCAN")),
            "{context}: {operation:?}"
        );
    }
}

fn assert_bounded_membership_checks(checks: &[Document], expect_growth: bool) {
    let mut largest_batch = 0;
    for check in checks {
        let pipeline = check
            .get_document("command")
            .unwrap()
            .get_array("pipeline")
            .unwrap();
        let conditions = pipeline[0]
            .as_document()
            .unwrap()
            .get_document("$match")
            .unwrap()
            .get_array("$and")
            .unwrap();
        let ids = conditions[1]
            .as_document()
            .unwrap()
            .get_document("resource_id")
            .unwrap()
            .get_array("$in")
            .unwrap();
        largest_batch = largest_batch.max(ids.len());
        assert!(ids.len() <= COUNT_CANDIDATE_BATCH_ROWS);
        assert!(
            pipeline
                .iter()
                .all(|stage| !stage.as_document().unwrap().contains_key("$unionWith"))
        );
    }
    if expect_growth {
        assert!(largest_batch > PAGE_CANDIDATE_BATCH_ROWS);
    }
}

#[tokio::test]
async fn timestamp_pages_filter_before_offsets_and_preserve_cursor_totals() {
    let Some(data) = TimestampFixture::new("timestamp-pages").await else {
        return;
    };
    // `supports_query` accepts an empty sort because this is the default order.
    assert_eq!(
        data.fixture
            .backend
            .build_sort_document(&SearchQuery::new("Observation"), false)
            .unwrap(),
        doc! { "last_updated": -1_i32, "id": -1_i32 }
    );
    for conditions in [
        vec!["frequent"],
        vec!["sparse"],
        vec!["late"],
        vec!["empty"],
        vec!["left", "right"],
    ] {
        for sort in [None, Some("-_lastUpdated"), Some("_lastUpdated")] {
            let mut expected = data.expected(&conditions);
            if sort == Some("_lastUpdated") {
                expected.sort_by(|a, b| {
                    data.timestamps[a]
                        .cmp(&data.timestamps[b])
                        .then_with(|| b.cmp(a))
                });
            }
            let mut query = data.query(&conditions);
            if let Some(sort) = sort {
                query = query.with_sort(SortDirective::parse(sort));
            }
            data.force_broad_observations(&query);
            assert!(matches!(
                data.fixture
                    .backend
                    .resource_filter_plan(
                        &data.fixture.db,
                        PLAN_TENANT,
                        &query,
                        SearchFilterPurpose::Page,
                        NoTotalPagePolicy::Streaming,
                        &mut ProbeFacts::default(),
                    )
                    .await
                    .unwrap(),
                Some(SearchFilterPlan::ResourceBatches(_))
            ));
            for offset in [0, 20, 127, 128, 257, 4097] {
                query.offset = Some(offset);
                let result = data.search_ids(&query).await;
                assert_eq!(
                    result_ids(&result),
                    expected
                        .iter()
                        .skip(offset as usize)
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>(),
                    "conditions={conditions:?} sort={sort:?} offset={offset}"
                );
                assert_eq!(result.total, None);
            }
            query.offset = None;
            for mode in [TotalMode::Accurate, TotalMode::Estimate] {
                query.total = Some(mode);
                let first = data.search_ids(&query).await;
                assert_eq!(
                    result_ids(&first),
                    expected.iter().take(20).cloned().collect::<Vec<_>>()
                );
                let total = if mode == TotalMode::Accurate {
                    expected.len() as u64
                } else {
                    data.indexed_total(&conditions)
                };
                assert_eq!(
                    first.total,
                    Some(total),
                    "conditions={conditions:?} mode={mode:?}"
                );
                if expected.len() > 20 {
                    let next = data
                        .search_ids(
                            &query
                                .clone()
                                .with_cursor(first.resources.page_info.next_cursor.unwrap()),
                        )
                        .await;
                    assert_eq!(
                        result_ids(&next),
                        expected
                            .iter()
                            .skip(20)
                            .take(20)
                            .cloned()
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(next.total, Some(total));
                    let previous = data
                        .search_ids(
                            &query
                                .clone()
                                .with_cursor(next.resources.page_info.previous_cursor.unwrap()),
                        )
                        .await;
                    assert_eq!(
                        result_ids(&previous),
                        expected.iter().take(20).cloned().collect::<Vec<_>>()
                    );
                    assert_eq!(previous.total, Some(total));
                }
            }
        }
    }
    // Ascending timestamp + ascending ID is also supplied by the type-scan index.
    let query = data
        .query(&["frequent"])
        .with_sort(SortDirective::parse("_lastUpdated"))
        .with_sort(SortDirective::parse("_id"));
    let result = data.search_ids(&query).await;
    assert_eq!(
        result_ids(&result),
        data.ordered
            .iter()
            .rev()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
    );
    data.fixture.close().await;
}

#[tokio::test]
async fn resource_scan_applies_offsets_only_to_matching_rows() {
    let Some(data) = TimestampFixture::new("resource-scan-match-count").await else {
        return;
    };
    let filters = vec![predicate("code", "left"), predicate("code", "right")];
    let matching = data.expected(&["left", "right"]);
    assert_eq!(matching.len(), 3);
    let nonmatching = &data.ordered[..2];
    for (offset, skipped_remaining, retained) in [(0, 0, 3), (2, 0, 1), (10, 7, 0)] {
        let mut scan = ResourcePageScan::new(&filters, &[], offset, 21);
        scan.batch = nonmatching.to_vec();
        scan.match_batch(&data.fixture.db).await.unwrap();
        assert_eq!(
            scan.skip,
            u64::from(offset),
            "nonmatches do not consume offsets"
        );
        assert!(scan.ids.is_empty(), "nonmatches do not fill a page");
        scan.batch = matching.clone();
        scan.match_batch(&data.fixture.db).await.unwrap();
        assert_eq!(scan.skip, skipped_remaining, "offset={offset}");
        assert_eq!(scan.ids.len(), retained, "offset={offset}");
    }
    data.fixture.close().await;
}

#[tokio::test]
async fn timestamp_pages_grow_batches_and_continue_through_sparse_prefixes() {
    let Some(data) = TimestampFixture::new("timestamp-work").await else {
        return;
    };
    let db = &data.fixture.db;
    let live_rows = data.ordered.len() as i64;
    let first_batch = PAGE_CANDIDATE_BATCH_ROWS as i64;
    // Dense pages finish in one batch; sparse and late matches keep scanning.
    // Empty and short results require exhausting the candidate stream.
    for (conditions, rows) in [
        (vec!["frequent"], 1..=first_batch),
        (vec!["sparse"], first_batch + 1..=live_rows - 1),
        (vec!["late"], live_rows..=live_rows),
        (vec!["empty"], live_rows..=live_rows),
        (vec!["left", "right"], live_rows..=live_rows),
    ] {
        start_profiling(db).await;
        let context = format!("resource-batches-{conditions:?}");
        let mut filters = vec![predicate("code", "base")];
        filters.extend(
            conditions
                .iter()
                .map(|condition| predicate("code", condition)),
        );
        let plan = ResourceBatchPlan {
            filters,
            exclusions: Vec::new(),
        };
        let result = plan
            .execute_page(
                db,
                resource_filter(),
                None,
                doc! { "last_updated": -1, "id": -1 },
                0,
                21,
            )
            .await
            .unwrap();
        let expected = data.expected(&conditions);
        assert_eq!(
            result,
            expected.iter().take(21).cloned().collect::<Vec<_>>(),
            "{context}"
        );
        // system.profile records both find and getMore. Count returned candidates,
        // not elapsed time, so this assertion is independent of machine load.
        let operations = candidate_stream_operations(db).await;
        assert_covered_index_order(&operations, &context);
        let returned = profile_sum(&operations, "nreturned");
        assert!(
            rows.contains(&returned),
            "{context}: returned {returned} rows, expected {rows:?}"
        );
        let checks = profiled_operations(db, doc! { "command.aggregate": "search_index" }).await;
        assert_bounded_membership_checks(&checks, conditions != ["frequent"]);
    }
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}

#[tokio::test]
async fn timestamp_cursor_pages_seek_the_index_and_preserve_late_matches() {
    let Some(data) = TimestampFixture::new("timestamp-cursor-work").await else {
        return;
    };
    let db = &data.fixture.db;
    for sort in [None, Some("_lastUpdated")] {
        let mut expected = data.expected(&["frequent"]);
        let mut query = data.query(&["frequent"]);
        if let Some(sort) = sort {
            query = query.with_sort(SortDirective::parse(sort));
            expected.sort_by(|a, b| {
                data.timestamps[a]
                    .cmp(&data.timestamps[b])
                    .then_with(|| b.cmp(a))
            });
        }
        let mut cursor = None;
        // Each page walks a different cursor shape: none, next, then previous.
        for (page, slice) in [
            ("first", &expected[..20]),
            ("next", &expected[20..40]),
            ("previous", &expected[..20]),
        ] {
            start_profiling(db).await;
            let request = match &cursor {
                None => query.clone(),
                Some(cursor) => query.clone().with_cursor(String::clone(cursor)),
            };
            let result = data.search_ids(&request).await;
            assert_eq!(result_ids(&result), slice, "{page}");
            let operations = candidate_stream_operations(db).await;
            assert_covered_index_order(&operations, &format!("{sort:?} {page}"));
            // The keyset must position the scan near the cursor, not filter rows
            // read from the start of the type.
            let keys = profile_sum(&operations, "keysExamined");
            assert!(
                keys < data.ordered.len() as i64 / 2,
                "{page}: keysExamined {keys} must stay near the page"
            );
            cursor = match page {
                "first" => result.resources.page_info.next_cursor,
                _ => result.resources.page_info.previous_cursor,
            };
            if page == "previous" {
                assert!(cursor.is_none(), "the first page has no previous page");
            } else {
                assert!(cursor.is_some(), "{page}");
            }
        }
        // A forward scan from the start cannot pass this bound for a deep cursor.
        let offset = expected.len() * 3 / 4;
        let mut anchor_query = query.clone();
        anchor_query.offset = Some(offset as u32);
        let anchor = data.search_ids(&anchor_query).await;
        let next = query
            .clone()
            .with_cursor(anchor.resources.page_info.next_cursor.unwrap());
        start_profiling(db).await;
        let result = data.search_ids(&next).await;
        assert_eq!(result_ids(&result), expected[offset + 20..offset + 40]);
        let operations = candidate_stream_operations(db).await;
        assert_covered_index_order(&operations, "deep next");
        assert!(
            profile_sum(&operations, "keysExamined") < data.ordered.len() as i64 / 2,
            "a deep next page must seek rather than scan from the start"
        );
    }
    // An empty prefix must not prevent returning later matches.
    for conditions in [vec!["late"], vec!["empty"]] {
        let expected = data.expected(&conditions);
        let first = data.search_ids(&data.query(&conditions)).await;
        assert_eq!(
            result_ids(&first),
            expected.iter().take(20).cloned().collect::<Vec<_>>(),
            "{conditions:?}"
        );
        if expected.len() > 20 {
            let next = data
                .search_ids(
                    &data
                        .query(&conditions)
                        .with_cursor(first.resources.page_info.next_cursor.unwrap()),
                )
                .await;
            assert_eq!(
                result_ids(&next),
                expected
                    .iter()
                    .skip(20)
                    .take(20)
                    .cloned()
                    .collect::<Vec<_>>(),
                "{conditions:?}"
            );
        }
    }
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}

#[tokio::test]
async fn ascending_timestamp_cursors_cross_groups_without_skips_or_duplicates() {
    let Some(data) = TimestampFixture::new("ascending-timestamp-cursors").await else {
        return;
    };
    let mut expected = data.expected(&["frequent"]);
    expected.sort_by(|a, b| {
        data.timestamps[a]
            .cmp(&data.timestamps[b])
            .then_with(|| b.cmp(a))
    });
    // These 129-row pages cross timestamp groups, including groups with ties.
    let query = data
        .query(&["frequent"])
        .with_sort(SortDirective::parse("_lastUpdated"))
        .with_count(129);
    let mut current = data.search_ids(&query).await;
    let mut pages = Vec::new();
    loop {
        pages.push(result_ids(&current));
        let Some(next) = current.resources.page_info.next_cursor else {
            break;
        };
        assert!(pages.len() <= expected.len().div_ceil(129));
        current = data.search_ids(&query.clone().with_cursor(next)).await;
    }
    assert_eq!(pages.concat(), expected);
    for previous_page in pages[..pages.len() - 1].iter().rev() {
        let previous = current.resources.page_info.previous_cursor.unwrap();
        current = data.search_ids(&query.clone().with_cursor(previous)).await;
        assert_eq!(result_ids(&current), *previous_page);
    }
    assert!(current.resources.page_info.previous_cursor.is_none());
    data.fixture.close().await;
}

#[tokio::test]
async fn mixed_timestamp_orders_stream_large_ties_without_blocking_sort() {
    let Some(data) = TimestampFixture::new("timestamp-large-tie").await else {
        return;
    };
    let db = &data.fixture.db;
    db.collection::<Document>("resources")
        .update_many(
            resource_filter(),
            doc! { "$set": { "last_updated": DateTime::from_millis(1_700_000_000_000) } },
        )
        .await
        .unwrap();
    let mut expected = data.expected(&["frequent"]);
    expected.sort();
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "frequent")],
        exclusions: Vec::new(),
    };
    for (timestamp_direction, id_direction) in [(1, -1), (-1, 1)] {
        start_profiling(db).await;
        let actual = plan
            .execute_page(
                db,
                resource_filter(),
                None,
                doc! { "last_updated": timestamp_direction, "id": id_direction },
                0,
                21,
            )
            .await
            .unwrap();
        let mut ordered = expected.clone();
        if id_direction == -1 {
            ordered.reverse();
        }
        assert_eq!(actual, ordered[..21]);
        let operations = candidate_stream_operations(db).await;
        assert_covered_index_order(
            &operations,
            "both timestamp discovery and ID traversal must use index order",
        );
        let returned = profile_sum(&operations, "nreturned");
        assert!(returned > 0 && returned < expected.len() as i64 / 2);
    }
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}

#[tokio::test]
async fn complement_pages_batch_exclusions_and_preserve_absence_and_cursors() {
    let Some(data) = TimestampFixture::new("complement-pages").await else {
        return;
    };
    let db = &data.fixture.db;
    let index = db.collection::<Document>("search_index");
    let missing: HashSet<_> = data.ordered.iter().step_by(7).cloned().collect();
    let forbidden: HashSet<_> = data
        .ordered
        .iter()
        .step_by(5)
        .filter(|id| !missing.contains(*id))
        .cloned()
        .collect();
    let missing_ids: Vec<_> = missing.iter().cloned().collect();
    index
        .delete_many(doc! {
            "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "param_name": "code", "resource_id": { "$in": &missing_ids },
        })
        .await
        .unwrap();
    db.collection::<Document>("resources")
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &missing_ids } },
            doc! { "$unset": { "data.code": "" } },
        )
        .await
        .unwrap();
    // An envelope with a null value is not evidence that a parameter is present.
    index
        .insert_many(missing.iter().map(|id| {
            let mut row = index_row(id, "code", "unused");
            row.insert("value_token_code", Bson::Null);
            row
        }))
        .await
        .unwrap();
    index
        .insert_many(forbidden.iter().flat_map(|id| {
            [
                index_row(id, "code", "forbidden"),
                index_row(id, "code", "forbidden"),
            ]
        }))
        .await
        .unwrap();
    let forbidden_ids: Vec<_> = forbidden.iter().cloned().collect();
    db.collection::<Document>("resources")
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": forbidden_ids } },
            doc! { "$push": { "data.code.coding": { "code": "forbidden" } } },
        )
        .await
        .unwrap();
    // Neither another tenant nor another resource type can exclude this live ID.
    let scoped_id = data
        .ordered
        .iter()
        .find(|id| !forbidden.contains(*id))
        .unwrap();
    let mut foreign = index_row(scoped_id, "code", "forbidden");
    foreign.insert("tenant_id", "another-tenant");
    let mut wrong_type = index_row(scoped_id, "code", "forbidden");
    wrong_type.insert("resource_type", "Patient");
    index.insert_many([foreign, wrong_type]).await.unwrap();

    let modified = |value, modifier| {
        let mut condition = parameter("code", value);
        condition.modifier = Some(modifier);
        condition
    };
    let not_forbidden = modified("forbidden", SearchModifier::Not);
    let absent = modified("true", SearchModifier::Missing);
    let present = modified("false", SearchModifier::Missing);
    let cases = [
        (
            "not",
            vec![not_forbidden.clone()],
            data.ordered
                .iter()
                .filter(|id| !forbidden.contains(*id))
                .cloned()
                .collect::<Vec<_>>(),
        ),
        (
            "missing",
            vec![absent],
            data.ordered
                .iter()
                .filter(|id| missing.contains(*id))
                .cloned()
                .collect(),
        ),
        (
            "present",
            vec![present.clone()],
            data.ordered
                .iter()
                .filter(|id| !missing.contains(*id))
                .cloned()
                .collect(),
        ),
        (
            "repeated-not",
            vec![not_forbidden.clone(), modified("base", SearchModifier::Not)],
            data.ordered
                .iter()
                .filter(|id| missing.contains(*id))
                .cloned()
                .collect(),
        ),
        (
            "positive-and-not",
            vec![parameter("code", "base"), not_forbidden.clone()],
            data.ordered
                .iter()
                .filter(|id| !missing.contains(*id) && !forbidden.contains(*id))
                .cloned()
                .collect(),
        ),
        (
            "present-and-not",
            vec![present, not_forbidden],
            data.ordered
                .iter()
                .filter(|id| !missing.contains(*id) && !forbidden.contains(*id))
                .cloned()
                .collect(),
        ),
    ];
    for (name, parameters, descending) in cases {
        for sort in [None, Some("_lastUpdated"), Some("_id")] {
            let mut expected = descending.clone();
            let mut query = SearchQuery::new("Observation").with_count(20);
            query.parameters = parameters.clone();
            query.total = Some(TotalMode::None);
            if let Some(sort) = sort {
                query = query.with_sort(SortDirective::parse(sort));
                if sort == "_id" {
                    expected.sort();
                } else {
                    expected.sort_by(|a, b| {
                        data.timestamps[a]
                            .cmp(&data.timestamps[b])
                            .then_with(|| b.cmp(a))
                    });
                }
            }
            let plan = data
                .fixture
                .backend
                .resource_filter_plan(
                    db,
                    PLAN_TENANT,
                    &query,
                    SearchFilterPurpose::Page,
                    NoTotalPagePolicy::Streaming,
                    &mut ProbeFacts::default(),
                )
                .await
                .unwrap();
            if matches!(name, "not" | "missing" | "repeated-not") && sort != Some("_id") {
                assert!(
                    matches!(plan, Some(SearchFilterPlan::ResourceBatches(_))),
                    "{name} {sort:?}"
                );
            }
            for offset in [0, 20, 127] {
                query.offset = Some(offset);
                let result = data
                    .fixture
                    .backend
                    .search(&data.fixture.tenant, &query)
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&result),
                    expected
                        .iter()
                        .skip(offset as usize)
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>(),
                    "{name} {sort:?} offset={offset}"
                );
                assert_eq!(result.total, None);
            }
            query.offset = None;
            for mode in [TotalMode::None, TotalMode::Accurate, TotalMode::Estimate] {
                query.total = Some(mode);
                let first = data
                    .fixture
                    .backend
                    .search(&data.fixture.tenant, &query)
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&first),
                    expected[..20],
                    "{name} {sort:?} {mode:?}"
                );
                assert_eq!(
                    first.total,
                    (mode != TotalMode::None).then_some(expected.len() as u64)
                );
                let next = data
                    .fixture
                    .backend
                    .search(
                        &data.fixture.tenant,
                        &query
                            .clone()
                            .with_cursor(first.resources.page_info.next_cursor.unwrap()),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&next),
                    expected[20..40],
                    "{name} {sort:?} {mode:?} next"
                );
                let previous = data
                    .fixture
                    .backend
                    .search(
                        &data.fixture.tenant,
                        &query
                            .clone()
                            .with_cursor(next.resources.page_info.previous_cursor.unwrap()),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&previous),
                    expected[..20],
                    "{name} {sort:?} {mode:?} previous"
                );
                assert_eq!(next.total, first.total);
                assert_eq!(previous.total, first.total);
            }
        }
    }
    let mut query = SearchQuery::new("Observation")
        .with_count(20)
        .with_parameter(modified("forbidden", SearchModifier::Not));
    query.total = Some(TotalMode::None);
    start_profiling(db).await;
    data.fixture
        .backend
        .search(&data.fixture.tenant, &query)
        .await
        .unwrap();
    let streams = candidate_stream_operations(db).await;
    assert_covered_index_order(&streams, "complement first page");
    assert!(profile_sum(&streams, "nreturned") <= PAGE_CANDIDATE_BATCH_ROWS as i64);
    let checks = profiled_operations(db, doc! { "command.aggregate": "search_index" }).await;
    assert_bounded_membership_checks(&checks, false);
    let operations =
        profiled_operations(db, doc! { "command.pipeline": { "$exists": true } }).await;
    for operation in operations {
        let pipeline = operation
            .get_document("command")
            .unwrap()
            .get_array("pipeline")
            .unwrap();
        assert!(
            pipeline
                .iter()
                .all(|stage| !stage.as_document().unwrap().contains_key("$lookup"))
        );
    }
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}

#[tokio::test]
async fn complement_planning_preserves_selective_and_unknown_probe_routes() {
    let Some(data) = TimestampFixture::new("complement-planning").await else {
        return;
    };
    let mut excluded = parameter("code", "right");
    excluded.modifier = Some(SearchModifier::Not);
    let mut query = SearchQuery::new("Observation")
        .with_count(20)
        .with_parameter(parameter("code", "left"))
        .with_parameter(excluded.clone());
    query.total = Some(TotalMode::None);
    let expected: Vec<_> = data
        .ordered
        .iter()
        .filter(|id| data.markers["left"].contains(id) && !data.markers["right"].contains(id))
        .cloned()
        .collect();
    let backend = &data.fixture.backend;
    let db = &data.fixture.db;
    let filter = backend
        .build_search_index_filter(PLAN_TENANT, "Observation", &query.parameters[0])
        .unwrap();
    let key = ProbeFacts::key(
        db,
        &filter,
        None,
        MAX_CANDIDATE_ROWS + 1,
        Some(backend.probe_timeout()),
    )
    .unwrap();
    for observation in [Some(2000), None] {
        let mut facts = ProbeFacts::default();
        facts.remember(key.clone(), observation);
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                &mut facts,
            )
            .await
            .unwrap();
        match (observation, plan) {
            (Some(_), Some(SearchFilterPlan::CandidateIds(ids))) => {
                assert_eq!(
                    ids.into_iter().collect::<HashSet<_>>(),
                    expected.iter().cloned().collect()
                );
            }
            (None, Some(SearchFilterPlan::ResourceBatches(plan))) => {
                let page = plan
                    .execute_page(
                        db,
                        resource_filter(),
                        None,
                        doc! { "last_updated": -1, "id": -1 },
                        20,
                        21,
                    )
                    .await
                    .unwrap();
                assert_eq!(page, expected[20..41]);
            }
            _ => panic!("unexpected complement plan for observation {observation:?}"),
        }
    }
    assert!(matches!(
        backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Count,
                NoTotalPagePolicy::Streaming,
                &mut ProbeFacts::default()
            )
            .await
            .unwrap(),
        Some(SearchFilterPlan::ResourceLookups { .. })
    ));

    let sparse = parameter("code", "sparse");
    let sparse_filter = backend
        .build_search_index_filter(PLAN_TENANT, "Observation", &sparse)
        .unwrap();
    let indexed_rows = db
        .collection::<Document>("search_index")
        .count_documents(sparse_filter)
        .await
        .unwrap();
    assert!((1..=SELECTIVE_SEARCH_ROW_LIMIT).contains(&indexed_rows));
    query.parameters = vec![sparse, excluded];
    assert!(
        backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                &mut ProbeFacts::default()
            )
            .await
            .unwrap()
            .is_none()
    );
    let sparse_expected: Vec<_> = data
        .ordered
        .iter()
        .filter(|id| data.markers["sparse"].contains(id) && !data.markers["right"].contains(id))
        .take(20)
        .cloned()
        .collect();
    let result = backend.search(&data.fixture.tenant, &query).await.unwrap();
    assert_eq!(result_ids(&result), sparse_expected);
    data.fixture.close().await;
}

#[tokio::test]
async fn complement_date_pages_reuse_metadata_without_hinting_presence() {
    let Some(data) = TimestampFixture::new("complement-date-metadata").await else {
        return;
    };
    let backend = &data.fixture.backend;
    let db = &data.fixture.db;
    let start = DateTime::from_millis(1_767_225_600_000);
    let end = DateTime::from_millis(start.timestamp_millis() + 86_400_000);
    db.collection::<Document>("search_index")
        .insert_many(data.fixture.rows.iter().map(|row| {
            doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation", "resource_id": &row.id,
            "param_name": "date", "value_date": start, "value_date_end": end }
        }))
        .await
        .unwrap();
    let mut date = parameter("date", "2026-01-01");
    date.param_type = SearchParamType::Date;
    date.values[0].prefix = SearchPrefix::Ge;
    let mut excluded = parameter("code", "right");
    excluded.modifier = Some(SearchModifier::Not);
    let mut query = SearchQuery::new("Observation")
        .with_count(20)
        .with_parameter(date.clone())
        .with_parameter(excluded);
    query.total = Some(TotalMode::None);
    let expected: Vec<_> = data
        .ordered
        .iter()
        .filter(|id| !data.markers["right"].contains(id))
        .cloned()
        .collect();
    let filter = backend
        .build_search_index_filter(PLAN_TENANT, "Observation", &date)
        .unwrap();
    let metadata = backend.planning_predicate(db, PLAN_TENANT, "Observation", &date, filter, true);
    let key = metadata
        .probe_key
        .clone()
        .expect("date must have a scoped cache key");
    assert_eq!(
        metadata.hint(),
        Some(Hint::Name(SEARCH_DATE_RANGE_INDEX.to_owned()))
    );
    backend
        .probe_cache
        .insert(key.clone(), ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1));
    for _ in 0..2 {
        let mut facts = ProbeFacts::default();
        let Some(SearchFilterPlan::ResourceBatches(plan)) = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                &mut facts,
            )
            .await
            .unwrap()
        else {
            panic!("cached broad date observation must select resource batches");
        };
        assert_eq!(plan.filters[0].hint(), metadata.hint());
        assert_eq!(plan.filters[0].probe_key, Some(key.clone()));
        assert_eq!(
            facts.requests, 1,
            "only the untimed 65-row probe needs a database request"
        );
        let page = plan
            .execute_page(
                db,
                resource_filter(),
                None,
                doc! { "last_updated": -1, "id": -1 },
                0,
                21,
            )
            .await
            .unwrap();
        assert_eq!(page, expected[..21]);
    }
    // A stale cached count may change candidate selection, never establish the 64-row fallback or the results.
    backend.probe_cache.clear();
    backend
        .probe_cache
        .insert(key.clone(), ProbeEstimate::Completed(1));
    let mut facts = ProbeFacts::default();
    assert!(matches!(
        backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                &mut facts
            )
            .await
            .unwrap(),
        Some(SearchFilterPlan::CandidateIds(_))
    ));
    assert_eq!(facts.requests, 1);
    let result = backend.search(&data.fixture.tenant, &query).await.unwrap();
    assert_eq!(result_ids(&result), expected[..20]);
    assert_eq!(
        backend.probe_cache.get(&key),
        Some(ProbeEstimate::Completed(1))
    );

    let mut present = date;
    present.modifier = Some(SearchModifier::Missing);
    present.values = vec![SearchValue::eq("false")];
    query.parameters[0] = present.clone();
    let presence = missing_presence_filter(PLAN_TENANT, "Observation", &present);
    let key = ProbeFacts::key(
        db,
        &presence,
        None,
        MAX_CANDIDATE_ROWS + 1,
        Some(backend.probe_timeout()),
    )
    .unwrap();
    let mut facts = ProbeFacts::default();
    facts.remember(key, None);
    let Some(SearchFilterPlan::ResourceBatches(plan)) = backend
        .resource_filter_plan(
            db,
            PLAN_TENANT,
            &query,
            SearchFilterPurpose::Page,
            NoTotalPagePolicy::Streaming,
            &mut facts,
        )
        .await
        .unwrap()
    else {
        panic!("unknown presence observation must select resource batches");
    };
    assert_eq!(
        plan.filters[0].hint(),
        None,
        "presence must not inherit a date-range hint"
    );
    assert_eq!(plan.filters[0].probe_key, None);
    assert_eq!(
        facts.requests, 0,
        "presence does not use the untimed selective probe"
    );
    let page = plan
        .execute_page(
            db,
            resource_filter(),
            None,
            doc! { "last_updated": -1, "id": -1 },
            0,
            21,
        )
        .await
        .unwrap();
    assert_eq!(page, expected[..21]);
    data.fixture.close().await;
}

/// A first batch with at most sixteen matches sends the page to the bounded
/// id set instead of scanning the rest of the type; the page is the same.
#[tokio::test]
async fn sparse_first_batches_page_from_the_id_set() {
    let Some(data) = TimestampFixture::new("sparse-fallback").await else {
        return;
    };
    let db = &data.fixture.db;
    let first_batch = PAGE_CANDIDATE_BATCH_ROWS as i64;
    let live_rows = data.ordered.len() as i64;
    // (conditions, candidates the stream may return, membership batches per
    // predicate): a dense page fills in one batch, a medium one keeps
    // scanning; sparse, late, empty and intersected pages stop after one
    // batch and come from the id set.
    for (conditions, rows, batches) in [
        (vec!["frequent"], 1..=first_batch, 1..=1),
        (vec!["medium"], first_batch + 1..=live_rows, 2..=i64::MAX),
        (vec!["sparse"], first_batch..=first_batch, 1..=1),
        (vec!["late"], first_batch..=first_batch, 1..=1),
        (vec!["empty"], first_batch..=first_batch, 1..=1),
        (vec!["left", "right"], first_batch..=first_batch, 1..=1),
    ] {
        let context = format!("search-{conditions:?}");
        let query = data.query(&conditions);
        start_profiling(db).await;
        let result = data.search_ids(&query).await;
        let expected = data.expected(&conditions);
        assert_eq!(
            result_ids(&result),
            expected.iter().take(20).cloned().collect::<Vec<_>>(),
            "{context}"
        );
        assert_eq!(
            result.resources.page_info.has_next,
            expected.len() > 20,
            "{context}"
        );
        let operations = candidate_stream_operations(db).await;
        let returned = profile_sum(&operations, "nreturned");
        assert!(
            rows.contains(&returned),
            "{context}: the candidate stream returned {returned} rows, expected {rows:?}"
        );
        let checks = membership_checks(db).await;
        let per_predicate = checks.len() as i64 / (conditions.len() as i64 + 1);
        assert!(
            batches.contains(&per_predicate),
            "{context}: {per_predicate} membership batches per predicate, expected {batches:?}"
        );
        if expected.len() > 20 {
            // A cursor page takes the same decision from the same first batch.
            let next = data
                .search_ids(
                    &query
                        .clone()
                        .with_cursor(result.resources.page_info.next_cursor.clone().unwrap()),
                )
                .await;
            assert_eq!(
                result_ids(&next),
                expected[20..].iter().take(20).cloned().collect::<Vec<_>>(),
                "{context} next"
            );
        }
    }
    // A complement condition collects its id set uncapped, so an empty
    // complement page keeps scanning the type and never reads the id set.
    // The positive predicate is forced broad here, not left to earlier cases.
    data.fixture.backend.probe_cache.clear();
    let positive = data.query(&[]);
    data.force_broad_observations(&positive);
    let mut excluded = parameter("code", "frequent");
    excluded.modifier = Some(SearchModifier::Not);
    let mut query = positive.with_parameter(excluded);
    query.total = Some(TotalMode::None);
    start_profiling(db).await;
    let result = data
        .fixture
        .backend
        .search(&data.fixture.tenant, &query)
        .await
        .unwrap();
    assert!(result_ids(&result).is_empty(), "complement page");
    assert_eq!(
        profile_sum(&candidate_stream_operations(db).await, "nreturned"),
        live_rows,
        "complement page scans the whole type"
    );
    assert!(
        membership_checks(db).await.len() > 2,
        "complement page keeps matching batches"
    );
    assert!(
        id_set_operations(db).await.is_empty(),
        "complement page never reads the id set"
    );
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}

/// The scan's per-batch membership checks: hinted aggregates on the index.
async fn membership_checks(db: &mongodb::Database) -> Vec<Document> {
    profiled_operations(
        db,
        doc! { "command.aggregate": "search_index", "command.hint": { "$exists": true } },
    )
    .await
}

/// The id-set pass reads the driver predicate's rows with a `find` on the
/// index, or groups the index by resource id without a hint for a complement;
/// the scan's membership checks are hinted aggregates and do neither.
async fn id_set_operations(db: &mongodb::Database) -> Vec<Document> {
    // Not `profiled_operations`: an empty result is a valid outcome here.
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "$or": [
                { "command.find": "search_index" },
                { "command.aggregate": "search_index", "command.hint": { "$exists": false } },
            ] })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
    .into_iter()
    .filter(|operation| {
        let command = operation.get_document("command").unwrap();
        command.contains_key("find")
            || command
                .get_array("pipeline")
                .ok()
                .and_then(|pipeline| pipeline.get(1))
                .and_then(|stage| stage.as_document())
                .and_then(|stage| stage.get_document("$group").ok())
                .is_some_and(|group| group.get_str("_id") == Ok("$resource_id"))
    })
    .collect()
}

/// The sample classifies the first batch, and a run that is not served another
/// way finishes the scan from where the sample stopped.
#[tokio::test]
async fn sampled_runs_classify_the_first_batch_and_resume() {
    let Some(data) = TimestampFixture::new("sparse-sample").await else {
        return;
    };
    let db = &data.fixture.db;
    let live_rows = data.ordered.len() as i64;
    for (conditions, sparse, rows) in [
        (
            vec!["frequent"],
            false,
            1..=PAGE_CANDIDATE_BATCH_ROWS as i64,
        ),
        (
            vec!["medium"],
            false,
            PAGE_CANDIDATE_BATCH_ROWS as i64 + 1..=live_rows,
        ),
        (
            vec!["sparse"],
            true,
            PAGE_CANDIDATE_BATCH_ROWS as i64 + 1..=live_rows,
        ),
        (vec!["late"], true, live_rows..=live_rows),
        (vec!["empty"], true, live_rows..=live_rows),
    ] {
        let context = format!("sample-{conditions:?}");
        let mut filters = vec![predicate("code", "base")];
        filters.extend(
            conditions
                .iter()
                .map(|condition| predicate("code", condition)),
        );
        let plan = ResourceBatchPlan {
            filters,
            exclusions: Vec::new(),
        };
        start_profiling(db).await;
        let mut run = plan
            .start_page(
                db,
                resource_filter(),
                None,
                doc! { "last_updated": -1, "id": -1 },
                0,
                21,
            )
            .await
            .unwrap();
        let expected = data.expected(&conditions);
        let page = match run.sample(db).await.unwrap() {
            ResourcePageSample::Complete(ids) => {
                assert_eq!(conditions, ["frequent"], "{context}");
                ids
            }
            ResourcePageSample::Dense => {
                assert!(!sparse, "{context}");
                run.finish(db).await.unwrap()
            }
            ResourcePageSample::Sparse { live_matches } => {
                assert!(sparse, "{context}");
                assert!(live_matches <= SPARSE_SAMPLE_MAX_LIVE, "{context}");
                run.finish(db).await.unwrap()
            }
        };
        assert_eq!(
            page,
            expected.iter().take(21).cloned().collect::<Vec<_>>(),
            "{context}"
        );
        let returned = profile_sum(&candidate_stream_operations(db).await, "nreturned");
        assert!(
            rows.contains(&returned),
            "{context}: the candidate stream returned {returned} rows, expected {rows:?}"
        );
    }
    // The sample itself does not know about complements: a plan with an
    // exclusion samples sparse like any other, and its run resumes correctly.
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "base")],
        exclusions: vec![predicate("code", "frequent")],
    };
    start_profiling(db).await;
    let mut run = plan
        .start_page(
            db,
            resource_filter(),
            None,
            doc! { "last_updated": -1, "id": -1 },
            0,
            21,
        )
        .await
        .unwrap();
    assert!(
        matches!(
            run.sample(db).await.unwrap(),
            ResourcePageSample::Sparse { live_matches: 0 }
        ),
        "an excluded batch samples sparse"
    );
    assert!(run.finish(db).await.unwrap().is_empty());
    assert_eq!(
        profile_sum(&candidate_stream_operations(db).await, "nreturned"),
        live_rows
    );
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    data.fixture.close().await;
}
