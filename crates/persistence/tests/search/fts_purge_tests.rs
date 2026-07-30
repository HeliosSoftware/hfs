//! SQLite bindings for the backend-agnostic full-text purge suite (issue #386).
//!
//! The scenarios live in `fts_purge_suite.rs` and are shared verbatim with the
//! PostgreSQL test binary; this file only supplies the SQLite backend, the
//! [`FtsProbe`] implementation, and one `#[tokio::test]` per scenario.
//!
//! ## Why these tests use a file-backed database
//!
//! Proving the fix requires reading `resource_fts` directly — a search-level
//! assertion is vacuous here (see the suite's module docs). `SqliteBackend`'s
//! connection pool is `pub(crate)` and unreachable from an integration test, and
//! the usual `:memory:` harness uses a private per-instance shared-cache name
//! that nothing else can attach to. A temporary file solves both without adding
//! a `#[doc(hidden)] pub` accessor to production code purely for tests.

#![cfg(feature = "sqlite")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::search::ReindexOperation;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

use super::fts_purge_suite::{self as suite, FtsProbe};

/// A file-backed SQLite backend plus the path its data lives at.
///
/// The `TempDir` is returned so the caller keeps it alive: dropping it deletes
/// the database out from under the backend.
fn file_backend() -> (SqliteBackend, tempfile::TempDir, PathBuf) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("fts_purge.db");

    let config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(&db_path, config).expect("file-backed SQLite backend");
    backend.init_schema().expect("init schema");
    (backend, dir, db_path)
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

/// Reads `resource_fts` over a second connection to the same database file.
struct SqliteFtsProbe(PathBuf);

impl SqliteFtsProbe {
    fn open(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.0).expect("probe connection")
    }
}

#[async_trait::async_trait]
impl FtsProbe for SqliteFtsProbe {
    async fn fts_row_count(&self, tenant_id: &str) -> u64 {
        let conn = self.open();
        conn.query_row(
            "SELECT COUNT(*) FROM resource_fts WHERE tenant_id = ?1",
            [tenant_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("resource_fts must exist in a bundled-rusqlite build") as u64
    }

    async fn fts_rows_containing(&self, needle: &str) -> u64 {
        let conn = self.open();
        let pattern = format!("%{needle}%");
        conn.query_row(
            "SELECT COUNT(*) FROM resource_fts \
             WHERE full_content LIKE ?1 OR narrative_text LIKE ?1",
            [&pattern],
            |row| row.get::<_, i64>(0),
        )
        .expect("resource_fts must exist in a bundled-rusqlite build") as u64
    }
}

/// FTS5 must be compiled into the test build.
///
/// Without it `create_fts_table` silently creates nothing and every full-text
/// assertion in this suite would be testing an absent feature. `rusqlite` is
/// pinned with the `bundled` feature, whose amalgamation is built with
/// `-DSQLITE_ENABLE_FTS5`; this fails loudly if that ever changes.
#[test]
fn fts5_is_compiled_into_the_test_build() {
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory connection");
    let modules: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_module_list WHERE name = 'fts5'",
            [],
            |row| row.get(0),
        )
        .expect("pragma_module_list should be queryable");
    assert_eq!(
        modules, 1,
        "FTS5 is absent from this SQLite build, so every _text/_content \
         assertion in this suite would be vacuous. rusqlite must keep its \
         `bundled` feature (crates/persistence/Cargo.toml)."
    );
}

/// The FTS5 table really is created by `init_schema` on a fresh database.
#[test]
fn resource_fts_table_exists_after_init_schema() {
    let (_backend, _dir, path) = file_backend();
    let conn = rusqlite::Connection::open(&path).expect("probe connection");
    let found: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='resource_fts'",
            [],
            |row| row.get(0),
        )
        .expect("sqlite_master query");
    assert_eq!(found, 1, "init_schema must create resource_fts");
}

macro_rules! sqlite_fts_test {
    ($name:ident, $scenario:ident) => {
        #[tokio::test]
        async fn $name() {
            let (backend, _dir, path) = file_backend();
            let probe = SqliteFtsProbe(path);
            suite::$scenario(&backend, &probe, &tenant("acme")).await;
        }
    };
}

sqlite_fts_test!(sqlite_purge_removes_fts_rows, purge_removes_fts_rows);
sqlite_fts_test!(
    sqlite_purge_all_removes_fts_rows,
    purge_all_removes_fts_rows
);
sqlite_fts_test!(
    sqlite_purge_tenant_data_removes_fts_rows,
    purge_tenant_data_removes_fts_rows
);
sqlite_fts_test!(
    sqlite_reuse_after_purge_does_not_resurrect_narrative,
    reuse_after_purge_does_not_resurrect_narrative
);
sqlite_fts_test!(
    sqlite_tenant_reuse_does_not_resurrect_narrative,
    tenant_reuse_does_not_resurrect_narrative
);
sqlite_fts_test!(
    sqlite_repeated_purge_and_recreate_does_not_grow_fts,
    repeated_purge_and_recreate_does_not_grow_fts
);

#[tokio::test]
async fn sqlite_purge_tenant_data_leaves_other_tenants_intact() {
    let (backend, _dir, path) = file_backend();
    let probe = SqliteFtsProbe(path);
    suite::purge_tenant_data_leaves_other_tenants_intact(
        &backend,
        &probe,
        &tenant("acme"),
        &tenant("globex"),
    )
    .await;
}

#[tokio::test]
async fn sqlite_reindex_preserves_full_text_search_without_clear() {
    let (backend, _dir, path) = file_backend();
    let probe = SqliteFtsProbe(path);
    let backend = Arc::new(backend);
    let reindex = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    suite::reindex_preserves_full_text_search(
        backend.as_ref(),
        &probe,
        &tenant("acme"),
        &reindex,
        false,
    )
    .await;
}

#[tokio::test]
async fn sqlite_reindex_preserves_full_text_search_with_clear() {
    let (backend, _dir, path) = file_backend();
    let probe = SqliteFtsProbe(path);
    let backend = Arc::new(backend);
    let reindex = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    suite::reindex_preserves_full_text_search(
        backend.as_ref(),
        &probe,
        &tenant("acme"),
        &reindex,
        true,
    )
    .await;
}

#[tokio::test]
async fn sqlite_repeated_reindex_does_not_duplicate_fts_rows() {
    let (backend, _dir, path) = file_backend();
    let probe = SqliteFtsProbe(path);
    let backend = Arc::new(backend);
    let reindex = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    suite::repeated_reindex_does_not_duplicate_fts_rows(
        backend.as_ref(),
        &probe,
        &tenant("acme"),
        &reindex,
    )
    .await;
}

/// A purge must succeed on a database with no FTS5 table.
///
/// FTS5 is an optional SQLite compile-time feature and `create_fts_table`
/// tolerates its absence, so the purge paths probe before deleting. Dropping the
/// table reproduces that state: purge must return `Ok`, not fail on
/// `no such table: resource_fts`.
#[tokio::test]
async fn sqlite_purge_succeeds_when_fts_table_is_absent() {
    let (backend, _dir, path) = file_backend();
    let tenant = tenant("acme");

    helios_persistence::core::ResourceStorage::create(
        &backend,
        &tenant,
        "Patient",
        suite::patient_with_narrative("p1", suite::PLANTED_TERM),
        helios_fhir::FhirVersion::default(),
    )
    .await
    .expect("seed create");

    drop_fts_table(&path);

    use helios_persistence::core::PurgableStorage;
    backend
        .purge(&tenant, "Patient", "p1")
        .await
        .expect("purge must tolerate a database without FTS5");
    backend
        .purge_all(&tenant, "Patient")
        .await
        .expect("purge_all must tolerate a database without FTS5");
    helios_persistence::core::ResourceStorage::purge_tenant_data(&backend, "acme")
        .await
        .expect("purge_tenant_data must tolerate a database without FTS5");
}

fn drop_fts_table(path: &Path) {
    let conn = rusqlite::Connection::open(path).expect("probe connection");
    conn.execute("DROP TABLE resource_fts", [])
        .expect("drop resource_fts");
}

/// The `v14 -> v15` migration sweeps orphans left by the pre-fix purge paths.
///
/// The code fix only stops *new* orphans. Every database that has already served
/// a purge is still holding the narrative and full body of resources its
/// operator was told were removed, so the fix is only half delivered without
/// this sweep.
///
/// Reproduces that state directly — an FTS row whose `resources` row is gone,
/// which is exactly what the old `purge` left behind — then reopens the database
/// at the pre-migration version and asserts the orphan is swept while a live
/// resource's row is untouched.
#[tokio::test]
async fn migration_sweeps_orphaned_fts_rows_from_existing_databases() {
    let (backend, _dir, path) = file_backend();
    let tenant = tenant("acme");

    // One resource that stays, one that will be orphaned.
    for id in ["keeper", "orphan"] {
        helios_persistence::core::ResourceStorage::create(
            &backend,
            &tenant,
            "Patient",
            suite::patient_with_narrative(id, suite::PLANTED_TERM),
            helios_fhir::FhirVersion::default(),
        )
        .await
        .expect("seed create");
    }
    drop(backend);

    {
        let conn = rusqlite::Connection::open(&path).expect("probe connection");
        // Delete the resource WITHOUT touching resource_fts — precisely what the
        // pre-fix purge paths did.
        conn.execute(
            "DELETE FROM resources WHERE tenant_id = 'acme' AND id = 'orphan'",
            [],
        )
        .expect("simulate a pre-fix purge");

        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_fts WHERE tenant_id = 'acme'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(
            rows, 2,
            "POSITIVE CONTROL: the simulated pre-fix purge must leave the orphan behind"
        );

        // Wind the recorded schema version back so the migration ladder re-runs.
        conn.execute("DELETE FROM schema_version", [])
            .expect("clear version");
        conn.execute("INSERT INTO schema_version (version) VALUES (14)", [])
            .expect("set version 14");
    }

    // Reopening runs migrate_v14_to_v15.
    let config = SqliteBackendConfig {
        data_dir: None,
        ..Default::default()
    };
    let reopened = SqliteBackend::with_config(&path, config).expect("reopen backend");
    reopened.init_schema().expect("migrate to v15");

    let conn = rusqlite::Connection::open(&path).expect("probe connection");
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM resource_fts WHERE tenant_id = 'acme'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        remaining, 1,
        "the v15 migration must sweep the orphaned resource_fts row and keep the live one"
    );

    let orphan_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM resource_fts WHERE resource_id = 'orphan'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(orphan_rows, 0, "the orphan's row specifically must be gone");

    let keeper_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM resource_fts WHERE resource_id = 'keeper'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        keeper_rows, 1,
        "the migration must not delete rows for resources that still exist"
    );
}

/// A soft-deleted resource keeps its `resources` row, so the sweep must not
/// touch its full-text entry — matching current behaviour, where a soft delete
/// does not drop the FTS row.
#[tokio::test]
async fn migration_preserves_fts_rows_for_soft_deleted_resources() {
    let (backend, _dir, path) = file_backend();
    let tenant = tenant("acme");

    helios_persistence::core::ResourceStorage::create(
        &backend,
        &tenant,
        "Patient",
        suite::patient_with_narrative("p1", suite::PLANTED_TERM),
        helios_fhir::FhirVersion::default(),
    )
    .await
    .expect("seed create");
    helios_persistence::core::ResourceStorage::delete(&backend, &tenant, "Patient", "p1")
        .await
        .expect("soft delete");
    drop(backend);

    {
        let conn = rusqlite::Connection::open(&path).expect("probe connection");
        conn.execute("DELETE FROM schema_version", []).unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (14)", [])
            .unwrap();
    }

    let reopened = SqliteBackend::with_config(
        &path,
        SqliteBackendConfig {
            data_dir: None,
            ..Default::default()
        },
    )
    .expect("reopen backend");
    reopened.init_schema().expect("migrate to v15");

    let conn = rusqlite::Connection::open(&path).expect("probe connection");
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM resource_fts WHERE tenant_id = 'acme'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        rows, 1,
        "a soft-deleted resource still has a `resources` row, so the sweep must \
         leave its full-text entry alone"
    );
}
