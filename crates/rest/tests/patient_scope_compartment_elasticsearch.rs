//! A `patient/` scope is confined to the launch patient's compartment (#1618)
//! — SQLite + Elasticsearch (`sqlite-elasticsearch`).
//!
//! Runs the scenarios in `common/patient_scope.rs` (see
//! `patient_scope_compartment.rs` for SQLite alone) against the composite
//! `hfs` builds for `HFS_STORAGE_BACKEND=sqlite-elasticsearch`: SQLite primary
//! with its own search indexing offloaded, so every search — the compartment
//! filter the confined grant adds included — is answered by Elasticsearch.
//! The composite syncs synchronously and Elasticsearch refreshes on write, so
//! a seeded resource is searchable on the next request.
//!
//! Requires Docker (testcontainers spins up a real Elasticsearch instance).

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/patient_scope.rs"]
mod patient_scope;

use std::collections::HashMap;
use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{Backend, BackendKind, BulkExportJobStore};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

struct SharedEs {
    url: String,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<ElasticSearch>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

/// Same image, heap cap and startup budget as the persistence suite's
/// `elasticsearch_tests.rs` (7.17: the 7.16 JDK crashes on cgroup v2 hosts,
/// and 8.x never logs the ready line the module waits on).
async fn shared_es() -> &'static SharedEs {
    SHARED_ES
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag("7.17.29")
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(std::time::Duration::from_secs(300)),
            )
            .start()
            .await
            .expect("failed to start Elasticsearch container");
            let port = container
                .get_host_port_ipv4(9200)
                .await
                .expect("failed to get host port");
            let host = container.get_host().await.expect("failed to get host");
            SharedEs {
                url: format!("http://{host}:{port}"),
                _container: container,
            }
        })
        .await
}

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let es = shared_es().await;

    // The SQLite primary, its search offloaded as `start_sqlite_elasticsearch`
    // does it.
    let mut sqlite = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(patient_scope::data_dir()),
            ..Default::default()
        },
    )
    .expect("in-memory sqlite");
    sqlite.init_schema().expect("init schema");
    sqlite.set_search_offloaded(true);
    let sqlite = Arc::new(sqlite);

    // Elasticsearch shares the primary's registry; a per-test index prefix
    // keeps the scenarios apart in the one container.
    let elastic = ElasticsearchBackend::with_shared_registry(
        ElasticsearchConfig {
            nodes: vec![es.url.clone()],
            index_prefix: format!("pscope_{}", uuid::Uuid::new_v4().simple()),
            number_of_replicas: 0,
            refresh_interval: "1ms".to_string(),
            write_refresh: WriteRefreshPolicy::WaitFor,
            ..Default::default()
        },
        sqlite.tenant_registries().clone(),
    )
    .expect("failed to create ElasticsearchBackend");
    elastic
        .initialize()
        .await
        .expect("failed to initialize Elasticsearch");
    let elastic = Arc::new(elastic);

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
    backends.insert("es".to_string(), elastic.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
    providers.insert("es".to_string(), elastic as DynSearchProvider);
    let composite = CompositeStorage::new(config, backends)
        .expect("composite storage")
        .with_search_providers(providers)
        .with_full_primary(sqlite.clone());

    // Export jobs live on the primary, as `hfs` wires them.
    let state = helios_rest::AppState::new(Arc::new(composite), ServerConfig::for_testing())
        .with_bulk_export(
            sqlite as Arc<dyn BulkExportJobStore>,
            patient_scope::export_output(),
            Arc::new(BearerScopeAuth),
        );
    Some(patient_scope::serve(
        helios_rest::routing::fhir_routes::create_routes(state),
    ))
}

patient_scope_suite!(create_test_server);
