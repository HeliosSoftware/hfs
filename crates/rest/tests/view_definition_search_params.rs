//! Integration tests for ViewDefinition SearchParameters (#570).
//!
//! Covers the ordinary search path for the SQL-on-FHIR IG's ViewDefinition
//! SearchParameters (`url`, `name`, `status`, `date`) once they are seeded
//! from the custom `data/sql-on-fhir-search-parameters.json` file, plus the
//! CapabilityStatement entries for `ViewDefinition` and `Library`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

/// Creates a test server backed by an in-memory SQLite database, wired to the
/// real workspace `data/` directory so the ViewDefinition SearchParameters
/// this ticket adds are actually loaded from `sql-on-fhir-search-parameters.json`
/// (a custom file, loaded additively alongside the spec bundles by
/// `SearchParameterLoader::load_custom_from_directory`) — the same setup
/// `search_integration.rs` and `rest_conformance.rs` use.
async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

/// The test tenant context used to seed data directly through the backend.
fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// A minimal, structurally valid ViewDefinition (per the SQL-on-FHIR IG):
/// `resourceType`, `status`, `resource`, and a non-empty `select` are the
/// fields the runner requires; the rest are metadata this test varies.
fn view_definition(id: &str, url: &str, name: &str, status: &str, date: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "id": id,
        "url": url,
        "name": name,
        "status": status,
        "date": date,
        "resource": "Patient",
        "select": [{"column": [{"path": "id", "name": "id"}]}]
    })
}

/// Seeds three ViewDefinitions spanning distinct `url`, `name`, `status`, and
/// `date` values, so a search on any one of those parameters can distinguish
/// a match from a miss.
async fn seed_view_definitions(backend: &SqliteBackend) {
    let tenant = test_tenant();

    let defs = vec![
        view_definition(
            "vd-active",
            "http://example.org/ViewDefinition/patient-flat",
            "PatientFlat",
            "active",
            "2024-01-15",
        ),
        view_definition(
            "vd-draft",
            "http://example.org/ViewDefinition/patient-draft",
            "PatientDraft",
            "draft",
            "2023-06-01",
        ),
        view_definition(
            "vd-retired",
            "http://example.org/ViewDefinition/patient-retired",
            "PatientRetired",
            "retired",
            "2022-03-10",
        ),
    ];

    for def in defs {
        backend
            .create(&tenant, "ViewDefinition", def, FhirVersion::R4)
            .await
            .expect("seed ViewDefinition");
    }
}

fn bundle_entries(body: &Value) -> Vec<&Value> {
    body["entry"]
        .as_array()
        .map(|entries| entries.iter().collect())
        .unwrap_or_default()
}

fn entry_ids(body: &Value) -> Vec<String> {
    bundle_entries(body)
        .iter()
        .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
        .collect()
}

mod search_by_url {
    use super::*;

    #[tokio::test]
    async fn matches_the_exact_canonical_url() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?url=http://example.org/ViewDefinition/patient-flat")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(entry_ids(&body), vec!["vd-active".to_string()]);
    }

    #[tokio::test]
    async fn no_match_returns_empty_bundle() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?url=http://example.org/ViewDefinition/does-not-exist")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert!(entry_ids(&body).is_empty());
    }
}

mod search_by_name {
    use super::*;

    #[tokio::test]
    async fn matches_the_computationally_friendly_name() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?name=PatientDraft")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(entry_ids(&body), vec!["vd-draft".to_string()]);
    }
}

mod search_by_status {
    use super::*;

    #[tokio::test]
    async fn matches_only_the_requested_status() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?status=retired")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(entry_ids(&body), vec!["vd-retired".to_string()]);
    }
}

mod search_by_date_range {
    use super::*;

    #[tokio::test]
    async fn ge_filters_out_earlier_publication_dates() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?date=ge2023-01-01")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        let mut ids = entry_ids(&body);
        ids.sort();
        assert_eq!(
            ids,
            vec!["vd-active".to_string(), "vd-draft".to_string()],
            "vd-retired (2022) predates the ge2023-01-01 bound"
        );
    }

    #[tokio::test]
    async fn last_updated_range_matches_freshly_created_resources() {
        let (server, backend) = create_test_server().await;
        seed_view_definitions(&backend).await;

        let response = server
            .get("/ViewDefinition?_lastUpdated=gt2000-01-01")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status_ok();
        let body: Value = response.json();
        assert_eq!(entry_ids(&body).len(), 3, "all three were just created");
    }
}

mod capability_statement {
    use super::*;

    fn find_resource<'a>(body: &'a Value, resource_type: &str) -> &'a Value {
        body["rest"][0]["resource"]
            .as_array()
            .expect("rest[0].resource is an array")
            .iter()
            .find(|r| r["type"] == resource_type)
            .unwrap_or_else(|| panic!("no rest.resource entry for {resource_type}"))
    }

    /// Every resource-level interaction the server advertises for ordinary
    /// resource types (`build_resource_capability`'s fixed interaction set).
    const FULL_INTERACTION_SET: &[&str] = &[
        "read",
        "vread",
        "update",
        "patch",
        "delete",
        "history-instance",
        "history-type",
        "create",
        "search-type",
    ];

    fn assert_full_interaction_set(resource: &Value, resource_type: &str) {
        let codes: Vec<&str> = resource["interaction"]
            .as_array()
            .unwrap_or_else(|| panic!("{resource_type}.interaction is an array"))
            .iter()
            .filter_map(|i| i["code"].as_str())
            .collect();
        for expected in FULL_INTERACTION_SET {
            assert!(
                codes.contains(expected),
                "{resource_type} is missing interaction {expected}; got {codes:?}"
            );
        }
    }

    /// The CapabilityStatement must advertise a `rest.resource` entry for
    /// `ViewDefinition` — the resource this whole ticket adds search support
    /// for — with the full interaction set and its registered SearchParameters
    /// (name + canonical `definition`).
    #[tokio::test]
    async fn advertises_view_definition_with_full_interactions_and_search_params() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();

        let vd = find_resource(&body, "ViewDefinition");
        assert_full_interaction_set(vd, "ViewDefinition");

        let search_params = vd["searchParam"]
            .as_array()
            .expect("ViewDefinition.searchParam is an array");
        let names: Vec<&str> = search_params
            .iter()
            .filter_map(|p| p["name"].as_str())
            .collect();
        for expected in ["url", "name", "status", "date"] {
            assert!(
                names.contains(&expected),
                "ViewDefinition.searchParam is missing {expected}; got {names:?}"
            );
        }

        // Each advertised param's `definition` names the real SQL-on-FHIR IG
        // canonical, not a placeholder.
        for param in search_params {
            let name = param["name"].as_str().unwrap();
            if ["url", "name", "status", "date"].contains(&name) {
                let definition = param["definition"].as_str().unwrap_or_default();
                assert_eq!(
                    definition,
                    format!("http://hl7.org/fhir/SearchParameter/ViewDefinition-{name}")
                );
            }
        }
    }

    /// `Library` is a core FHIR resource type (its `rest.resource` entry comes
    /// from the generated resource list, not from special-casing here); this
    /// asserts it keeps the same full interaction set as every other type.
    #[tokio::test]
    async fn library_has_full_interaction_set() {
        let (server, _backend) = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        let body: Value = response.json();

        let library = find_resource(&body, "Library");
        assert_full_interaction_set(library, "Library");
    }
}

mod reindex_coverage {
    use super::*;
    use axum::http::StatusCode;
    use helios_persistence::search::{ReindexOperation, ReindexTarget};

    /// Same backend/config as [`create_test_server`], plus `$reindex` wired —
    /// the generic operation (`ReindexRequest::for_types`/`for_params` with no
    /// resource-type allowlist) rather than a per-type special case.
    async fn create_test_server_with_reindex() -> (TestServer, Arc<SqliteBackend>) {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let backend_config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", backend_config)
            .expect("Failed to create SQLite backend");
        backend.init_schema().expect("Failed to init schema");
        let backend = Arc::new(backend);

        let reindex = Arc::new(ReindexOperation::new(
            backend.clone(),
            backend.tenant_registries().clone(),
        ));

        let config = ServerConfig {
            multitenancy: MultitenancyConfig {
                routing_mode: TenantRoutingMode::HeaderOnly,
                ..Default::default()
            },
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "test-tenant".to_string(),
            ..ServerConfig::for_testing()
        };

        let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_reindex(reindex);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("Failed to create test server");

        (server, backend)
    }

    async fn url_search_matches(server: &TestServer) -> usize {
        let response = server
            .get("/ViewDefinition?url=http://example.org/ViewDefinition/reindex-target")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status_ok();
        response.json::<Value>()["entry"]
            .as_array()
            .map_or(0, Vec::len)
    }

    /// `$reindex` covers `ViewDefinition` exactly like any other resource
    /// type: dropping the search index breaks `url=` search, and
    /// `POST /ViewDefinition/$reindex` — the step the README tells operators
    /// to run after this upgrade — restores it. The resolver behind
    /// `ReindexRequest`'s `None` resource-type case enumerates whatever
    /// `list_resource_types` returns from storage, so there is no allowlist
    /// that could silently omit `ViewDefinition`.
    #[tokio::test]
    async fn reindex_restores_view_definition_url_searchability() {
        let (server, backend) = create_test_server_with_reindex().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "ViewDefinition",
                view_definition(
                    "reindex-target",
                    "http://example.org/ViewDefinition/reindex-target",
                    "ReindexTarget",
                    "active",
                    "2024-01-01",
                ),
                FhirVersion::R4,
            )
            .await
            .expect("seed ViewDefinition");

        assert_eq!(
            url_search_matches(&server).await,
            1,
            "search must work before the index is dropped"
        );

        backend
            .clear_search_index(&tenant)
            .await
            .expect("clear index");
        assert_eq!(
            url_search_matches(&server).await,
            0,
            "search must be broken once the index is gone"
        );

        let kickoff = server
            .post("/ViewDefinition/$reindex")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        kickoff.assert_status(StatusCode::ACCEPTED);
        let job_id = kickoff.json::<Value>()["parameter"]
            .as_array()
            .and_then(|params| params.iter().find(|p| p["name"] == "jobId"))
            .and_then(|p| p["valueString"].as_str())
            .expect("job id")
            .to_string();

        let mut final_status = String::new();
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let status = server
                .get(&format!("/$reindex-status/{job_id}"))
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .await;
            if status.status_code() != StatusCode::OK {
                continue;
            }
            let state = status.json::<Value>()["parameter"]
                .as_array()
                .and_then(|params| params.iter().find(|p| p["name"] == "status"))
                .and_then(|p| p["valueCode"].as_str())
                .unwrap_or_default()
                .to_string();
            if state == "completed" || state == "failed" || state == "cancelled" {
                final_status = state;
                break;
            }
        }
        assert_eq!(final_status, "completed", "reindex job did not complete");

        assert_eq!(
            url_search_matches(&server).await,
            1,
            "search must work again once ViewDefinition is reindexed"
        );
    }
}
