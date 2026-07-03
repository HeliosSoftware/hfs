//! S3 backend tests.
//!
//! - Fast local tests live under `src/backends/s3/tests.rs` with a mock S3 client.
//! - Real AWS tests in this file are opt-in via `RUN_AWS_S3_TESTS=1`.

#![cfg(feature = "s3")]

use std::collections::HashMap;

use helios_fhir::FhirVersion;
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};
use helios_persistence::core::bulk_submit::{
    BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry, SubmissionId,
};
use helios_persistence::core::history::{HistoryParams, InstanceHistoryProvider};
use helios_persistence::core::transaction::{BundleEntry, BundleMethod, BundleProvider};
use helios_persistence::core::{BackendCapability, ResourceStorage, VersionedStorage};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;
use uuid::Uuid;

fn run_aws_tests() -> bool {
    std::env::var("RUN_AWS_S3_TESTS").ok().as_deref() == Some("1")
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn make_prefix_backend(prefix: String) -> S3Backend {
    let bucket = std::env::var("HFS_S3_TEST_BUCKET")
        .expect("HFS_S3_TEST_BUCKET must be set when RUN_AWS_S3_TESTS=1");

    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant { bucket },
        prefix: Some(prefix),
        region: std::env::var("AWS_REGION").ok(),
        validate_buckets_on_startup: true,
        ..Default::default()
    };

    S3Backend::from_env(config).expect("create S3 backend")
}

fn make_bucket_per_tenant_backend(prefix: String) -> Option<S3Backend> {
    let bucket_a = std::env::var("HFS_S3_TEST_BUCKET_TENANT_A").ok()?;
    let bucket_b = std::env::var("HFS_S3_TEST_BUCKET_TENANT_B").ok()?;

    let mut tenant_bucket_map = HashMap::new();
    tenant_bucket_map.insert("tenant-a".to_string(), bucket_a);
    tenant_bucket_map.insert("tenant-b".to_string(), bucket_b);

    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket: None,
        },
        prefix: Some(prefix),
        region: std::env::var("AWS_REGION").ok(),
        validate_buckets_on_startup: true,
        ..Default::default()
    };

    Some(S3Backend::from_env(config).expect("create bucket-per-tenant S3 backend"))
}

#[test]
fn test_s3_capabilities_declared() {
    let capabilities = S3Backend::declared_capabilities();

    assert!(capabilities.contains(&BackendCapability::Crud));
    assert!(capabilities.contains(&BackendCapability::Versioning));
    assert!(capabilities.contains(&BackendCapability::InstanceHistory));
    assert!(capabilities.contains(&BackendCapability::TypeHistory));
    assert!(capabilities.contains(&BackendCapability::SystemHistory));
    assert!(capabilities.contains(&BackendCapability::BulkExport));
    assert!(capabilities.contains(&BackendCapability::BulkSubmitIngest));
    assert!(!capabilities.contains(&BackendCapability::BulkSubmitRestWorker));
    assert!(!capabilities.contains(&BackendCapability::BasicSearch));
    assert!(!capabilities.contains(&BackendCapability::Transactions));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_aws_crud_versioning_history() {
    if !run_aws_tests() {
        eprintln!("skipping AWS test (set RUN_AWS_S3_TESTS=1)");
        return;
    }

    let backend = make_prefix_backend(format!("integration/{}/crud", Uuid::new_v4()));
    let tenant = tenant("aws-tenant-a");

    let id = format!("p-{}", Uuid::new_v4());
    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":id,"active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            created.version_id(),
            json!({"resourceType":"Patient","id":created.id(),"active":false}),
        )
        .await
        .unwrap();

    let first = backend
        .vread(&tenant, "Patient", created.id(), "1")
        .await
        .unwrap();
    assert!(first.is_some());

    let history = backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert!(history.items.len() >= 2);

    let stale = backend
        .update_with_match(
            &tenant,
            "Patient",
            updated.id(),
            "1",
            json!({"resourceType":"Patient","id":updated.id()}),
        )
        .await;
    assert!(stale.is_err());

    backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    let gone = backend.read(&tenant, "Patient", created.id()).await;
    assert!(matches!(
        gone,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_aws_bundle_bulk_export_and_submit() {
    if !run_aws_tests() {
        eprintln!("skipping AWS test (set RUN_AWS_S3_TESTS=1)");
        return;
    }

    let backend = make_prefix_backend(format!("integration/{}/bulk", Uuid::new_v4()));
    let tenant = tenant("aws-tenant-b");

    let entries = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({"resourceType":"Patient","id":format!("b-{}", Uuid::new_v4())})),
        ..Default::default()
    }];
    let bundle = backend.process_batch(&tenant, entries).await.unwrap();
    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].status, 201);

    // S3 no longer implements `BulkExportStorage` (job state lives in
    // SQLite/Postgres); only `ExportDataProvider` remains. Verify the data
    // feed instead of the removed kick-off/manifest path.
    let request = ExportRequest::system().with_types(vec!["Patient".to_string()]);
    let batch = backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 100)
        .await
        .unwrap();
    assert!(!batch.lines.is_empty());

    let submission_id = SubmissionId::new("aws-client", format!("sub-{}", Uuid::new_v4()));
    backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();
    let manifest_state = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let results = backend
        .process_entries(
            &tenant,
            &submission_id,
            &manifest_state.manifest_id,
            vec![NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType":"Patient","id":format!("s-{}", Uuid::new_v4())}),
            )],
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].is_success());
}

#[tokio::test]
async fn test_aws_bucket_per_tenant_mode_if_configured() {
    if !run_aws_tests() {
        eprintln!("skipping AWS test (set RUN_AWS_S3_TESTS=1)");
        return;
    }

    let Some(backend) =
        make_bucket_per_tenant_backend(format!("integration/{}/tenancy", Uuid::new_v4()))
    else {
        eprintln!(
            "skipping bucket-per-tenant AWS test (set HFS_S3_TEST_BUCKET_TENANT_A and HFS_S3_TEST_BUCKET_TENANT_B)"
        );
        return;
    };

    let tenant_a = tenant("tenant-a");
    let tenant_b = tenant("tenant-b");
    let id = format!("tenant-same-{}", Uuid::new_v4());

    backend
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType":"Patient","id":id,"flag":"a"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant_b,
            "Patient",
            json!({"resourceType":"Patient","id":id,"flag":"b"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let ra = backend
        .read(&tenant_a, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    let rb = backend
        .read(&tenant_b, "Patient", &id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(ra.content()["flag"], "a");
    assert_eq!(rb.content()["flag"], "b");
}
