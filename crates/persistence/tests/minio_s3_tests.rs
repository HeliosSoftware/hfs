//! MinIO S3-compatibility integration tests for the S3 backend.
//!
//! These tests are opt-in via `RUN_MINIO_S3_TESTS=1` and run against a
//! testcontainers-managed MinIO instance using `aws_sdk_s3`.

#![cfg(feature = "s3")]

use std::sync::Once;

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Credentials;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use helios_fhir::FhirVersion;
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};
use helios_persistence::core::bulk_submit::{
    BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider,
    NdjsonEntry, SubmissionId,
};
use helios_persistence::core::history::{
    HistoryParams, InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use helios_persistence::core::{ResourceStorage, SettingsStore, VersionedStorage};
use helios_persistence::error::{ConcurrencyError, ResourceError, SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{CursorValue, PageCursor, Pagination, PaginationMode};
use serde_json::json;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio::sync::OnceCell;
use uuid::Uuid;

const DEFAULT_MINIO_IMAGE: &str = "minio/minio";
const DEFAULT_MINIO_TAG: &str = "RELEASE.2025-02-28T09-55-16Z";
const DEFAULT_MINIO_ROOT_USER: &str = "minioadmin";
const DEFAULT_MINIO_ROOT_PASSWORD: &str = "minioadmin";

struct SharedMinio {
    endpoint_url: String,
    root_user: String,
    root_password: String,
    _container: testcontainers::ContainerAsync<GenericImage>,
}

struct MinioHarness {
    backend: S3Backend,
    sdk_client: Client,
    bucket: String,
    prefix: String,
}

static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();
static MINIO_AWS_ENV: Once = Once::new();

fn run_minio_tests() -> bool {
    std::env::var("RUN_MINIO_S3_TESTS").ok().as_deref() == Some("1")
}

fn skip_if_disabled(test_name: &str) -> bool {
    if run_minio_tests() {
        return false;
    }
    eprintln!("skipping MinIO test {test_name} (set RUN_MINIO_S3_TESTS=1 to enable)");
    true
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn test_bucket_name() -> String {
    std::env::var("HFS_MINIO_TEST_BUCKET")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("hfs-minio-{}", Uuid::new_v4().simple()))
}

fn ensure_backend_env_credentials(shared: &SharedMinio) {
    MINIO_AWS_ENV.call_once(|| {
        // SAFETY: This executes exactly once for this test binary before any
        // backend construction in this module, and values remain constant.
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", &shared.root_user);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", &shared.root_password);
            std::env::set_var("AWS_REGION", "us-east-1");
            std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        }
    });
}

async fn shared_minio() -> &'static SharedMinio {
    SHARED_MINIO
        .get_or_init(|| async {
            let image =
                std::env::var("MINIO_IMAGE").unwrap_or_else(|_| DEFAULT_MINIO_IMAGE.to_string());
            let tag = std::env::var("MINIO_TAG").unwrap_or_else(|_| DEFAULT_MINIO_TAG.to_string());
            let root_user = std::env::var("MINIO_ROOT_USER")
                .unwrap_or_else(|_| DEFAULT_MINIO_ROOT_USER.to_string());
            let root_password = std::env::var("MINIO_ROOT_PASSWORD")
                .unwrap_or_else(|_| DEFAULT_MINIO_ROOT_PASSWORD.to_string());

            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = GenericImage::new(image, tag)
                .with_wait_for(WaitFor::message_on_stderr("API:"))
                .with_exposed_port(9000.tcp())
                .with_exposed_port(9001.tcp())
                .with_env_var("MINIO_ROOT_USER", root_user.clone())
                .with_env_var("MINIO_ROOT_PASSWORD", root_password.clone())
                .with_env_var("MINIO_CONSOLE_ADDRESS", ":9001")
                .with_cmd(["server", "/data", "--console-address", ":9001"])
                .with_label("github.run_id", &run_id)
                .start()
                .await
                .expect("failed to start MinIO container");

            let host = container
                .get_host()
                .await
                .expect("failed to resolve MinIO host")
                .to_string();
            let port = container
                .get_host_port_ipv4(9000)
                .await
                .expect("failed to resolve MinIO API port");

            SharedMinio {
                endpoint_url: format!("http://{host}:{port}"),
                root_user,
                root_password,
                _container: container,
            }
        })
        .await
}

async fn build_minio_sdk_client(shared: &SharedMinio) -> Client {
    let creds = Credentials::new(
        shared.root_user.clone(),
        shared.root_password.clone(),
        None,
        None,
        "minio-tests",
    );

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .endpoint_url(shared.endpoint_url.clone())
        .credentials_provider(creds)
        .load()
        .await;

    let s3_config = aws_sdk_s3::config::Builder::from(&sdk_config)
        .force_path_style(true)
        .build();
    Client::from_conf(s3_config)
}

async fn ensure_bucket_exists(client: &Client, bucket: &str) {
    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        return;
    }

    client
        .create_bucket()
        .bucket(bucket)
        .send()
        .await
        .expect("failed to create MinIO test bucket");
}

async fn make_prefix_backend(scope: &str) -> MinioHarness {
    let shared = shared_minio().await;
    ensure_backend_env_credentials(shared);

    let sdk_client = build_minio_sdk_client(shared).await;
    let bucket = test_bucket_name();
    ensure_bucket_exists(&sdk_client, &bucket).await;

    let prefix = format!("integration/{}/{}", Uuid::new_v4(), scope);
    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        prefix: Some(prefix.clone()),
        region: Some("us-east-1".to_string()),
        endpoint_url: Some(shared.endpoint_url.clone()),
        force_path_style: true,
        allow_http: true,
        validate_buckets_on_startup: true,
        ..Default::default()
    };

    let backend = S3Backend::from_env(config).expect("create S3 backend for MinIO");
    MinioHarness {
        backend,
        sdk_client,
        bucket,
        prefix,
    }
}

fn is_precondition_failed<E>(err: &aws_sdk_s3::error::SdkError<E>) -> bool
where
    E: ProvideErrorMetadata + std::fmt::Debug,
{
    err.as_service_error()
        .and_then(|service_err| service_err.code())
        .map(|code| code == "PreconditionFailed")
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_smoke_crud_versioning_history() {
    if skip_if_disabled("test_minio_smoke_crud_versioning_history") {
        return;
    }

    let harness = make_prefix_backend("smoke").await;
    let tenant = tenant("minio-tenant-smoke");

    let id = format!("p-{}", Uuid::new_v4());
    let created = harness
        .backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":id,"active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = harness
        .backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            created.version_id(),
            json!({"resourceType":"Patient","id":created.id(),"active":false}),
        )
        .await
        .unwrap();
    assert_eq!(updated.version_id(), "2");

    let first = harness
        .backend
        .vread(&tenant, "Patient", created.id(), "1")
        .await
        .unwrap();
    assert!(first.is_some());

    let history = harness
        .backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert!(history.items.len() >= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_if_none_match_concurrent_single_winner() {
    if skip_if_disabled("test_minio_if_none_match_concurrent_single_winner") {
        return;
    }

    let harness = make_prefix_backend("if-none-match").await;
    let key = format!("locks/{}/create-only-lock", Uuid::new_v4());
    let attempts = 8usize;
    let mut tasks = Vec::new();

    for i in 0..attempts {
        let client = harness.sdk_client.clone();
        let bucket = harness.bucket.clone();
        let key = key.clone();
        tasks.push(tokio::spawn(async move {
            client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(ByteStream::from(format!("writer-{i}").into_bytes()))
                .if_none_match("*")
                .send()
                .await
        }));
    }

    let mut success_count = 0usize;
    let mut precondition_count = 0usize;

    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => success_count += 1,
            Err(err) if is_precondition_failed(&err) => precondition_count += 1,
            Err(err) => panic!("unexpected MinIO error for if-none-match race: {err:?}"),
        }
    }

    assert_eq!(success_count, 1);
    assert_eq!(precondition_count, attempts - 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_backend_create_race_single_winner() {
    if skip_if_disabled("test_minio_backend_create_race_single_winner") {
        return;
    }

    let harness = make_prefix_backend("backend-create-race").await;
    let backend = harness.backend;
    let tenant = tenant("minio-tenant-create-race");
    let id = format!("race-{}", Uuid::new_v4());

    let b1 = backend.clone();
    let b2 = backend.clone();
    let t1 = tenant.clone();
    let t2 = tenant.clone();
    let id_a = id.clone();
    let id_b = id.clone();

    let fut1 = tokio::spawn(async move {
        b1.create(
            &t1,
            "Patient",
            json!({"resourceType":"Patient","id":id_a}),
            FhirVersion::default(),
        )
        .await
    });

    let fut2 = tokio::spawn(async move {
        b2.create(
            &t2,
            "Patient",
            json!({"resourceType":"Patient","id":id_b}),
            FhirVersion::default(),
        )
        .await
    });

    let r1 = fut1.await.unwrap();
    let r2 = fut2.await.unwrap();

    let success_count = [r1.is_ok(), r2.is_ok()].into_iter().filter(|v| *v).count();
    let exists_count = [r1, r2]
        .into_iter()
        .filter(|r| {
            matches!(
                r,
                Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
            )
        })
        .count();

    assert_eq!(success_count, 1);
    assert_eq!(exists_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_if_match_stale_etag_conflicts() {
    if skip_if_disabled("test_minio_if_match_stale_etag_conflicts") {
        return;
    }

    let harness = make_prefix_backend("if-match").await;
    let key = format!("locks/{}/optimistic-lock", Uuid::new_v4());

    let first = harness
        .sdk_client
        .put_object()
        .bucket(&harness.bucket)
        .key(&key)
        .body(ByteStream::from_static(br#"{"version":1}"#))
        .send()
        .await
        .unwrap();
    let stale_etag = first
        .e_tag()
        .expect("first put should return ETag")
        .to_string();

    let second = harness
        .sdk_client
        .put_object()
        .bucket(&harness.bucket)
        .key(&key)
        .body(ByteStream::from_static(br#"{"version":2}"#))
        .if_match(stale_etag.clone())
        .send()
        .await;
    assert!(second.is_ok(), "fresh if-match update should succeed");

    let stale = harness
        .sdk_client
        .put_object()
        .bucket(&harness.bucket)
        .key(&key)
        .body(ByteStream::from_static(br#"{"version":3}"#))
        .if_match(stale_etag)
        .send()
        .await;
    assert!(
        stale.as_ref().is_err_and(is_precondition_failed),
        "stale if-match should fail with PreconditionFailed, got: {stale:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_backend_update_with_match_conflict() {
    if skip_if_disabled("test_minio_backend_update_with_match_conflict") {
        return;
    }

    let harness = make_prefix_backend("backend-if-match").await;
    let tenant = tenant("minio-tenant-if-match");
    let id = format!("patient-{}", Uuid::new_v4());

    let created = harness
        .backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":id,"active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    harness
        .backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            created.version_id(),
            json!({"resourceType":"Patient","id":created.id(),"active":false}),
        )
        .await
        .unwrap();

    let stale = harness
        .backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            created.version_id(),
            json!({"resourceType":"Patient","id":created.id(),"active":true}),
        )
        .await;

    assert!(matches!(
        stale,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_vread_returns_previous_versions() {
    if skip_if_disabled("test_minio_vread_returns_previous_versions") {
        return;
    }

    let harness = make_prefix_backend("vread").await;
    let tenant = tenant("minio-tenant-vread");
    let id = format!("patient-{}", Uuid::new_v4());

    let created = harness
        .backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":id,"active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = harness
        .backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            created.version_id(),
            json!({"resourceType":"Patient","id":created.id(),"active":false}),
        )
        .await
        .unwrap();

    let v1 = harness
        .backend
        .vread(&tenant, "Patient", created.id(), "1")
        .await
        .unwrap()
        .expect("expected version 1");
    let v2 = harness
        .backend
        .vread(&tenant, "Patient", created.id(), "2")
        .await
        .unwrap()
        .expect("expected version 2");

    assert_eq!(v1.version_id(), "1");
    assert_eq!(v2.version_id(), "2");
    assert_eq!(v1.content()["active"], true);
    assert_eq!(updated.version_id(), "2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_history_instance_type_system_cursor_and_invalid_cursor() {
    if skip_if_disabled("test_minio_history_instance_type_system_cursor_and_invalid_cursor") {
        return;
    }

    let harness = make_prefix_backend("history").await;
    let tenant = tenant("minio-tenant-history");
    let id = format!("patient-{}", Uuid::new_v4());

    let created = harness
        .backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":id}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = harness
        .backend
        .update(
            &tenant,
            &created,
            json!({"resourceType":"Patient","id":created.id(),"active":true}),
        )
        .await
        .unwrap();

    harness
        .backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    let history = harness
        .backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert_eq!(history.items.len(), 3);
    assert_eq!(history.items[0].resource.version_id(), "3");
    assert_eq!(history.items[1].resource.version_id(), updated.version_id());

    let type_history = harness
        .backend
        .history_type(
            &tenant,
            "Patient",
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert!(type_history.items.len() >= 3);

    let system_history = harness
        .backend
        .history_system(&tenant, &HistoryParams::new().include_deleted(true))
        .await
        .unwrap();
    assert!(system_history.items.len() >= 3);

    let page1 = harness
        .backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams {
                pagination: Pagination {
                    count: 1,
                    mode: PaginationMode::Offset(0),
                },
                ..HistoryParams::new().include_deleted(true)
            },
        )
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 1);
    assert!(page1.page_info.next_cursor.is_some());

    let cursor = PageCursor::decode(page1.page_info.next_cursor.as_ref().unwrap()).unwrap();
    let page2 = harness
        .backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams {
                pagination: Pagination {
                    count: 1,
                    mode: PaginationMode::Cursor(Some(cursor)),
                },
                ..HistoryParams::new().include_deleted(true)
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 1);
    assert_ne!(
        page1.items[0].resource.version_id(),
        page2.items[0].resource.version_id()
    );

    let bad_cursor = PageCursor::new(vec![CursorValue::String("bad".to_string())], "oops").encode();
    let invalid = harness
        .backend
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams {
                pagination: Pagination {
                    count: 10,
                    mode: PaginationMode::Cursor(Some(PageCursor::decode(&bad_cursor).unwrap())),
                },
                ..HistoryParams::new()
            },
        )
        .await;
    assert!(matches!(
        invalid,
        Err(StorageError::Search(SearchError::InvalidCursor { .. }))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_bulk_export_lifecycle_manifest_and_outputs() {
    if skip_if_disabled("test_minio_bulk_export_lifecycle_manifest_and_outputs") {
        return;
    }

    let harness = make_prefix_backend("bulk-export").await;
    let tenant = tenant("minio-tenant-export");

    for i in 0..3 {
        harness
            .backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":format!("e-{i}-{}", Uuid::new_v4())}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // S3 no longer implements `BulkExportStorage` (job state lives in
    // SQLite/Postgres; see Phase 2 §2b). Verify that the S3 backend's
    // `ExportDataProvider` data feed still returns the seeded resources.
    let request = ExportRequest::system().with_types(vec!["Patient".to_string()]);
    let batch = harness
        .backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 100)
        .await
        .unwrap();
    assert_eq!(batch.lines.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_bulk_submit_ingest_raw_and_rollback() {
    if skip_if_disabled("test_minio_bulk_submit_ingest_raw_and_rollback") {
        return;
    }

    let harness = make_prefix_backend("bulk-submit").await;
    let tenant = tenant("minio-tenant-submit");

    let submission_id = SubmissionId::new("minio-client", format!("sub-{}", Uuid::new_v4()));
    harness
        .backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();

    let manifest = harness
        .backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let entry_id = format!("bs-{}", Uuid::new_v4());
    let results = harness
        .backend
        .process_entries(
            &tenant,
            &submission_id,
            &manifest.manifest_id,
            vec![NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType":"Patient","id":entry_id}),
            )],
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, BulkEntryOutcome::Success);

    let raw_prefix = format!(
        "{}/{}/bulk/submit/{}/{}/raw/{}/",
        harness.prefix.trim_matches('/'),
        tenant.tenant_id().as_str(),
        submission_id.submitter,
        submission_id.submission_id,
        manifest.manifest_id
    );
    let raw_objects = harness
        .sdk_client
        .list_objects_v2()
        .bucket(&harness.bucket)
        .prefix(raw_prefix)
        .send()
        .await
        .unwrap();
    assert!(
        !raw_objects.contents().is_empty(),
        "expected at least one raw NDJSON artifact for bulk submit"
    );

    let changes = harness
        .backend
        .list_changes(&tenant, &submission_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);

    let rolled_back = harness
        .backend
        .rollback_change(&tenant, &submission_id, &changes[0])
        .await
        .unwrap();
    assert!(rolled_back);

    let read_after_rollback = harness.backend.read(&tenant, "Patient", &entry_id).await;
    assert!(matches!(
        read_after_rollback,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_pagination_over_1000_history_and_export() {
    if skip_if_disabled("test_minio_pagination_over_1000_history_and_export") {
        return;
    }

    let harness = make_prefix_backend("pagination").await;
    let tenant = tenant("minio-tenant-pagination");

    for i in 0..1005 {
        harness
            .backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":format!("p-{i}-{}", Uuid::new_v4())}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let type_history_count = harness
        .backend
        .history_type_count(&tenant, "Patient")
        .await
        .unwrap();
    assert_eq!(type_history_count, 1005);

    let request = ExportRequest::system();
    let batch1 = harness
        .backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 1000)
        .await
        .unwrap();
    assert_eq!(batch1.lines.len(), 1000);
    assert!(!batch1.is_last);
    assert!(batch1.next_cursor.is_some());

    let batch2 = harness
        .backend
        .fetch_export_batch(
            &tenant,
            &request,
            "Patient",
            batch1.next_cursor.as_deref(),
            1000,
        )
        .await
        .unwrap();
    assert_eq!(batch2.lines.len(), 5);
    assert!(batch2.is_last);
}

// ============================================================================
// Phase 2 — S3OutputStore tests against MinIO.
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_s3_output_store_round_trip() {
    use helios_persistence::backends::s3::{
        AccessTokenMode, AwsS3Client, AwsS3ClientOptions, S3OutputStore,
    };
    use helios_persistence::core::bulk_export::ExportJobId;
    use helios_persistence::core::bulk_export_output::{ExportOutputStore, ExportPartKey};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    if skip_if_disabled("test_minio_s3_output_store_round_trip") {
        return;
    }

    let shared = shared_minio().await;
    ensure_backend_env_credentials(shared);
    let sdk_client = build_minio_sdk_client(shared).await;
    let bucket = test_bucket_name();
    ensure_bucket_exists(&sdk_client, &bucket).await;

    let region = aws_config::Region::new("us-east-1");
    let credentials = aws_sdk_s3::config::Credentials::new(
        &shared.root_user,
        &shared.root_password,
        None,
        None,
        "minio-test",
    );
    let sdk_config = aws_config::SdkConfig::builder()
        .region(region)
        .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(
            credentials,
        ))
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build();
    let s3_client = Arc::new(AwsS3Client::from_sdk_config_with_options(
        &sdk_config,
        AwsS3ClientOptions {
            endpoint_url: Some(shared.endpoint_url.clone()),
            force_path_style: true,
        },
    ));

    let store = S3OutputStore::new(
        s3_client,
        bucket.clone(),
        "http://localhost:8080",
        AccessTokenMode::Auto,
        Duration::from_secs(60),
    );

    let job_id = ExportJobId::new();
    let key = ExportPartKey::output("tenant-a", job_id.clone(), "Patient", 0, 1);

    // Write two NDJSON lines and finalize.
    let mut writer = store.open_writer(&key).await.unwrap();
    writer
        .write_line(r#"{"resourceType":"Patient","id":"a"}"#)
        .await
        .unwrap();
    writer
        .write_line(r#"{"resourceType":"Patient","id":"b"}"#)
        .await
        .unwrap();
    let finalized = store.finalize_part(&key, writer).await.unwrap();
    assert_eq!(finalized.line_count, 2);
    assert!(finalized.size_bytes > 0);

    // Pre-signed GET URL.
    let url = store
        .download_url(&key, Duration::from_secs(60))
        .await
        .unwrap();
    assert!(!url.requires_access_token);
    assert!(url.url.contains("X-Amz-Signature") || url.url.contains("Signature="));

    // open_reader streams the same bytes back.
    let mut reader = store.open_reader(&key).await.unwrap();
    let mut content = String::new();
    reader.read_to_string(&mut content).await.unwrap();
    assert_eq!(content.lines().count(), 2);

    // delete_job_outputs removes the part; idempotent on second call.
    let tenant = tenant("tenant-a");
    store.delete_job_outputs(&tenant, &job_id).await.unwrap();
    store.delete_job_outputs(&tenant, &job_id).await.unwrap();
    assert!(store.open_reader(&key).await.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-user settings store
//
// The mock-backed unit tests prove the store's *logic*. These prove the thing
// the mock cannot: that a real S3-compatible service actually enforces the
// conditional-write preconditions the store's optimistic locking is built on.
// Conditional PutObject is a comparatively recent S3 feature, so this is the
// whole risk of the design — if the store silently ignored `If-Match`, every
// concurrent settings write would be a lost update, with no error anywhere.
// ─────────────────────────────────────────────────────────────────────────────

/// A user key unique to one test run, so tests never share a settings object.
fn unique_user_key(scope: &str) -> String {
    format!(
        "https://idp.example.com/realms/test|{scope}-{}",
        Uuid::new_v4()
    )
}

#[tokio::test]
async fn test_minio_settings_round_trip() {
    if skip_if_disabled("test_minio_settings_round_trip") {
        return;
    }

    let harness = make_prefix_backend("settings-round-trip").await;
    let user = unique_user_key("round-trip");

    assert!(harness.backend.get_settings(&user).await.unwrap().is_none());

    let doc = json!({"theme": "dark", "defaultTenant": "acme"});
    let stored = harness
        .backend
        .put_settings(&user, doc.clone(), None)
        .await
        .unwrap();
    assert_eq!(stored.version, 1);

    let fetched = harness.backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(fetched.document, doc);
    assert_eq!(fetched.version, 1);

    let patched = harness
        .backend
        .patch_settings(
            &user,
            json!({"theme": "light", "defaultTenant": null}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(patched.document, json!({"theme": "light"}));
    assert_eq!(patched.version, 2);

    let refetched = harness.backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(refetched.document, json!({"theme": "light"}));
    assert_eq!(refetched.version, 2);
}

/// `If-None-Match: *` really is enforced: of N concurrent "create if absent"
/// writes, exactly one wins and the rest see an optimistic-lock failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_settings_create_only_single_winner() {
    if skip_if_disabled("test_minio_settings_create_only_single_winner") {
        return;
    }

    let harness = make_prefix_backend("settings-create-race").await;
    let user = unique_user_key("create-race");
    let attempts = 8usize;
    let mut tasks = Vec::new();

    for i in 0..attempts {
        let backend = harness.backend.clone();
        let user = user.clone();
        tasks.push(tokio::spawn(async move {
            // `Some(0)` asserts "this user has no settings yet".
            backend
                .put_settings(&user, json!({"writer": i}), Some(0))
                .await
        }));
    }

    let mut winners = 0usize;
    let mut lock_failures = 0usize;
    for task in tasks {
        match task.await.unwrap() {
            Ok(stored) => {
                assert_eq!(stored.version, 1);
                winners += 1;
            }
            Err(StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })) => {
                lock_failures += 1;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert_eq!(winners, 1, "create-if-absent admitted more than one winner");
    assert_eq!(lock_failures, attempts - 1);

    let stored = harness.backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(stored.version, 1);
}

/// `If-Match` really is enforced: a write pinned to a stale version is rejected,
/// and one pinned to the live version succeeds.
#[tokio::test]
async fn test_minio_settings_stale_if_match_conflicts() {
    if skip_if_disabled("test_minio_settings_stale_if_match_conflicts") {
        return;
    }

    let harness = make_prefix_backend("settings-if-match").await;
    let user = unique_user_key("if-match");

    harness
        .backend
        .put_settings(&user, json!({"a": 1}), None)
        .await
        .unwrap(); // version 1

    // Stale precondition: asserts "not yet created", but it now exists.
    let err = harness
        .backend
        .put_settings(&user, json!({"a": 2}), Some(0))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
    ));

    // Live precondition succeeds.
    let updated = harness
        .backend
        .put_settings(&user, json!({"a": 2}), Some(1))
        .await
        .unwrap();
    assert_eq!(updated.version, 2);

    // The rejected write left nothing behind.
    let stored = harness.backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(stored.document, json!({"a": 2}));
    assert_eq!(stored.version, 2);
}

/// The canonical lost-update proof, against a real object store: concurrent
/// unconditional merge-patches each adding a distinct key must all survive.
/// Every writer that loses the compare-and-swap re-reads the winner's document
/// and merges onto it, so no key may go missing and the version must equal the
/// number of writers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_minio_settings_concurrent_patches_never_lose_an_update() {
    if skip_if_disabled("test_minio_settings_concurrent_patches_never_lose_an_update") {
        return;
    }

    let harness = make_prefix_backend("settings-lost-update").await;
    let user = unique_user_key("lost-update");
    let writers = 4usize;
    let mut tasks = Vec::new();

    for i in 0..writers {
        let backend = harness.backend.clone();
        let user = user.clone();
        tasks.push(tokio::spawn(async move {
            // Each writer merges in a key of its own, so a lost update is
            // visible as a missing key rather than an overwritten value.
            let mut patch = serde_json::Map::new();
            patch.insert(format!("key{i}"), json!(i));
            backend
                .patch_settings(&user, serde_json::Value::Object(patch), None)
                .await
        }));
    }

    for task in tasks {
        task.await
            .unwrap()
            .expect("an unconditional patch must retry on conflict, not fail");
    }

    let stored = harness.backend.get_settings(&user).await.unwrap().unwrap();
    for i in 0..writers {
        assert_eq!(
            stored.document.get(format!("key{i}")),
            Some(&json!(i)),
            "writer {i}'s update was lost: {:?}",
            stored.document
        );
    }
    assert_eq!(stored.version, writers as i64);
}

/// `delete_settings` removes the object and reports whether one was there,
/// which is what makes the #270 legacy-key migration able to move a document
/// rather than duplicate it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_settings_delete_is_idempotent() {
    if skip_if_disabled("test_minio_settings_delete_is_idempotent") {
        return;
    }

    let harness = make_prefix_backend("settings-delete").await;
    let user = unique_user_key("delete");

    // Deleting an absent document is not an error, and reports "nothing there".
    assert!(!harness.backend.delete_settings(&user).await.unwrap());

    harness
        .backend
        .put_settings(&user, json!({"theme": "dark"}), None)
        .await
        .unwrap();
    assert!(harness.backend.get_settings(&user).await.unwrap().is_some());

    assert!(harness.backend.delete_settings(&user).await.unwrap());
    assert!(harness.backend.get_settings(&user).await.unwrap().is_none());

    // Idempotent: a second delete succeeds and reports nothing was removed.
    assert!(!harness.backend.delete_settings(&user).await.unwrap());
}
