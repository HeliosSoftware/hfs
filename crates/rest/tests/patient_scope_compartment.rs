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

/// The capture the suites rely on (#2045) reaches an internal error's detail,
/// which the generic 500 OperationOutcome withholds from the client. This
/// does not need a real 500 from a backend, so it proves the filter, target,
/// writer and thread-local dispatch work on every run. It uses
/// `HFS_TEST_SERVER_LOG_FILTER` like the suites, so a filter that drops
/// `helios_rest` ERROR events makes it fail.
#[test]
fn the_server_log_records_an_internal_errors_detail() {
    let mut log = patient_scope::ServerLog::capture("self-check");
    log.disarm();
    let response =
        axum::response::IntoResponse::into_response(helios_rest::RestError::InternalError {
            message: "detail-2045: Error code 112 (WriteConflict)".to_string(),
        });
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let text = log.text();
    assert!(
        text.contains("internal error while processing request"),
        "{text}"
    );
    assert!(text.contains("detail-2045"), "{text}");
}
