//! Access paths and result parity for the bounded legacy candidate collector.

use super::super::super::CANDIDATE_BATCH_SIZE;
use super::super::super::MissingSupport;
use super::super::super::search_access::SEARCH_TOKEN_MEMBERSHIP_INDEX;
use super::*;
use crate::backends::mongodb::schema::RESOURCES_TYPE_SCAN_INDEX;

fn result_ids(result: &crate::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_owned())
        .collect()
}

async fn profile_start(db: &mongodb::Database) {
    // Exact counts record several 4096-ID commands after the page. The
    // default 1 MiB profiler can evict its early sparse-fallback evidence.
    restart_profiling(db, 32 * 1024 * 1024).await;
}

async fn profile_rows(db: &mongodb::Database, filter: Document) -> Vec<Document> {
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(filter)
            .sort(doc! { "$natural": 1 })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
}

fn numeric(row: &Document, field: &str) -> i64 {
    row.get_i64(field)
        .or_else(|_| row.get_i32(field).map(i64::from))
        .unwrap()
}

fn candidate_ids(command: &Document) -> &[Bson] {
    command.get_array("pipeline").unwrap()[0]
        .as_document()
        .unwrap()
        .get_document("$match")
        .unwrap()
        .get_array("$and")
        .unwrap()[1]
        .as_document()
        .unwrap()
        .get_document("resource_id")
        .unwrap()
        .get_array("$in")
        .unwrap()
}

fn date_row(id: &str, value: &str) -> Document {
    let start = DateTime::parse_rfc3339_str(value).unwrap();
    doc! {
        "tenant_id": PLAN_TENANT, "resource_type": "Observation", "resource_id": id,
        "param_name": "date", "value_date": start,
        "value_date_end": DateTime::from_millis(start.timestamp_millis() + 1),
    }
}

async fn add_dates(fixture: &Fixture, recent: usize) {
    let index = fixture.db.collection::<Document>("search_index");
    let rows: Vec<_> = fixture
        .rows
        .iter()
        .enumerate()
        .map(|(position, row)| {
            date_row(
                &row.id,
                if position < recent {
                    "2025-06-01T00:00:00Z"
                } else {
                    "2024-06-01T00:00:00Z"
                },
            )
        })
        .collect();
    for chunk in rows.chunks(500) {
        index.insert_many(chunk.to_vec()).await.unwrap();
    }
}

#[test]
fn date_access_is_shared_and_disabled_until_indexes_are_ready() {
    for value in ["ge2025-01-01", "lt2025-07-01", "ne2025-01-01"] {
        let parameter = date_parameter(value);
        for ready in [false, true] {
            let predicate = IndexPredicate::new(doc! {}, &parameter, ready);
            assert_eq!(
                predicate.hint(),
                ready.then(|| Hint::Name(SEARCH_DATE_RANGE_INDEX.into()))
            );
        }
        let mut presence = parameter;
        presence.modifier = Some(SearchModifier::Missing);
        assert_eq!(IndexPredicate::new(doc! {}, &presence, true).hint(), None);
    }
}

#[tokio::test]
async fn sparse_positive_date_pages_preserve_discovery_and_membership_access() {
    let Some(fixture) = Fixture::with_probe_timeout("sparse-date-access", 121_000, 10_000).await
    else {
        return;
    };
    // Both date predicates exceed the bounded planner's 10k cap. More than
    // 100k timestamp/ID-ordered candidates precede the first match, forcing
    // the capped-ID fallback after the ordered continuation.
    let recent = 10_500;
    add_dates(&fixture, recent).await;
    let mut expected: Vec<_> = fixture.rows[1..recent]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    // Six-digit suffixes do not sort numerically beside five-digit suffixes.
    expected.sort_by(|a, b| b.cmp(a));
    let ordered = fixture.expected(false, -1, 0, usize::MAX);
    assert!(
        ordered.iter().position(|id| id == &expected[0]).unwrap() >= RESOURCE_PAGE_SCAN_BUDGET,
        "the real index order must put matches beyond the continuation budget"
    );
    let index = fixture.db.collection::<Document>("search_index");
    let mut foreign = date_row("r15099", "2025-06-01T00:00:00Z");
    foreign.insert("tenant_id", "foreign");
    let mut wrong_type = date_row("r15098", "2025-06-01T00:00:00Z");
    wrong_type.insert("resource_type", "Patient");
    index
        .insert_many([
            foreign,
            wrong_type,
            date_row("r10499", "2025-06-01T00:00:00Z"),
        ])
        .await
        .unwrap();

    for total in [TotalMode::None, TotalMode::Accurate] {
        let mut query = SearchQuery::new("Observation")
            .with_parameter(date_parameter("ge2025-01-01"))
            .with_parameter(date_parameter("lt2025-07-01"))
            .with_sort(SortDirective::parse("_lastUpdated"))
            .with_count(20);
        query.total = Some(total);
        profile_start(&fixture.db).await;
        let first = fixture
            .backend
            .search(&fixture.tenant, &query)
            .await
            .unwrap();
        fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
        assert_eq!(result_ids(&first), expected[..20]);
        assert_eq!(
            first.total,
            (total == TotalMode::Accurate).then_some(expected.len() as u64)
        );
        let next_query = query
            .clone()
            .with_cursor(first.resources.page_info.next_cursor.unwrap());
        let next = fixture
            .backend
            .search(&fixture.tenant, &next_query)
            .await
            .unwrap();
        assert_eq!(result_ids(&next), expected[20..40]);
        let previous_query = query
            .clone()
            .with_cursor(next.resources.page_info.previous_cursor.unwrap());
        let previous = fixture
            .backend
            .search(&fixture.tenant, &previous_query)
            .await
            .unwrap();
        assert_eq!(result_ids(&previous), expected[..20]);
        fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();

        let probes = profile_rows(
            &fixture.db,
            doc! {
                "command.aggregate": "search_index", "command.hint": SEARCH_DATE_RANGE_INDEX,
            },
        )
        .await;
        assert!(
            !probes.is_empty(),
            "date probes must keep their access path"
        );
        let drivers = profile_rows(
            &fixture.db,
            doc! {
                "command.find": "search_index", "command.hint": SEARCH_DATE_RANGE_INDEX,
            },
        )
        .await;
        assert!(
            !drivers.is_empty(),
            "the sparse fallback must open a hinted date driver"
        );
        let resources_namespace = format!("{}.resources", fixture.db.name());
        let operations = profile_rows(
            &fixture.db,
            doc! { "$or": [
                { "command.aggregate": "search_index", "command.hint": SEARCH_COMPOSITE_INDEX },
                { "command.find": "search_index", "command.hint": SEARCH_DATE_RANGE_INDEX },
                { "ns": &resources_namespace, "op": "query" },
            ] },
        )
        .await;
        let mut legacy_membership = false;
        let mut legacy_page_fetched = false;
        let mut saw_legacy_batch = false;
        let mut membership_count = 0;
        for check in operations {
            let command = check.get_document("command").unwrap();
            if command.get_str("find") == Ok("search_index") {
                legacy_membership = true;
                continue;
            }
            // The large final ID filter makes MongoDB replace the whole
            // command with `$truncated`. Namespace and operation survive, so
            // they mark the page fetch without parsing truncated command text.
            if check.get_str("ns") == Ok(resources_namespace.as_str())
                && check.get_str("op") == Ok("query")
            {
                legacy_page_fetched |= legacy_membership;
                legacy_membership = false;
                continue;
            }
            membership_count += 1;
            let ids = candidate_ids(command);
            let limit = if legacy_membership {
                CANDIDATE_BATCH_SIZE
            } else {
                // Ordered continuation and the separate exact count use the
                // larger bounded batches; the ID-set collector still uses 512.
                COUNT_CANDIDATE_BATCH_ROWS
            };
            assert!(
                !ids.is_empty() && ids.len() <= limit,
                "total={total:?}, candidate count={}, limit={limit}",
                ids.len()
            );
            saw_legacy_batch |= legacy_membership && ids.len() == CANDIDATE_BATCH_SIZE;
            let keys = numeric(&check, "keysExamined");
            assert!(
                keys <= 4 * ids.len() as i64 + 10,
                "membership must seek candidate IDs: {check:?}"
            );
        }
        assert!(
            legacy_page_fetched,
            "the legacy collector must reach its page fetch"
        );
        assert!(
            !legacy_membership,
            "the legacy membership phase must be closed"
        );
        assert!(
            membership_count > 0,
            "the other date bound must check candidates"
        );
        assert!(
            saw_legacy_batch,
            "the fallback must check a full 512-ID batch"
        );
        assert!(
            profile_rows(&fixture.db, doc! { "command.distinct": "search_index" })
                .await
                .is_empty()
        );
        assert!(
            profile_rows(
                &fixture.db,
                doc! {
                    "command.hint": { "$exists": false },
                    "$or": [
                        { "command.find": "search_index" },
                        { "command.aggregate": "search_index" },
                    ],
                }
            )
            .await
            .is_empty()
        );
        assert!(
            !profile_rows(
                &fixture.db,
                doc! {
                    "command.find": "resources", "command.hint": RESOURCES_TYPE_SCAN_INDEX,
                }
            )
            .await
            .is_empty(),
            "the first resource batch must actually be sampled"
        );
    }
    // An empty ordered prefix stops once even an optimistic match rate cannot
    // fill the page within the budget, well before reading the whole budget,
    // even though this fixture contains substantially more live resources.
    let empty = ResourceBatchPlan {
        filters: vec![predicate("code", "never-present")],
        exclusions: Vec::new(),
        missing: Vec::new(),
    };
    let mut run = empty
        .start_page(
            &fixture.db,
            resource_filter(),
            None,
            doc! { "last_updated": -1, "id": -1 },
            0,
            21,
        )
        .await
        .unwrap();
    assert!(matches!(
        run.sample(&fixture.db).await.unwrap(),
        ResourcePageSample::Sparse { live_matches: 0 }
    ));
    assert!(
        run.try_finish_sparse_page(&fixture.db)
            .await
            .unwrap()
            .is_none()
    );
    // 256 + 512 + 1,024 + 2,048 still projects within the budget; the next
    // 4,096-row batch does not.
    assert_eq!(run.scan.candidates_checked, 7_936);
    assert!(run.scan.projected_past(RESOURCE_PAGE_SCAN_BUDGET));
    assert_eq!(run.scan.live_matches, 0);
    assert_eq!(run.scan.batch_rows, COUNT_CANDIDATE_BATCH_ROWS);
    assert!(!run.exhausted);
    drop(run);

    let date = date_parameter("ge2025-01-01");
    let predicate = fixture
        .backend
        .build_search_index_filter(PLAN_TENANT, "Observation", &date)
        .unwrap();
    assert!(predicate.contains_key("$or"));
    let candidates: HashSet<_> = fixture.rows[10_300..10_812]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    profile_start(&fixture.db).await;
    let actual = matching_index_ids(
        &index,
        &candidates,
        &predicate,
        Some(Hint::Name(SEARCH_COMPOSITE_INDEX.into())),
    )
    .await
    .unwrap();
    assert_eq!(
        actual,
        fixture.rows[10_300..recent]
            .iter()
            .map(|row| row.id.clone())
            .collect()
    );
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let checks = profile_rows(
        &fixture.db,
        doc! {
            "command.aggregate": "search_index", "command.hint": SEARCH_COMPOSITE_INDEX,
        },
    )
    .await;
    assert_eq!(checks.len(), 1);
    assert!(
        numeric(&checks[0], "keysExamined") < 2_100,
        "{:?}",
        checks[0]
    );
    // The ID-ordered control keeps the existing batched plan.
    let mut control = SearchQuery::new("Observation")
        .with_parameter(date_parameter("ge2025-01-01"))
        .with_parameter(date_parameter("lt2025-07-01"))
        .with_sort(SortDirective::parse("_id"))
        .with_count(20);
    control.total = Some(TotalMode::None);
    profile_start(&fixture.db).await;
    let result = fixture
        .backend
        .search(&fixture.tenant, &control)
        .await
        .unwrap();
    assert_eq!(
        result_ids(&result),
        fixture.rows[1..21]
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>()
    );
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert!(
        profile_rows(
            &fixture.db,
            doc! {
                "command.find": "search_index", "command.hint": SEARCH_DATE_RANGE_INDEX,
            }
        )
        .await
        .is_empty(),
        "_id control must not enter the legacy driver"
    );
    fixture.close().await;
}

#[tokio::test]
async fn legacy_date_discovery_retries_only_a_missing_date_index() {
    let Some(fixture) = Fixture::new("legacy-date-fallback", 700).await else {
        return;
    };
    add_dates(&fixture, 600).await;
    let expected: HashSet<_> = fixture.rows[..600]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let mut seed = parameter("_id", "");
    seed.values = fixture.rows[..650]
        .iter()
        .map(|row| SearchValue::eq(&row.id))
        .collect();
    let queries = [
        SearchQuery::new("Observation").with_parameter(date_parameter("ge2025-01-01")),
        SearchQuery::new("Observation")
            .with_parameter(date_parameter("ge2025-01-01"))
            .with_parameter(date_parameter("lt2025-07-01")),
        SearchQuery::new("Observation")
            .with_parameter(date_parameter("ge2025-01-01"))
            .with_parameter(seed),
    ];
    for dropped in [false, true] {
        if dropped {
            fixture
                .db
                .collection::<Document>("search_index")
                .drop_index(SEARCH_DATE_RANGE_INDEX)
                .await
                .unwrap();
        }
        profile_start(&fixture.db).await;
        for query in &queries {
            let actual = fixture
                .backend
                .matching_resource_ids(
                    &fixture.db,
                    PLAN_TENANT,
                    "Observation",
                    query,
                    &MissingSupport::none(),
                )
                .await
                .unwrap();
            assert_eq!(actual, Some(expected.clone()));
        }
        fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
        let rejected = profile_rows(
            &fixture.db,
            doc! { "command.hint": SEARCH_DATE_RANGE_INDEX, "errCode": 2 },
        )
        .await;
        let unhinted = profile_rows(&fixture.db, doc! {
            "command.hint": { "$exists": false },
            "$or": [{ "command.find": "search_index" }, { "command.aggregate": "search_index" }],
        }).await;
        assert_eq!(!rejected.is_empty(), dropped);
        assert_eq!(!unhinted.is_empty(), dropped);
    }
    fixture.close().await;
}

#[tokio::test]
async fn legacy_discovery_errors_and_required_membership_index_errors_propagate() {
    let Some(fixture) = Fixture::new("legacy-discovery-errors", 16).await else {
        return;
    };
    add_dates(&fixture, 12).await;
    let single = SearchQuery::new("Observation").with_parameter(date_parameter("ge2025-01-01"));
    let pair = single
        .clone()
        .with_parameter(date_parameter("lt2025-07-01"));
    let Some(session) = fixture.failpoints("legacy-discovery-errors").await else {
        fixture.close().await;
        return;
    };
    for (command, query) in [("find", &single), ("aggregate", &pair)] {
        let before = fixture
            .arm_error(&session, command, doc! { "times": 1 }, 50)
            .await;
        let result = fixture
            .backend
            .matching_resource_ids(
                &fixture.db,
                PLAN_TENANT,
                "Observation",
                query,
                &MissingSupport::none(),
            )
            .await;
        let after = disable_failpoint(&session).await;
        assert!(
            result.is_err(),
            "{command}: real errors must not be retried"
        );
        assert_eq!(failpoint_hits(&after) - failpoint_hits(&before), 1);
    }
    drop(session);
    fixture
        .db
        .collection::<Document>("search_index")
        .drop_index(SEARCH_COMPOSITE_INDEX)
        .await
        .unwrap();
    profile_start(&fixture.db).await;
    let error = fixture
        .backend
        .matching_resource_ids(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            &pair,
            &MissingSupport::none(),
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("hint provided does not correspond"),
        "{error}"
    );
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(profile_rows(&fixture.db, doc! {
        "command.aggregate": "search_index", "command.hint": SEARCH_COMPOSITE_INDEX, "errCode": 2,
    }).await.len(), 1);
    assert!(
        profile_rows(
            &fixture.db,
            doc! {
                "command.find": "search_index", "command.hint": { "$exists": false },
            }
        )
        .await
        .is_empty(),
        "required identity access must not retry unhinted"
    );
    fixture.close().await;
}

#[tokio::test]
async fn bounded_membership_preserves_types_multivalues_and_candidate_origins() {
    let Some(fixture) = Fixture::new("legacy-membership-types", 8).await else {
        return;
    };
    let index = fixture.db.collection::<Document>("search_index");
    let cases = [
        (
            "marker",
            SearchParamType::Token,
            "red",
            doc! { "value_token_code": "red" },
            doc! { "value_token_code": "blue" },
        ),
        (
            "label",
            SearchParamType::String,
            "alpha",
            doc! { "value_string": "alpha" },
            doc! { "value_string": "beta" },
        ),
        (
            "score",
            SearchParamType::Number,
            "gt10",
            doc! { "value_number": 20.0 },
            doc! { "value_number": 5.0 },
        ),
        (
            "amount",
            SearchParamType::Quantity,
            "gt10|http://unitsofmeasure.org|mg",
            doc! { "value_quantity_value": 20.0, "value_quantity_system": "http://unitsofmeasure.org", "value_quantity_unit": "mg" },
            doc! { "value_quantity_value": 5.0, "value_quantity_system": "http://unitsofmeasure.org", "value_quantity_unit": "mg" },
        ),
        (
            "link",
            SearchParamType::Uri,
            "http://example.org/selected",
            doc! { "value_uri": "http://example.org/selected" },
            doc! { "value_uri": "http://example.org/other" },
        ),
        (
            "patient",
            SearchParamType::Reference,
            "Patient/selected",
            doc! { "value_reference": "Patient/selected/_history/2" },
            doc! { "value_reference": "Patient/other" },
        ),
        (
            "date",
            SearchParamType::Date,
            "ge2025-01-01",
            doc! { "value_date": DateTime::parse_rfc3339_str("2025-06-01T00:00:00Z").unwrap(), "value_date_end": DateTime::parse_rfc3339_str("2025-06-02T00:00:00Z").unwrap() },
            doc! { "value_date": DateTime::parse_rfc3339_str("2024-06-01T00:00:00Z").unwrap(), "value_date_end": DateTime::parse_rfc3339_str("2024-06-02T00:00:00Z").unwrap() },
        ),
    ];
    let candidates: HashSet<_> = fixture.rows[1..7]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let expected: HashSet<_> = fixture.rows[1..4]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let driver = parameter("code", "base");
    for (name, kind, value, matching, other) in cases {
        let mut rows = Vec::new();
        for position in 1..7 {
            let mut row = doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "resource_id": &fixture.rows[position].id, "param_name": name };
            row.extend(if position == 4 {
                other.clone()
            } else {
                matching.clone()
            });
            if position == 5 {
                row.insert("tenant_id", "foreign");
            }
            if position == 6 {
                row.insert("resource_type", "Patient");
            }
            rows.push(row.clone());
            if position == 2 {
                rows.push(row);
            }
        }
        index.insert_many(rows).await.unwrap();
        let mut sibling = parameter(name, value);
        sibling.param_type = kind;
        sibling.values = vec![SearchValue::parse(value)];
        // Both origins use the same bounded set. A parameter driver skips
        // its proven code filter; an explicit ID seed checks that filter too.
        for skip in [Some(0), None] {
            profile_start(&fixture.db).await;
            let actual = fixture
                .backend
                .retain_matching_candidates(
                    &fixture.db,
                    &index,
                    PLAN_TENANT,
                    "Observation",
                    &[&driver, &sibling],
                    &HashMap::new(),
                    &[],
                    &[],
                    &MissingSupport::none(),
                    None,
                    skip,
                    candidates.clone(),
                )
                .await
                .unwrap();
            assert_eq!(actual, expected, "{name}, skip={skip:?}");
            fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
            let checks =
                profile_rows(&fixture.db, doc! { "command.aggregate": "search_index" }).await;
            assert_eq!(checks.len(), if skip.is_some() { 1 } else { 2 });
            for check in checks {
                let command = check.get_document("command").unwrap();
                assert!(!candidate_ids(command).is_empty());
                let predicate = command.get_array("pipeline").unwrap()[0]
                    .as_document()
                    .unwrap()
                    .get_document("$match")
                    .unwrap()
                    .get_array("$and")
                    .unwrap()[0]
                    .as_document()
                    .unwrap();
                let expected_hint = if predicate.get_str("param_name") == Ok("marker") {
                    None
                } else {
                    Some(SEARCH_COMPOSITE_INDEX)
                };
                assert_eq!(command.get_str("hint").ok(), expected_hint);
            }
            assert!(
                profile_rows(&fixture.db, doc! { "command.distinct": "search_index" })
                    .await
                    .is_empty()
            );
        }
    }
    let mut missing = parameter("score", "true");
    missing.param_type = SearchParamType::Number;
    missing.modifier = Some(SearchModifier::Missing);
    let mut excluded = parameter("marker", "red");
    excluded.modifier = Some(SearchModifier::Not);
    let compartment = crate::types::CompartmentMembership {
        params: vec!["patient".into()],
        reference: "Patient/selected".into(),
    };
    for skip in [Some(0), None] {
        let absent = fixture
            .backend
            .retain_matching_candidates(
                &fixture.db,
                &index,
                PLAN_TENANT,
                "Observation",
                &[&driver],
                &HashMap::new(),
                &[&missing],
                &[],
                &fixture.backend.classify_missing(
                    PLAN_TENANT,
                    "Observation",
                    &SearchQuery::new("Observation").with_parameter(missing.clone()),
                ),
                None,
                skip,
                candidates.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            absent,
            fixture.rows[5..7]
                .iter()
                .map(|row| row.id.clone())
                .collect()
        );
        let not_red = fixture
            .backend
            .retain_matching_candidates(
                &fixture.db,
                &index,
                PLAN_TENANT,
                "Observation",
                &[&driver],
                &HashMap::new(),
                &[],
                &[&excluded],
                &MissingSupport::none(),
                None,
                skip,
                candidates.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            not_red,
            fixture.rows[4..7]
                .iter()
                .map(|row| row.id.clone())
                .collect()
        );
        let in_compartment = fixture
            .backend
            .retain_matching_candidates(
                &fixture.db,
                &index,
                PLAN_TENANT,
                "Observation",
                &[&driver],
                &HashMap::new(),
                &[],
                &[],
                &MissingSupport::none(),
                Some(&compartment),
                skip,
                candidates.clone(),
            )
            .await
            .unwrap();
        assert_eq!(in_compartment, expected);
    }
    fixture.close().await;
}

#[tokio::test]
async fn legacy_token_membership_shares_named_system_fallback() {
    let Some(fixture) = Fixture::new("legacy-token-fallback", 8).await else {
        return;
    };
    let index = fixture.db.collection::<Document>("search_index");
    let candidates: HashSet<_> = fixture.rows[1..7]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let expected: HashSet<_> = fixture.rows[1..4]
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let driver = parameter("code", "base");
    let sibling = parameter("marker", "urn:marker|red");
    let mut rows = Vec::new();
    for position in 1..7 {
        let row = doc! {
            "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "resource_id": &fixture.rows[position].id, "param_name": "marker",
            "value_token_system": if position < 4 { "urn:marker" } else { "urn:other" },
            "value_token_code": "red",
        };
        rows.push(row.clone());
        if position == 2 {
            rows.push(row);
        }
    }
    index.insert_many(rows).await.unwrap();
    for dropped in [false, true] {
        if dropped {
            index
                .drop_index(SEARCH_TOKEN_MEMBERSHIP_INDEX)
                .await
                .unwrap();
        }
        for skip in [Some(0), None] {
            profile_start(&fixture.db).await;
            let actual = fixture
                .backend
                .retain_matching_candidates(
                    &fixture.db,
                    &index,
                    PLAN_TENANT,
                    "Observation",
                    &[&driver, &sibling],
                    &HashMap::new(),
                    &[],
                    &[],
                    &MissingSupport::none(),
                    None,
                    skip,
                    candidates.clone(),
                )
                .await
                .unwrap();
            assert_eq!(actual, expected);
            fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
            let value = profile_rows(
                &fixture.db,
                doc! {
                    "command.aggregate": "search_index",
                    "command.hint": SEARCH_TOKEN_MEMBERSHIP_INDEX,
                },
            )
            .await;
            assert_eq!(value.len(), 1);
            assert_eq!(value[0].get_i32("errCode").ok(), dropped.then_some(2));
            let identity = profile_rows(
                &fixture.db,
                doc! {
                    "command.aggregate": "search_index",
                    "command.hint": SEARCH_COMPOSITE_INDEX,
                },
            )
            .await;
            assert_eq!(
                identity.len(),
                usize::from(skip.is_none()) + usize::from(dropped)
            );
            for operation in value.iter().chain(&identity) {
                assert!(
                    candidate_ids(operation.get_document("command").unwrap())
                        .iter()
                        .all(|id| candidates.contains(id.as_str().unwrap()))
                );
            }
            assert!(
                profile_rows(
                    &fixture.db,
                    doc! {
                        "command.aggregate": "search_index",
                        "command.hint": { "$exists": false },
                    }
                )
                .await
                .is_empty(),
                "missing token index retries through candidate IDs only"
            );
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn membership_intersects_occurrences_without_requiring_one_matching_value() {
    let Some(fixture) = Fixture::new("membership-occurrences", 4).await else {
        return;
    };
    let index = fixture.db.collection::<Document>("search_index");
    index
        .insert_many([
            date_row("r00001", "2024-06-01T00:00:00Z"),
            date_row("r00001", "2026-06-01T00:00:00Z"),
            date_row("r00002", "2026-06-01T00:00:00Z"),
            date_row("r00003", "2024-06-01T00:00:00Z"),
        ])
        .await
        .unwrap();
    let driver = parameter("code", "base");
    let lower = date_parameter("ge2025-01-01");
    let upper = date_parameter("lt2025-07-01");
    let initial: HashSet<_> = fixture.rows[1..].iter().map(|row| row.id.clone()).collect();
    let expected = HashSet::from(["r00001".to_owned()]);
    for skip in [Some(0), None] {
        let actual = fixture
            .backend
            .retain_matching_candidates(
                &fixture.db,
                &index,
                PLAN_TENANT,
                "Observation",
                &[&driver, &lower, &upper],
                &HashMap::new(),
                &[],
                &[],
                &MissingSupport::none(),
                None,
                skip,
                initial.clone(),
            )
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }
    let predicates: Vec<_> = [&lower, &upper]
        .into_iter()
        .map(|parameter| {
            let filter = fixture
                .backend
                .build_search_index_filter(PLAN_TENANT, "Observation", parameter)
                .unwrap();
            IndexPredicate::new(filter, parameter, true)
        })
        .collect();
    let filters = predicates.iter().collect::<Vec<_>>();
    for source in [
        CandidateSource::Unfiltered,
        CandidateSource::IndexedPredicate,
    ] {
        let mut actual = initial.clone();
        filter_candidates(&fixture.db, &mut actual, &filters, source)
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }
    fixture.close().await;
}
