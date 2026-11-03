//! #2352: a build refuses a database whose `schema_version` is newer than its
//! own `SCHEMA_VERSION`, and never lowers the recorded version.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`build_test_database_name`, `raw_test_client`,
//! `shared_mongo`, `TEST_BACKEND_MAX_POOL`), the same arrangement as
//! `tests/mongodb/count_by_tenant_index.rs`.
//!
//! The backends boot with `IndexBuildMode::Off`, so a test sees exactly what
//! `initialize_schema_async` and `migrate_schema_async` do and not the
//! `search_index` builder. `idx_resources_live_tenant` (created by
//! `ensure_resources_indexes`, the first `ensure_*` call of both paths) is the
//! probe for "something was written": a refused boot must not recreate it.

use super::*;
use helios_persistence::backends::mongodb::SCHEMA_VERSION;

/// Created by schema init (v12, #1910); the probe for a write by a refused boot.
const LIVE_TENANT_INDEX: &str = "idx_resources_live_tenant";

/// A database that has finished its first boot.
struct Booted {
    config: MongoBackendConfig,
    db: mongodb::Database,
}

/// Boots a backend on `config` and waits for the index builders `init_schema`
/// leaves running detached: the search_index one writes
/// `search_indexes.contained_rows_moved` to the `schema_version` document
/// (`move_contained_rows` runs in every mode, `off` included), and that write
/// must not land after a test has read the document.
async fn boot(config: &MongoBackendConfig) -> Result<(), StorageError> {
    let backend = MongoBackend::new(config.clone()).expect("create MongoBackend");
    backend.init_schema().await?;
    backend.wait_for_search_index_build().await;
    backend.wait_for_missing_index_build().await;
    Ok(())
}

/// Boots a backend on a fresh database and returns its config and a raw handle
/// to the database. `None` (after saying so) when no MongoDB is available.
async fn first_boot(test_name: &str) -> Option<Booted> {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        eprintln!("Skipping {test_name} (requires Docker or HFS_TEST_MONGODB_URL)");
        return None;
    };
    let config = MongoBackendConfig {
        connection_string: connection_string.clone(),
        database_name: build_test_database_name(test_name),
        index_build: IndexBuildMode::Off,
        max_connections: Some(TEST_BACKEND_MAX_POOL),
        ..Default::default()
    };
    boot(&config).await.expect("first boot");
    let db = raw_test_client(&connection_string)
        .await
        .expect("raw MongoDB client")
        .database(&config.database_name);
    Some(Booted { config, db })
}

/// A backend that has not booted yet, on the same database.
fn backend(booted: &Booted) -> MongoBackend {
    MongoBackend::new(booted.config.clone()).expect("create MongoBackend")
}

fn schema_version_collection(db: &mongodb::Database) -> mongodb::Collection<Document> {
    db.collection::<Document>("schema_version")
}

/// The whole `schema_version` document, `None` if there is none.
async fn stored(db: &mongodb::Database) -> Option<Document> {
    schema_version_collection(db)
        .find_one(doc! { "_id": "schema_version" })
        .await
        .expect("read schema_version")
}

/// The stored `version` field, with its BSON type.
async fn stored_version(db: &mongodb::Database) -> Option<Bson> {
    stored(db).await.and_then(|doc| doc.get("version").cloned())
}

/// Writes `version` the way a newer build, or an operator in mongosh, would.
async fn record(db: &mongodb::Database, version: Bson) {
    schema_version_collection(db)
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "version": version } },
        )
        .upsert(true)
        .await
        .expect("record the schema version");
}

async fn live_tenant_index_exists(db: &mongodb::Database) -> bool {
    db.collection::<Document>("resources")
        .list_index_names()
        .await
        .expect("list resources indexes")
        .iter()
        .any(|name| name == LIVE_TENANT_INDEX)
}

async fn drop_live_tenant_index(db: &mongodb::Database) {
    db.run_command(doc! { "dropIndexes": "resources", "index": LIVE_TENANT_INDEX })
        .await
        .expect("drop idx_resources_live_tenant");
}

/// A database a newer build has upgraded: `init_schema` and the migrate path
/// both refuse it, naming both versions and saying a newer HFS wrote it, and
/// neither writes anything. The version counts whatever BSON type holds it,
/// because mongosh writes doubles.
#[tokio::test]
async fn a_newer_schema_version_is_refused_before_anything_is_written() {
    let Some(booted) =
        first_boot("a_newer_schema_version_is_refused_before_anything_is_written").await
    else {
        return;
    };
    let db = &booted.db;
    drop_live_tenant_index(db).await;
    let newer = SCHEMA_VERSION + 1;

    for recorded in [
        Bson::Int32(newer),
        Bson::Double(f64::from(newer)),
        Bson::Int64(i64::from(newer)),
        // Newer, but not a whole i32: the guard compares on the server.
        Bson::Double(f64::from(SCHEMA_VERSION) + 0.5),
        Bson::Int64(i64::from(i32::MAX) + 1),
    ] {
        record(db, recorded.clone()).await;
        let before = stored(db).await;
        assert_eq!(
            before.as_ref().and_then(|doc| doc.get("version")),
            Some(&recorded),
            "the test setup records {recorded:?}"
        );

        let error = backend(&booted).init_schema().await.expect_err(&format!(
            "init_schema must refuse a database at {recorded:?}"
        ));
        assert!(
            matches!(
                &error,
                StorageError::Backend(BackendError::Internal { backend_name, .. })
                    if backend_name == "mongodb"
            ),
            "{error:?}"
        );
        let message = error.to_string();
        for expected in [
            format!("has schema_version {recorded}"),
            format!("the {SCHEMA_VERSION} this HFS build supports"),
            "written by a newer HFS".to_string(),
            "The schema was not changed".to_string(),
        ] {
            assert!(
                message.contains(&expected),
                "{expected:?} not in init_schema's error for {recorded:?}: {message}"
            );
        }

        // The `Backend` entry points `hfs` uses wrap the same error.
        let initialize = backend(&booted)
            .initialize()
            .await
            .expect_err("initialize must refuse a newer database");
        assert!(
            initialize.to_string().contains("written by a newer HFS"),
            "{initialize}"
        );
        let migrate = backend(&booted)
            .migrate()
            .await
            .expect_err(&format!("migrate must refuse a database at {recorded:?}"));
        let migrate = migrate.to_string();
        assert!(migrate.contains("written by a newer HFS"), "{migrate}");
        assert!(
            migrate.contains(&format!("has schema_version {recorded}")),
            "{migrate}"
        );

        assert_eq!(
            stored(db).await,
            before,
            "a refused boot leaves the whole schema_version document as it was \
             (recorded {recorded:?})"
        );
        assert!(
            !live_tenant_index_exists(db).await,
            "a refused boot creates no index (recorded {recorded:?})"
        );
    }
}

/// How a database stands before a boot in
/// [`a_database_at_or_below_the_build_version_initializes_and_migrates_as_before`].
async fn set_up(db: &mongodb::Database, case: &str) {
    let collection = schema_version_collection(db);
    match case {
        "at" => record(db, Bson::Int32(SCHEMA_VERSION)).await,
        "below" => {
            record(db, Bson::Int32(SCHEMA_VERSION - 1)).await;
            if live_tenant_index_exists(db).await {
                drop_live_tenant_index(db).await;
            }
        }
        "absent" => {
            collection
                .delete_one(doc! { "_id": "schema_version" })
                .await
                .expect("delete the schema_version document");
            if live_tenant_index_exists(db).await {
                drop_live_tenant_index(db).await;
            }
        }
        // The document `set_search_index_generation` leaves before any boot
        // records a version: a `search_indexes` record and no `version`.
        "no version field" => {
            collection
                .replace_one(
                    doc! { "_id": "schema_version" },
                    doc! { "_id": "schema_version", "search_indexes": { "generation": 1_i32 } },
                )
                .upsert(true)
                .await
                .expect("replace the schema_version document");
            if live_tenant_index_exists(db).await {
                drop_live_tenant_index(db).await;
            }
        }
        other => panic!("unknown case {other}"),
    }
}

/// What a boot must leave behind, whichever case it started from.
async fn check_after_boot(db: &mongodb::Database, case: &str, path: &str) {
    assert_eq!(
        stored_version(db).await,
        Some(Bson::Int32(SCHEMA_VERSION)),
        "{case}, {path}: the boot records the current schema version"
    );
    assert!(
        live_tenant_index_exists(db).await,
        "{case}, {path}: the index of the current schema exists"
    );
    if case == "no version field" {
        let generation = stored(db)
            .await
            .and_then(|doc| doc.get_document("search_indexes").ok().cloned())
            .and_then(|record| record.get("generation").cloned());
        assert_eq!(
            generation,
            Some(Bson::Int32(1)),
            "{case}, {path}: the version write keeps the sibling search_indexes record"
        );
    }
}

/// Nothing changes for a database at the build's version, below it, with no
/// `schema_version` document, or with one that has no `version`: both entry
/// points initialize it and the stored version ends at `SCHEMA_VERSION`. (This
/// passes without the guard too; it pins that the guard and the conditional
/// write refuse nothing they should accept.)
#[tokio::test]
async fn a_database_at_or_below_the_build_version_initializes_and_migrates_as_before() {
    let Some(booted) =
        first_boot("a_database_at_or_below_the_build_version_initializes_and_migrates_as_before")
            .await
    else {
        return;
    };
    let db = &booted.db;
    assert_eq!(
        stored_version(db).await,
        Some(Bson::Int32(SCHEMA_VERSION)),
        "the first boot records the current schema version"
    );

    for case in ["at", "below", "absent", "no version field"] {
        set_up(db, case).await;
        boot(&booted.config)
            .await
            .unwrap_or_else(|error| panic!("{case}: init_schema: {error}"));
        check_after_boot(db, case, "init_schema").await;

        set_up(db, case).await;
        backend(&booted)
            .migrate()
            .await
            .unwrap_or_else(|error| panic!("{case}: migrate: {error}"));
        check_after_boot(db, case, "migrate").await;
    }
}

/// Two builds starting at once never lower the version. A newer build raises
/// the version while an older one migrates, at a different point of the older
/// one's run each round; the raise only matches while the old value is still
/// stored, so whether it matched says which write came first. If the raise
/// landed first, the older build refuses and the version stays raised, and in
/// some round it does so after its guard and `ensure_*` calls, in
/// `set_schema_version`'s conditional write. If the migrate recorded its
/// version first, it succeeds and that version stays. Several builds
/// recording the same version into a database with no `schema_version`
/// document all succeed.
#[tokio::test]
async fn concurrent_boots_never_lower_a_newer_version() {
    let Some(booted) = first_boot("concurrent_boots_never_lower_a_newer_version").await else {
        return;
    };
    let db = &booted.db;
    let boot = backend(&booted);
    boot.migrate().await.expect("warm the connection pool");
    let newer = SCHEMA_VERSION + 1;

    // The raise lands at a different point of the older build's migrate each
    // round: before its read, between its read and its write, and after it.
    let mut refused_after_the_guard = 0;
    for round in 0..30_u64 {
        record(db, Bson::Int32(SCHEMA_VERSION - 1)).await;
        if live_tenant_index_exists(db).await {
            drop_live_tenant_index(db).await;
        }
        let (migrated, raised) = tokio::join!(boot.migrate(), async {
            tokio::time::sleep(std::time::Duration::from_millis(round * 2)).await;
            // Raises only the older value, so it matches nothing once the
            // migrate has recorded its own version.
            schema_version_collection(db)
                .update_one(
                    doc! { "_id": "schema_version", "version": SCHEMA_VERSION - 1 },
                    doc! { "$set": { "version": newer } },
                )
                .await
        });
        let raised = raised
            .expect("a newer build records its version")
            .matched_count
            == 1;
        if raised {
            // The raise landed before the migrate's version write: that write
            // refuses.
            assert_eq!(
                stored_version(db).await,
                Some(Bson::Int32(newer)),
                "round {round}: an older boot lowered a newer version: {migrated:?}"
            );
            let error = migrated.expect_err(&format!(
                "round {round}: an older boot accepted a database raised to {newer}"
            ));
            assert!(
                error.to_string().contains("written by a newer HFS"),
                "round {round}: {error}"
            );
            if live_tenant_index_exists(db).await {
                // It passed the guard and ran its ensure_* calls: the
                // conditional write in set_schema_version refused it.
                refused_after_the_guard += 1;
            }
        } else {
            // The migrate recorded its version first; the raise found nothing
            // to raise.
            migrated.unwrap_or_else(|error| panic!("round {round}: {error}"));
            assert_eq!(
                stored_version(db).await,
                Some(Bson::Int32(SCHEMA_VERSION)),
                "round {round}"
            );
        }
    }
    eprintln!("rounds refused after the guard: {refused_after_the_guard} of 30");
    assert!(
        refused_after_the_guard > 0,
        "no round landed the raise between the guard and the version write, so \
         set_schema_version's refusal was not exercised"
    );

    // Several boots recording the same version into a database with no
    // schema_version document all succeed; whether a round takes the
    // duplicate-key retry depends on timing.
    for round in 0..10 {
        schema_version_collection(db)
            .delete_one(doc! { "_id": "schema_version" })
            .await
            .expect("delete the schema_version document");
        let (a, b, c, d) = tokio::join!(
            boot.migrate(),
            boot.migrate(),
            boot.migrate(),
            boot.migrate()
        );
        for (name, result) in [("a", a), ("b", b), ("c", c), ("d", d)] {
            result.unwrap_or_else(|error| panic!("round {round}, boot {name}: {error}"));
        }
        assert_eq!(
            stored_version(db).await,
            Some(Bson::Int32(SCHEMA_VERSION)),
            "round {round}: every boot recorded the build's version"
        );
    }
}
