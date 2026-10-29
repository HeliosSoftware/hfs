//! A tenant purge from `/ui/tenants` deletes the tenant's bulk export and
//! submit output files (#2279).
//!
//! The purge drops the export job rows the expiry sweeps start from, so files
//! it leaves behind are never reached again. The output stores are registered
//! process-wide ([`helios_ui::register_tenant_purge_output`]), which is why
//! this test has its own binary: no other test shares the registry.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{ExportJobId, ExportOutputStore, ExportPartKey, ResourceStorage};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tower::ServiceExt;

#[tokio::test]
async fn purge_deletes_the_tenants_output_files() {
    let root = std::env::temp_dir().join(format!(
        "hfs-ui-purge-outputs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = Arc::new(LocalFsOutputStore::new(&root, "http://localhost:8080"));
    for tenant in ["acme", "other"] {
        let key = ExportPartKey::output(tenant, ExportJobId::new(), "Patient", 0, 1);
        let mut writer = output.open_writer(&key).await.unwrap();
        writer.write_line("{}").await.unwrap();
        output.finalize_part(&key, writer).await.unwrap();
    }
    helios_ui::register_tenant_purge_output(output as Arc<dyn ExportOutputStore>);

    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let store: Arc<dyn ResourceStorage> = Arc::new(backend);
    let acme = TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access());
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

    assert!(!root.join("acme").exists(), "acme's output files survived");
    assert!(
        root.join("other").exists(),
        "another tenant's files were deleted"
    );
    assert!(!store.tenant_has_resources(&acme).await.unwrap());
    let _ = std::fs::remove_dir_all(&root);
}
