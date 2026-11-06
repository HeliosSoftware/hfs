//! Reindex retries a transiently failed page from its row snapshot and keeps the
//! failure retryable once the attempts run out; it never resends an insert batch.
//! Failpoints share the suite's server-global lock, but match only this test's
//! unique client application name and collection namespace.
//!
//! Most tests wrap their body in `catch_unwind` and re-raise the panic. That dates
//! from when each test dropped its own database after the body, panic or not.
//! The `db_cleanup` sweeper does that now (#2326), so those outer wrappers are
//! plain pass-throughs; unwrap them after #2341 and #2346 have merged.

use super::*;

use crate::bulk_submit::FailPoint;
use futures::{FutureExt, TryStreamExt};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{ReindexPageStats, ReindexTarget};
use helios_persistence::types::StoredResource;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

async fn failure_fixture(
    overlap: bool,
    retry_writes: bool,
) -> Option<(
    MongoBackend,
    TenantContext,
    Vec<StoredResource>,
    mongodb::Database,
)> {
    let connection_string = shared_mongo::connection_string().await?;
    let config = MongoBackendConfig {
        connection_string: append_query_param(
            &connection_string,
            if retry_writes {
                "retryWrites=true"
            } else {
                "retryWrites=false"
            },
        ),
        database_name: build_test_database_name("reindex_failures"),
        app_name: Some(format!(
            "reindex-failures-{}",
            uuid::Uuid::new_v4().simple()
        )),
        data_dir: Some(repo_data_dir()),
        reindex_overlap: overlap,
        reindex_prepare_threads: 1,
        // These cases count the runs a failpoint gets before the page gives
        // up; on a loaded runner the default 30 s budget can end it a run
        // early (#2290).
        reindex_page_budget: std::time::Duration::from_secs(600),
        ..Default::default()
    };
    let backend = build_backend(config).await?;
    let tenant = create_tenant("reindex-failures");
    let mut page = Vec::new();
    // More than REINDEX_SUBBATCH_FIRST (32), on a multi-thread runtime,
    // guarantees that overlap=true enters the overlapped writer. Seeding via
    // CRUD does not teach the page writer an adaptive sub-batch size.
    for n in 0..40 {
        let id = format!("obs-{n:02}");
        let (resource, _) = backend
            .seed_put(
                &tenant,
                "Observation",
                &id,
                json!({
                    "resourceType": "Observation", "id": id,
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                    "contained": [{
                        "resourceType": "Patient", "id": "contained",
                        "name": [{"family": "Contained"}]
                    }]
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed reindex fixture");
        page.push(resource);
    }
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    for collection in ["search_index", "search_index_contained"] {
        assert!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap()
                > 0,
            "fixture must exercise {collection}"
        );
    }
    Some((backend, tenant, page, db))
}

/// Disable the failpoint before propagating a writer panic or asserting its
/// result. A generous finite hit count catches unexpected retries instead of
/// allowing a retry to succeed after consuming a one-shot failpoint.
/// `REINDEX_PAGE_RETRY.max_attempts`, which the crate keeps private.
const REINDEX_PAGE_RETRY_ATTEMPTS: i64 = 4;

async fn write_with_failpoint(
    backend: &MongoBackend,
    tenant: &TenantContext,
    page: &[StoredResource],
    data: Document,
) -> Option<(Vec<StorageResult<usize>>, ReindexPageStats, i64)> {
    let Some(failpoint) = FailPoint::enable(
        backend.config().app_name_or_default(),
        data,
        doc! { "times": 100 },
    )
    .await
    else {
        skip_or_fail_unavailable("reindex failure tests require enableTestCommands=1");
        return None;
    };
    let mut stats = ReindexPageStats::default();
    let result =
        AssertUnwindSafe(backend.write_search_entries_page_timed(tenant, page, &mut stats))
            .catch_unwind()
            .await;
    let hits = failpoint.off_and_count().await;
    let outcomes = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    Some((outcomes, stats, hits))
}

fn transient_failure_data(command: &str, namespace: &str, write_concern: bool) -> Document {
    let mut data = doc! {
        "failCommands": [command],
        "namespace": namespace,
        "errorLabels": ["RetryableWriteError"],
    };
    if write_concern {
        data.insert(
            "writeConcernError",
            doc! {
                "code": 91, "codeName": "ShutdownInProgress",
                "errmsg": "reindex test write concern failure",
            },
        );
    } else {
        // An otherwise permanent command error becomes transient only because
        // of the explicit label, avoiding reliance on topology side effects.
        data.insert("errorCode", 2);
    }
    data
}

/// Every live fixture Observation of `page`.
fn live_fixture_scope(tenant: &TenantContext, page: &[StoredResource]) -> Document {
    doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": "Observation",
        "id": {"$in": page.iter().map(|resource| resource.id()).collect::<Vec<_>>()},
        "is_deleted": false,
    }
}

/// Removes `missing_params` from every live fixture Observation of `page`.
async fn clear_missing_params(
    db: &mongodb::Database,
    tenant: &TenantContext,
    page: &[StoredResource],
) {
    let cleared = db
        .collection::<Document>("resources")
        .update_many(
            live_fixture_scope(tenant, page),
            doc! {"$unset": {"missing_params": ""}},
        )
        .await
        .unwrap();
    assert_eq!(cleared.matched_count, page.len() as u64);
}

async fn page_failure_matrix(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex failure fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        for (command, collection, context) in [
            ("delete", "search_index", "Failed to delete search entries"),
            (
                "delete",
                "search_index_contained",
                "Failed to delete search_index_contained entries",
            ),
            (
                "insert",
                "search_index",
                "Failed to insert search index entries",
            ),
            (
                "insert",
                "search_index_contained",
                "Failed to insert search_index_contained entries",
            ),
            // These Observations omit date. The successful page below proves
            // the writer restores their missing-parameter metadata, so this
            // failpoint must encounter a resources update, not just index writes.
            ("update", "resources", "metadata"),
        ] {
            for write_concern in [false, true] {
                if collection == "resources" {
                    // The page leaves out a metadata update the stored
                    // resource already satisfies (#2005), and seeding through
                    // CRUD already wrote it, so remove it to make one due.
                    clear_missing_params(&db, &tenant, &page).await;
                }
                let data = transient_failure_data(
                    command,
                    &format!("{}.{}", db.name(), collection),
                    write_concern,
                );
                let Some((outcomes, _, hits)) =
                    write_with_failpoint(&backend, &tenant, &page, data).await
                else {
                    return;
                };
                let case = format!(
                    "overlap={overlap} {command} {collection} writeConcern={write_concern}"
                );
                // A failpoint that never clears exhausts the page's runs
                // (#2290); a metadata write-concern failure is not transient
                // and is not replayed.
                let expected_hits = if command == "update" && write_concern {
                    1
                } else {
                    REINDEX_PAGE_RETRY_ATTEMPTS
                };
                assert_eq!(hits, expected_hits, "page runs before giving up: {case}");
                assert_eq!(outcomes.len(), page.len(), "{case}");
                assert!(
                    outcomes.iter().all(|outcome| matches!(
                        outcome,
                        Err(StorageError::Backend(BackendError::Unavailable { .. }))
                    )),
                    "page-level label must survive every resource outcome: {case}: {outcomes:?}"
                );
                assert!(
                    outcomes.iter().all(|outcome| outcome
                        .as_ref()
                        .unwrap_err()
                        .to_string()
                        .contains(context)),
                    "phase context must survive classification: {case}: {outcomes:?}"
                );
            }
        }
        // Positive control for the metadata failpoint: remove only fixture
        // metadata, then prove an unfailed page restores it for every live ID.
        // Keep this after the matrix so a successful page cannot warm its
        // adaptive sub-batch size before the failure cases exercise overlap.
        let resources = db.collection::<Document>("resources");
        let scope = live_fixture_scope(&tenant, &page);
        clear_missing_params(&db, &tenant, &page).await;
        let mut without_metadata = scope.clone();
        without_metadata.insert("missing_params", doc! {"$exists": false});
        assert_eq!(
            resources.count_documents(without_metadata).await.unwrap(),
            page.len() as u64,
            "metadata positive control must start with missing_params absent for every fixture ID"
        );
        // A separate, explicit request succeeds after all injected failures.
        // This is not an automatic replay; the FHIR resource content is unchanged.
        let mut stats = ReindexPageStats::default();
        let recovered = backend
            .write_search_entries_page_timed(&tenant, &page, &mut stats)
            .await;
        assert_eq!(recovered.len(), page.len());
        assert!(
            recovered
                .iter()
                .all(|result| matches!(result, Ok(n) if *n > 0)),
            "{recovered:?}"
        );
        let mut restored_metadata = scope;
        restored_metadata.insert("missing_params", doc! {"$type": "array", "$all": ["date"]});
        assert_eq!(
            resources.count_documents(restored_metadata).await.unwrap(),
            page.len() as u64,
            "unfailed reindex must restore missing_params arrays containing the absent Observation date for every fixture ID; otherwise the resources-update failpoint has no verified positive control"
        );
        if overlap {
            assert!(
                stats.sub_batches > 1,
                "overlapped writer must split the fixture: {stats:?}"
            );
        } else {
            assert_eq!(stats.sub_batches, 1);
        }
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_page_failure_classification() {
    page_failure_matrix(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_page_failure_classification() {
    page_failure_matrix(true).await;
}

async fn permanent_document_failure(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex failure fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        // A real per-document validation error (121), not a command failure,
        // rejects only the first resource's own index documents. Repeat with an
        // injected write-concern error to exercise a mixed driver response:
        // known rejected owners remain permanent, all other owners are uncertain.
        for write_concern in [false, true] {
            db.run_command(doc! {
                "collMod": "search_index",
                "validator": {"resource_id": {"$ne": page[0].id()}},
                "validationLevel": "strict", "validationAction": "error",
            })
            .await
            .unwrap();
            let result = AssertUnwindSafe(async {
                if write_concern {
                    let data = transient_failure_data(
                        "insert",
                        &format!("{}.search_index", db.name()),
                        true,
                    );
                    write_with_failpoint(&backend, &tenant, &page, data).await
                } else {
                    let mut stats = ReindexPageStats::default();
                    let outcomes = backend
                        .write_search_entries_page_timed(&tenant, &page, &mut stats)
                        .await;
                    Some((outcomes, stats, 0))
                }
            })
            .catch_unwind()
            .await;
            db.run_command(doc! {"collMod": "search_index", "validator": {}})
                .await
                .unwrap();
            let Some((outcomes, stats, hits)) =
                result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            else {
                return;
            };
            assert_eq!(outcomes.len(), page.len());
            assert!(
                matches!(
                    &outcomes[0],
                    Err(StorageError::Backend(BackendError::Internal { .. }))
                ),
                "{outcomes:?}"
            );
            if write_concern {
                assert_eq!(hits, 1, "mixed write errors must not trigger page replay");
                assert!(
                    outcomes[1..].iter().all(|outcome| matches!(
                        outcome,
                        Err(StorageError::Backend(BackendError::Unavailable { .. }))
                    )),
                    "unacknowledged peers must retain transient classification: {outcomes:?}"
                );
            } else {
                assert!(
                    outcomes[1..].iter().all(Result::is_ok),
                    "document errors must not poison other resources: {outcomes:?}"
                );
            }
            if overlap && !write_concern {
                assert!(stats.sub_batches > 1, "{stats:?}");
            }
        }
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_document_failure_stays_permanent() {
    permanent_document_failure(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_document_failure_stays_permanent() {
    permanent_document_failure(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_driver_retry_is_not_page_replay() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, true).await else {
        skip_or_fail_unavailable("reindex driver retry test requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let hello = db.run_command(doc! {"hello": 1}).await.unwrap();
        if !hello.contains_key("setName") || !hello.contains_key("logicalSessionTimeoutMinutes") {
            skip_or_fail_unavailable("reindex driver retry test requires a replica set with sessions");
            return;
        }
        let data = transient_failure_data("insert", &format!("{}.search_index", db.name()), false);
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(), data, doc! {"times": 1},
        ).await else {
            skip_or_fail_unavailable("reindex driver retry test requires enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let result = AssertUnwindSafe(
            backend.write_search_entries_page_timed(&tenant, &page, &mut stats),
        ).catch_unwind().await;
        let hits = failpoint.off_and_count().await;
        let outcomes = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        assert_eq!(hits, 1);
        assert_eq!(outcomes.len(), page.len());
        assert!(outcomes.iter().all(Result::is_ok), "retryWrites=true must let the driver recover a single labelled insert failure: {outcomes:?}");
        assert_eq!(stats.sub_batches, 1, "driver retry must not re-extract a page");
        assert_eq!(stats.insert_commands, 2, "one Helios own insert and one contained insert; retries happen inside the driver");
    }).catch_unwind().await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// One transient failure of `command` on `collection` (`times: 1`): the page
/// runs again from its snapshot, ends indexed, and holds exactly the rows a
/// clean page holds, with no duplicate left by the failed run (#2290).
async fn transient_page_failure_is_retried(
    overlap: bool,
    command: &str,
    collection: &str,
    seed_rows: bool,
) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex page retry test requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let own = db.collection::<Document>("search_index");
        let contained = db.collection::<Document>("search_index_contained");
        if !seed_rows {
            // First build into an empty index: the delete finds nothing, so
            // only the forced verification protects a re-run.
            let scope = doc! {"tenant_id": tenant.tenant_id().as_str()};
            own.delete_many(scope.clone()).await.unwrap();
            contained.delete_many(scope).await.unwrap();
        }
        // Reference: rows of a clean page, taken before any failure.
        let mut clean = ReindexPageStats::default();
        let first = backend
            .write_search_entries_page_timed(&tenant, &page, &mut clean)
            .await;
        assert!(first.iter().all(Result::is_ok), "{first:?}");
        let expected_own = own.count_documents(doc! {}).await.unwrap();
        let expected_contained = contained.count_documents(doc! {}).await.unwrap();
        if !seed_rows {
            let scope = doc! {"tenant_id": tenant.tenant_id().as_str()};
            own.delete_many(scope.clone()).await.unwrap();
            contained.delete_many(scope).await.unwrap();
        }

        let data = transient_failure_data(command, &format!("{}.{}", db.name(), collection), false);
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            data,
            doc! {"times": 1},
        )
        .await
        else {
            skip_or_fail_unavailable("reindex page retry test requires enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let outcomes = backend
            .write_search_entries_page_timed(&tenant, &page, &mut stats)
            .await;
        let hits = failpoint.off_and_count().await;
        let case = format!("overlap={overlap} {command} {collection} seeded={seed_rows}");
        assert_eq!(hits, 1, "the failpoint fired once: {case}");
        assert_eq!(outcomes.len(), page.len(), "{case}");
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome, Ok(n) if *n > 0)),
            "a transient failure must end with the page indexed: {case}: {outcomes:?}"
        );
        assert_eq!(
            own.count_documents(doc! {}).await.unwrap(),
            expected_own,
            "own rows must match a clean page, with no duplicates: {case}"
        );
        assert_eq!(
            contained.count_documents(doc! {}).await.unwrap(),
            expected_contained,
            "contained rows must match a clean page, with no duplicates: {case}"
        );
        for resource in &page {
            assert_eq!(
                rows_of(&db, &tenant, resource.id()).await,
                rows_of(&db, &tenant, page[0].id()).await,
                "every resource holds the same row count: {case}"
            );
        }
    })
    .catch_unwind()
    .await;
    db.drop().await.unwrap();
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_delete_failure_is_retried() {
    transient_page_failure_is_retried(false, "delete", "search_index", true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_delete_failure_is_retried() {
    transient_page_failure_is_retried(true, "delete", "search_index", true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_insert_failure_is_retried_without_duplicates() {
    transient_page_failure_is_retried(false, "insert", "search_index", true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_insert_failure_is_retried_without_duplicates() {
    transient_page_failure_is_retried(true, "insert", "search_index", true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_insert_failure_is_retried_into_an_empty_index() {
    transient_page_failure_is_retried(false, "insert", "search_index_contained", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_insert_failure_is_retried_into_an_empty_index() {
    transient_page_failure_is_retried(true, "insert", "search_index_contained", false).await;
}

/// Old rows that survive the page's delete are what a delete cut short by a
/// chunk migration leaves behind. The page's verification finds them, removes
/// them, counts them, and every resource still succeeds.
async fn stale_rows_are_cleaned_up(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex stale-row fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let scope = doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation" };
        let count = |collection: &str, filter: Document| {
            let rows = db.collection::<Document>(collection);
            async move { rows.count_documents(filter).await.unwrap() }
        };
        // One generation of rows per collection is the baseline the page must
        // restore.
        let baseline_own = count("search_index", scope.clone()).await;
        let baseline_contained = count("search_index_contained", scope.clone()).await;
        // Copies of three resources' rows, `_id`s included, to plant once the
        // page's delete of the old rows has run: the same rows come back, as
        // if the delete had skipped them.
        let planted_ids: Vec<&str> = page[..3].iter().map(|resource| resource.id()).collect();
        let mut planted_scope = scope.clone();
        planted_scope.insert("resource_id", doc! { "$in": &planted_ids });
        let mut copies: Vec<(&str, Vec<Document>)> = Vec::new();
        for collection in ["search_index", "search_index_contained"] {
            let rows: Vec<Document> = db
                .collection::<Document>(collection)
                .find(planted_scope.clone())
                .await
                .unwrap()
                .try_collect::<Vec<Document>>()
                .await
                .unwrap();
            assert!(!rows.is_empty(), "{collection} must hold rows to duplicate");
            copies.push((collection, rows));
        }
        let planted: u64 = copies.iter().map(|(_, rows)| rows.len() as u64).sum();

        // Verification starts by reading the resources' versions, after the
        // page's inserts and its delete of the old rows. Hold that read before
        // it executes, so the planted survivors are there for verification to
        // find.
        let plant = async {
            for (collection, rows) in copies {
                let present = count(collection, planted_scope.clone()).await;
                assert_eq!(
                    present,
                    rows.len() as u64,
                    "the page's delete removed only the old rows, so only the new ones are in {collection}"
                );
                db.collection::<Document>(collection)
                    .insert_many(rows)
                    .await
                    .unwrap();
            }
        };
        let Some((outcomes, stats, ())) =
            page_with_held_command_observing(&backend, &tenant, &page, &db, "resources", plant)
                .await
        else {
            return;
        };
        assert_eq!(outcomes.len(), page.len());
        assert!(
            outcomes.iter().all(|outcome| matches!(outcome, Ok(n) if *n > 0)),
            "{outcomes:?}"
        );
        assert_eq!(stats.stale_rows_found, planted, "{stats:?}");
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
        assert!(stats.verify > Duration::ZERO, "{stats:?}");
        // The planted rows are gone: both collections are back at one
        // generation of rows, and no resource has two `_id` rows.
        assert_eq!(count("search_index", scope.clone()).await, baseline_own);
        assert_eq!(count("search_index_contained", scope.clone()).await, baseline_contained);
        let duplicates = db
            .collection::<Document>("search_index")
            .aggregate(vec![
                doc! { "$match": { "tenant_id": tenant.tenant_id().as_str(), "param_name": "_id" } },
                doc! { "$group": { "_id": "$resource_id", "rows": { "$sum": 1 } } },
                doc! { "$match": { "rows": { "$gt": 1 } } },
            ])
            .await
            .unwrap()
            .try_collect::<Vec<Document>>()
            .await
            .unwrap();
        assert!(duplicates.is_empty(), "{duplicates:?}");
        if overlap {
            assert!(stats.sub_batches > 1, "{stats:?}");
        }
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_page_cleans_up_stale_rows() {
    stale_rows_are_cleaned_up(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_page_cleans_up_stale_rows() {
    stale_rows_are_cleaned_up(true).await;
}

/// A failing verification ID read is a page failure with the transient
/// classification, not a silent pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_verification_id_read_failure_fails_the_page() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex verification test requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        // The page's first find on search_index is its pre-insert row snapshot;
        // skipping it fails the next one, the post-insert ID read.
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            transient_failure_data("find", &format!("{}.search_index", db.name()), false),
            doc! { "skip": 1 },
        )
        .await
        else {
            skip_or_fail_unavailable("verification failure requires enableTestCommands=1");
            return;
        };
        let outcomes = backend.write_search_entries_page(&tenant, &page).await;
        assert_eq!(
            failpoint.off_and_count().await,
            REINDEX_PAGE_RETRY_ATTEMPTS,
            "the failing verification read exhausts the page's runs"
        );
        assert_eq!(outcomes.len(), page.len());
        assert!(
            outcomes.iter().all(|outcome| matches!(
                outcome,
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            )),
            "{outcomes:?}"
        );
        assert!(
            outcomes.iter().all(|outcome| outcome
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("search index")),
            "{outcomes:?}"
        );
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// Runs a page write while one server command is held, and runs `during`
/// once the page has reached that command. The hold lets a concurrent writer
/// act at a chosen point of the page: at its row snapshot (`find`), before its
/// inserts (`insert`), before its old rows are deleted (`delete`), or before
/// verification versions (`resources`).
async fn page_with_held_command(
    backend: &MongoBackend,
    tenant: &TenantContext,
    page: &[StoredResource],
    db: &mongodb::Database,
    command: &str,
    during: impl std::future::Future<Output = ()>,
) -> Option<(Vec<StorageResult<usize>>, ReindexPageStats)> {
    let (outcomes, stats, ()) =
        page_with_held_command_observing(backend, tenant, page, db, command, during).await?;
    Some((outcomes, stats))
}

/// [`page_with_held_command`], returning what `during` observed while the
/// page was held.
async fn page_with_held_command_observing<T>(
    backend: &MongoBackend,
    tenant: &TenantContext,
    page: &[StoredResource],
    db: &mongodb::Database,
    command: &str,
    during: impl std::future::Future<Output = T>,
) -> Option<(Vec<StorageResult<usize>>, ReindexPageStats, T)> {
    let (command, collection) = if command == "resources" {
        ("find", "resources")
    } else {
        (command, "search_index")
    };
    let Some(failpoint) = FailPoint::enable(
        backend.config().app_name_or_default(),
        doc! {
            "failCommands": [command],
            "namespace": format!("{}.{}", db.name(), collection),
            "blockConnection": true,
            "blockTimeMS": 4000,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        skip_or_fail_unavailable("held-command tests require enableTestCommands=1");
        return None;
    };
    let mut stats = ReindexPageStats::default();
    let write = backend.write_search_entries_page_timed(tenant, page, &mut stats);
    let act = async {
        failpoint.wait_until_entered(1).await;
        during.await
    };
    let (outcomes, observed) = tokio::join!(write, act);
    assert_eq!(
        failpoint.off_and_count().await,
        1,
        "the command was held once"
    );
    Some((outcomes, stats, observed))
}

fn observation(id: &str, status: &str, with_code: bool) -> serde_json::Value {
    let mut resource = json!({
        "resourceType": "Observation", "id": id, "status": status,
        "contained": [{
            "resourceType": "Patient", "id": "contained",
            "name": [{"family": "Contained"}]
        }]
    });
    if with_code {
        resource["code"] = json!({"coding": [{"system": "http://loinc.org", "code": "1234-5"}]});
    }
    resource
}

/// Rows of one Observation for one search parameter; version 1 of the test
/// Observations has a `code`, the updated version 2 has none.
async fn param_rows_of(
    db: &mongodb::Database,
    tenant: &TenantContext,
    id: &str,
    param: &str,
) -> u64 {
    db.collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": id,
            "param_name": param,
        })
        .await
        .unwrap()
}

async fn rows_of(db: &mongodb::Database, tenant: &TenantContext, id: &str) -> u64 {
    rows_in(db, "search_index", tenant, id).await
}

async fn rows_in(
    db: &mongodb::Database,
    collection: &str,
    tenant: &TenantContext,
    id: &str,
) -> u64 {
    db.collection::<Document>(collection)
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": id,
        })
        .await
        .unwrap()
}

/// An update that commits between the page's snapshot and its insert writes the
/// newer version's rows first; the page's rows for that resource are the
/// older generation and go, the newer rows stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_update_between_snapshot_and_insert_keeps_the_newer_version() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id().to_owned();
        let page_rows = rows_of(&db, &tenant, &target).await;
        let page_contained_rows = rows_in(&db, "search_index_contained", &tenant, &target).await;
        let update = async {
            // Version 2 drops the code, so it has fewer rows than the page writes.
            let (updated, _) = backend
                .create_or_update(
                    &tenant,
                    "Observation",
                    &target,
                    observation(&target, "amended", false),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            assert_eq!(updated.version_id(), "2");
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "insert", update).await
        else {
            return;
        };
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "the catch-up rounds rewrite the changed resource, no error: {outcomes:?}"
        );
        assert!(
            rows_of(&db, &tenant, &target).await < page_rows,
            "version 2 has fewer rows than version 1"
        );
        assert_eq!(
            param_rows_of(&db, &tenant, &target, "code").await,
            0,
            "no row of version 1 remains"
        );
        assert_eq!(param_rows_of(&db, &tenant, &target, "status").await, 1);
        assert_eq!(
            stats.stale_rows_found,
            page_rows + page_contained_rows,
            "the page's own rows, in both collections, were the stale ones: {stats:?}"
        );
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// An update that commits after the page's insert has already replaced the
/// page's rows with the newer version's. Verification leaves the new rows
/// alone and removes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_update_after_insert_is_left_alone() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id().to_owned();
        let page_rows = rows_of(&db, &tenant, &target).await;
        let update = async {
            // Version 2 adds nothing and loses the code: fewer rows, newer version.
            backend
                .create_or_update(
                    &tenant,
                    "Observation",
                    &target,
                    observation(&target, "amended", false),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "resources", update).await
        else {
            return;
        };
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert!(
            rows_of(&db, &tenant, &target).await < page_rows,
            "the newer version's rows stayed, the page's were replaced"
        );
        assert_eq!(
            param_rows_of(&db, &tenant, &target, "code").await,
            0,
            "no row of version 1 remains"
        );
        assert_eq!(param_rows_of(&db, &tenant, &target, "status").await, 1);
        assert_eq!(
            stats.stale_rows_found, 0,
            "the update's rows are not stale: {stats:?}"
        );
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// A deficit in one resource cannot hide unknown extra rows of its sibling.
/// Rows planted after the old-row snapshot might belong to another writer;
/// preserve them and report the affected owner for recovery.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_offsetting_rows_do_not_hide_unknown_ones() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let (a, b) = (page[0].id().to_owned(), page[1].id().to_owned());
        let a_rows = rows_of(&db, &tenant, &a).await;
        let copies: Vec<Document> = db
            .collection::<Document>("search_index")
            .find(doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_id": &a })
            .await
            .unwrap()
            .try_collect::<Vec<Document>>()
            .await
            .unwrap()
            .into_iter()
            .map(|mut row| {
                row.remove("_id");
                row
            })
            .collect();
        let during = async {
            // The page's rows exist: plant unknown rows of A and delete B's
            // rows, exactly offsetting in a page-wide count.
            db.collection::<Document>("search_index")
                .insert_many(copies)
                .await
                .unwrap();
            db.collection::<Document>("search_index")
                .delete_many(doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_id": &b })
                .await
                .unwrap();
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "resources", during).await
        else {
            return;
        };
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "unknown rows must be retryable: {outcomes:?}"
        );
        assert!(
            matches!(
                &outcomes[1],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "missing rows must also be retryable: {outcomes:?}"
        );
        assert!(
            outcomes[2..].iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_eq!(
            rows_of(&db, &tenant, &a).await,
            a_rows * 2,
            "unknown rows must not be deleted"
        );
        assert_eq!(
            rows_of(&db, &tenant, &b).await,
            0,
            "B's deficit is not the page's to repair"
        );
        assert!(stats.stale_rows_remaining >= a_rows, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// One resource's insert failure does not waive verification of its siblings:
/// the failed resource keeps its insert error, a sibling whose old rows
/// survive the delete is cleaned and succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_failed_sibling_does_not_skip_verification() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let (failing, sibling) = (page[0].id().to_owned(), page[1].id().to_owned());
        let sibling_rows = rows_of(&db, &tenant, &sibling).await;
        // The sibling's old rows, `_id`s included: planted again once the
        // page has deleted them, they are old rows that survived the delete.
        let copies: Vec<Document> = db
            .collection::<Document>("search_index")
            .find(doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_id": &sibling })
            .await
            .unwrap()
            .try_collect::<Vec<Document>>()
            .await
            .unwrap();
        db.run_command(doc! {
            "collMod": "search_index",
            "validator": {"resource_id": {"$ne": &failing}},
            "validationLevel": "strict", "validationAction": "error",
        })
        .await
        .unwrap();
        let plant = async {
            db.collection::<Document>("search_index")
                .insert_many(copies)
                .await
                .unwrap();
        };
        let result =
            page_with_held_command(&backend, &tenant, &page, &db, "resources", plant).await;
        db.run_command(doc! {"collMod": "search_index", "validator": {}})
            .await
            .unwrap();
        let Some((outcomes, stats)) = result else {
            return;
        };
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Internal { .. }))
            ),
            "the rejected resource keeps its insert error: {:?}",
            outcomes[0]
        );
        assert!(
            outcomes[1..].iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_eq!(rows_of(&db, &tenant, &sibling).await, sibling_rows);
        assert_eq!(stats.stale_rows_found, sibling_rows, "{stats:?}");
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// An update that commits while the cleanup's own delete is held: the cleanup
/// deletes only captured old ids, so the update's rows survive, and the
/// re-read of the resource's version explains them, so nothing is failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_update_during_cleanup_keeps_the_update_rows() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id().to_owned();
        let scope = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": &target,
        };
        let search_index = db.collection::<Document>("search_index");
        // The resource's old rows, `_id`s included: planted again once the
        // page has deleted them, they are old rows that survived the delete.
        let copies: Vec<Document> = search_index
            .find(scope.clone())
            .await
            .unwrap()
            .try_collect::<Vec<Document>>()
            .await
            .unwrap();
        let planted = copies.len() as u64;
        // The update comes from another instance so the hold below never
        // applies to its own writes.
        let mut other_config = backend.config().clone();
        other_config.app_name = Some(format!("reindex-other-{}", uuid::Uuid::new_v4().simple()));
        let other = build_backend(other_config).await.unwrap();
        // One hold per matching command on search_index after the page's
        // snapshot (a find) and its delete of the old rows, which pass: the
        // post-insert ID read (1), the cleanup delete (2), the cleanup's
        // re-read (3) and the re-read after the version check explains the
        // mismatch (4).
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            doc! {
                "failCommands": ["find", "delete"],
                "namespace": format!("{}.search_index", db.name()),
                "blockConnection": true,
                "blockTimeMS": 1500,
            },
            doc! { "skip": 2 },
        )
        .await
        else {
            skip_or_fail_unavailable("held-command tests require enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let write = backend.write_search_entries_page_timed(&tenant, &page, &mut stats);
        let act = async {
            failpoint.wait_until_entered(1).await;
            search_index.insert_many(copies).await.unwrap();
            failpoint.wait_until_entered(2).await;
            // A real foreground update, including resource/history, own rows
            // and contained rows, while cleanup's exact-ID delete waits.
            other
                .create_or_update(
                    &tenant,
                    "Observation",
                    &target,
                    observation(&target, "amended", false),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        };
        let (outcomes, ()) = tokio::join!(write, act);
        assert_eq!(
            failpoint.off_and_count().await,
            4,
            "the ID read, the cleanup delete, its re-read and the re-read after the version check were held"
        );
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "the re-read version explains the update's rows: {outcomes:?}"
        );
        assert_eq!(param_rows_of(&db, &tenant, &target, "code").await, 0);
        assert_eq!(param_rows_of(&db, &tenant, &target, "status").await, 1);
        assert!(rows_in(&db, "search_index_contained", &tenant, &target).await > 0);
        assert!(stats.stale_rows_found >= planted, "{stats:?}");
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// A second page writing the same resource version owns a different set of
/// row IDs. The first page must not delete that writer's rows as stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_same_version_writer_rows_are_preserved() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex concurrency fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let mut other_config = backend.config().clone();
        other_config.app_name = Some(format!("reindex-other-{}", uuid::Uuid::new_v4().simple()));
        let other = build_backend(other_config).await.unwrap();
        let target = page[0].id();
        let own_rows = rows_of(&db, &tenant, target).await;
        let contained_rows = rows_in(&db, "search_index_contained", &tenant, target).await;
        let foreign_ids =
            std::sync::Arc::new(parking_lot::Mutex::new(Vec::<(String, Vec<Bson>)>::new()));
        let recorded = foreign_ids.clone();
        let during = async {
            // First page's old-row snapshot has completed (it saw no rows).
            // The other instance finishes its own page before the first inserts.
            let outcomes = other.write_search_entries_page(&tenant, &page[..1]).await;
            assert!(
                outcomes.iter().all(|outcome| outcome.is_ok()),
                "{outcomes:?}"
            );
            for name in ["search_index", "search_index_contained"] {
                let docs: Vec<Document> = db
                    .collection::<Document>(name)
                    .find(doc! {
                        "tenant_id": tenant.tenant_id().as_str(),
                        "resource_type": "Observation",
                        "resource_id": target,
                    })
                    .projection(doc! { "_id": 1 })
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                recorded.lock().push((
                    name.to_owned(),
                    docs.into_iter()
                        .map(|row| row.get("_id").unwrap().clone())
                        .collect(),
                ));
            }
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "insert", during).await
        else {
            return;
        };
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "unknown competing rows must remain retryable: {outcomes:?}"
        );
        assert!(
            outcomes[1..].iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        let foreign_ids = foreign_ids.lock().clone();
        for (name, ids) in foreign_ids {
            assert!(!ids.is_empty());
            assert_eq!(
                db.collection::<Document>(&name)
                    .count_documents(doc! { "_id": { "$in": &ids } })
                    .await
                    .unwrap(),
                ids.len() as u64,
                "the competing writer's {name} rows must survive"
            );
        }
        assert_eq!(rows_of(&db, &tenant, target).await, own_rows * 2);
        assert_eq!(
            rows_in(&db, "search_index_contained", &tenant, target).await,
            contained_rows * 2
        );
        assert!(
            stats.stale_rows_remaining >= own_rows + contained_rows,
            "{stats:?}"
        );
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

async fn changed_resource_with_no_contained_rows(delete: bool, overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex changed-resource fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        let during = async {
            if delete {
                backend.delete(&tenant, "Observation", target).await.unwrap();
            } else {
                let mut updated = observation(target, "amended", false);
                updated.as_object_mut().unwrap().remove("contained");
                backend.create_or_update(&tenant, "Observation", target, updated,
                    FhirVersion::default()).await.unwrap();
            }
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "insert", during).await
        else { return; };
        assert!(outcomes.iter().all(|outcome| outcome.is_ok()), "{outcomes:?}");
        assert_eq!(rows_in(&db, "search_index_contained", &tenant, target).await, 0,
            "the page must remove obsolete contained rows even when the count equals what it inserted");
        if delete {
            assert_eq!(rows_of(&db, &tenant, target).await, 0,
                "a deleted resource must not retain this page's rows");
        } else {
            assert_eq!(param_rows_of(&db, &tenant, target, "code").await, 0);
            assert_eq!(param_rows_of(&db, &tenant, target, "status").await, 1);
        }
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    }).catch_unwind().await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_removed_contained_rows_do_not_return() {
    for overlap in [false, true] {
        changed_resource_with_no_contained_rows(false, overlap).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_deleted_resource_rows_do_not_return() {
    for overlap in [false, true] {
        changed_resource_with_no_contained_rows(true, overlap).await;
    }
}

/// A resource whose stored document has no readable version is treated like
/// one this store does not hold: the page's copy stays authoritative and its
/// rows are kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_resource_without_version_keeps_the_page_rows() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex version fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        let page_rows = rows_of(&db, &tenant, target).await;
        let during = async {
            db.collection::<Document>("resources")
                .update_one(
                    doc! {
                        "tenant_id": tenant.tenant_id().as_str(),
                        "resource_type": "Observation",
                        "id": target,
                    },
                    doc! { "$unset": { "version_id": "" } },
                )
                .await
                .unwrap();
        };
        let Some((outcomes, _)) =
            page_with_held_command(&backend, &tenant, &page, &db, "insert", during).await
        else {
            return;
        };
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_eq!(
            rows_of(&db, &tenant, target).await,
            page_rows,
            "the page's rows stay, as for a resource this store does not hold"
        );
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// A failing cleanup delete is a page failure with the transient
/// classification. The resource was updated before the page ran, so the
/// cleanup has the page's own rows to remove; the page's delete of the old rows
/// is skipped and the cleanup delete fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_cleanup_delete_failure_is_retryable() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex cleanup failure fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        backend
            .seed_put(
                &tenant,
                "Observation",
                target,
                observation(target, "amended", false),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            transient_failure_data("delete", &format!("{}.search_index", db.name()), false),
            doc! { "skip": 1 },
        )
        .await
        else {
            skip_or_fail_unavailable("cleanup failure test requires enableTestCommands=1");
            return;
        };
        let outcomes = backend.write_search_entries_page(&tenant, &page).await;
        assert_eq!(
            failpoint.off_and_count().await,
            REINDEX_PAGE_RETRY_ATTEMPTS,
            "the failing cleanup delete exhausts the page's runs"
        );
        // The failpoint stays on after the skipped first delete, so every
        // replay fails at its own first delete, not at the stale cleanup.
        assert!(
            outcomes.iter().all(|outcome| matches!(
                outcome,
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            )),
            "{outcomes:?}"
        );
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

async fn update_at_version_observation_boundary(after_snapshot: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex observation boundary fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        // After snapshot: the page's old-row snapshot is taken and its inserts
        // have not started, so the newer writer's IDs are outside the snapshot
        // and the delete of the old rows cannot reach them.
        // Before resource read: snapshot and all page writes are complete, but
        // the resource version has not yet been observed by verification.
        let (command, collection) = if after_snapshot {
            ("insert", "search_index")
        } else {
            ("find", "resources")
        };
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            doc! {
                "failCommands": [command],
                "namespace": format!("{}.{}", db.name(), collection),
                "blockConnection": true, "blockTimeMS": 4000,
            },
            doc! { "times": 1 },
        )
        .await
        else {
            skip_or_fail_unavailable("observation boundary test requires enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let write = backend.write_search_entries_page_timed(&tenant, &page, &mut stats);
        let act = async {
            failpoint.wait_until_entered(1).await;
            let (updated, _) = backend
                .create_or_update(
                    &tenant,
                    "Observation",
                    target,
                    observation(target, "amended", false),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            assert_eq!(updated.version_id(), "2");
            let mut written = Vec::new();
            for name in ["search_index", "search_index_contained"] {
                let docs: Vec<Document> = db
                    .collection::<Document>(name)
                    .find(doc! {
                        "tenant_id": tenant.tenant_id().as_str(),
                        "resource_type": "Observation", "resource_id": target,
                    })
                    .projection(doc! { "_id": 1 })
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                let ids: Vec<Bson> = docs
                    .into_iter()
                    .map(|row| row.get("_id").unwrap().clone())
                    .collect();
                assert!(!ids.is_empty());
                written.push((name, ids));
            }
            written
        };
        let (outcomes, written) = tokio::join!(write, act);
        assert_eq!(
            failpoint.off_and_count().await,
            1,
            "the update must occur at the selected observation boundary"
        );
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "the changed resource is left to the catch-up rounds: {outcomes:?}"
        );
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
        for (name, ids) in written {
            assert_eq!(
                db.collection::<Document>(name)
                    .count_documents(doc! { "_id": { "$in": &ids } })
                    .await
                    .unwrap(),
                ids.len() as u64,
                "newer writer's exact {name} IDs must survive cleanup"
            );
        }
        assert_eq!(param_rows_of(&db, &tenant, target, "code").await, 0);
        assert_eq!(param_rows_of(&db, &tenant, target, "status").await, 1);
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_snapshot_cannot_claim_newer_writer_rows() {
    update_at_version_observation_boundary(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_version_read_observes_update_after_snapshot() {
    update_at_version_observation_boundary(false).await;
}

async fn unexpected_row_identity_is_retryable(replace: bool, collection: &str) {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex exact-ID fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        let rows = db.collection::<Document>(collection);
        let scope = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation", "resource_id": target,
        };
        let expected = rows.count_documents(scope.clone()).await.unwrap();
        assert!(expected > 0);
        let during = async {
            // The page has finished both inserts. Alter exactly one expected
            // row without changing the resource version or sibling resources.
            let mut row = rows.find_one(scope.clone()).await.unwrap().unwrap();
            let id = row.remove("_id").unwrap();
            assert_eq!(
                rows.delete_one(doc! { "_id": id })
                    .await
                    .unwrap()
                    .deleted_count,
                1
            );
            if replace {
                rows.insert_one(row).await.unwrap();
            }
        };
        let Some((outcomes, stats)) =
            page_with_held_command(&backend, &tenant, &page, &db, "resources", during).await
        else {
            return;
        };
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "an exact-ID mismatch requires a retry even when counts match: {outcomes:?}"
        );
        assert!(
            outcomes[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("1 missing expected rows"),
            "the removed page-owned ID must be diagnosed: {outcomes:?}"
        );
        assert!(
            outcomes[1..].iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_eq!(
            rows.count_documents(scope).await.unwrap(),
            expected - u64::from(!replace),
            "unfamiliar replacements must survive; verification cannot recreate missing rows"
        );
        assert_eq!(stats.stale_rows_remaining, u64::from(replace), "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_equal_count_replacement_requires_retry() {
    for collection in ["search_index", "search_index_contained"] {
        unexpected_row_identity_is_retryable(true, collection).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_missing_expected_row_requires_retry() {
    for collection in ["search_index", "search_index_contained"] {
        unexpected_row_identity_is_retryable(false, collection).await;
    }
}

/// Recreate one captured ID after each successful cleanup delete. This is an
/// adversarial survivor simulation of the real bounded loop, not a balancer
/// reproduction or a command error that would exit before retries occur.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_cleanup_stops_after_three_delete_rounds() {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("reindex cleanup exhaustion fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        let rows = db.collection::<Document>("search_index");
        let scope = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation", "resource_id": target,
        };
        let expected = rows.count_documents(scope.clone()).await.unwrap();
        let mut survivor = rows.find_one(scope.clone()).await.unwrap().unwrap();
        let survivor_id = Bson::ObjectId(mongodb::bson::oid::ObjectId::new());
        survivor.insert("_id", survivor_id.clone());
        // Old-row snapshot, final ID verification, then the three cleanup rereads.
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            doc! {
                "failCommands": ["find"],
                "namespace": format!("{}.search_index", db.name()),
                "blockConnection": true, "blockTimeMS": 4000,
            },
            doc! { "times": 5 },
        )
        .await
        else {
            skip_or_fail_unavailable("cleanup exhaustion requires enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let write = backend.write_search_entries_page_timed(&tenant, &page, &mut stats);
        let act = async {
            // Planted before the old-row snapshot executes, so that its id is
            // one of the old rows the page deletes.
            failpoint.wait_until_entered(1).await;
            rows.insert_one(survivor.clone()).await.unwrap();
            // The page's delete of the old rows has run: the survivor is gone
            // with the rest, and comes back before verification reads it.
            failpoint.wait_until_entered(2).await;
            assert_eq!(
                rows.count_documents(doc! { "_id": &survivor_id })
                    .await
                    .unwrap(),
                0,
                "the delete of the old rows removes a row that was there at the snapshot"
            );
            rows.insert_one(survivor.clone()).await.unwrap();
            for read in 3..=5 {
                failpoint.wait_until_entered(read).await;
                assert_eq!(
                    rows.count_documents(doc! { "_id": &survivor_id })
                        .await
                        .unwrap(),
                    0,
                    "each reread must follow a successful exact-ID cleanup delete"
                );
                // Keep the exact same ID: later observations must never grant
                // cleanup ownership over an unfamiliar replacement ID.
                rows.insert_one(survivor.clone()).await.unwrap();
            }
        };
        let (outcomes, ()) = tokio::join!(write, act);
        assert_eq!(
            failpoint.off_and_count().await,
            5,
            "the page must stop after three delete/reread rounds"
        );
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "surviving known rows must request resource recovery: {outcomes:?}"
        );
        assert!(
            outcomes[1..].iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_eq!(
            rows.count_documents(doc! { "_id": survivor_id })
                .await
                .unwrap(),
            1
        );
        assert_eq!(rows.count_documents(scope).await.unwrap(), expected + 1);
        assert_eq!(stats.stale_rows_found, 1, "{stats:?}");
        assert_eq!(stats.stale_rows_remaining, 1, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

async fn changed_resource_cleanup_exhaustion_is_retryable(deleted: bool, collection: &str) {
    let Some((backend, tenant, page, db)) = failure_fixture(false, false).await else {
        skip_or_fail_unavailable("changed-resource cleanup exhaustion fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let target = page[0].id();
        // Keep the saved page at version 1 while the current resource changes.
        if deleted {
            backend
                .seed_delete(&tenant, "Observation", target)
                .await
                .unwrap();
        } else {
            let (updated, _) = backend
                .seed_put(
                    &tenant,
                    "Observation",
                    target,
                    observation(target, "amended", false),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            assert_ne!(updated.version_id(), page[0].version_id());
        }
        let rows = db.collection::<Document>(collection);
        let scope = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation", "resource_id": target,
        };
        // Post-delete snapshot, post-insert verification, three cleanup rereads.
        let Some(failpoint) = FailPoint::enable(
            backend.config().app_name_or_default(),
            doc! {
                "failCommands": ["find"],
                "namespace": format!("{}.{}", db.name(), collection),
                "blockConnection": true, "blockTimeMS": 4000,
            },
            doc! { "times": 5 },
        )
        .await
        else {
            skip_or_fail_unavailable("cleanup exhaustion requires enableTestCommands=1");
            return;
        };
        let mut stats = ReindexPageStats::default();
        let write = backend.write_search_entries_page_timed(&tenant, &page, &mut stats);
        let act = async {
            failpoint.wait_until_entered(2).await;
            // The page's inserts have finished. Preserve one of its exact IDs
            // after each delete, alongside an ID the page never owned.
            let survivor = rows.find_one(scope.clone()).await.unwrap().unwrap();
            let survivor_id = survivor.get("_id").unwrap().clone();
            let mut foreign = survivor.clone();
            foreign.insert("_id", mongodb::bson::oid::ObjectId::new());
            rows.insert_one(foreign.clone()).await.unwrap();
            for read in 3..=5 {
                failpoint.wait_until_entered(read).await;
                assert_eq!(
                    rows.count_documents(doc! { "_id": &survivor_id })
                        .await
                        .unwrap(),
                    0,
                    "each reread must follow a successful exact-ID cleanup delete"
                );
                rows.insert_one(survivor.clone()).await.unwrap();
            }
            (survivor_id, foreign)
        };
        let (outcomes, (survivor_id, foreign)) = tokio::join!(write, act);
        assert_eq!(
            failpoint.off_and_count().await,
            5,
            "changed/deleted resources must not restart the exhausted cleanup loop"
        );
        assert!(
            matches!(
                &outcomes[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ),
            "a page-owned survivor requires recovery: deleted={deleted} {collection}: {outcomes:?}"
        );
        assert!(
            outcomes[1..].iter().all(|outcome| outcome.is_ok()),
            "unaffected page owners must succeed: {outcomes:?}"
        );
        assert_eq!(
            rows.count_documents(doc! { "_id": survivor_id })
                .await
                .unwrap(),
            1,
            "the exhausted loop must report the remaining obsolete ID"
        );
        assert_eq!(
            rows.find_one(doc! { "_id": foreign.get("_id").unwrap() })
                .await
                .unwrap(),
            Some(foreign),
            "cleanup must preserve the unfamiliar writer's exact row"
        );
        assert_eq!(rows.count_documents(scope).await.unwrap(), 2);
        assert_eq!(stats.stale_rows_remaining, 1, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_changed_resource_cleanup_exhaustion_requires_retry() {
    for collection in ["search_index", "search_index_contained"] {
        changed_resource_cleanup_exhaustion_is_retryable(false, collection).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_deleted_resource_cleanup_exhaustion_requires_retry() {
    for collection in ["search_index", "search_index_contained"] {
        changed_resource_cleanup_exhaustion_is_retryable(true, collection).await;
    }
}

// ---------------------------------------------------------------------------
// #2424: a page's resources keep their search rows while the page is rewritten
// ---------------------------------------------------------------------------
//
// A page write used to delete the page's rows and then insert the new ones, as
// two commands with nothing between them to keep either. A search in that gap,
// or a crash, a dropped future or a failed command, found the page's resources
// with no rows at all, and nothing recorded it. The page now snapshots the ids
// of the rows it will replace, inserts the new rows, and only then deletes
// exactly those ids, so every resource has a complete set of rows (the old
// one, the new one, or both) at every instant. The cases hold or fail the
// command that used to open the gap and look at the rows and at what a search
// returns.

type RowsByResource = std::collections::BTreeMap<String, i64>;

/// What a search over the fixture's Observations returns.
#[derive(Debug, PartialEq, Eq)]
struct SearchView {
    /// Ids of `status=final`, sorted.
    ids: Vec<String>,
    /// The accurate total of that search.
    total: Option<u64>,
    /// First 15 ids of `status=final` sorted by `code`, in result order.
    sorted_first_page: Vec<String>,
}

async fn search_view(backend: &MongoBackend, tenant: &TenantContext) -> SearchView {
    let final_status = || SearchParameter {
        name: "status".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("final")],
        chain: vec![],
        components: vec![],
    };
    let mut everything = SearchQuery::new("Observation")
        .with_parameter(final_status())
        .with_count(100);
    everything.total = Some(TotalMode::Accurate);
    let found = backend.search(tenant, &everything).await.unwrap();
    let mut ids: Vec<String> = found
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_string())
        .collect();
    ids.sort();
    let sorted = SearchQuery::new("Observation")
        .with_parameter(final_status())
        .with_sort(SortDirective {
            parameter: "code".to_string(),
            direction: helios_persistence::types::SortDirection::Ascending,
            param_type: Some(SearchParamType::Token),
        })
        .with_count(15);
    let sorted_first_page = backend
        .search(tenant, &sorted)
        .await
        .unwrap()
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_string())
        .collect();
    SearchView {
        ids,
        total: found.total,
        sorted_first_page,
    }
}

/// Rows per resource id of `collection` for the tenant's Observations.
async fn rows_by_resource(
    db: &mongodb::Database,
    collection: &str,
    tenant: &TenantContext,
) -> RowsByResource {
    db.collection::<Document>(collection)
        .aggregate(vec![
            doc! { "$match": {
                "tenant_id": tenant.tenant_id().as_str(),
                "resource_type": "Observation",
            } },
            doc! { "$group": { "_id": "$resource_id", "rows": { "$sum": 1 } } },
        ])
        .await
        .unwrap()
        .try_collect::<Vec<Document>>()
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let rows = match row.get("rows") {
                Some(Bson::Int32(n)) => i64::from(*n),
                Some(Bson::Int64(n)) => *n,
                other => panic!("unexpected row count {other:?}"),
            };
            (row.get_str("_id").unwrap().to_owned(), rows)
        })
        .collect()
}

/// Both collections' rows per resource.
async fn rows_in_both(
    db: &mongodb::Database,
    tenant: &TenantContext,
) -> (RowsByResource, RowsByResource) {
    (
        rows_by_resource(db, "search_index", tenant).await,
        rows_by_resource(db, "search_index_contained", tenant).await,
    )
}

/// Every resource holds `factor` times the rows of `baseline`, in both
/// collections, and no other resource holds any.
fn assert_rows_are(
    context: &str,
    now: &(RowsByResource, RowsByResource),
    baseline: &(RowsByResource, RowsByResource),
    factor: i64,
) {
    for (name, now, base) in [
        ("search_index", &now.0, &baseline.0),
        ("search_index_contained", &now.1, &baseline.1),
    ] {
        let expected: RowsByResource = base
            .iter()
            .map(|(id, rows)| (id.clone(), rows * factor))
            .collect();
        assert_eq!(*now, expected, "{context}: {name} rows per resource");
    }
}

/// Between the page's snapshot and its first insert the old rows are all still
/// there and a search finds every resource. This is the state a crash, a drop
/// or a search at that instant meets, and what the delete-first order left
/// empty.
async fn page_stays_searchable_until_its_inserts_land(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex searchability fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let baseline = rows_in_both(&db, &tenant).await;
        assert_eq!(baseline.0.len(), page.len(), "{:?}", baseline.0);
        let before = search_view(&backend, &tenant).await;
        assert_eq!(before.ids.len(), page.len(), "{before:?}");

        let observe = async {
            (
                rows_in_both(&db, &tenant).await,
                search_view(&backend, &tenant).await,
            )
        };
        let Some((outcomes, stats, (held_rows, held_search))) =
            page_with_held_command_observing(&backend, &tenant, &page, &db, "insert", observe)
                .await
        else {
            return;
        };
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_rows_are("while the first insert is held", &held_rows, &baseline, 1);
        assert_eq!(
            held_search, before,
            "a search while the page's first insert is held must see every resource"
        );
        // Afterwards the page holds one generation, the new one.
        assert_rows_are(
            "after the page",
            &rows_in_both(&db, &tenant).await,
            &baseline,
            1,
        );
        assert_eq!(search_view(&backend, &tenant).await, before);
        if overlap {
            assert!(stats.sub_batches > 1, "{stats:?}");
        }
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_page_stays_searchable_until_its_inserts_land() {
    page_stays_searchable_until_its_inserts_land(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_page_stays_searchable_until_its_inserts_land() {
    page_stays_searchable_until_its_inserts_land(true).await;
}

/// With the new rows in and the old rows not yet deleted, a resource has both
/// generations. Searches must not tell: they group by resource id.
async fn both_generations_search_like_one(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex duplicate-window fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let baseline = rows_in_both(&db, &tenant).await;
        let before = search_view(&backend, &tenant).await;
        let observe = async {
            (
                rows_in_both(&db, &tenant).await,
                search_view(&backend, &tenant).await,
            )
        };
        let Some((outcomes, _, (held_rows, held_search))) =
            page_with_held_command_observing(&backend, &tenant, &page, &db, "delete", observe)
                .await
        else {
            return;
        };
        assert!(
            outcomes.iter().all(|outcome| outcome.is_ok()),
            "{outcomes:?}"
        );
        assert_rows_are(
            "while the old rows' delete is held",
            &held_rows,
            &baseline,
            2,
        );
        assert_eq!(
            held_search, before,
            "two generations of rows must search like one"
        );
        assert_rows_are(
            "after the page",
            &rows_in_both(&db, &tenant).await,
            &baseline,
            1,
        );
        assert_eq!(search_view(&backend, &tenant).await, before);
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_two_generations_of_rows_search_like_one() {
    both_generations_search_like_one(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_two_generations_of_rows_search_like_one() {
    both_generations_search_like_one(true).await;
}

/// A page write that stops after its inserts and before its delete (a crash,
/// a dropped future, a command error that is not retried) leaves both
/// generations: searchable, and removed by the next write of the page. Stands
/// in for the stop with a delete that fails without a retry.
async fn interrupted_page_leaves_rows_the_next_write_replaces(overlap: bool) {
    let Some((backend, tenant, page, db)) = failure_fixture(overlap, false).await else {
        skip_or_fail_unavailable("reindex interrupted-page fixture requires MongoDB");
        return;
    };
    let checked = AssertUnwindSafe(async {
        let baseline = rows_in_both(&db, &tenant).await;
        let before = search_view(&backend, &tenant).await;

        let data = doc! {
            "failCommands": ["delete"],
            "namespace": format!("{}.search_index", db.name()),
            // A permanent command error: the page is not run again.
            "errorCode": 2,
        };
        let Some((outcomes, _, hits)) = write_with_failpoint(&backend, &tenant, &page, data).await
        else {
            return;
        };
        assert_eq!(hits, 1, "a permanent failure ends the page at once");
        assert!(
            outcomes.iter().all(|outcome| outcome.is_err()),
            "{outcomes:?}"
        );
        assert_rows_are(
            "after the interrupted page",
            &rows_in_both(&db, &tenant).await,
            &baseline,
            2,
        );
        assert_eq!(
            search_view(&backend, &tenant).await,
            before,
            "an interrupted page must leave every resource searchable"
        );

        let mut stats = ReindexPageStats::default();
        let rewritten = backend
            .write_search_entries_page_timed(&tenant, &page, &mut stats)
            .await;
        assert!(
            rewritten
                .iter()
                .all(|outcome| matches!(outcome, Ok(n) if *n > 0)),
            "{rewritten:?}"
        );
        assert_rows_are(
            "after the next write of the page",
            &rows_in_both(&db, &tenant).await,
            &baseline,
            1,
        );
        assert_eq!(search_view(&backend, &tenant).await, before);
        assert_eq!(stats.stale_rows_remaining, 0, "{stats:?}");
    })
    .catch_unwind()
    .await;
    checked.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_serial_interrupted_page_leaves_rows_the_next_write_replaces() {
    interrupted_page_leaves_rows_the_next_write_replaces(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mongodb_integration_reindex_overlapped_interrupted_page_leaves_rows_the_next_write_replaces()
 {
    interrupted_page_leaves_rows_the_next_write_replaces(true).await;
}
