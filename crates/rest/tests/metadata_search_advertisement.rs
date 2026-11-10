//! #514: `/metadata` advertises search exactly where the storage serves it.
//!
//! `search-type` and the `searchParam` / `searchInclude` / `searchRevInclude`
//! lists used to be listed for every resource type on every backend. Standalone
//! S3 answers every search `501` (its `search` returns `UnsupportedCapability`),
//! so the statement promised an interaction no request could use. Both now read
//! one source, `SearchProvider::supports_search`.
//!
//! Each case below asserts the two together, so the test fails if the
//! statement and the route drift apart again: where `/metadata` lists search the
//! route answers `200`, and where it does not the route answers `501`.
//!
//! Standalone S3 is stood in for by a storage whose `supports_search()` is
//! `false` and whose `search` returns `UnsupportedCapability`, with real SQLite
//! behind its CRUD, so no AWS client or container is needed. It sits behind a
//! `CompositeStorage` because that is the shape `AppState` can carry without
//! re-implementing every storage trait; the composite answers for the provider
//! its `search` reaches (unit-tested in `composite/storage.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{BackendKind, ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchQuery, StoredResource};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::Value;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// In-memory SQLite with the spec search parameters loaded, so `name` is a
/// known `Patient` parameter and a search for it parses.
fn sqlite() -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");
    backend
}

fn config() -> ServerConfig {
    ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    }
}

/// A storage that cannot search, as standalone S3: `supports_search()` is
/// `false` and `search` / `search_count` answer `UnsupportedCapability`.
struct NoSearch {
    inner: Arc<SqliteBackend>,
}

fn unsupported(capability: &str) -> StorageError {
    StorageError::Backend(BackendError::UnsupportedCapability {
        backend_name: "no-search".to_string(),
        capability: capability.to_string(),
    })
}

#[async_trait]
impl ResourceStorage for NoSearch {
    fn backend_name(&self) -> &'static str {
        "no-search"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

#[async_trait]
impl SearchProvider for NoSearch {
    fn supports_search(&self) -> bool {
        false
    }

    async fn search(
        &self,
        _tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        Err(unsupported("search"))
    }

    async fn search_count(
        &self,
        _tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<u64> {
        Err(unsupported("search_count"))
    }

    fn search_param_registry(
        &self,
        tenant: &TenantContext,
    ) -> Arc<parking_lot::RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }
}

fn serve_sqlite(storage: SqliteBackend) -> TestServer {
    let state = helios_rest::AppState::new(Arc::new(storage), config());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

fn serve_composite(storage: CompositeStorage) -> TestServer {
    let state = helios_rest::AppState::new(Arc::new(storage), config());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

/// Plain SQLite: searches, says so.
fn sqlite_server() -> TestServer {
    serve_sqlite(sqlite())
}

/// A primary that cannot search and no search backend, so `search` reaches
/// the primary's provider: the standalone-S3 shape.
fn storage_without_search_server() -> TestServer {
    let primary = Arc::new(sqlite());
    let no_search = Arc::new(NoSearch {
        inner: primary.clone(),
    });

    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), no_search as DynSearchProvider);

    serve_composite(
        CompositeStorage::new(composite_config, backends)
            .expect("composite")
            .with_search_providers(providers)
            .with_full_primary(primary),
    )
}

/// The same primary that cannot search, with a dedicated search backend (a
/// second SQLite standing in for Elasticsearch): the `s3-elasticsearch` shape.
/// `search` goes to the search backend, so the deployment searches.
fn storage_with_search_backend_server() -> TestServer {
    let primary = Arc::new(sqlite());
    let no_search = Arc::new(NoSearch {
        inner: primary.clone(),
    });
    let index = Arc::new(sqlite());

    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Synchronous)
        .fhir_version(FhirVersion::default())
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), no_search as DynSearchProvider);
    providers.insert("search".to_string(), index as DynSearchProvider);

    serve_composite(
        CompositeStorage::new(composite_config, backends)
            .expect("composite")
            .with_search_providers(providers)
            .with_full_primary(primary)
            .start_sync_workers(),
    )
}

/// Every `rest.resource` entry of `/metadata`, for every enabled FHIR version.
async fn resource_entries(server: &TestServer) -> Vec<(FhirVersion, Vec<Value>)> {
    let mut all = Vec::new();
    for version in FhirVersion::enabled_versions() {
        let accept = format!(
            "application/fhir+json; fhirVersion={}",
            version.as_mime_param()
        );
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                header::ACCEPT,
                HeaderValue::from_str(&accept).expect("accept"),
            )
            .await;
        response.assert_status_ok();
        let statement: Value = serde_json::from_str(&response.text()).expect("json");
        let resources = statement["rest"][0]["resource"]
            .as_array()
            .expect("resources")
            .clone();
        assert!(!resources.is_empty(), "{version:?}");
        all.push((*version, resources));
    }
    all
}

fn lists_search_type(entry: &Value) -> bool {
    entry["interaction"]
        .as_array()
        .expect("interaction")
        .iter()
        .any(|i| i["code"] == "search-type")
}

/// What `GET /Patient?name=x` answers.
async fn search_status(server: &TestServer) -> StatusCode {
    server
        .get("/Patient?name=x")
        .add_header(X_TENANT_ID, tenant())
        .await
        .status_code()
}

/// Search is advertised for every resource type and the route serves it.
async fn assert_search_advertised_and_served(server: &TestServer) {
    for (version, resources) in resource_entries(server).await {
        for entry in &resources {
            let ty = &entry["type"];
            assert!(lists_search_type(entry), "{version:?} {ty}: no search-type");
            assert!(
                entry["searchParam"]
                    .as_array()
                    .is_some_and(|params| !params.is_empty()),
                "{version:?} {ty}: empty searchParam"
            );
        }
        let patient = resources
            .iter()
            .find(|r| r["type"] == "Patient")
            .expect("Patient");
        assert!(
            patient["searchParam"]
                .as_array()
                .expect("searchParam")
                .iter()
                .any(|p| p["name"] == "name"),
            "{version:?}: Patient lists `name`"
        );
    }
    assert_eq!(search_status(server).await, StatusCode::OK);
}

/// No resource type, `AuditEvent` included, lists `search-type` or any search
/// block, and the route answers `501`. Everything else is still there.
async fn assert_search_not_advertised_and_refused(server: &TestServer) {
    for (version, resources) in resource_entries(server).await {
        for entry in &resources {
            let ty = &entry["type"];
            assert!(
                !lists_search_type(entry),
                "{version:?} {ty}: advertises search-type"
            );
            for key in ["searchParam", "searchInclude", "searchRevInclude"] {
                assert!(entry.get(key).is_none(), "{version:?} {ty}: has {key}");
            }
            // Nothing else about the entry changed.
            assert_eq!(entry["versioning"], "versioned", "{version:?} {ty}");
            assert_eq!(entry["conditionalRead"], "full-support", "{version:?} {ty}");
            assert!(
                entry["interaction"]
                    .as_array()
                    .expect("interaction")
                    .iter()
                    .any(|i| i["code"] == "read"),
                "{version:?} {ty}: read"
            );
        }
        assert!(
            resources.iter().any(|r| r["type"] == "AuditEvent"),
            "{version:?}: the AuditEvent branch is covered"
        );
    }
    assert_eq!(search_status(server).await, StatusCode::NOT_IMPLEMENTED);
}

/// The real SQLite backend: search is advertised and `GET /Patient?name=x`
/// is served.
#[tokio::test]
async fn sqlite_advertises_search_and_serves_it() {
    assert_search_advertised_and_served(&sqlite_server()).await;
}

/// A storage whose `supports_search()` is `false` and whose `search` is
/// `UnsupportedCapability`: nothing about search in `/metadata`, `501` on the
/// route.
#[tokio::test]
async fn a_storage_that_cannot_search_advertises_nothing_and_answers_501() {
    assert_search_not_advertised_and_refused(&storage_without_search_server()).await;
}

/// The primary cannot search but a dedicated search backend does: the
/// deployment searches, and `/metadata` says so (the statement follows the
/// provider `search` reaches, not the primary).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_search_backend_makes_the_deployment_searchable_and_advertised() {
    assert_search_advertised_and_served(&storage_with_search_backend_server()).await;
}
