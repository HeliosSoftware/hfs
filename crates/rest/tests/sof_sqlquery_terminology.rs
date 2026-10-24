//! SQL query ValueSet/ConceptMap dependencies (#1824), end to end on the
//! SQLite backend. The scenarios are shared by every backend; see
//! `common/sof_terminology_e2e.rs`.

#[path = "common/sof_terminology_e2e.rs"]
mod sof_terminology_e2e;

use std::sync::Arc;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use sof_terminology_e2e::Harness;

async fn sqlite(terminology: Option<String>) -> Option<Harness> {
    let config = SqliteBackendConfig {
        data_dir: Some(sof_terminology_e2e::data_dir()),
        ..Default::default()
    };
    let backend =
        SqliteBackend::with_config(":memory:", config).expect("failed to create SQLite backend");
    backend.init_schema().expect("failed to init schema");
    Some(sof_terminology_e2e::harness(Arc::new(backend), terminology))
}

sof_terminology_e2e_suite!(sqlite);
