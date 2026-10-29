//! #2268: the #1391 reindex reminder (`search_index date rows written before
//! #1391 have no value_date_end ...`) needs `search_index` rows that were there
//! at boot. Rows a fresh database takes while its first build runs, in
//! `background` mode or before a restart or a later `off`-to-`inline` boot
//! records the generation, are this version's own and must not trigger it; a
//! real upgrade still gets exactly one reminder; a boot-time count that fails
//! gets a different, hedged line. Child module of the `mongodb_tests` root:
//! `use super::*;` reaches its private harness (`create_tenant`,
//! `raw_test_client`, `build_test_database_name`, `repo_data_dir`,
//! `pin_tracing_interest`, `shared_mongo`, plus `Document`/`doc`/`json`/
//! `FhirVersion`/`BuildOutcome`/`IndexBuildMode`, all imported at the root).
//!
//! Each test captures the builder's log lines with a scoped subscriber and
//! asserts a positive control next to every absence: the builder's own
//! `search_index build complete` INFO line, so an absence cannot pass because
//! the capture is broken.

use super::*;

use crate::bulk_submit::FailPoint;
use futures::stream::TryStreamExt;

/// The #1391 reminder, by a phrase only it contains.
const REINDEX_REMINDER: &str = "have no value_date_end";

/// The hedged line logged when the boot-time read failed.
const UNKNOWN_ROWS_REMINDER: &str =
    "could not tell whether search_index held rows before this boot";

/// The builder's INFO line after its `createIndexes` returns: the positive
/// control that the capture sees this builder at all.
const BUILD_COMPLETE: &str = "search_index build complete";

/// How long the failpoint holds the builder's first `listIndexes`. The writes
/// and the `$currentOp` check must finish inside it, so it leaves room for a
/// slow CI runner; the hold cannot be released early, so it is also how long
/// the test waits for the build.
const HOLD_MS: i64 = 15_000;

/// A scoped (not global) subscriber that records the builder's INFO and WARN
/// lines into a fresh buffer. `set_default` is thread-local and never replaces
/// the process-wide subscriber; that is safe here because `#[tokio::test]` is
/// current-thread, so the builder task `init_schema` spawns logs on the test
/// thread. Keep the guard alive until the log assertions are done.
fn capture_builder_logs() -> (
    &'static std::sync::Mutex<Vec<u8>>,
    tracing::dispatcher::DefaultGuard,
) {
    pin_tracing_interest();
    let buf: &'static std::sync::Mutex<Vec<u8>> =
        Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
    let writer = tracing_test::internal::MockWriter::new(buf);
    let dispatch = tracing_test::internal::get_subscriber(
        writer,
        "helios_persistence::backends::mongodb::search_index_builder=info",
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    (buf, guard)
}

/// The captured lines that contain `needle`.
fn captured(buf: &std::sync::Mutex<Vec<u8>>, needle: &str) -> Vec<String> {
    let buf = buf.lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_string)
        .collect()
}

/// Boots a backend on `db_name` in `mode` and runs `init_schema`.
///
/// `missing_metadata: false` keeps `MissingIndexBuilder` from listing the
/// `resources` indexes, so the builder's `inspect` is the only `listIndexes`
/// this client sends at boot (the failpoint in the first test relies on it).
async fn boot(cs: &str, db_name: &str, app: &str, mode: IndexBuildMode) -> MongoBackend {
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: cs.to_string(),
        database_name: db_name.to_string(),
        app_name: app.to_string(),
        index_build: mode,
        missing_metadata: false,
        max_connections: TEST_BACKEND_MAX_POOL,
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .unwrap();
    backend.initialize().await.expect("boot");
    backend
}

/// The `search_indexes` subdocument of the `schema_version` record, or an
/// empty document when there is none.
async fn search_indexes_record(cs: &str, db_name: &str) -> Document {
    let db = raw_test_client(cs).await.unwrap().database(db_name);
    db.collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .unwrap()
        .and_then(|record| record.get_document("search_indexes").ok().cloned())
        .unwrap_or_default()
}

/// How many rows `search_index` holds, from collection metadata.
async fn search_index_rows(cs: &str, db_name: &str) -> u64 {
    let db = raw_test_client(cs).await.unwrap().database(db_name);
    db.collection::<Document>("search_index")
        .estimated_document_count()
        .await
        .unwrap()
}

/// Writes `n` Patients with a `birthDate`, so each leaves `search_index` rows.
async fn write_patients(backend: &MongoBackend, tenant: &TenantContext, n: usize) {
    for i in 0..n {
        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": format!("p{i}"),
                    "birthDate": "1970-01-01",
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create Patient p{i}: {e}"));
    }
}

/// Whether a `listIndexes` on `search_index` from the client named `app` is
/// running on the server right now (`$currentOp`). The builder's `inspect` runs
/// before its `createIndexes` and before it records the generation, so while
/// this is true the record step has not run.
async fn builder_still_listing_indexes(cs: &str, app: &str) -> bool {
    let admin = raw_test_client(cs).await.unwrap().database("admin");
    let mut cursor = admin
        .aggregate(vec![
            doc! { "$currentOp": { "allUsers": true } },
            doc! { "$match": { "appName": app, "command.listIndexes": "search_index" } },
        ])
        .await
        .expect("$currentOp");
    cursor
        .try_next()
        .await
        .expect("$currentOp cursor")
        .is_some()
}

/// A brand-new database in `background` mode takes writes while its first
/// build is held before `createIndexes`: every `search_index` row is this
/// version's own, so the build must not log the #1391 reminder.
#[tokio::test]
async fn a_fresh_database_written_during_its_first_background_build_logs_no_reminder() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let app = "fp-2268-fresh-background";
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["listIndexes"], "blockConnection": true, "blockTimeMS": HOLD_MS },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_builder_logs();
    let db_name = build_test_database_name("si_2268_fresh_bg");

    let backend = boot(&cs, &db_name, app, IndexBuildMode::Background).await;
    // The builder's first `listIndexes` is held: it is before `createIndexes`
    // and before the generation is recorded.
    fail_point.wait_until_entered(1).await;
    write_patients(&backend, &create_tenant("si-2268-fresh-bg"), 3).await;

    assert!(
        builder_still_listing_indexes(&cs, app).await,
        "the rows must land while the builder is held before createIndexes and its record \
         step, otherwise this test proves nothing"
    );
    assert!(
        search_index_rows(&cs, &db_name).await > 0,
        "the writes made while the build was held must have indexed rows"
    );

    let outcome = backend.wait_for_search_index_build().await;
    fail_point.off().await;
    assert!(
        matches!(outcome, Some(BuildOutcome::Built { .. })),
        "{outcome:?}"
    );

    assert_eq!(
        captured(logs, BUILD_COMPLETE).len(),
        1,
        "positive control: the capture saw the build"
    );
    assert!(
        captured(logs, REINDEX_REMINDER).is_empty(),
        "{:?}",
        captured(logs, REINDEX_REMINDER)
    );
    assert!(captured(logs, UNKNOWN_ROWS_REMINDER).is_empty());

    let record = search_indexes_record(&cs, &db_name).await;
    assert_eq!(record.get_i32("generation"), Ok(4));
    assert!(
        !record.contains_key("empty_at_generation"),
        "recording the generation removes the marker: {record:?}"
    );
}

/// Rows written between a boot that recorded nothing (`off` mode, or a process
/// that stopped before its first build recorded) and the next boot are this
/// version's own: the next boot must not take them for an upgrade's. This is
/// the `schema_version` state a `background` boot leaves when the process stops
/// before its first build records.
#[tokio::test]
async fn rows_written_before_the_first_recorded_generation_log_no_reminder_on_the_next_boot() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("si_2268_off_then_inline");
    let tenant = create_tenant("si-2268-off-inline");

    let first = boot(&cs, &db_name, "si-2268-first-off", IndexBuildMode::Off).await;
    assert!(
        matches!(
            first.wait_for_search_index_build().await,
            Some(BuildOutcome::Skipped { .. })
        ),
        "an off-mode boot on a fresh database builds and records nothing"
    );
    let record = search_indexes_record(&cs, &db_name).await;
    assert!(
        !record.contains_key("generation"),
        "nothing recorded yet: {record:?}"
    );
    assert_eq!(
        record.get_i32("empty_at_generation"),
        Ok(4),
        "the first boot found search_index empty with no generation: {record:?}"
    );

    write_patients(&first, &tenant, 3).await;
    assert!(search_index_rows(&cs, &db_name).await > 0);

    let (logs, _guard) = capture_builder_logs();
    let second = boot(
        &cs,
        &db_name,
        "si-2268-second-inline",
        IndexBuildMode::Inline,
    )
    .await;
    let outcome = second.wait_for_search_index_build().await;
    assert!(
        matches!(outcome, Some(BuildOutcome::Built { .. })),
        "{outcome:?}"
    );

    assert_eq!(
        captured(logs, BUILD_COMPLETE).len(),
        1,
        "positive control: the capture saw the second boot's build"
    );
    assert!(
        captured(logs, REINDEX_REMINDER).is_empty(),
        "{:?}",
        captured(logs, REINDEX_REMINDER)
    );
    assert!(captured(logs, UNKNOWN_ROWS_REMINDER).is_empty());

    let record = search_indexes_record(&cs, &db_name).await;
    assert_eq!(record.get_i32("generation"), Ok(4));
    assert!(!record.contains_key("empty_at_generation"), "{record:?}");
}

/// A real upgrade: `search_index` already holds a row and no generation was
/// ever recorded. The reminder is still logged, once, with the same fields.
#[tokio::test]
async fn an_upgraded_database_with_rows_and_no_recorded_generation_still_gets_one_reminder() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("si_2268_upgrade");
    // A pre-#1391 date row: no `value_date_end`, and no `schema_version` yet.
    raw_test_client(&cs)
        .await
        .unwrap()
        .database(&db_name)
        .collection::<Document>("search_index")
        .insert_one(doc! {
            "tenant_id": "si-2268-upgrade",
            "resource_type": "Patient",
            "resource_id": "old",
            "param_name": "birthdate",
            "param_type": "date",
            "value_date": mongodb::bson::DateTime::from_millis(0),
        })
        .await
        .unwrap();

    let (logs, _guard) = capture_builder_logs();
    // Background mode, and nothing is written while it builds: on 7.0 writes
    // during an index build can hit catalog-change conflicts (#2231), and the
    // upgrade case does not need them.
    let backend = boot(&cs, &db_name, "si-2268-upgrade", IndexBuildMode::Background).await;
    let outcome = backend.wait_for_search_index_build().await;
    assert!(
        matches!(outcome, Some(BuildOutcome::Built { .. })),
        "{outcome:?}"
    );

    assert_eq!(
        captured(logs, BUILD_COMPLETE).len(),
        1,
        "positive control: the capture saw the build"
    );
    let lines = captured(logs, REINDEX_REMINDER);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("from_generation=None"), "{}", lines[0]);
    assert!(captured(logs, UNKNOWN_ROWS_REMINDER).is_empty());

    let record = search_indexes_record(&cs, &db_name).await;
    assert_eq!(record.get_i32("generation"), Ok(4));
    assert!(!record.contains_key("empty_at_generation"), "{record:?}");

    // A later boot is on the current generation: up to date, and silent.
    let again = boot(
        &cs,
        &db_name,
        "si-2268-upgrade-again",
        IndexBuildMode::Inline,
    )
    .await;
    assert!(
        matches!(
            again.wait_for_search_index_build().await,
            Some(BuildOutcome::UpToDate)
        ),
        "a database already at generation 4 is up to date"
    );
    assert_eq!(captured(logs, REINDEX_REMINDER).len(), 1);
}

/// The boot-time count fails on a fresh database. HFS cannot tell an upgrade
/// from a new database, so it logs the hedged line (not the regular reminder),
/// records no marker, and still boots and records the generation.
#[tokio::test]
async fn a_failed_boot_count_logs_the_hedged_reminder_and_records_no_marker() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let app = "fp-2268-count-fails";
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["count"], "errorCode": 2_i32 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_builder_logs();
    let db_name = build_test_database_name("si_2268_count_fails");

    let backend = boot(&cs, &db_name, app, IndexBuildMode::Inline).await;
    assert_eq!(
        fail_point.off_and_count().await,
        1,
        "the builder's boot count is the only `count` this client sends"
    );
    let outcome = backend.wait_for_search_index_build().await;
    assert!(
        matches!(outcome, Some(BuildOutcome::Built { .. })),
        "a failed boot count must not fail boot: {outcome:?}"
    );

    assert_eq!(
        captured(logs, BUILD_COMPLETE).len(),
        1,
        "positive control: the capture saw the build"
    );
    let hedged = captured(logs, UNKNOWN_ROWS_REMINDER);
    assert_eq!(hedged.len(), 1, "{hedged:?}");
    // The error kind, not the driver's raw server response.
    assert!(hedged[0].contains("BadValue"), "{}", hedged[0]);
    assert!(!hedged[0].contains("RawDocumentBuf"), "{}", hedged[0]);
    assert!(
        captured(logs, REINDEX_REMINDER).is_empty(),
        "{:?}",
        captured(logs, REINDEX_REMINDER)
    );

    let record = search_indexes_record(&cs, &db_name).await;
    assert_eq!(record.get_i32("generation"), Ok(4));
    assert!(
        !record.contains_key("empty_at_generation"),
        "a failed count records no marker: {record:?}"
    );
}
