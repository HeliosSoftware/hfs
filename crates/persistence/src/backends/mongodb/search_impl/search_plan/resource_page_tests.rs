//! Resource-page contracts with independent fixture expectations and work counts.

use super::*;
use crate::backends::mongodb::schema::RESOURCES_TYPE_SCAN_INDEX;
use std::collections::BTreeMap;

#[test]
fn only_bounded_missing_pages_override_the_timestamp_hint() {
    for (missing, bounded, page, expected) in [
        (true, true, true, Some("idx_resources_identity")),
        (true, false, true, Some("idx_resources_missing_params")),
        (false, true, true, None),
        (true, true, false, Some("idx_resources_missing_params")),
        (false, true, false, None),
    ] {
        let mut scope = resource_filter();
        if missing {
            scope.insert("missing_params", "method");
        }
        let plan = if bounded {
            SearchFilterPlan::CandidateIds(vec!["selected".into()])
        } else {
            SearchFilterPlan::ResourceLookups {
                index_filter: None,
                stages: vec![],
            }
        };
        let pipeline = plan
            .pipeline(
                scope,
                page.then(|| doc! { "last_updated": -1, "id": -1 }),
                None,
            )
            .unwrap();
        assert_eq!(
            pipeline.hint,
            expected.map(|name| Hint::Name(name.into())),
            "missing={missing}, bounded={bounded}, page={page}"
        );
    }
}

#[tokio::test]
async fn missing_membership_work_is_bounded_by_candidate_ids() {
    let Some(mut fixture) = Fixture::new("bounded-missing-membership", 4096).await else {
        return;
    };
    fixture.mark_deleted(2048, 2049).await;
    let db = &fixture.db;
    let resources = db.collection::<Document>("resources");
    let index = db.collection::<Document>("search_index");
    // Presence is independent of metadata generation and of the positive driver.
    let present: Vec<_> = fixture
        .rows
        .iter()
        .enumerate()
        .filter(|(position, _)| position % 3 == 0)
        .map(|(_, row)| row.id.clone())
        .collect();
    resources
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &present } },
            doc! { "$set": { "data.method": { "coding": [{ "code": "measured" }] } } },
        )
        .await
        .unwrap();
    index
        .insert_many(present.iter().map(|id| index_row(id, "method", "measured")))
        .await
        .unwrap();
    let selected: Vec<_> = fixture
        .rows
        .iter()
        .skip(2048)
        .step_by(7)
        .take(129)
        .map(|row| row.id.clone())
        .collect();
    let orphan = "orphan".to_owned();
    for size in [65, 129] {
        let tag = format!("selected-{size}");
        index
            .insert_many(
                selected[..size]
                    .iter()
                    .chain([&selected[0], &orphan])
                    .map(|id| index_row(id, "code", &tag)),
            )
            .await
            .unwrap();
    }
    index
        .insert_many(selected.iter().chain([&selected[0], &orphan]).map(|id| {
            doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
            "resource_id": id, "param_name": "patient",
            "value_reference": "Patient/selected" }
        }))
        .await
        .unwrap();
    resources
        .insert_many([
            doc! { "tenant_id": "foreign", "resource_type": "Observation", "id": "orphan",
            "is_deleted": false, "missing_params": ["method"] },
            doc! { "tenant_id": PLAN_TENANT, "resource_type": "Patient", "id": "orphan",
            "is_deleted": false, "missing_params": ["method"] },
        ])
        .await
        .unwrap();
    resources
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &selected } },
            doc! { "$set": { "data.subject.reference": "Patient/selected" } },
        )
        .await
        .unwrap();
    let mut absent = parameter("method", "true");
    absent.modifier = Some(SearchModifier::Missing);
    let missing = [missing_presence_filter(PLAN_TENANT, "Observation", &absent)];
    for generation in ["old", "mixed", "new"] {
        resources
            .update_many(
                doc! { "tenant_id": PLAN_TENANT },
                doc! { "$unset": { "missing_params": "" } },
            )
            .await
            .unwrap();
        let known: HashSet<_> = fixture
            .rows
            .iter()
            .enumerate()
            .filter(|(position, _)| {
                generation == "new" || (generation == "mixed" && position % 2 == 0)
            })
            .map(|(_, row)| row.id.clone())
            .collect();
        for is_present in [false, true] {
            let ids: Vec<_> = known
                .iter()
                .filter(|id| present.contains(id) == is_present)
                .cloned()
                .collect();
            if !ids.is_empty() {
                let names: Vec<&str> = if is_present { vec![] } else { vec!["method"] };
                resources
                    .update_many(
                        doc! { "tenant_id": PLAN_TENANT, "id": { "$in": ids } },
                        doc! { "$set": { "missing_params": names } },
                    )
                    .await
                    .unwrap();
            }
        }
        trust_missing_metadata(&fixture.backend, db).await;
        for (size, reference) in [(65, false), (129, false), (129, true)] {
            let positive = if reference {
                let mut patient = parameter("patient", "Patient/selected");
                patient.param_type = SearchParamType::Reference;
                patient
            } else {
                parameter("code", &format!("selected-{size}"))
            };
            let mut query = SearchQuery::new("Observation")
                .with_count(20)
                .with_parameter(positive.clone())
                .with_parameter(absent.clone());
            query.total = Some(TotalMode::None);
            let predicate = fixture.backend.planning_predicate(
                db,
                PLAN_TENANT,
                "Observation",
                &positive,
                fixture
                    .backend
                    .build_search_index_filter(PLAN_TENANT, "Observation", &positive)
                    .unwrap(),
                true,
            );
            fixture.backend.probe_cache.insert(
                predicate.probe_key.unwrap(),
                ProbeEstimate::Completed(size as u64),
            );
            let expected: HashSet<_> = selected[..size]
                .iter()
                .filter(|id| {
                    !present.contains(id)
                        && fixture
                            .rows
                            .iter()
                            .any(|row| row.id == **id && !row.deleted)
                })
                .cloned()
                .collect();
            for sort in [None, Some("_id"), Some("-_id")] {
                query.sort = sort.map(SortDirective::parse).into_iter().collect();
                start_profiling(db).await;
                let plan = fixture
                    .backend
                    .resource_filter_plan(
                        db,
                        PLAN_TENANT,
                        &query,
                        SearchFilterPurpose::Page,
                        NoTotalPagePolicy::Streaming,
                        None,
                        &mut ProbeFacts::default(),
                    )
                    .await
                    .unwrap();
                let Some(SearchFilterPlan::CandidateIds(ids)) = plan else {
                    panic!(
                        "{generation}/{size}/{reference}/{sort:?}: positive driver must reach CandidateIds after the 64-row probe"
                    );
                };
                assert_eq!(ids.into_iter().collect::<HashSet<_>>(), expected);
                assert_bounded_missing_resource_checks(db, size + 1, 2).await;
                db.run_command(doc! { "profile": 0 }).await.unwrap();
                if sort.is_none() {
                    start_profiling(db).await;
                    let result = fixture
                        .backend
                        .search(&fixture.tenant, &query)
                        .await
                        .unwrap();
                    db.run_command(doc! { "profile": 0 }).await.unwrap();
                    let mut ordered: Vec<_> = expected.iter().cloned().collect();
                    ordered.sort_by(|a, b| b.cmp(a));
                    assert_eq!(result_ids(&result), ordered[..20]);
                    let fetches = profiled_operations(
                        db,
                        doc! { "$or": [
                            { "command.aggregate": "resources" },
                            { "originatingCommand.aggregate": "resources" },
                        ] },
                    )
                    .await;
                    for operation in &fetches {
                        let command = operation
                            .get_document("originatingCommand")
                            .or_else(|_| operation.get_document("command"))
                            .unwrap();
                        assert_eq!(command.get_str("hint"), Ok("idx_resources_identity"));
                    }
                    assert!(
                        profile_sum(&fetches, "keysExamined") <= (2 * size + 2) as i64,
                        "final timestamp fetch must seek candidate IDs: {fetches:?}"
                    );
                    assert!(
                        profile_sum(&fetches, "docsExamined") <= size as i64,
                        "final timestamp sort must be bounded by candidate IDs: {fetches:?}"
                    );
                }
                if let Some(sort) = sort {
                    let mut ordered: Vec<_> = expected.iter().cloned().collect();
                    ordered.sort();
                    if sort == "-_id" {
                        ordered.reverse();
                    }
                    assert_missing_id_navigation(&fixture, &query, &ordered).await;
                    assert!(matches!(
                        fixture
                            .backend
                            .resource_filter_plan(
                                db,
                                PLAN_TENANT,
                                &query,
                                SearchFilterPurpose::Count,
                                NoTotalPagePolicy::Streaming,
                                None,
                                &mut ProbeFacts::default(),
                            )
                            .await
                            .unwrap(),
                        Some(SearchFilterPlan::ResourceLookups {
                            index_filter: None,
                            ..
                        })
                    ));
                    if reference {
                        let mut excluded = parameter("status", "final");
                        excluded.modifier = Some(SearchModifier::Not);
                        let repeated = query
                            .clone()
                            .with_parameter(positive.clone())
                            .with_parameter(absent.clone())
                            .with_parameter(excluded);
                        let expected: Vec<_> = ordered
                            .into_iter()
                            .filter(|id| {
                                fixture
                                    .rows
                                    .iter()
                                    .any(|row| row.id == *id && row.status != "final")
                            })
                            .collect();
                        assert_missing_id_navigation(&fixture, &repeated, &expected).await;
                    }
                }
            }

            // ResourceBatches has already applied the metadata condition, but
            // old candidates still need typed-presence validation.
            let scoped: Vec<_> = selected[..size]
                .iter()
                .filter(|id| {
                    (!known.contains(*id) || !present.contains(id))
                        && fixture
                            .rows
                            .iter()
                            .any(|row| row.id == **id && !row.deleted)
                })
                .cloned()
                .collect();
            let filters = [IndexPredicate::from(
                fixture
                    .backend
                    .build_search_index_filter(PLAN_TENANT, "Observation", &positive)
                    .unwrap(),
            )];
            let mut scan = ResourcePageScan::new(&filters, &[], &missing, 0, size);
            scan.batch = scoped.clone();
            start_profiling(db).await;
            scan.match_batch(db).await.unwrap();
            assert_eq!(scan.ids.into_iter().collect::<HashSet<_>>(), expected);
            assert_bounded_missing_resource_checks(db, scoped.len(), 1).await;
        }
    }
    start_profiling(db).await;
    filter_missing_candidates(db, &mut HashSet::new(), &missing, false)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("system.profile")
            .count_documents(doc! { "command.find": "resources" })
            .await
            .unwrap(),
        0
    );
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    fixture.close().await;
}

async fn assert_missing_id_navigation(fixture: &Fixture, query: &SearchQuery, expected: &[String]) {
    let first = fixture
        .backend
        .search(&fixture.tenant, query)
        .await
        .unwrap();
    assert_eq!(result_ids(&first), expected[..expected.len().min(20)]);
    assert_eq!(first.total, None);
    let mut offset_query = query.clone();
    offset_query.offset = Some(20);
    let offset = fixture
        .backend
        .search(&fixture.tenant, &offset_query)
        .await
        .unwrap();
    assert_eq!(
        result_ids(&offset),
        expected
            .iter()
            .skip(20)
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
    );
    if expected.len() > 20 {
        let next = fixture
            .backend
            .search(
                &fixture.tenant,
                &query
                    .clone()
                    .with_cursor(first.resources.page_info.next_cursor.unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(result_ids(&next), result_ids(&offset));
        let previous = fixture
            .backend
            .search(
                &fixture.tenant,
                &query
                    .clone()
                    .with_cursor(next.resources.page_info.previous_cursor.unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(result_ids(&previous), expected[..20]);
    } else {
        assert!(first.resources.page_info.next_cursor.is_none());
    }
}

#[tokio::test]
async fn missing_id_pages_preserve_small_count_and_broad_fallbacks() {
    let Some(fixture) = Fixture::new("missing-id-fallbacks", 129).await else {
        return;
    };
    let db = &fixture.db;
    let backend = &fixture.backend;
    let index = db.collection::<Document>("search_index");
    index
        .insert_many(
            fixture.rows[..64]
                .iter()
                .map(|row| index_row(&row.id, "code", "small")),
        )
        .await
        .unwrap();
    // A stale small estimate must not truncate when the actual read crosses its row cap.
    index
        .insert_many(
            (0..=MAX_CANDIDATE_ROWS).map(|_| index_row(&fixture.rows[1].id, "code", "grown")),
        )
        .await
        .unwrap();
    db.collection::<Document>("resources")
        .update_many(
            resource_filter(),
            doc! { "$set": { "missing_params": ["method"] } },
        )
        .await
        .unwrap();
    trust_missing_metadata(backend, db).await;
    let mut absent = parameter("method", "true");
    absent.modifier = Some(SearchModifier::Missing);
    let mut query = SearchQuery::new("Observation")
        .with_parameter(parameter("code", "base"))
        .with_parameter(absent.clone())
        .with_sort(SortDirective::parse("_id"))
        .with_count(20);
    query.total = Some(TotalMode::None);
    for (code, observed) in [
        ("base", Some(MAX_CANDIDATE_ROWS + 1)),
        ("base", None),
        ("grown", Some(1)),
    ] {
        query.parameters[0] = parameter("code", code);
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
        let mut facts = ProbeFacts::default();
        facts.remember(key, observed);
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                None,
                &mut facts,
            )
            .await
            .unwrap();
        let Some(SearchFilterPlan::ResourceBatches(plan)) = plan else {
            panic!("{code}/{observed:?}: fallback must batch every predicate");
        };
        assert_eq!(facts.reuses, 1);
        let filter = backend
            .build_resource_filter(PLAN_TENANT, "Observation", &query, None, None)
            .unwrap();
        let actual = plan
            .execute_page(db, filter, None, doc! { "id": 1 }, 0, 21)
            .await
            .unwrap();
        let expected = if code == "grown" {
            vec![fixture.rows[1].id.clone()]
        } else {
            fixture.expected(false, 1, 0, 21)
        };
        assert_eq!(actual, expected);
    }
    for (code, purpose, small) in [
        ("small", SearchFilterPurpose::Page, true),
        ("base", SearchFilterPurpose::Count, false),
    ] {
        query.parameters[0] = parameter("code", code);
        let mut facts = ProbeFacts::default();
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                purpose,
                NoTotalPagePolicy::Streaming,
                None,
                &mut facts,
            )
            .await
            .unwrap();
        if small {
            assert!(plan.is_none());
        } else {
            assert!(matches!(
                plan,
                Some(SearchFilterPlan::ResourceLookups {
                    index_filter: None,
                    ..
                })
            ));
        }
        assert_eq!(
            facts.requests, 1,
            "small and count routes must not add a bounded probe"
        );
        let (selected, ids) = backend
            .plan_or_matching_ids(db, PLAN_TENANT, &query, plan)
            .await
            .unwrap();
        if small {
            assert!(selected.is_none());
            assert!(
                ids.is_some(),
                "a missing-aware query without a plan still resolves its candidate IDs"
            );
        } else {
            assert!(matches!(
                selected,
                Some(SearchFilterPlan::ResourceLookups {
                    index_filter: None,
                    ..
                })
            ));
            assert!(
                ids.is_none(),
                "the metadata count plan must not materialize a full ID set"
            );
        }
    }
    query.parameters = vec![absent];
    for metadata in [true, false] {
        if !metadata {
            db.collection::<Document>("resources")
                .update_many(
                    resource_filter(),
                    doc! { "$unset": { "missing_params": "" } },
                )
                .await
                .unwrap();
        }
        let mut facts = ProbeFacts::default();
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                None,
                &mut facts,
            )
            .await
            .unwrap();
        assert!(
            matches!(
                plan,
                Some(SearchFilterPlan::ResourceLookups {
                    index_filter: None,
                    ..
                })
            ),
            "pure missing pages keep their existing route, metadata={metadata}"
        );
        assert_eq!(
            facts.requests, 0,
            "pure missing pages must not acquire a probe"
        );
        let (selected, ids) = backend
            .plan_or_matching_ids(db, PLAN_TENANT, &query, plan)
            .await
            .unwrap();
        assert!(matches!(
            selected,
            Some(SearchFilterPlan::ResourceLookups {
                index_filter: None,
                ..
            })
        ));
        assert!(
            ids.is_none(),
            "metadata pages retain their executor for both old and new resources"
        );
        let result = backend.search(&fixture.tenant, &query).await.unwrap();
        assert_eq!(result_ids(&result), fixture.expected(false, 1, 0, 20));
    }
    for complement in [false, true] {
        query.parameters = vec![parameter("code", "base")];
        if complement {
            let mut excluded = parameter("code", "excluded");
            excluded.modifier = Some(SearchModifier::Not);
            query.parameters.push(excluded);
        }
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                None,
                &mut ProbeFacts::default(),
            )
            .await
            .unwrap();
        if complement {
            assert!(matches!(
                plan,
                Some(SearchFilterPlan::ResourceLookups {
                    index_filter: Some(_),
                    ..
                })
            ));
        } else {
            assert!(matches!(plan, Some(SearchFilterPlan::BatchedIndex(_))));
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn missing_id_cursor_candidates_bound_probes_reads_and_cache_keys() {
    use super::super::super::CursorKeyset;
    use crate::types::PageCursor;

    let Some(fixture) = Fixture::new("missing-id-cursor", 10_050).await else {
        return;
    };
    let backend = &fixture.backend;
    let db = &fixture.db;
    // Half the resources retain the pre-metadata representation.
    let known: Vec<_> = fixture
        .rows
        .iter()
        .step_by(2)
        .map(|row| row.id.clone())
        .collect();
    db.collection::<Document>("resources")
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": known } },
            doc! { "$set": { "missing_params": ["method"] } },
        )
        .await
        .unwrap();
    trust_missing_metadata(backend, db).await;
    let positive = parameter("code", "base");
    let filter = backend
        .build_search_index_filter(PLAN_TENANT, "Observation", &positive)
        .unwrap();
    let global = backend.planning_predicate(
        db,
        PLAN_TENANT,
        "Observation",
        &positive,
        filter.clone(),
        true,
    );
    let global_key = global.probe_key.unwrap();
    backend.probe_cache.insert(
        global_key.clone(),
        ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1),
    );
    let mut absent = parameter("method", "true");
    absent.modifier = Some(SearchModifier::Missing);
    let mut boundary_keys = Vec::new();
    for (sort, previous, position) in [
        ("_id", true, 90),
        ("_id", false, 9960),
        ("-_id", true, 9960),
        ("-_id", false, 90),
    ] {
        let boundary_id = &fixture.rows[position].id;
        let cursor = if previous {
            PageCursor::previous(vec![boundary_id.clone().into()], boundary_id)
        } else {
            PageCursor::new(vec![boundary_id.clone().into()], boundary_id)
        };
        let mut query = SearchQuery::new("Observation")
            .with_parameter(positive.clone())
            .with_parameter(absent.clone())
            .with_sort(SortDirective::parse(sort))
            .with_count(20)
            .with_cursor(cursor.encode());
        query.total = Some(TotalMode::None);
        let keyset = CursorKeyset::for_query(&query).unwrap();
        let boundary = backend
            .build_index_cursor_filter(Some((&cursor, &keyset)))
            .unwrap()
            .unwrap();
        let bounded_filter = doc! { "$and": [filter.clone(), boundary.clone()] };
        let bounded = backend.planning_predicate(
            db,
            PLAN_TENANT,
            "Observation",
            &positive,
            bounded_filter.clone(),
            true,
        );
        let key = bounded.probe_key.unwrap();
        assert_ne!(
            key, global_key,
            "bounded observations must not replace the global estimate"
        );
        let direction = if sort == "_id" { 1 } else { -1 };
        let ordered = fixture.expected(false, direction, 0, usize::MAX);
        let at = ordered.iter().position(|id| id == boundary_id).unwrap();
        let matching = if previous {
            &ordered[..at]
        } else {
            &ordered[at + 1..]
        };
        let expected_page = if previous {
            matching[matching.len().saturating_sub(20)..].to_vec()
        } else {
            matching[..matching.len().min(20)].to_vec()
        };
        backend
            .probe_cache
            .insert(key.clone(), ProbeEstimate::Completed(matching.len() as u64));
        boundary_keys.push(key);
        let plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Page,
                NoTotalPagePolicy::Streaming,
                Some(&boundary),
                &mut ProbeFacts::default(),
            )
            .await
            .unwrap();
        let Some(SearchFilterPlan::CandidateIds(ids)) = plan else {
            panic!(
                "{sort}/{previous}: the cursor slice, not the global population, must choose candidates"
            );
        };
        assert_eq!(
            ids.into_iter().collect::<HashSet<_>>(),
            matching.iter().cloned().collect()
        );
        start_profiling(db).await;
        let result = backend.search(&fixture.tenant, &query).await.unwrap();
        assert_eq!(result_ids(&result), expected_page, "{sort}/{previous}");
        let reads = profiled_operations(
            db,
            doc! {
                "command.find": "search_index", "command.filter": { "$eq": bounded_filter },
            },
        )
        .await;
        assert_eq!(reads.len(), 1, "the page must read the cursor-bounded seed");
        assert!(profile_sum(&reads, "keysExamined") <= 200, "{reads:?}");
        db.run_command(doc! { "profile": 0 }).await.unwrap();
        let count_plan = backend
            .resource_filter_plan(
                db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Count,
                NoTotalPagePolicy::Streaming,
                Some(&boundary),
                &mut ProbeFacts::default(),
            )
            .await
            .unwrap();
        assert!(matches!(
            count_plan,
            Some(SearchFilterPlan::ResourceLookups {
                index_filter: None,
                ..
            })
        ));
    }
    assert_ne!(boundary_keys[0], boundary_keys[1]);
    assert_eq!(
        boundary_keys[0], boundary_keys[3],
        "equal ID ranges may reuse observations across sort directions"
    );
    assert_eq!(boundary_keys[1], boundary_keys[2]);
    assert_eq!(
        backend.probe_cache.get(&global_key),
        Some(ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1))
    );
    fixture.close().await;
}

// Force the existing broad observation, including each cursor's own cache key,
// so navigation exercises batch execution rather than the small-candidate path.
fn force_missing_id_batches(fixture: &Fixture, query: &SearchQuery) {
    use super::super::super::CursorKeyset;
    use crate::types::PageCursor;

    let cursor = query
        .cursor
        .as_ref()
        .map(|value| PageCursor::decode(value).unwrap());
    let keyset = CursorKeyset::for_query(query);
    let boundary = fixture
        .backend
        .build_index_cursor_filter(cursor.as_ref().zip(keyset.as_ref()))
        .unwrap();
    for param in &query.parameters {
        if matches!(
            param.modifier,
            Some(SearchModifier::Missing | SearchModifier::Not)
        ) {
            continue;
        }
        let filter = fixture
            .backend
            .build_search_index_filter(PLAN_TENANT, "Observation", param)
            .unwrap();
        let filter = match &boundary {
            Some(boundary) => doc! { "$and": [filter, boundary.clone()] },
            None => filter,
        };
        let predicate = fixture.backend.planning_predicate(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            param,
            filter,
            true,
        );
        fixture.backend.probe_cache.insert(
            predicate.probe_key.unwrap(),
            ProbeEstimate::AtLeast(MAX_CANDIDATE_ROWS + 1),
        );
    }
}

#[tokio::test]
async fn missing_id_batches_navigate_fifty_pages_with_independent_date_bounds() {
    use crate::backends::mongodb::schema::RESOURCES_IDENTITY_INDEX;

    let Some(mut fixture) = Fixture::new("missing-id-fifty-pages", 4096).await else {
        return;
    };
    fixture.mark_deleted(64, 68).await;
    let db = &fixture.db;
    let resources = db.collection::<Document>("resources");
    let index = db.collection::<Document>("search_index");
    let date_groups: [&[&str]; 4] = [
        &["2026-06-15T12:00:00Z"],
        &["2026-08-15T12:00:00Z"],
        &["2026-10-15T12:00:00Z"],
        // Separate repeated values may satisfy the two date conditions.
        &["2026-06-15T12:00:00Z", "2026-10-15T12:00:00Z"],
    ];
    for (group, events) in date_groups.iter().enumerate() {
        let ids: Vec<_> = fixture
            .rows
            .iter()
            .enumerate()
            .filter(|(position, _)| position % 4 == group)
            .map(|(_, row)| row.id.clone())
            .collect();
        resources
            .update_many(
                doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &ids } },
                doc! { "$set": { "data.effectiveTiming.event": events.to_vec() } },
            )
            .await
            .unwrap();
        let rows: Vec<_> = ids
            .iter()
            .flat_map(|id| {
                events.iter().map(move |event| {
                    let start = DateTime::parse_rfc3339_str(event).unwrap();
                    doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation",
                    "resource_id": id, "param_name": "date", "value_date": start,
                    "value_date_end": DateTime::from_millis(start.timestamp_millis() + 1000) }
                })
            })
            .collect();
        index.insert_many(rows).await.unwrap();
    }
    let present: Vec<_> = fixture
        .rows
        .iter()
        .enumerate()
        .filter(|(position, _)| position % 5 == 0)
        .map(|(_, row)| row.id.clone())
        .collect();
    resources
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &present } },
            doc! { "$set": { "data.method.coding": [{ "code": "measured" }] } },
        )
        .await
        .unwrap();
    index
        .insert_many(present.iter().map(|id| index_row(id, "method", "measured")))
        .await
        .unwrap();
    let excluded: Vec<_> = fixture
        .rows
        .iter()
        .step_by(11)
        .map(|row| row.id.clone())
        .collect();
    resources
        .update_many(
            doc! { "tenant_id": PLAN_TENANT, "id": { "$in": &excluded } },
            doc! { "$push": { "data.code.coding": { "code": "excluded" } } },
        )
        .await
        .unwrap();
    index
        .insert_many(excluded.iter().map(|id| index_row(id, "code", "excluded")))
        .await
        .unwrap();
    let bodies: Vec<Document> = resources
        .find(resource_filter())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut expected: Vec<_> = bodies
        .iter()
        .filter(|row| {
            let body = row.get_document("data").unwrap();
            let events = body
                .get_document("effectiveTiming")
                .unwrap()
                .get_array("event")
                .unwrap();
            !body.contains_key("method")
                && !body
                    .get_document("code")
                    .unwrap()
                    .get_array("coding")
                    .unwrap()
                    .iter()
                    .any(|coding| coding.as_document().unwrap().get_str("code") == Ok("excluded"))
                && events
                    .iter()
                    .any(|value| value.as_str().unwrap() >= "2026-08-01")
                && events
                    .iter()
                    .any(|value| value.as_str().unwrap() < "2026-09-01")
        })
        .map(|row| row.get_str("id").unwrap().to_owned())
        .collect();
    expected.sort();
    assert!(
        expected.len() > 1000,
        "fixture must have more than fifty full pages"
    );
    // Colliding IDs in other scopes cannot replace a deleted local resource.
    for (tenant, resource_type) in [("foreign", "Observation"), (PLAN_TENANT, "Patient")] {
        let mut row = bodies[0].clone();
        row.remove("_id");
        row.insert("tenant_id", tenant);
        row.insert("resource_type", resource_type);
        row.insert("id", "r00000");
        row.insert("missing_params", vec!["method"]);
        resources.insert_one(row).await.unwrap();
    }
    let mut lower = parameter("date", "2026-08-01");
    lower.param_type = SearchParamType::Date;
    lower.values[0].prefix = SearchPrefix::Ge;
    let mut upper = parameter("date", "2026-09-01");
    upper.param_type = SearchParamType::Date;
    upper.values[0].prefix = SearchPrefix::Lt;
    let mut absent = parameter("method", "true");
    absent.modifier = Some(SearchModifier::Missing);
    let mut not_excluded = parameter("code", "excluded");
    not_excluded.modifier = Some(SearchModifier::Not);
    let mut base = SearchQuery::new("Observation")
        .with_parameter(lower)
        .with_parameter(upper)
        .with_parameter(absent)
        .with_parameter(not_excluded)
        .with_count(20);
    base.total = Some(TotalMode::None);

    for generation in ["old", "mixed", "new"] {
        resources
            .update_many(
                resource_filter(),
                doc! { "$unset": { "missing_params": "" } },
            )
            .await
            .unwrap();
        for is_present in [false, true] {
            let ids: Vec<_> = fixture
                .rows
                .iter()
                .enumerate()
                .filter(|(position, _)| {
                    (position % 5 == 0) == is_present
                        && (generation == "new" || (generation == "mixed" && position % 2 == 0))
                })
                .map(|(_, row)| row.id.clone())
                .collect();
            if !ids.is_empty() {
                let missing = if is_present {
                    Vec::<&str>::new()
                } else {
                    vec!["method"]
                };
                resources.update_many(
                    doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation", "id": { "$in": ids } },
                    doc! { "$set": { "missing_params": missing } },
                ).await.unwrap();
            }
        }
        trust_missing_metadata(&fixture.backend, &fixture.db).await;
        for direction in ["_id", "-_id"] {
            let mut ordered = expected.clone();
            if direction == "-_id" {
                ordered.reverse();
            }
            let query = base.clone().with_sort(SortDirective::parse(direction));
            force_missing_id_batches(&fixture, &query);
            assert!(matches!(
                fixture
                    .backend
                    .resource_filter_plan(
                        db,
                        PLAN_TENANT,
                        &query,
                        SearchFilterPurpose::Page,
                        NoTotalPagePolicy::Streaming,
                        None,
                        &mut ProbeFacts::default(),
                    )
                    .await
                    .unwrap(),
                Some(SearchFilterPlan::ResourceBatches(_))
            ));
            let count_plan = fixture
                .backend
                .resource_filter_plan(
                    db,
                    PLAN_TENANT,
                    &query,
                    SearchFilterPurpose::Count,
                    NoTotalPagePolicy::Streaming,
                    None,
                    &mut ProbeFacts::default(),
                )
                .await
                .unwrap();
            assert!(matches!(
                count_plan,
                Some(SearchFilterPlan::ResourceLookups {
                    index_filter: None,
                    ..
                })
            ));

            let mut pages = Vec::new();
            let mut request = query.clone();
            for page in 0..50 {
                force_missing_id_batches(&fixture, &request);
                let result = fixture
                    .backend
                    .search(&fixture.tenant, &request)
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&result),
                    ordered[page * 20..(page + 1) * 20],
                    "generation={generation}, direction={direction}, forward page={}",
                    page + 1
                );
                assert_eq!(result.total, None);
                request = query
                    .clone()
                    .with_cursor(result.resources.page_info.next_cursor.clone().unwrap());
                pages.push(result.resources.page_info);
            }
            request = query
                .clone()
                .with_cursor(pages[49].previous_cursor.clone().unwrap());
            for page in (0..49).rev() {
                let profile = generation == "new" && page == 38;
                if profile {
                    start_profiling(db).await;
                }
                force_missing_id_batches(&fixture, &request);
                let result = fixture
                    .backend
                    .search(&fixture.tenant, &request)
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&result),
                    ordered[page * 20..(page + 1) * 20],
                    "generation={generation}, direction={direction}, backward page={}",
                    page + 1
                );
                if profile {
                    db.run_command(doc! { "profile": 0 }).await.unwrap();
                    let reads = profiled_operations(db, doc! { "$or": [
                        { "command.find": "resources", "command.hint": RESOURCES_IDENTITY_INDEX, "command.sort.id": { "$exists": true } },
                        { "originatingCommand.find": "resources", "originatingCommand.hint": RESOURCES_IDENTITY_INDEX, "originatingCommand.sort.id": { "$exists": true } },
                    ] }).await;
                    assert!(
                        profile_sum(&reads, "keysExamined")
                            <= (4 * PAGE_CANDIDATE_BATCH_ROWS) as i64,
                        "cursor must seek the current page instead of scanning from the beginning: {reads:?}"
                    );
                    assert!(
                        profile_sum(&reads, "docsExamined") > 0,
                        "the identity index does not cover live and metadata checks"
                    );
                    let checks = profiled_operations(db, doc! {
                        "command.aggregate": "search_index", "command.hint": SEARCH_COMPOSITE_INDEX,
                    }).await;
                    assert_bounded_membership_checks(&checks, false);
                    assert!(
                        checks.len() <= 3,
                        "date checks must be per batch, not per resource: {checks:?}"
                    );
                }
                if page > 0 {
                    request = query
                        .clone()
                        .with_cursor(result.resources.page_info.previous_cursor.unwrap());
                }
            }
            for offset in [380, 580, 780, 980] {
                let mut request = query.clone();
                request.offset = Some(offset);
                force_missing_id_batches(&fixture, &request);
                let result = fixture
                    .backend
                    .search(&fixture.tenant, &request)
                    .await
                    .unwrap();
                assert_eq!(
                    result_ids(&result),
                    ordered[offset as usize..offset as usize + 20]
                );
            }
            for mode in [TotalMode::Accurate, TotalMode::Estimate] {
                let mut request = query.clone();
                request.total = Some(mode);
                force_missing_id_batches(&fixture, &request);
                let result = fixture
                    .backend
                    .search(&fixture.tenant, &request)
                    .await
                    .unwrap();
                assert_eq!(result_ids(&result), ordered[..20]);
                assert_eq!(result.total, Some(ordered.len() as u64));
                assert_eq!(
                    fixture
                        .backend
                        .search_count(&fixture.tenant, &request)
                        .await
                        .unwrap(),
                    ordered.len() as u64
                );
            }
        }
    }
    fixture.close().await;
}

async fn assert_bounded_missing_resource_checks(
    db: &mongodb::Database,
    size: usize,
    checks: usize,
) {
    let operations = profiled_operations(
        db,
        doc! { "$or": [
            { "command.find": "resources" }, { "originatingCommand.find": "resources" },
        ] },
    )
    .await;
    assert_eq!(
        operations
            .iter()
            .filter(|operation| operation
                .get_document("command")
                .is_ok_and(|command| command.get_str("find") == Ok("resources")))
            .count(),
        checks
    );
    for operation in &operations {
        let command = operation
            .get_document("originatingCommand")
            .or_else(|_| operation.get_document("command"))
            .unwrap();
        assert_eq!(command.get_str("hint"), Ok("idx_resources_identity"));
        assert!(
            profile_count(operation, "docsExamined") <= size as i64,
            "{operation:?}"
        );
        // Point seeks may examine a boundary key as well as each candidate.
        assert!(
            profile_count(operation, "keysExamined") <= (2 * size + 1) as i64,
            "{operation:?}"
        );
    }
    assert!(profile_sum(&operations, "docsExamined") <= (checks * size) as i64);
    assert!(profile_sum(&operations, "keysExamined") <= (checks * (2 * size + 1)) as i64);
}

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
        Self::with_count(label, 2048).await
    }

    async fn with_count(label: &str, count: usize) -> Option<Self> {
        // Late matches lie beyond the first 256 + 512 + 1,024 candidates.
        let mut fixture = Fixture::new(label, count).await?;
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
    db.create_collection("system.profile")
        .capped(true)
        .size(32 * 1024 * 1024)
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
                        None,
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
        let mut scan = ResourcePageScan::new(&filters, &[], &[], offset, 21);
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
            missing: Vec::new(),
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
        missing: Vec::new(),
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
async fn mixed_timestamp_orders_read_small_groups_from_one_stream() {
    let Some(data) = TimestampFixture::new("timestamp-small-groups").await else {
        return;
    };
    let db = &data.fixture.db;
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "frequent")],
        exclusions: Vec::new(),
        missing: Vec::new(),
    };
    let rows = data.expected(&["frequent"]);
    for (timestamp_direction, id_direction) in [(1, -1), (-1, 1)] {
        let mut expected = rows.clone();
        expected.sort_by(|a, b| {
            let by_time = data.timestamps[a].cmp(&data.timestamps[b]);
            let by_id = a.cmp(b);
            (if timestamp_direction == 1 {
                by_time
            } else {
                by_time.reverse()
            })
            .then(if id_direction == 1 {
                by_id
            } else {
                by_id.reverse()
            })
        });
        start_profiling(db).await;
        let actual = plan
            .execute_page(
                db,
                resource_filter(),
                None,
                doc! { "last_updated": timestamp_direction, "id": id_direction },
                0,
                expected.len() + 1,
            )
            .await
            .unwrap();
        assert_eq!(actual, expected);
        let operations = candidate_stream_operations(db).await;
        assert_covered_index_order(&operations, "the grouped stream must use index order");
        // Every fixture group fits the buffer, so all twelve timestamps come
        // from one find and its getMores rather than two finds per timestamp.
        let finds = operations
            .iter()
            .filter(|operation| operation.get_str("op") == Ok("query"))
            .count();
        assert_eq!(finds, 1, "{operations:?}");
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
            doc! { "$unset": { "data.code": "" }, "$addToSet": { "missing_params": "code" } },
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
                    None,
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
                None,
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
                None,
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
                None,
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
    trust_missing_metadata(backend, db).await;
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
                None,
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
                None,
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
            None,
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
        facts.requests, 1,
        "indexed presence uses the same selective probe as other positive predicates"
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

/// A sparse first batch does not abandon an ordered scan that can fill the
/// page or exhaust a small resource type within the continuation budget.
#[tokio::test]
async fn sparse_prefixes_finish_within_the_ordered_scan_budget() {
    let Some(data) = TimestampFixture::new("sparse-fallback").await else {
        return;
    };
    let db = &data.fixture.db;
    let first_batch = PAGE_CANDIDATE_BATCH_ROWS as i64;
    let live_rows = data.ordered.len() as i64;
    // Dense pages retain their one-batch fast path. Sparse pages keep their
    // ordered cursor until the page fills or this small fixture is exhausted.
    for (conditions, rows, batches) in [
        (vec!["frequent"], 1..=first_batch, 1..=1),
        (vec!["medium"], first_batch + 1..=live_rows, 2..=i64::MAX),
        (
            vec!["sparse"],
            first_batch + 1..=live_rows - 1,
            2..=i64::MAX,
        ),
        (vec!["late"], live_rows..=live_rows, 2..=i64::MAX),
        (vec!["empty"], live_rows..=live_rows, 2..=i64::MAX),
        (vec!["left", "right"], live_rows..=live_rows, 2..=i64::MAX),
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
        assert!(id_set_operations(db).await.is_empty(), "{context}");
        if expected.len() > 20 {
            // Cursor pages retain the same bounded continuation rule.
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

/// Membership aggregates check an explicit candidate set, with or without a hint.
fn membership_check_filter() -> Document {
    doc! {
        "command.aggregate": "search_index",
        "command.pipeline.0.$match.$and": {
            "$elemMatch": { "resource_id.$in": { "$type": "array" } },
        },
    }
}

async fn membership_checks(db: &mongodb::Database) -> Vec<Document> {
    profiled_operations(db, membership_check_filter()).await
}

/// The id-set pass reads the driver with a `find`, or groups the index for a
/// complement. Bounded membership checks belong to neither, regardless of hint.
async fn id_set_operations(db: &mongodb::Database) -> Vec<Document> {
    // Not `profiled_operations`: an empty result is a valid outcome here.
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "$or": [
                { "command.find": "search_index" },
                {
                    "command.aggregate": "search_index",
                    "command.hint": { "$exists": false },
                    "$nor": [membership_check_filter()],
                },
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
            missing: Vec::new(),
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
        missing: Vec::new(),
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

#[tokio::test]
async fn bounded_ordered_continuation_preserves_offsets_and_resume_state() {
    let Some(data) = TimestampFixture::new("bounded-page-resume").await else {
        return;
    };
    let db = &data.fixture.db;
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "base"), predicate("code", "sparse")],
        exclusions: Vec::new(),
        missing: Vec::new(),
    };
    let expected = data.expected(&["sparse"]);
    for (budget, next_batch) in [(768, 1024), (1792, 2048)] {
        for offset in [0, 10] {
            let mut run = plan
                .start_page(
                    db,
                    resource_filter(),
                    None,
                    doc! { "last_updated": -1, "id": -1 },
                    offset,
                    50,
                )
                .await
                .unwrap();
            assert!(matches!(
                run.sample(db).await.unwrap(),
                ResourcePageSample::Sparse { .. }
            ));
            assert!(
                run.try_finish_page_with_budget(db, budget, false)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(run.scan.candidates_checked, budget);
            assert_eq!(run.scan.batch_rows, next_batch);
            if budget == 1792 {
                assert!(run.scan.live_matches > SPARSE_SAMPLE_MAX_LIVE);
            }
            // An oversized fallback set must resume this exact state, including
            // matches already skipped for the offset and IDs already retained.
            assert_eq!(
                run.finish(db).await.unwrap(),
                expected
                    .iter()
                    .skip(offset as usize)
                    .take(50)
                    .cloned()
                    .collect::<Vec<_>>()
            );
        }
    }
    data.fixture.close().await;
}

#[tokio::test]
async fn bounded_ordered_continuation_matches_a_carried_last_row_before_reading() {
    let Some(data) = TimestampFixture::new("bounded-page-carried-row").await else {
        return;
    };
    let db = &data.fixture.db;
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "base")],
        exclusions: Vec::new(),
        missing: Vec::new(),
    };
    let mut run = plan
        .start_page(db, resource_filter(), None, doc! { "id": -1 }, 0, 1)
        .await
        .unwrap();
    let carried = data.ordered[0].clone();
    // Reproduce the state after a byte-budget flush: one row is already in
    // the next batch, and exactly one checked row remains in the total budget.
    run.scan.candidates_checked = RESOURCE_PAGE_SCAN_BUDGET - 1;
    run.scan.bytes = carried.len() + 16;
    run.scan.batch.push(carried.clone());
    let page = run
        .try_finish_page_with_budget(db, RESOURCE_PAGE_SCAN_BUDGET, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page, [carried]);
    assert_eq!(run.scan.candidates_checked, RESOURCE_PAGE_SCAN_BUDGET);
    assert!(run.scan.batch.is_empty());
    data.fixture.close().await;
}

#[tokio::test]
async fn sparse_ordered_pages_cross_fifty_pages_without_collecting_the_full_id_set() {
    let Some(mut data) = TimestampFixture::with_count("bounded-page-walk", 21_000).await else {
        return;
    };
    let selected: Vec<_> = data.ordered.iter().skip(300).step_by(20).cloned().collect();
    assert!(selected.len() > 1000);
    let index = data.fixture.db.collection::<Document>("search_index");
    for chunk in selected.chunks(500) {
        index
            .insert_many(
                chunk
                    .iter()
                    .map(|id| index_row(id, "code", "walk"))
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
    }
    data.markers.insert("walk".into(), selected);
    let db = &data.fixture.db;
    let expected = data.expected(&["walk"]);
    let query = data.query(&["walk"]);
    data.force_broad_observations(&query);
    start_profiling(db).await;
    let mut request = query.clone();
    let mut all_ids = Vec::new();
    for page in 0..50 {
        let result = data.search_ids(&request).await;
        let actual = result_ids(&result);
        assert_eq!(actual, expected[page * 20..(page + 1) * 20]);
        assert_eq!(result.total, None);
        all_ids.extend(actual);
        request = query
            .clone()
            .with_cursor(result.resources.page_info.next_cursor.unwrap());
    }
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(all_ids, expected[..1000]);
    assert!(
        id_set_operations(db).await.is_empty(),
        "every page must stay on the ordered path"
    );
    let operations = candidate_stream_operations(db).await;
    assert_covered_index_order(&operations, "fifty sparse pages");
    assert!(profile_sum(&operations, "nreturned") < 50 * 2048);

    // The independent expected order also covers mixed timestamp/ID directions,
    // ties, offsets, both total modes, previous links and a short final page.
    for sort in [None, Some("-_lastUpdated"), Some("_lastUpdated")] {
        let mut ordered = expected.clone();
        if sort == Some("_lastUpdated") {
            ordered.sort_by(|a, b| {
                data.timestamps[a]
                    .cmp(&data.timestamps[b])
                    .then_with(|| b.cmp(a))
            });
        }
        for mode in [TotalMode::None, TotalMode::Accurate] {
            let mut query = data.query(&["walk"]);
            query.total = Some(mode);
            if let Some(sort) = sort {
                query = query.with_sort(SortDirective::parse(sort));
            }
            let first = data.search_ids(&query).await;
            assert_eq!(result_ids(&first), ordered[..20]);
            assert_eq!(
                first.total,
                (mode == TotalMode::Accurate).then_some(ordered.len() as u64)
            );
            let next = data
                .search_ids(
                    &query
                        .clone()
                        .with_cursor(first.resources.page_info.next_cursor.unwrap()),
                )
                .await;
            assert_eq!(result_ids(&next), ordered[20..40]);
            let previous = data
                .search_ids(
                    &query
                        .clone()
                        .with_cursor(next.resources.page_info.previous_cursor.unwrap()),
                )
                .await;
            assert_eq!(result_ids(&previous), ordered[..20]);
            for offset in [7, ordered.len() - 7] {
                let mut offset_query = query.clone();
                offset_query.offset = Some(offset as u32);
                let result = data.search_ids(&offset_query).await;
                assert_eq!(
                    result_ids(&result),
                    ordered
                        .iter()
                        .skip(offset)
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    result.resources.page_info.has_next,
                    offset + 20 < ordered.len()
                );
            }
        }
    }
    data.fixture.close().await;
}
