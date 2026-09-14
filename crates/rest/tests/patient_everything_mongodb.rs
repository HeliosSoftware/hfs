//! `Patient/$everything` router test — MongoDB-backed.
//!
//! Mirrors `patient_everything.rs` (SQLite in-memory) but wires the HTTP
//! server's storage backend to a MongoDB container via `testcontainers`, the
//! same way `mongodb_include_iterate.rs` does. Proves the paged walk
//! (`_count`-driven `next` links) returns the same match set as the unpaged
//! walk against a real document-store backend, not just SQLite.
//!
//! Requires Docker (testcontainers spins up a real MongoDB instance); skips
//! cleanly (reported as skipped, not passing) when Docker is unavailable —
//! matching every other MongoDB integration suite in this workspace.

#![cfg(feature = "mongodb")]

mod patient_everything_mongodb_tests {
    use axum::http::StatusCode;
    use axum_test::TestServer;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::Backend;
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    // =========================================================================
    // Shared container setup — copied from `mongodb_include_iterate.rs`.
    // =========================================================================

    struct SharedMongo {
        connection_string: String,
        /// `None` when `HFS_TEST_MONGODB_URL` pointed us at an instance we do
        /// not own, so there is nothing to tear down.
        _container: Option<testcontainers::ContainerAsync<Mongo>>,
    }

    static SHARED_MONGO: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

    async fn shared_mongo() -> Option<&'static SharedMongo> {
        SHARED_MONGO
            .get_or_init(|| async {
                // Prefer an instance the caller already has, matching the
                // contract the persistence suite documents. Starting a
                // container per test binary is how this machine accumulated
                // orphaned mongo containers; honouring the variable lets a
                // developer or CI lane point every suite at one server.
                if let Ok(url) = std::env::var("HFS_TEST_MONGODB_URL") {
                    if !url.trim().is_empty() {
                        return Some(SharedMongo {
                            connection_string: url,
                            _container: None,
                        });
                    }
                }

                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = Mongo::default()
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(std::time::Duration::from_secs(120))
                    .start()
                    .await
                    .ok()?;
                let port = container.get_host_port_ipv4(27017).await.ok()?;
                let host = container.get_host().await.ok()?.to_string();
                Some(SharedMongo {
                    connection_string: format!("mongodb://{host}:{port}"),
                    _container: Some(container),
                })
            })
            .await
            .as_ref()
    }

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    async fn create_test_server(name: &str) -> Option<TestServer> {
        let mongo = shared_mongo().await?;

        let config = MongoBackendConfig {
            connection_string: mongo.connection_string.clone(),
            // MongoDB caps database names at 64 bytes; a short fixed prefix
            // plus the UUID (32 hex chars) stays well under that.
            database_name: format!("everything_{}", uuid::Uuid::new_v4().simple()),
            data_dir: Some(data_dir()),
            ..Default::default()
        };
        let backend = MongoBackend::new(config).expect("failed to create MongoBackend");
        backend
            .initialize()
            .await
            .expect("failed to initialize MongoDB schema");

        let tenant_id = format!("everything_mongo_{name}_{}", uuid::Uuid::new_v4().simple());
        let server_config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: tenant_id,
            everything_max_unpaged: 10_000,
            ..ServerConfig::for_testing()
        };

        let state = helios_rest::AppState::new(Arc::new(backend), server_config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        Some(TestServer::new(app).expect("failed to create test server"))
    }

    // =========================================================================
    // Shared helpers — copied from Task 7's `crates/rest/tests/common/everything.rs`.
    // =========================================================================

    async fn put(server: &TestServer, resource: Value) {
        let rt = resource["resourceType"].as_str().unwrap();
        let id = resource["id"].as_str().unwrap();
        let resp = server.put(&format!("/{rt}/{id}")).json(&resource).await;
        assert!(
            resp.status_code().is_success(),
            "PUT {rt}/{id}: {}",
            resp.text()
        );
    }

    /// Seeds patient `p1` with 3 Observations, 2 Encounters, 1 Condition, a
    /// Practitioner and an Organization they reference; and a control patient
    /// `p2` with one Observation. Returns nothing; ids are fixed.
    async fn seed(server: &TestServer) {
        put(
            server,
            json!({"resourceType": "Organization", "id": "org1", "name": "Org"}),
        )
        .await;
        put(
            server,
            json!({"resourceType": "Practitioner", "id": "dr1", "name": [{"family": "Who"}]}),
        )
        .await;
        put(
            server,
            json!({"resourceType": "Patient", "id": "p1", "managingOrganization": {"reference": "Organization/org1"}}),
        )
        .await;
        put(server, json!({"resourceType": "Patient", "id": "p2"})).await;
        for (i, date) in [(1, "2019-05-01"), (2, "2020-05-01"), (3, "2021-05-01")] {
            put(
                server,
                json!({"resourceType": "Observation", "id": format!("o{i}"), "status": "final",
                    "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}, "effectiveDateTime": date,
                    "performer": [{"reference": "Practitioner/dr1"}]}),
            )
            .await;
        }
        for (i, date) in [(1, "2019-06-01"), (2, "2021-06-01")] {
            put(
                server,
                json!({"resourceType": "Encounter", "id": format!("e{i}"), "status": "finished",
                    "class": {"code": "AMB"}, "subject": {"reference": "Patient/p1"},
                    "period": {"start": date}, "serviceProvider": {"reference": "Organization/org1"}}),
            )
            .await;
        }
        put(
            server,
            json!({"resourceType": "Condition", "id": "c1", "subject": {"reference": "Patient/p1"},
                "onsetDateTime": "2020-01-15"}),
        )
        .await;
        put(
            server,
            json!({"resourceType": "Observation", "id": "other", "status": "final",
                "code": {"text": "x"}, "subject": {"reference": "Patient/p2"}}),
        )
        .await;
    }

    fn entries(bundle: &Value, mode: &str) -> Vec<String> {
        bundle["entry"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["search"]["mode"] == mode)
            .map(|e| {
                format!(
                    "{}/{}",
                    e["resource"]["resourceType"].as_str().unwrap(),
                    e["resource"]["id"].as_str().unwrap()
                )
            })
            .collect()
    }

    fn next_link(bundle: &Value) -> Option<String> {
        bundle["link"]
            .as_array()?
            .iter()
            .find(|l| l["relation"] == "next")?["url"]
            .as_str()
            .map(str::to_string)
    }

    fn path_of(url: &str) -> String {
        url.strip_prefix("http://localhost:8080")
            .unwrap()
            .to_string()
    }

    async fn walk(server: &TestServer, first: &str) -> (Vec<String>, Vec<Value>) {
        let mut path = first.to_string();
        let mut matches = Vec::new();
        let mut pages = Vec::new();
        loop {
            let resp = server.get(&path).await;
            assert_eq!(
                resp.status_code(),
                StatusCode::OK,
                "{path}: {}",
                resp.text()
            );
            let b: Value = resp.json();
            matches.extend(entries(&b, "match"));
            let next = next_link(&b);
            pages.push(b);
            match next {
                Some(n) => path = path_of(&n),
                None => break,
            }
            assert!(pages.len() < 50, "runaway paging");
        }
        (matches, pages)
    }

    #[tokio::test]
    async fn mongodb_everything_paged_walk_matches_unpaged() {
        let Some(server) = create_test_server("everything-mongo").await else {
            eprintln!("Skipping mongodb_everything_paged_walk_matches_unpaged (requires Docker)");
            return;
        };
        seed(&server).await;
        let (unpaged, _) = walk(&server, "/Patient/p1/$everything").await;
        let (paged, pages) = walk(&server, "/Patient/p1/$everything?_count=2").await;
        assert!(pages.len() >= 4);
        let mut a = unpaged.clone();
        a.sort();
        let mut b = paged.clone();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(paged.len(), unpaged.len());
        assert_eq!(unpaged[0], "Patient/p1");
        assert_eq!(unpaged.len(), 7, "{unpaged:?}");
    }
}
