//! SQL query ValueSet/ConceptMap dependencies (#2185), end to end on the
//! `postgres-elasticsearch` composite: PostgreSQL primary with its search
//! offloaded to Elasticsearch, wired as `hfs`'s
//! `start_postgres_elasticsearch` wires it. The composite hands out its
//! primary's terminology store, so the relations live in PostgreSQL while
//! every `url=` lookup the write and query paths make is answered by
//! Elasticsearch. The scenarios are shared by every backend; see
//! `common/sof_terminology_e2e.rs`.
//!
//! Requires Docker (testcontainers spins up PostgreSQL and Elasticsearch).

#![cfg(all(feature = "postgres", feature = "elasticsearch"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[path = "common/sof_terminology_e2e.rs"]
mod sof_terminology_e2e;

use std::collections::HashMap;
use std::sync::Arc;

use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{Backend, BackendKind};
use sof_terminology_e2e::Harness;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

struct SharedPg {
    host: String,
    port: u16,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

fn config(host: &str, port: u16) -> PostgresConfig {
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(sof_terminology_e2e::data_dir()),
        ..Default::default()
    }
}

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            // Pinned: testcontainers-modules defaults to postgres:11, which
            // predates `plan_cache_mode` and rejects every connection.
            let container = container_cleanup::with_cleanup_label(
                Postgres::default()
                    .with_tag("16-alpine")
                    .with_label("github.run_id", &run_id),
            )
            .start()
            .await
            .expect("failed to start PostgreSQL container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("failed to get host")
                .to_string();
            // Migrate once, so concurrent scenarios do not race the schema.
            PostgresBackend::new(config(&host, port))
                .await
                .expect("failed to create PostgresBackend")
                .init_schema()
                .await
                .expect("failed to initialize schema");
            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

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

async fn postgres_elasticsearch(terminology: Option<String>) -> Option<Harness> {
    let pg = shared_pg().await;
    let es = shared_es().await;

    let mut postgres = PostgresBackend::new(config(&pg.host, pg.port))
        .await
        .expect("failed to create PostgresBackend");
    postgres.set_search_offloaded(true);
    let postgres = Arc::new(postgres);

    // Elasticsearch shares the primary's registry; a per-scenario index
    // prefix keeps the scenarios apart in the one container.
    let elastic = ElasticsearchBackend::with_shared_registry(
        ElasticsearchConfig {
            nodes: vec![es.url.clone()],
            index_prefix: format!("softerm_{}", uuid::Uuid::new_v4().simple()),
            number_of_replicas: 0,
            refresh_interval: "1ms".to_string(),
            write_refresh: WriteRefreshPolicy::WaitFor,
            ..Default::default()
        },
        postgres.tenant_registries().clone(),
    )
    .expect("failed to create ElasticsearchBackend");
    elastic
        .initialize()
        .await
        .expect("failed to initialize Elasticsearch");
    let elastic = Arc::new(elastic);

    let config = CompositeConfig::builder()
        .primary("postgres", BackendKind::Postgres)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("postgres".to_string(), postgres.clone() as DynStorage);
    backends.insert("es".to_string(), elastic.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert(
        "postgres".to_string(),
        postgres.clone() as DynSearchProvider,
    );
    providers.insert("es".to_string(), elastic as DynSearchProvider);
    let composite = CompositeStorage::new(config, backends)
        .expect("composite storage")
        .with_search_providers(providers)
        .with_full_primary(postgres);
    Some(sof_terminology_e2e::harness(
        Arc::new(composite),
        terminology,
    ))
}

sof_terminology_e2e_suite!(postgres_elasticsearch);
