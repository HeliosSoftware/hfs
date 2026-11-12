//! #2554: a write whose body has an empty object, an empty array or a `null`
//! member is stored without them, on the REST write paths — MongoDB.
//!
//! Runs the scenarios in `common/empty_elements.rs` (see
//! `empty_elements_on_write.rs` for SQLite). Each scenario gets its own
//! database. The container is a single-node replica set, as in
//! `patient_scope_compartment_mongodb.rs`, so the transaction scenario runs
//! instead of being answered 501.
//!
//! Requires Docker; honours `HFS_TEST_MONGODB_URL` (which must then be a
//! replica set too), and skips (reported as `Skipping`, not passing) when
//! neither is available.

#![cfg(feature = "mongodb")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/empty_elements.rs"]
mod empty_elements;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum_test::TestServer;
use helios_persistence::backends::mongodb::{IndexBuildMode, MongoBackend, MongoBackendConfig};
use helios_persistence::core::Backend;
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use testcontainers::ImageExt;
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;
use tokio::sync::OnceCell;

struct SharedMongo {
    connection_string: String,
    /// `None` when `HFS_TEST_MONGODB_URL` pointed us at an instance we do
    /// not own, so there is nothing to tear down.
    _container: Option<testcontainers::ContainerAsync<Mongo>>,
}

static SHARED_MONGO: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

/// Polls until the one-member replica set has elected itself primary.
async fn wait_for_writable_primary(container: &testcontainers::ContainerAsync<Mongo>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(mut exec) = container
            .exec(ExecCommand::new([
                "mongosh",
                "--quiet",
                "--eval",
                "db.hello().isWritablePrimary",
            ]))
            .await
            && let Ok(stdout) = exec.stdout_to_vec().await
            && String::from_utf8_lossy(&stdout).trim() == "true"
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "replica set rs0 did not report a writable primary within 60s"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn shared_mongo() -> Option<&'static SharedMongo> {
    SHARED_MONGO
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_MONGODB_URL")
                && !url.trim().is_empty()
            {
                return Some(SharedMongo {
                    connection_string: url,
                    _container: None,
                });
            }
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = container_cleanup::with_cleanup_label(
                Mongo::default()
                    .with_tag("7.0")
                    .with_label("github.run_id", &run_id)
                    .with_cmd([
                        "mongod",
                        "--bind_ip_all",
                        "--wiredTigerCacheSizeGB",
                        "0.25",
                        "--replSet",
                        "rs0",
                        "--oplogSize",
                        "128",
                    ])
                    .with_startup_timeout(Duration::from_secs(120)),
            )
            .start()
            .await
            .ok()?;
            let mut initiate = container
                .exec(ExecCommand::new([
                    "mongosh",
                    "--quiet",
                    "--eval",
                    "rs.initiate({_id:\"rs0\",members:[{_id:0,host:\"localhost:27017\"}]})",
                ]))
                .await
                .expect("rs.initiate on the Mongo container");
            let _ = initiate.stdout_to_vec().await;
            wait_for_writable_primary(&container).await;
            let port = container.get_host_port_ipv4(27017).await.ok()?;
            let host = container.get_host().await.ok()?.to_string();
            Some(SharedMongo {
                connection_string: format!("mongodb://{host}:{port}/?directConnection=true"),
                _container: Some(container),
            })
        })
        .await
        .as_ref()
}

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let mongo = shared_mongo().await?;
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: mongo.connection_string.clone(),
        // MongoDB caps database names at 64 bytes.
        database_name: format!("empty_{}", uuid::Uuid::new_v4().simple()),
        data_dir: Some(empty_elements::data_dir()),
        // Indexes are built before the scenario writes, as in
        // `patient_scope_compartment_mongodb.rs`.
        index_build: IndexBuildMode::Inline,
        ..Default::default()
    })
    .expect("failed to create MongoBackend");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema");
    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: empty_elements::TENANT.to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    Some(TestServer::new(app).expect("Failed to create test server"))
}

empty_elements_suite!(create_test_server);
