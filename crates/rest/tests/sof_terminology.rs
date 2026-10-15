//! "Terminology in SQL" (#1824) — SQLite.
//!
//! Runs the scenarios in `common/terminology_scenarios.rs` against an
//! in-memory SQLite backend, the way `sof_sqlquery_terminology.rs` and
//! `sof_terminology_materialize.rs` did before they became these shared
//! bodies. The `_postgres`, `_mongodb`, `_sqlite_elasticsearch` and `_s3`
//! twins run the same scenarios on their backends.

#![cfg(feature = "sqlite")]

#[path = "common/terminology_scenarios.rs"]
mod terminology_scenarios;

use std::sync::Arc;

use async_trait::async_trait;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::ServerConfig;

use terminology_scenarios::{Driver, Hfs};

struct Sqlite;

#[async_trait]
impl Driver for Sqlite {
    async fn server(&self, config: ServerConfig, scopes: Option<&str>) -> Hfs {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        terminology_scenarios::build(Arc::new(backend), config, scopes, true, true).await
    }
}

async fn driver(_name: &str) -> Option<Sqlite> {
    Some(Sqlite)
}

terminology_suite!(driver);
