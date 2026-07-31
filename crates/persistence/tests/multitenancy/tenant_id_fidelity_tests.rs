//! SQLite arm of the backend-agnostic tenant-id fidelity suite (issue #447).
//!
//! SQLite stores `tenant_id` as a `TEXT` column in each table's composite
//! primary key and compares it with a bound `=` under the default BINARY
//! collation, so the tenant scoping is the identity mapping and no derivation
//! can lose information. That is a code reading; these run it.
//!
//! See [`super::tenant_id_fidelity_suite`] for why the claim is asserted per
//! backend rather than argued once.

#![cfg(feature = "sqlite")]

use helios_persistence::backends::sqlite::SqliteBackend;

use super::tenant_id_fidelity_suite as suite;

fn backend() -> SqliteBackend {
    let backend = SqliteBackend::in_memory().expect("in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    backend
}

#[tokio::test]
async fn sqlite_distinct_tenant_ids_never_share_data() {
    // Each test builds its own in-memory database, so a fixed base id is safe.
    suite::distinct_tenant_ids_never_share_data(&backend(), "acme").await;
}

#[tokio::test]
async fn sqlite_purging_one_tenant_leaves_the_look_alikes_intact() {
    suite::purging_one_tenant_leaves_the_look_alikes_intact(&backend(), "acme").await;
}
