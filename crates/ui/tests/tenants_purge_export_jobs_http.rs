//! A tenant purge from `/ui/tenants` also deletes the tenant's bulk export
//! jobs from the export job store when that store is not the primary, as on a
//! MongoDB or S3 primary with its embedded SQLite sidecar.
//!
//! The job stores are registered process-wide
//! ([`helios_ui::register_tenant_purge_export_jobs`]), which is why this test
//! has its own binary: no other test shares the registry.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{
    BulkExportJobStore, BulkExportStorage, ExportRequest, ResourceStorage, StartExportInput,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tower::ServiceExt;

#[tokio::test]
async fn purge_deletes_the_tenants_jobs_from_a_separate_job_store() {
    let sidecar = SqliteBackend::in_memory().expect("in-memory sqlite");
    sidecar.init_schema().expect("init schema");
    let sidecar = Arc::new(sidecar);
    let acme = TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access());
    let other = TenantContext::new(TenantId::new("other"), TenantPermissions::full_access());
    let mut jobs = Vec::new();
    for tenant in [&acme, &other] {
        jobs.push(
            sidecar
                .start_export(
                    tenant,
                    StartExportInput {
                        request: ExportRequest::system(),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost:8080/$export".to_string(),
                        owner_subject: None,
                        fhir_version: FhirVersion::R4,
                    },
                )
                .await
                .expect("export job"),
        );
    }
    helios_ui::register_tenant_purge_export_jobs(sidecar.clone() as Arc<dyn BulkExportJobStore>);

    let primary = SqliteBackend::in_memory().expect("in-memory sqlite");
    primary.init_schema().expect("init schema");
    let store: Arc<dyn ResourceStorage> = Arc::new(primary);
    store
        .create(
            &acme,
            "Patient",
            serde_json::json!({"resourceType": "Patient"}),
            FhirVersion::R4,
        )
        .await
        .unwrap();

    let router = helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        Some(Arc::clone(&store)),
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    );
    let res = router
        .oneshot(
            Request::delete("/ui/tenants/acme?purge=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    assert!(
        sidecar.get_export_status(&acme, &jobs[0]).await.is_err(),
        "acme's export job survived the purge"
    );
    assert!(
        sidecar.get_export_status(&other, &jobs[1]).await.is_ok(),
        "another tenant's export job was purged"
    );
}
