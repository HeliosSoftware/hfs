//! End-to-end integration tests for S3 + Elasticsearch composite mode.

#![cfg(all(feature = "s3", feature = "elasticsearch"))]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, Instant};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Credentials;
use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::backends::s3::{
    S3Backend, S3BackendConfig, S3TenancyMode, S3ToElasticsearchReindexOptions,
};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, SyncMode};
use helios_persistence::core::{BackendKind, ResourceStorage, SearchProvider};
use helios_persistence::search::{SearchParameterLoader, SearchParameterRegistry};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};
use parking_lot::RwLock;
use serde_json::json;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;
use tokio::time::sleep;
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

struct SharedEs {
    endpoint_url: String,
    _container: testcontainers::ContainerAsync<ElasticSearch>,
}

struct Harness {
    s3: Arc<S3Backend>,
    es: Arc<ElasticsearchBackend>,
    composite: CompositeStorage,
}

static SHARED_MINIO: OnceCell<SharedMinio> = OnceCell::const_new();
static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();
static MINIO_AWS_ENV: Once = Once::new();

fn run_tests() -> bool {
    std::env::var("RUN_MINIO_S3_ES_TESTS").ok().as_deref() == Some("1")
}

fn skip_if_disabled(test_name: &str) -> bool {
    if run_tests() {
        return false;
    }
    eprintln!("skipping S3+ES test {test_name} (set RUN_MINIO_S3_ES_TESTS=1 to enable)");
    true
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn ensure_backend_env_credentials(shared: &SharedMinio) {
    MINIO_AWS_ENV.call_once(|| {
        // SAFETY: this is executed once before backend construction in this file.
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

            let container = GenericImage::new(image, tag)
                .with_wait_for(WaitFor::message_on_stderr("API:"))
                .with_exposed_port(9000.tcp())
                .with_exposed_port(9001.tcp())
                .with_env_var("MINIO_ROOT_USER", root_user.clone())
                .with_env_var("MINIO_ROOT_PASSWORD", root_password.clone())
                .with_env_var("MINIO_CONSOLE_ADDRESS", ":9001")
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

async fn shared_es() -> &'static SharedEs {
    SHARED_ES
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = ElasticSearch::default()
                .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                .with_label("github.run_id", &run_id)
                .with_startup_timeout(Duration::from_secs(120))
                .start()
                .await
                .expect("failed to start Elasticsearch container");

            let host = container
                .get_host()
                .await
                .expect("failed to resolve ES host")
                .to_string();
            let port = container
                .get_host_port_ipv4(9200)
                .await
                .expect("failed to resolve ES port");

            SharedEs {
                endpoint_url: format!("http://{host}:{port}"),
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
        "minio-s3-es-tests",
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

fn build_registry() -> Arc<RwLock<SearchParameterRegistry>> {
    let loader = SearchParameterLoader::new(FhirVersion::default());
    let registry = Arc::new(RwLock::new(SearchParameterRegistry::new()));
    if let Ok(params) = loader.load_embedded() {
        let mut reg = registry.write();
        for param in params {
            let _ = reg.register(param);
        }
    }
    registry
}

async fn build_harness(scope: &str) -> Harness {
    let minio = shared_minio().await;
    ensure_backend_env_credentials(minio);

    let es_shared = shared_es().await;
    let sdk_client = build_minio_sdk_client(minio).await;

    let bucket = format!("hfs-minio-es-{}", Uuid::new_v4().simple());
    ensure_bucket_exists(&sdk_client, &bucket).await;

    let prefix = format!("integration/{}/{}", Uuid::new_v4(), scope);
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

    let s3 = Arc::new(S3Backend::from_env(s3_config).expect("create S3 backend for MinIO"));

    let es_config = ElasticsearchConfig {
        nodes: vec![es_shared.endpoint_url.clone()],
        index_prefix: format!("hfs_s3_es_{}", Uuid::new_v4().simple()),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        ..Default::default()
    };

    let es = Arc::new(
        ElasticsearchBackend::with_shared_registry(es_config, build_registry())
            .expect("create Elasticsearch backend"),
    );

    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("build composite config");

    let mut backends = HashMap::new();
    backends.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    let composite = CompositeStorage::new(composite_config, backends)
        .expect("create composite")
        .with_search_providers(search_providers)
        .with_full_primary(s3.clone());

    Harness { s3, es, composite }
}

fn query_by_id(resource_type: &str, id: &str) -> SearchQuery {
    SearchQuery::new(resource_type).with_parameter(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(id)],
        chain: vec![],
        components: vec![],
    })
}

async fn assert_eventually<F, Fut>(timeout: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if check().await {
            return;
        }
        if Instant::now() >= deadline {
            panic!("condition not met within {:?}", timeout);
        }
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minio_s3_elasticsearch_composite_flow() {
    if skip_if_disabled("test_minio_s3_elasticsearch_composite_flow") {
        return;
    }

    let harness = build_harness("composite-flow").await;
    let tenant_a = tenant("clinic-a");
    let tenant_b = tenant("clinic-b");

    // CRUD roundtrip (canonical read path from S3).
    let created = harness
        .composite
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType":"Patient","id":"p-sync","active":true}),
            FhirVersion::default(),
        )
        .await
        .expect("create patient in composite");

    let read_back = harness
        .composite
        .read(&tenant_a, "Patient", created.id())
        .await
        .expect("read result")
        .expect("resource exists");
    assert_eq!(read_back.id(), created.id());

    // Search served by ES after write-through sync.
    harness
        .es
        .refresh_index(tenant_a.tenant_id().as_str(), "Patient")
        .await
        .expect("refresh index");

    assert_eventually(Duration::from_secs(10), || async {
        harness
            .composite
            .search(&tenant_a, &query_by_id("Patient", "p-sync"))
            .await
            .map(|result| result.resources.items.len() == 1)
            .unwrap_or(false)
    })
    .await;

    // Tenant isolation in ES search path.
    let result_b = harness
        .composite
        .search(&tenant_b, &query_by_id("Patient", "p-sync"))
        .await
        .expect("tenant-b search");
    assert!(result_b.resources.items.is_empty());

    // Delete propagation removes the resource from ES search results.
    harness
        .composite
        .delete(&tenant_a, "Patient", "p-sync")
        .await
        .expect("delete patient");
    harness
        .es
        .refresh_index(tenant_a.tenant_id().as_str(), "Patient")
        .await
        .expect("refresh after delete");

    assert_eventually(Duration::from_secs(10), || async {
        harness
            .composite
            .search(&tenant_a, &query_by_id("Patient", "p-sync"))
            .await
            .map(|result| result.resources.items.is_empty())
            .unwrap_or(false)
    })
    .await;

    // Create another resource, wipe from ES, then rebuild via reindex.
    harness
        .composite
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType":"Patient","id":"p-reindex","active":true}),
            FhirVersion::default(),
        )
        .await
        .expect("create reindex patient");
    harness
        .es
        .refresh_index(tenant_a.tenant_id().as_str(), "Patient")
        .await
        .expect("refresh before manual delete");

    harness
        .es
        .delete(&tenant_a, "Patient", "p-reindex")
        .await
        .expect("manual es delete");
    harness
        .es
        .refresh_index(tenant_a.tenant_id().as_str(), "Patient")
        .await
        .expect("refresh after manual delete");

    assert_eventually(Duration::from_secs(10), || async {
        harness
            .composite
            .search(&tenant_a, &query_by_id("Patient", "p-reindex"))
            .await
            .map(|result| result.resources.items.is_empty())
            .unwrap_or(false)
    })
    .await;

    let report = harness
        .s3
        .reindex_to_elasticsearch(
            &tenant_a,
            harness.es.as_ref(),
            S3ToElasticsearchReindexOptions {
                batch_size: 50,
                clear_existing: false,
                resource_types: Some(vec!["Patient".to_string()]),
            },
        )
        .await
        .expect("reindex run");

    assert!(
        report.indexed >= 1,
        "expected at least one indexed resource"
    );

    harness
        .es
        .refresh_index(tenant_a.tenant_id().as_str(), "Patient")
        .await
        .expect("refresh after reindex");

    assert_eventually(Duration::from_secs(10), || async {
        harness
            .composite
            .search(&tenant_a, &query_by_id("Patient", "p-reindex"))
            .await
            .map(|result| result.resources.items.len() == 1)
            .unwrap_or(false)
    })
    .await;
}
