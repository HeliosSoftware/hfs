//! A `patient/` scope is confined to the launch patient's compartment (#1618)
//! — MongoDB.
//!
//! Runs the scenarios in `common/patient_scope.rs` (see
//! `patient_scope_compartment.rs` for SQLite) against a MongoDB container.
//! Bulk export jobs live in an in-memory SQLite store, as `hfs` wires its
//! embedded job-store sidecar for MongoDB. Each scenario gets its own
//! database.
//!
//! The container is a single-node replica set, as in the persistence suite's
//! `mongodb_tests.rs`: a standalone mongod has no oplog, so `hfs` answers
//! every transaction Bundle with 501 there, and the transaction scenario would
//! never reach the compartment checks.
//!
//! Requires Docker; honours `HFS_TEST_MONGODB_URL` (which must then be a
//! replica set too), and skips (reported as skipped, not passing) when neither
//! is available — matching every other MongoDB suite in this workspace.

#![cfg(all(feature = "mongodb", feature = "sqlite"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/patient_scope.rs"]
mod patient_scope;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{Backend, BulkExportJobStore};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use std::time::{Duration, Instant};

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

/// Polls until the one-member replica set has elected itself primary, so no
/// test races a connection against a node still in `STARTUP2`/`SECONDARY`.
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

/// A single-node replica set, started and initiated as
/// `mongodb_tests.rs`'s `shared_mongo` does it.
async fn shared_mongo() -> Option<&'static SharedMongo> {
    SHARED_MONGO
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_MONGODB_URL") {
                if !url.trim().is_empty() {
                    return Some(SharedMongo {
                        connection_string: url,
                        _container: None,
                    });
                }
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
                        // Every scenario builds its own database's indexes
                        // while the others write in transactions; mongod's
                        // 5ms transaction lock wait turns that overlap into
                        // `LockTimeout` 500s on the seed writes.
                        "--setParameter",
                        "maxTransactionLockRequestTimeoutMillis=5000",
                    ])
                    // Each scenario's database holds WiredTiger handles for
                    // every collection and index: the suite peaks around 930
                    // open files, and the CI Docker host allows 1024. Past
                    // that mongod panics and every request fails with 500
                    // (as in `mongodb_tests.rs` and #1944).
                    .with_ulimit("nofile", 64000, Some(64000))
                    .with_startup_timeout(Duration::from_secs(120)),
            )
            .start()
            .await
            .ok()?;
            // The member advertises its in-container address; the client
            // connects with `directConnection=true` through the mapped port.
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
        database_name: format!("pscope_{}", uuid::Uuid::new_v4().simple()),
        data_dir: Some(patient_scope::data_dir()),
        ..Default::default()
    })
    .expect("failed to create MongoBackend");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema");
    let backend = Arc::new(backend);

    let jobs = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite job store"));
    jobs.init_schema().expect("init job store schema");
    let state = helios_rest::AppState::new(backend, ServerConfig::for_testing()).with_bulk_export(
        jobs as Arc<dyn BulkExportJobStore>,
        patient_scope::export_output(),
        Arc::new(BearerScopeAuth),
    );
    Some(patient_scope::serve(
        helios_rest::routing::fhir_routes::create_routes(state),
    ))
}

patient_scope_suite!(create_test_server);
