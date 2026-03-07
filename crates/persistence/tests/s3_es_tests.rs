//! S3 + Elasticsearch composite backend integration tests.
//!
//! These tests verify that the S3 backend (primary) and Elasticsearch backend (search)
//! work together correctly as a composite storage unit.
//!
//! Tests are opt-in via `RUN_MINIO_S3_ES_TESTS=1` and require Docker to be running.
//! MinIO is used as an S3-compatible store; Elasticsearch is the search backend.
//!
//! Run with:
//!   RUN_MINIO_S3_ES_TESTS=1 cargo test -p helios-persistence \
//!     --features s3,elasticsearch -- s3_es

#![cfg(all(feature = "s3", feature = "elasticsearch"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::json;

use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage,
};
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::{Backend, BackendKind, ResourceStorage};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::search::{SearchParameterLoader, SearchParameterRegistry};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Credentials;

use testcontainers::GenericImage;
use testcontainers::ImageExt;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;
use uuid::Uuid;

// ============================================================================
// Container setup
// ============================================================================

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

struct SharedEs {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<ElasticSearch>,
}

static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();
static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();
static MINIO_AWS_ENV: std::sync::Once = std::sync::Once::new();

fn run_s3_es_tests() -> bool {
    std::env::var("RUN_MINIO_S3_ES_TESTS").ok().as_deref() == Some("1")
}

fn skip_if_disabled(test_name: &str) -> bool {
    if run_s3_es_tests() {
        return false;
    }
    eprintln!("skipping S3+ES test {test_name} (set RUN_MINIO_S3_ES_TESTS=1 to enable)");
    true
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

            let container = GenericImage::new(image, tag)
                .with_wait_for(WaitFor::message_on_stderr("API:"))
                .with_exposed_port(9000.tcp())
                .with_env_var("MINIO_ROOT_USER", root_user.clone())
                .with_env_var("MINIO_ROOT_PASSWORD", root_password.clone())
                .with_cmd(["server", "/data", "--console-address", ":9001"])
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
                .expect("failed to resolve MinIO port");

            SharedMinio {
                endpoint_url: format!("http://{host}:{port}"),
                root_user,
                root_password,
                _container: container,
            }
        })
        .await
}

async fn shared_es() -> &'static SharedEs {
    SHARED_ES
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = ElasticSearch::default()
                .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                .with_label("github.run_id", &run_id)
                .with_startup_timeout(std::time::Duration::from_secs(120))
                .start()
                .await
                .expect("failed to start Elasticsearch container");

            let port = container
                .get_host_port_ipv4(9200)
                .await
                .expect("failed to get ES port");
            let host = container
                .get_host()
                .await
                .expect("failed to get ES host")
                .to_string();

            SharedEs {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn ensure_minio_env(shared: &SharedMinio) {
    MINIO_AWS_ENV.call_once(|| {
        // SAFETY: executes exactly once before any S3 backend construction.
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", &shared.root_user);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", &shared.root_password);
            std::env::set_var("AWS_REGION", "us-east-1");
            std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        }
    });
}

async fn build_sdk_client(shared: &SharedMinio) -> Client {
    let creds = Credentials::new(
        shared.root_user.clone(),
        shared.root_password.clone(),
        None,
        None,
        "s3-es-tests",
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

async fn ensure_bucket(client: &Client, bucket: &str) {
    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        return;
    }
    client
        .create_bucket()
        .bucket(bucket)
        .send()
        .await
        .expect("failed to create test bucket");
}

fn build_search_registry() -> Arc<RwLock<SearchParameterRegistry>> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let loader = SearchParameterLoader::new(FhirVersion::default());
    let mut registry = SearchParameterRegistry::new();

    if let Ok(params) = loader.load_embedded() {
        for p in params {
            let _ = registry.register(p);
        }
    }
    if let Ok(params) = loader.load_from_spec_file(&data_dir) {
        for p in params {
            let _ = registry.register(p);
        }
    }

    Arc::new(RwLock::new(registry))
}

// ============================================================================
// Composite harness
// ============================================================================

struct S3EsHarness {
    composite: CompositeStorage,
    #[allow(dead_code)]
    bucket: String,
}

async fn make_harness(scope: &str) -> S3EsHarness {
    let minio = shared_minio().await;
    let es = shared_es().await;
    ensure_minio_env(minio);

    let sdk_client = build_sdk_client(minio).await;
    let bucket = format!("hfs-s3es-{}", Uuid::new_v4().simple());
    ensure_bucket(&sdk_client, &bucket).await;

    let prefix = format!("test/{}/{}", Uuid::new_v4(), scope);

    // S3 backend
    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        prefix: Some(prefix),
        region: Some("us-east-1".to_string()),
        endpoint_url: Some(minio.endpoint_url.clone()),
        force_path_style: true,
        allow_http: true,
        validate_buckets_on_startup: true,
        ..Default::default()
    };
    let s3 = Arc::new(S3Backend::from_env(s3_config).expect("create S3 backend"));

    // Elasticsearch backend
    let unique_prefix = format!("hfs_{}", Uuid::new_v4().simple());
    let es_config = ElasticsearchConfig {
        nodes: vec![format!("http://{}:{}", es.host, es.port)],
        index_prefix: unique_prefix,
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        ..Default::default()
    };
    let search_registry = build_search_registry();
    let es_backend = Arc::new(
        ElasticsearchBackend::with_shared_registry(es_config, search_registry)
            .expect("create ES backend"),
    );
    es_backend
        .initialize()
        .await
        .expect("initialize ES backend");

    // Composite
    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()
        .expect("build composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("s3".to_string(), s3.clone() as DynStorage);
    backends.insert("es".to_string(), es_backend.clone() as DynStorage);

    let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
    search_providers.insert("s3".to_string(), s3.clone() as DynSearchProvider);
    search_providers.insert("es".to_string(), es_backend.clone() as DynSearchProvider);

    let composite = CompositeStorage::new(composite_config, backends)
        .expect("create composite storage")
        .with_search_providers(search_providers)
        .with_full_primary(s3);

    S3EsHarness { composite, bucket }
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

// ============================================================================
// Tests
// ============================================================================

/// Write a Patient to S3, then verify it appears in ES search by name.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_create_then_search() {
    if skip_if_disabled("s3_es_test_create_then_search") {
        return;
    }

    let harness = make_harness("create-search").await;
    let tenant = tenant("s3-es-tenant");

    let family = format!("TestFamily-{}", Uuid::new_v4().simple());
    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "name": [{"family": family}]
            }),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    // Allow ES refresh (1ms interval configured, but give a little time)
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(&family)],
        chain: vec![],
        components: vec![],
    });

    let results = harness
        .composite
        .search(&tenant, &query)
        .await
        .expect("search should succeed");

    assert!(
        !results.resources.items.is_empty(),
        "expected at least one result for family={family}"
    );
    assert!(results.resources.items.iter().any(|r| r.id() == created.id()));
}

/// Update a Patient, then verify search returns updated fields.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_update_then_search() {
    if skip_if_disabled("s3_es_test_update_then_search") {
        return;
    }

    let harness = make_harness("update-search").await;
    let tenant = tenant("s3-es-tenant");

    let original_family = format!("OrigFamily-{}", Uuid::new_v4().simple());
    let updated_family = format!("UpdatedFamily-{}", Uuid::new_v4().simple());

    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": original_family}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    harness
        .composite
        .update(
            &tenant,
            &created,
            json!({"resourceType": "Patient", "id": created.id(), "name": [{"family": updated_family}]}),
        )
        .await
        .expect("update should succeed");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Search by new family name — should find it
    let query_new = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(&updated_family)],
        chain: vec![],
        components: vec![],
    });
    let results = harness
        .composite
        .search(&tenant, &query_new)
        .await
        .expect("search by new family should succeed");
    assert!(
        results.resources.items.iter().any(|r| r.id() == created.id()),
        "updated resource should appear in search"
    );
}

/// Delete a resource, verify it no longer appears in search results.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_delete_then_search() {
    if skip_if_disabled("s3_es_test_delete_then_search") {
        return;
    }

    let harness = make_harness("delete-search").await;
    let tenant = tenant("s3-es-tenant");

    let family = format!("DeleteMe-{}", Uuid::new_v4().simple());

    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": family}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    harness
        .composite
        .delete(&tenant, "Patient", created.id())
        .await
        .expect("delete should succeed");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(&family)],
        chain: vec![],
        components: vec![],
    });
    let results = harness
        .composite
        .search(&tenant, &query)
        .await
        .expect("search after delete should not error");
    assert!(
        results.resources.items.iter().all(|r| r.id() != created.id()),
        "deleted resource should not appear in search"
    );
}

/// Call `_history` on a resource after updates — versions must come from S3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_history_from_s3() {
    if skip_if_disabled("s3_es_test_history_from_s3") {
        return;
    }

    use helios_persistence::core::history::{HistoryParams, InstanceHistoryProvider};

    let harness = make_harness("history-s3").await;
    let tenant = tenant("s3-es-tenant");

    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "active": true}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    harness
        .composite
        .update(
            &tenant,
            &created,
            json!({"resourceType": "Patient", "id": created.id(), "active": false}),
        )
        .await
        .expect("update should succeed");

    let history = harness
        .composite
        .history_instance(
            &tenant,
            "Patient",
            created.id(),
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .expect("history should succeed");

    assert!(
        history.items.len() >= 2,
        "expected at least 2 history versions, got {}",
        history.items.len()
    );
    let ids: Vec<&str> = history
        .items
        .iter()
        .map(|e| e.resource.version_id())
        .collect();
    assert!(ids.contains(&"1"), "version 1 should be in history");
    assert!(ids.contains(&"2"), "version 2 should be in history");
}

/// Confirm `_vread` reads from S3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_vread_from_s3() {
    if skip_if_disabled("s3_es_test_vread_from_s3") {
        return;
    }

    use helios_persistence::core::VersionedStorage;

    let harness = make_harness("vread-s3").await;
    let tenant = tenant("s3-es-tenant");

    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "active": true}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    harness
        .composite
        .update(
            &tenant,
            &created,
            json!({"resourceType": "Patient", "id": created.id(), "active": false}),
        )
        .await
        .expect("update should succeed");

    let v1 = harness
        .composite
        .vread(&tenant, "Patient", created.id(), "1")
        .await
        .expect("vread v1 should succeed")
        .expect("v1 should exist");

    assert_eq!(v1.version_id(), "1");
    assert_eq!(v1.content()["active"], true);
}

/// Multi-tenant isolation: resource written to tenant-a must not appear in tenant-b search.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_multi_tenant_isolation() {
    if skip_if_disabled("s3_es_test_multi_tenant_isolation") {
        return;
    }

    let harness = make_harness("tenant-isolation").await;
    let tenant_a = tenant("tenant-a");
    let tenant_b = tenant("tenant-b");

    let family = format!("IsolatedFamily-{}", Uuid::new_v4().simple());

    harness
        .composite
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": family}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create in tenant-a should succeed");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(&family)],
        chain: vec![],
        components: vec![],
    });

    // tenant-b should NOT see tenant-a's data
    let results_b = harness
        .composite
        .search(&tenant_b, &query)
        .await
        .expect("tenant-b search should succeed");
    assert!(
        results_b.resources.items.is_empty(),
        "tenant-b should not see tenant-a's resource"
    );

    // tenant-a should see its own data
    let results_a = harness
        .composite
        .search(&tenant_a, &query)
        .await
        .expect("tenant-a search should succeed");
    assert!(
        !results_a.resources.items.is_empty(),
        "tenant-a should see its own resource"
    );
}

/// Read returns the resource after it was written to S3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_read_from_s3() {
    if skip_if_disabled("s3_es_test_read_from_s3") {
        return;
    }

    let harness = make_harness("read-s3").await;
    let tenant = tenant("s3-es-tenant");

    let id = format!("patient-{}", Uuid::new_v4());
    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "active": true}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    let read = harness
        .composite
        .read(&tenant, "Patient", created.id())
        .await
        .expect("read should succeed")
        .expect("resource should exist");

    assert_eq!(read.id(), created.id());
    assert_eq!(read.content()["active"], true);
}

/// Confirm read returns Not Found for missing resources.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_read_missing_returns_none() {
    if skip_if_disabled("s3_es_test_read_missing_returns_none") {
        return;
    }

    let harness = make_harness("read-missing").await;
    let tenant = tenant("s3-es-tenant");

    let result = harness
        .composite
        .read(&tenant, "Patient", "does-not-exist")
        .await
        .expect("read of missing should not error");
    assert!(result.is_none());
}

/// Verify that S3 SearchProvider returns UnsupportedCapability, confirming
/// the composite routes search to ES and not S3.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_s3_search_returns_unsupported() {
    if skip_if_disabled("s3_es_test_s3_search_returns_unsupported") {
        return;
    }

    let minio = shared_minio().await;
    ensure_minio_env(minio);

    let sdk_client = build_sdk_client(minio).await;
    let bucket = format!("hfs-s3-only-{}", Uuid::new_v4().simple());
    ensure_bucket(&sdk_client, &bucket).await;

    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        region: Some("us-east-1".to_string()),
        endpoint_url: Some(minio.endpoint_url.clone()),
        force_path_style: true,
        allow_http: true,
        validate_buckets_on_startup: true,
        ..Default::default()
    };
    let s3 = S3Backend::from_env(s3_config).expect("create S3 backend");
    let tenant_ctx = tenant("s3-search-test");

    let query = SearchQuery::new("Patient");
    let result = s3.search(&tenant_ctx, &query).await;

    // S3 alone must return UnsupportedCapability for search
    // S3 search returns StorageError::Backend(BackendError::UnsupportedCapability)
    assert!(
        result.is_err(),
        "S3 SearchProvider stub must return an error, got: {result:?}"
    );
}

/// After delete, reading the resource returns a Gone error (S3 marks it deleted).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_es_test_read_after_delete_returns_gone() {
    if skip_if_disabled("s3_es_test_read_after_delete_returns_gone") {
        return;
    }

    let harness = make_harness("gone").await;
    let tenant = tenant("s3-es-tenant");

    let created = harness
        .composite
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "active": true}),
            FhirVersion::default(),
        )
        .await
        .expect("create should succeed");

    harness
        .composite
        .delete(&tenant, "Patient", created.id())
        .await
        .expect("delete should succeed");

    let result = harness
        .composite
        .read(&tenant, "Patient", created.id())
        .await;

    assert!(
        matches!(
            result,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "read after delete should return Gone, got: {result:?}"
    );
}

