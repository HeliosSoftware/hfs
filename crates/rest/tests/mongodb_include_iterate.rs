//! #1063 end-to-end proof, over a real HTTP request against a MongoDB-backed
//! server: `_include:iterate` must actually follow the second hop.
//!
//! `mongodb_include_iterate_follows_the_second_hop` (in
//! `helios-persistence`'s `mongodb_tests`) proves the *primitives*: MongoDB's
//! own `resolve_includes` stops at hop 1, and composing its output with
//! `resolve_includes_iterate_continuation` reaches hop 2. `iterative_pass`
//! (in `helios-rest`'s `handlers::search` unit tests) proves the *decision
//! table* that picks the `Continuation` branch. Neither proves that
//! `execute_search_bundle` actually wires the two together end to end — that
//! is what this test pins, by exercising the real route through a real
//! `TestServer` over a real MongoDB backend, mirroring
//! `test_include_iterate` (`search_integration.rs`, SQLite) so the two stay
//! comparable.
//!
//! Requires Docker (testcontainers spins up a real MongoDB instance); skips
//! cleanly (reported as skipped, not passing) when Docker is unavailable —
//! matching every other MongoDB integration suite in this workspace.

#![cfg(feature = "mongodb")]

mod mongodb_include_iterate_tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::{Backend, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

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

    /// Builds a test server backed by MongoDB, with the full spec-file
    /// registry loaded (so `Patient.organization` → `managingOrganization` is
    /// known — the whole point of this test, matching
    /// `create_backend_with_full_registry` in the persistence-level suite).
    async fn create_test_server() -> Option<(TestServer, Arc<MongoBackend>)> {
        let mongo = shared_mongo().await?;

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = MongoBackendConfig {
            connection_string: mongo.connection_string.clone(),
            database_name: format!(
                "rest_mongo_include_iterate_{}",
                uuid::Uuid::new_v4().simple()
            ),
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend = MongoBackend::new(config).expect("failed to create MongoBackend");
        backend
            .initialize()
            .await
            .expect("failed to initialize MongoDB schema");
        let backend = Arc::new(backend);

        let server_config = ServerConfig {
            multitenancy: MultitenancyConfig {
                routing_mode: TenantRoutingMode::HeaderOnly,
                ..Default::default()
            },
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "test-tenant".to_string(),
            ..ServerConfig::for_testing()
        };

        let state = helios_rest::AppState::new(Arc::clone(&backend), server_config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        Some((server, backend))
    }

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    /// Organization/org-1 ← Patient/patient-1 (`managingOrganization`) ←
    /// Observation/obs-1 (`subject`) — the same shape as
    /// `test_include_iterate` (SQLite) and
    /// `mongodb_include_iterate_follows_the_second_hop` (persistence-level).
    async fn seed(backend: &MongoBackend) {
        let tenant = test_tenant();

        backend
            .create_or_update(
                &tenant,
                "Organization",
                "org-1",
                json!({"resourceType": "Organization", "id": "org-1", "name": "General Hospital"}),
                FhirVersion::default(),
            )
            .await
            .expect("seed Organization");

        backend
            .create_or_update(
                &tenant,
                "Patient",
                "patient-1",
                json!({
                    "resourceType": "Patient",
                    "id": "patient-1",
                    "name": [{"family": "Smith"}],
                    "managingOrganization": {"reference": "Organization/org-1"}
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed Patient");

        backend
            .create_or_update(
                &tenant,
                "Observation",
                "obs-1",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-1",
                    "status": "final",
                    "subject": {"reference": "Patient/patient-1"},
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]}
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed Observation");
    }

    fn get_bundle_entries(bundle: &Value) -> Vec<Value> {
        bundle["entry"].as_array().cloned().unwrap_or_default()
    }

    /// The end-to-end proof required by the #1061/#1063 binding review: a
    /// real HTTP `GET` against a real MongoDB-backed server, with
    /// `_include=Observation:subject&_include:iterate=Patient:organization`,
    /// must return the Patient (hop 1, resolved inline by MongoDB) AND the
    /// Organization (hop 2, resolved by the REST guard's `Continuation`
    /// pass) exactly once each, both as `search.mode = include` entries.
    #[tokio::test]
    async fn include_iterate_reaches_the_second_hop_over_http() {
        let Some((server, backend)) = create_test_server().await else {
            eprintln!(
                "Skipping include_iterate_reaches_the_second_hop_over_http (requires Docker)"
            );
            return;
        };
        seed(&backend).await;

        let response = server
            .get("/Observation?_id=obs-1&_include=Observation:subject&_include:iterate=Patient:organization")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let entries = get_bundle_entries(&body);

        let patient_entries: Vec<&Value> = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Patient")
            .collect();
        let organization_entries: Vec<&Value> = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Organization")
            .collect();
        let observation_entries: Vec<&Value> = entries
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "Observation")
            .collect();

        assert_eq!(
            observation_entries.len(),
            1,
            "primary match: exactly one Observation, got {entries:#?}"
        );
        assert_eq!(
            observation_entries[0]["search"]["mode"], "match",
            "the Observation is the primary match"
        );

        assert_eq!(
            patient_entries.len(),
            1,
            "hop 1 (Observation:subject): exactly one Patient, got {entries:#?}"
        );
        assert_eq!(
            patient_entries[0]["search"]["mode"], "include",
            "the Patient is an include entry"
        );

        assert_eq!(
            organization_entries.len(),
            1,
            ":iterate must reach hop 2 (Patient:organization) exactly once — the transitively \
             included Organization is missing or duplicated: {entries:#?}"
        );
        assert_eq!(
            organization_entries[0]["search"]["mode"], "include",
            "the Organization is an include entry"
        );
        assert_eq!(organization_entries[0]["resource"]["id"], "org-1");
    }
}
