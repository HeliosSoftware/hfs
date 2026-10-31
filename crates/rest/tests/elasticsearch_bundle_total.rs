//! `Bundle.total` over HTTP on Elasticsearch (#2283): a search carries a total
//! only when `_total` asks for one (`accurate` or `estimate`), as on every
//! other backend. The persistence-level table is
//! `crates/persistence/tests/search/total_mode_suite.rs`.
//!
//! Requires Docker (testcontainers spins up a real Elasticsearch instance).

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

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
use helios_persistence::core::{Backend, BackendKind};
use helios_rest::ServerConfig;
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;

#[tokio::test]
async fn bundle_total_only_when_requested() {
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
    let port = container.get_host_port_ipv4(9200).await.expect("port");
    let host = container.get_host().await.expect("host");

    // SQLite primary with its search offloaded to Elasticsearch, as the
    // `sqlite-elasticsearch` build wires it.
    let mut sqlite = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        },
    )
    .expect("in-memory sqlite");
    sqlite.init_schema().expect("init schema");
    sqlite.set_search_offloaded(true);
    let sqlite = Arc::new(sqlite);
    let elastic = ElasticsearchBackend::with_shared_registry(
        ElasticsearchConfig {
            nodes: vec![format!("http://{host}:{port}")],
            index_prefix: format!("btotal_{}", uuid::Uuid::new_v4().simple()),
            number_of_replicas: 0,
            refresh_interval: "1ms".to_string(),
            write_refresh: WriteRefreshPolicy::WaitFor,
            ..Default::default()
        },
        sqlite.tenant_registries().clone(),
    )
    .expect("failed to create ElasticsearchBackend");
    elastic.initialize().await.expect("initialize");
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
        .with_full_primary(sqlite);
    let state = helios_rest::AppState::new(Arc::new(composite), ServerConfig::for_testing());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).unwrap();

    for i in 0..3 {
        server
            .put(&format!("/Patient/bt-{i}"))
            .json(&json!({"resourceType": "Patient", "id": format!("bt-{i}")}))
            .await
            .assert_status_success();
    }

    let search = |query: &'static str| {
        let server = &server;
        async move {
            let response = server.get(&format!("/Patient{query}")).await;
            response.assert_status_ok();
            response.json::<Value>()
        }
    };

    let accurate = search("?_total=accurate").await;
    assert_eq!(accurate["total"], json!(3), "_total=accurate: Bundle.total");

    let estimate = search("?_total=estimate").await;
    assert_eq!(estimate["total"], json!(3), "_total=estimate: Bundle.total");

    for query in ["", "?_total=none"] {
        let bundle = search(query).await;
        assert!(
            bundle.get("total").is_none(),
            "{query:?}: no Bundle.total, got {bundle}"
        );
        assert_eq!(bundle["entry"].as_array().map(Vec::len), Some(3));
    }

    // A type with no index yet: no total unless asked, then 0.
    let empty = server.get("/Device").await;
    empty.assert_status_ok();
    assert!(empty.json::<Value>().get("total").is_none());
    let empty = server.get("/Device?_total=accurate").await;
    assert_eq!(empty.json::<Value>()["total"], json!(0));
}
