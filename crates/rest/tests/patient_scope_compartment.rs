//! A `patient/` scope is confined to the launch patient's compartment (#1618)
//! — SQLite in-memory.
//!
//! The scenarios live in `common/patient_scope.rs` and run unchanged against
//! PostgreSQL (`patient_scope_compartment_postgres.rs`), MongoDB
//! (`patient_scope_compartment_mongodb.rs`) and SQLite + Elasticsearch
//! (`patient_scope_compartment_elasticsearch.rs`). The issue's repro is
//! `the_issue_repro_is_closed`.

#[path = "common/patient_scope.rs"]
mod patient_scope;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::BulkExportJobStore;
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(patient_scope::data_dir()),
                ..Default::default()
            },
        )
        .expect("in-memory sqlite"),
    );
    backend.init_schema().expect("init schema");
    let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
        .with_bulk_export(
            backend.clone() as Arc<dyn BulkExportJobStore>,
            patient_scope::export_output(),
            Arc::new(BearerScopeAuth),
        );
    Some(patient_scope::serve(
        helios_rest::routing::fhir_routes::create_routes(state),
    ))
}

patient_scope_suite!(create_test_server);
