use super::*;
use crate::error::{BackendError, StorageError};

fn ids(result: &crate::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|row| row.id().to_owned())
        .collect()
}

fn assert_timeout<T>(result: crate::error::StorageResult<T>) {
    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Timeout { .. }))
        ),
        "expected a timeout, never partial results"
    );
}

async fn commands(db: &mongodb::Database) -> Vec<Document> {
    let rows = db
        .collection::<Document>("system.profile")
        .find(doc! { "$or": [
            { "command.find": { "$in": ["resources", "search_index"] } },
            { "command.aggregate": { "$in": ["resources", "search_index"] } },
            { "command.distinct": "search_index" },
        ] })
        .await
        .unwrap();
    collect_documents(rows)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get_document("command").unwrap().clone())
        .collect()
}

#[tokio::test]
async fn configured_search_preserves_totals_and_cursor_pages_and_budgets_commands() {
    let Some(fixture) = Fixture::with_search_timeout("deadline-results", 1024, Some(5000)).await
    else {
        return;
    };
    fixture.db.run_command(doc! { "profile": 2 }).await.unwrap();
    for total in [TotalMode::None, TotalMode::Accurate, TotalMode::Estimate] {
        for direction in [1, -1] {
            let mut query = SearchQuery::new("Observation")
                .with_count(20)
                .with_parameter(parameter("code", "base"))
                .with_parameter(parameter("status", "final"));
            query.sort = vec![SortDirective::parse(if direction == 1 {
                "_id"
            } else {
                "-_id"
            })];
            query.total = Some(total);
            let first = fixture
                .backend
                .search(&fixture.tenant, &query)
                .await
                .unwrap();
            assert_eq!(ids(&first), fixture.expected(true, direction, 0, 20));
            assert_eq!(
                first.total,
                match total {
                    TotalMode::None => None,
                    TotalMode::Accurate => Some(255),
                    TotalMode::Estimate => Some(256),
                }
            );
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
            assert_eq!(ids(&next), fixture.expected(true, direction, 20, 20));
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
            assert_eq!(ids(&previous), fixture.expected(true, direction, 0, 20));
            let mut offset_query = query.clone();
            offset_query.offset = Some(20);
            let offset = fixture
                .backend
                .search(&fixture.tenant, &offset_query)
                .await
                .unwrap();
            assert_eq!(ids(&offset), fixture.expected(true, direction, 20, 20));
            assert_eq!(
                fixture
                    .backend
                    .search_count(&fixture.tenant, &query)
                    .await
                    .unwrap(),
                if total == TotalMode::Estimate {
                    256
                } else {
                    255
                }
            );
        }
    }
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let observed = commands(&fixture.db).await;
    assert!(
        observed.len() > 30,
        "must observe page, count and validation commands"
    );
    for command in observed {
        let budget = command
            .get_i64("maxTimeMS")
            .or_else(|_| command.get_i32("maxTimeMS").map(i64::from))
            .unwrap();
        assert!((1..=5000).contains(&budget), "{command:?}");
    }
    fixture.close().await;
}

#[tokio::test]
async fn unconfigured_search_does_not_add_a_command_budget() {
    let Some(fixture) = Fixture::new("deadline-unset", 32).await else {
        return;
    };
    fixture.db.run_command(doc! { "profile": 2 }).await.unwrap();
    let mut query = SearchQuery::new("Observation").with_count(20);
    query.total = Some(TotalMode::None);
    let result = fixture
        .backend
        .search(&fixture.tenant, &query)
        .await
        .unwrap();
    assert_eq!(result.resources.items.len(), 20);
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let observed = commands(&fixture.db).await;
    assert!(!observed.is_empty());
    assert!(
        observed
            .iter()
            .all(|command| !command.contains_key("maxTimeMS"))
    );
    fixture.close().await;
}

#[tokio::test]
async fn configured_soft_timeout_awaits_driver_completion_and_releases_admission() {
    let Some(fixture) = Fixture::with_search_timeout("deadline-cleanup", 1024, Some(100)).await
    else {
        return;
    };
    let Some(session) = fixture.failpoints("deadline-cleanup").await else {
        fixture.close().await;
        return;
    };
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
            "data": { "failCommands": ["aggregate"], "appName": &fixture.backend.config().app_name,
                "blockConnection": true, "blockTimeMS": 500 },
        })
        .await
        .unwrap();
    let query = SearchQuery::new("Observation").with_parameter(parameter("status", "final"));
    let started = std::time::Instant::now();
    assert_timeout(fixture.backend.search_count(&fixture.tenant, &query).await);
    assert!(started.elapsed() >= Duration::from_millis(400));
    let gate = fixture.backend.broad_search_gate().unwrap();
    assert_eq!(
        gate.available_permits(),
        1,
        "driver work finishes before returning"
    );
    let disabled = disable_failpoint(&session).await;
    assert_eq!(failpoint_hits(&disabled) - failpoint_hits(&armed), 1);
    let plain = SearchQuery::new("Observation").with_count(1);
    assert_eq!(
        fixture
            .backend
            .search(&fixture.tenant, &plain)
            .await
            .unwrap()
            .resources
            .items
            .len(),
        1
    );
    fixture.close().await;
}

#[tokio::test]
async fn completed_count_held_until_after_deadline_is_returned() {
    let Some(fixture) = Fixture::new("deadline-finished-count", 32).await else {
        return;
    };
    let expected = fixture.rows.iter().filter(|row| !row.deleted).count() as u64;
    let started = tokio::time::Instant::now();
    let count = search_deadline::run(1000, async {
        let count = fixture
            .backend
            .search_count(&fixture.tenant, &SearchQuery::new("Observation"))
            .await?;
        // Hold a complete result; do not ask MongoDB to ignore maxTimeMS.
        tokio::time::sleep_until(started + Duration::from_millis(1100)).await;
        assert_timeout(search_deadline::check());
        Ok(count)
    })
    .await
    .unwrap();
    assert_eq!(count, expected);
    fixture.close().await;
}

#[tokio::test]
async fn configured_cursor_getmore_timeout_finishes_and_allows_followup_reads() {
    let Some(fixture) = Fixture::new("deadline-getmore", 64).await else {
        return;
    };
    let Some(session) = fixture.failpoints("deadline-getmore").await else {
        fixture.close().await;
        return;
    };
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
            "data": { "failCommands": ["getMore"], "appName": &fixture.backend.config().app_name,
                "blockConnection": true, "blockTimeMS": 500 },
        })
        .await
        .unwrap();
    let db = fixture.db.clone();
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let result = search_deadline::run(100, async move {
        let result = async {
            let mut cursor = db
                .collection::<Document>("resources")
                .find(resource_filter())
                .search_deadline()?
                .batch_size(1)
                .await
                .or_query_error("open deadline cursor")?;
            assert!(cursor.advance().await?);
            cursor.advance().await?;
            search_deadline::check()?;
            Ok(())
        }
        .await;
        let _ = finished_tx.send(());
        result
    })
    .await;
    assert_timeout(result);
    tokio::time::timeout(Duration::from_secs(5), finished_rx)
        .await
        .unwrap()
        .unwrap();
    let disabled = disable_failpoint(&session).await;
    assert_eq!(failpoint_hits(&disabled) - failpoint_hits(&armed), 1);
    assert_eq!(
        fixture
            .db
            .collection::<Document>("resources")
            .count_documents(resource_filter())
            .await
            .unwrap(),
        63
    );
    fixture.close().await;
}

#[tokio::test]
async fn nested_search_and_inline_include_keep_the_original_deadline() {
    use crate::types::{IncludeDirective, IncludeType};
    let Some(fixture) = Fixture::with_search_timeout("deadline-include", 32, Some(5000)).await
    else {
        return;
    };
    fixture
        .db
        .collection::<Document>("resources")
        .update_many(
            resource_filter(),
            doc! {
                "$set": { "data.subject": { "reference": "Patient/patient" } }
            },
        )
        .await
        .unwrap();
    let now = DateTime::now();
    fixture.db.collection::<Document>("resources").insert_one(doc! {
        "tenant_id": PLAN_TENANT, "resource_type": "Patient", "id": "patient",
        "version_id": "1", "created_at": now, "last_updated": now,
        "is_deleted": false, "fhir_version": "4.0", "data": {"resourceType": "Patient", "id": "patient"}
    }).await.unwrap();
    fixture.db.run_command(doc! { "profile": 2 }).await.unwrap();
    let backend = &fixture.backend;
    let tenant = fixture.tenant.clone();
    let result = search_deadline::run(1000, async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut query = SearchQuery::new("Observation").with_count(2);
        query.total = Some(TotalMode::None);
        query.includes = vec![IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".into(),
            search_param: "subject".into(),
            target_type: Some("Patient".into()),
            iterate: false,
        }];
        backend.search(&tenant, &query).await
    })
    .await
    .unwrap();
    assert_eq!(result.included.len(), 1);
    assert_eq!(result.included[0].id(), "patient");
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    let observed = commands(&fixture.db).await;
    assert!(
        observed.len() >= 2,
        "must observe primary and include queries"
    );
    for command in observed {
        let budget = command
            .get_i64("maxTimeMS")
            .or_else(|_| command.get_i32("maxTimeMS").map(i64::from))
            .unwrap();
        assert!(
            (1..=800).contains(&budget),
            "nested search reset its deadline: {command:?}"
        );
    }
    fixture.close().await;
}

#[tokio::test]
async fn parallel_exact_batches_drain_after_deadline_without_partial_count() {
    let Some(fixture) =
        Fixture::with_search_timeout("deadline-parallel-count", 12_290, Some(5000)).await
    else {
        return;
    };
    let Some(session) = fixture.failpoints("deadline-parallel-count").await else {
        fixture.close().await;
        return;
    };
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 2 },
            "data": { "failCommands": ["aggregate"], "appName": &fixture.backend.config().app_name,
                "namespace": format!("{}.resources", fixture.db.name()),
                "blockConnection": true, "blockTimeMS": 1500 },
        })
        .await
        .unwrap();
    let db = fixture.db.clone();
    let gate = fixture.backend.broad_search_gate().unwrap();
    let worker_gate = gate;
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let result = search_deadline::run(750, async move {
        let _permit = worker_gate.acquire().await.unwrap();
        let result = batched_plan(vec![])
            .execute(&db, resource_filter(), None, BatchSelection::Count)
            .await;
        let _ = finished_tx.send(());
        result
    })
    .await;
    assert_timeout(result);
    assert_eq!(
        gate.available_permits(),
        1,
        "both active batches finish before returning"
    );
    tokio::time::timeout(Duration::from_secs(5), finished_rx)
        .await
        .unwrap()
        .unwrap();
    let disabled = disable_failpoint(&session).await;
    assert_eq!(
        failpoint_hits(&disabled) - failpoint_hits(&armed),
        2,
        "both parallel exact batches must reach MongoDB"
    );
    assert_eq!(gate.available_permits(), 1);
    assert_eq!(
        fixture
            .backend
            .search_count(&fixture.tenant, &SearchQuery::new("Observation"))
            .await
            .unwrap(),
        12_289
    );
    fixture.close().await;
}

/// A scripted command exposes each budget chosen by the common retry path.
struct TimedCommand<'a> {
    seen: &'a std::cell::RefCell<Vec<Duration>>,
    delay: Duration,
    result: mongodb::error::Result<u64>,
}

impl SearchCommand for TimedCommand<'_> {
    fn search_deadline(self) -> StorageResult<Self> {
        let budget = search_deadline::ProbeBudget::new(Duration::from_secs(60))?;
        self.seen.borrow_mut().push(budget.duration());
        Ok(self)
    }

    fn search_probe_deadline(
        self,
        probe: Duration,
    ) -> StorageResult<(Self, search_deadline::ProbeBudget)> {
        let budget = search_deadline::ProbeBudget::new(probe)?;
        self.seen.borrow_mut().push(budget.duration());
        Ok((self, budget))
    }
}

impl IntoFuture for TimedCommand<'_> {
    type Output = mongodb::error::Result<u64>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            self.result
        })
    }
}

#[tokio::test(start_paused = true)]
async fn date_hint_retry_recomputes_search_and_shared_probe_budgets() {
    for probe in [false, true] {
        let seen = std::cell::RefCell::new(Vec::new());
        search_deadline::run(100, async {
            let started = tokio::time::Instant::now();
            let operation = |hint: Option<Hint>| TimedCommand {
                seen: &seen,
                delay: if hint.is_some() {
                    Duration::from_millis(40)
                } else {
                    Duration::ZERO
                },
                result: if hint.is_some() {
                    Err(command_error(2, MISSING_INDEX))
                } else {
                    Ok(7)
                },
            };
            if probe {
                let result = with_probe_hint_fallback(
                    date_hint(),
                    || Duration::from_millis(80).saturating_sub(started.elapsed()),
                    operation,
                )
                .await?;
                assert_eq!(result, Some(7));
            } else {
                assert_eq!(
                    with_hint_fallback(date_hint(), operation).await?.unwrap(),
                    7
                );
            }
            Ok(())
        })
        .await
        .unwrap();
        let budgets = seen.into_inner();
        assert_eq!(budgets.len(), 2);
        assert_eq!(
            budgets[0],
            Duration::from_millis(if probe { 80 } else { 100 })
        );
        assert!(budgets[1] <= Duration::from_millis(if probe { 40 } else { 60 }));
        assert!(budgets[1] >= Duration::from_millis(1));
    }
}

#[tokio::test(start_paused = true)]
async fn hint_attempt_returns_late_success_or_original_error() {
    for fails in [false, true] {
        let seen = std::cell::RefCell::new(Vec::new());
        let result = search_deadline::run(
            100,
            with_hint_fallback(date_hint(), |_| TimedCommand {
                seen: &seen,
                delay: Duration::from_millis(150),
                result: if fails {
                    Err(command_error(13, "unauthorized"))
                } else {
                    Ok(42)
                },
            }),
        )
        .await
        .unwrap();
        if fails {
            assert!(matches!(
                result.unwrap_err().kind.as_ref(),
                mongodb::error::ErrorKind::Command(error) if error.code == 13
            ));
        } else {
            assert_eq!(result.unwrap(), 42);
        }
        assert_eq!(seen.borrow().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn expired_date_hint_attempt_never_starts_a_retry() {
    let seen = std::cell::RefCell::new(Vec::new());
    assert_timeout(
        search_deadline::run(100, async {
            with_hint_fallback(date_hint(), |_| TimedCommand {
                seen: &seen,
                delay: Duration::from_millis(150),
                result: Err(command_error(2, MISSING_INDEX)),
            })
            .await?
            .or_query_error("scripted missing index")
        })
        .await,
    );
    assert_eq!(seen.borrow().len(), 1);
}

#[tokio::test]
async fn capped_tie_break_waits_for_driver_and_only_caches_optional_expiry() {
    let Some(fixture) = Fixture::with_probe_timeout("deadline-capped", 8, 50).await else {
        return;
    };
    let Some(session) = fixture.failpoints("deadline-capped").await else {
        fixture.close().await;
        return;
    };
    let filters = capped_filters(&fixture);
    for search_ms in [100, 5000] {
        fixture.backend.probe_cache.clear();
        let mut facts = remember_capped(&fixture, &filters);
        let mut counts = vec![Some(MAX_CANDIDATE_ROWS + 1); filters.len()];
        let armed = session.admin.run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
            "data": { "failCommands": ["aggregate"], "appName": &fixture.backend.config().app_name,
                "namespace": format!("{}.search_index", fixture.db.name()),
                "blockConnection": true, "blockTimeMS": 500, "errorCode": 50 },
        }).await.unwrap();
        let started = std::time::Instant::now();
        let result = search_deadline::run(
            search_ms,
            count_capped_predicates(
                &fixture.db,
                &filters,
                &mut counts,
                &fixture.backend.probe_cache,
                &mut facts,
                fixture.backend.probe_timeout(),
                SearchFilterPurpose::Count,
            ),
        )
        .await;
        assert!(
            started.elapsed() >= Duration::from_millis(400),
            "an active driver future was cancelled"
        );
        let disabled = disable_failpoint(&session).await;
        assert_eq!(failpoint_hits(&disabled) - failpoint_hits(&armed), 1);
        assert_eq!(counts, vec![Some(MAX_CANDIDATE_ROWS + 1); filters.len()]);
        let key = ProbeKey::full_count(filters[0].probe_key.as_ref().unwrap()).unwrap();
        let fact_key = ProbeFacts::key(
            &fixture.db,
            &filters[0].filter,
            filters[0].hint(),
            FULL_COUNT,
            Some(fixture.backend.probe_timeout()),
        )
        .unwrap();
        if search_ms == 100 {
            assert_timeout(result);
            assert_eq!(
                fixture.backend.probe_cache.get(&key),
                None,
                "search expiry is not an unknown probe"
            );
            assert_eq!(facts.reuse(&fact_key), None);
        } else {
            assert_eq!(result.unwrap(), None);
            assert_eq!(
                fixture.backend.probe_cache.get(&key),
                Some(ProbeEstimate::Unknown)
            );
            assert_eq!(facts.reuse(&fact_key), Some(None));
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn sparse_resource_sample_resume_and_fallback_share_expiry() {
    let Some(fixture) = Fixture::new("deadline-resource-sample", 1024).await else {
        return;
    };
    let plan = ResourceBatchPlan {
        filters: vec![predicate("code", "absent")],
        exclusions: vec![],
    };
    // Cover both the ordinary cursor and opposite-direction timestamp groups.
    for (id_direction, resume) in [(1, true), (-1, false)] {
        assert_timeout(
            search_deadline::run(1000, async {
                let mut page = plan
                    .start_page(
                        &fixture.db,
                        resource_filter(),
                        None,
                        doc! { "last_updated": 1, "id": id_direction },
                        0,
                        20,
                    )
                    .await?;
                assert!(matches!(
                    page.sample(&fixture.db).await?,
                    ResourcePageSample::Sparse { live_matches: 0 }
                ));
                tokio::time::sleep(Duration::from_millis(1000)).await;
                if resume {
                    page.finish(&fixture.db).await.map(|_| ())
                } else {
                    bounded_candidates(
                        &fixture.db,
                        &plan.filters,
                        &fixture.backend.probe_cache,
                        &mut ProbeFacts::default(),
                        fixture.backend.probe_timeout(),
                    )
                    .await
                    .map(|_| ())
                }
            })
            .await,
        );
    }
    fixture.close().await;
}
