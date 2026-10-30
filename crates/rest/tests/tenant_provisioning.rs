//! Integration tests for provisioned-only tenant enforcement
//! (`HFS_TENANT_REQUIRE_PROVISIONED`): a request resolving to a tenant that was
//! never provisioned through the admin API is rejected, while provisioned
//! tenants work normally.
//!
//! The second half drives the tenant extractor through a probe route over a
//! scripted tenant registry, so axum renders the real rejection and the test
//! sees the status, the `Retry-After` header and the OperationOutcome a client
//! would (#2333).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::{Router, routing::get};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{ResourceStorage, TenantRecord};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_rest::AppState;
use helios_rest::ServerConfig;
use helios_rest::config::MultitenancyConfig;
use helios_rest::extractors::TenantExtractor;
use serde_json::Value;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

async fn create_server(require_provisioned: bool) -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            require_provisioned_tenant: require_provisioned,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");
    (server, backend)
}

#[tokio::test]
async fn unprovisioned_tenant_is_rejected_when_enforced() {
    let (server, backend) = create_server(true).await;
    backend
        .register_tenant("acme", None)
        .await
        .expect("provision acme");

    // A provisioned tenant is served normally.
    let ok = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(ok.status_code(), StatusCode::OK);

    // An unprovisioned tenant is rejected.
    let rejected = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("ghost"))
        .await;
    assert_eq!(rejected.status_code(), StatusCode::NOT_FOUND);

    // The rejection is an OperationOutcome that keeps the provisioning hint.
    let outcome = rejected.json::<Value>();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    assert_eq!(outcome["issue"][0]["code"], "not-found");
    let text = outcome["issue"][0]["details"]["text"]
        .as_str()
        .expect("details.text");
    assert!(
        text.contains("Unknown tenant 'ghost'"),
        "unexpected text: {text}"
    );
}

#[tokio::test]
async fn unprovisioned_tenant_is_allowed_when_not_enforced() {
    // Default (backward-compatible) behavior: any tenant id works.
    let (server, _backend) = create_server(false).await;
    let resp = server
        .get("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static("ghost"))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
}

// -- #2333: a failed tenant-registry lookup is not a blanket 500 --------------

/// What the scripted registry answers to `get_tenant`.
#[derive(Clone, Copy, Debug)]
enum Lookup {
    Provisioned,
    Missing,
    Unavailable,
    ConnectionFailed,
    PoolExhausted,
    Internal,
}

/// A tenant registry whose `get_tenant` answer is scripted. Nothing else on it
/// is reachable: the probe route reads no resources.
struct ScriptedRegistry(Lookup);

const STUB_BACKEND: &str = "stubdb-2333";

/// The shape of MongoDB's `read_driver_error` message: safe to send to a client.
const SAFE_UNAVAILABLE: &str =
    "tenant lookup: the database is temporarily unavailable; retry the request";

/// Driver text that must never reach a client.
const DRIVER_TEXT: &str = "SQLSTATE 08006: server closed the connection unexpectedly";

#[async_trait]
impl ResourceStorage for ScriptedRegistry {
    fn backend_name(&self) -> &'static str {
        STUB_BACKEND
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<TenantRecord>> {
        match self.0 {
            Lookup::Provisioned => Ok(Some(TenantRecord {
                id: id.to_string(),
                display_name: None,
                created_at: "2026-10-30T00:00:00Z".to_string(),
            })),
            Lookup::Missing => Ok(None),
            Lookup::Unavailable => Err(StorageError::Backend(BackendError::Unavailable {
                backend_name: STUB_BACKEND.to_string(),
                message: SAFE_UNAVAILABLE.to_string(),
            })),
            Lookup::ConnectionFailed => {
                Err(StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: STUB_BACKEND.to_string(),
                    message: SAFE_UNAVAILABLE.to_string(),
                }))
            }
            Lookup::PoolExhausted => Err(StorageError::Backend(BackendError::PoolExhausted {
                backend_name: STUB_BACKEND.to_string(),
            })),
            Lookup::Internal => Err(StorageError::Backend(BackendError::Internal {
                backend_name: STUB_BACKEND.to_string(),
                message: DRIVER_TEXT.to_string(),
                source: None,
            })),
        }
    }

    async fn create(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        unimplemented!("not reached: the probe route reads no resources")
    }

    async fn create_or_update(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        unimplemented!("not reached: the probe route reads no resources")
    }

    async fn read(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        unimplemented!("not reached: the probe route reads no resources")
    }

    async fn update(
        &self,
        _tenant: &TenantContext,
        _current: &StoredResource,
        _resource: Value,
    ) -> StorageResult<StoredResource> {
        unimplemented!("not reached: the probe route reads no resources")
    }

    async fn delete(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<()> {
        unimplemented!("not reached: the probe route reads no resources")
    }

    async fn count(
        &self,
        _tenant: &TenantContext,
        _resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        unimplemented!("not reached: the probe route reads no resources")
    }
}

/// The handler does nothing but extract the tenant, so the response is the
/// extractor's own verdict.
async fn probe(tenant: TenantExtractor) -> String {
    tenant.tenant_id().to_string()
}

fn probe_server(lookup: Lookup) -> TestServer {
    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            require_provisioned_tenant: true,
            ..Default::default()
        },
        default_tenant: "default".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = AppState::new(Arc::new(ScriptedRegistry(lookup)), config);
    let app = Router::new().route("/probe", get(probe)).with_state(state);
    TestServer::new(app).expect("create test server")
}

async fn probe_as(server: &TestServer, tenant: &'static str) -> axum_test::TestResponse {
    server
        .get("/probe")
        .add_header(X_TENANT_ID, HeaderValue::from_static(tenant))
        .await
}

#[tokio::test]
async fn transient_tenant_lookup_failure_is_503_with_retry_after() {
    let server = probe_server(Lookup::Unavailable);
    let resp = probe_as(&server, "acme").await;

    assert_eq!(resp.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    let retry_after: u64 = resp
        .headers()
        .get("retry-after")
        .expect("a transient failure must carry Retry-After")
        .to_str()
        .expect("ascii header")
        .parse()
        .expect("Retry-After is delta-seconds");
    assert!(retry_after > 0);

    let outcome = resp.json::<Value>();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    assert_eq!(outcome["issue"][0]["code"], "transient");
    assert_eq!(outcome["issue"][0]["details"]["text"], SAFE_UNAVAILABLE);

    // The backend name and the Display wrapper are what the old body leaked.
    let raw = resp.text();
    for leaked in [STUB_BACKEND, "backend unavailable", "Tenant lookup failed"] {
        assert!(!raw.contains(leaked), "body leaked {leaked:?}: {raw}");
    }
}

#[tokio::test]
async fn every_transient_lookup_kind_is_503_with_retry_after() {
    // Not asserting the backend name is absent: `RestError` words PoolExhausted
    // as "connection pool exhausted for {backend_name}", which is existing
    // behaviour for every handler.
    for lookup in [Lookup::ConnectionFailed, Lookup::PoolExhausted] {
        let server = probe_server(lookup);
        let resp = probe_as(&server, "acme").await;

        assert_eq!(
            resp.status_code(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{lookup:?}"
        );
        assert!(
            resp.headers().get("retry-after").is_some(),
            "{lookup:?} must carry Retry-After"
        );
        assert_eq!(
            resp.json::<Value>()["issue"][0]["code"],
            "transient",
            "{lookup:?}"
        );
    }
}

#[tokio::test]
async fn non_transient_tenant_lookup_failure_is_500_without_driver_text() {
    let server = probe_server(Lookup::Internal);
    let resp = probe_as(&server, "acme").await;

    assert_eq!(resp.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        resp.headers().get("retry-after").is_none(),
        "a server fault is not retryable"
    );

    let outcome = resp.json::<Value>();
    assert_eq!(outcome["issue"][0]["code"], "exception");
    assert_eq!(
        outcome["issue"][0]["details"]["text"],
        "An internal error occurred while processing the request."
    );

    let raw = resp.text();
    for leaked in [
        "SQLSTATE",
        "08006",
        "server closed the connection",
        STUB_BACKEND,
        "Tenant lookup failed",
    ] {
        assert!(!raw.contains(leaked), "body leaked {leaked:?}: {raw}");
    }
}

#[tokio::test]
async fn unknown_tenant_stays_404() {
    let server = probe_server(Lookup::Missing);
    let resp = probe_as(&server, "ghost").await;

    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);
    assert!(resp.headers().get("retry-after").is_none());

    let outcome = resp.json::<Value>();
    assert_eq!(outcome["issue"][0]["code"], "not-found");
    assert_eq!(
        outcome["issue"][0]["details"]["text"],
        "Unknown tenant 'ghost': tenants must be provisioned via the admin API"
    );
}

#[tokio::test]
async fn provisioned_tenant_passes_the_extractor() {
    let server = probe_server(Lookup::Provisioned);
    let resp = probe_as(&server, "acme").await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(resp.text(), "acme");
}
