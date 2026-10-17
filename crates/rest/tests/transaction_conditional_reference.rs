//! #1047: a `transaction` whose resource bodies carry conditional references
//! (`"reference": "Organization?identifier=…"`) resolves them through a
//! search. On a composite backend that search reads the secondary, which
//! lags the primary the server has already returned `201` from — so a
//! transaction posted right after the referenced resource was created was
//! rejected with `400 "matches no existing resource"`, and the identical
//! request succeeded a few seconds later.
//!
//! Drives the real REST stack over a production-shaped composite: a SQLite
//! primary with its own index offloaded, and a search secondary that delays
//! every indexing write so the lag is deterministic. The default
//! (asynchronous) sync mode is used, because that is where the ordinary
//! "load reference data, then load the transactions that point at it"
//! sequence runs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::search::{SearchProvider, SearchResult};
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::error::StorageResult;
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchQuery, StoredResource};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

/// How long the search secondary sits on each write before indexing it.
const INDEX_LAG: Duration = Duration::from_millis(300);

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

/// A search secondary whose writes land late — an Elasticsearch index
/// between a write's acknowledgement and its next refresh.
struct LaggingIndex {
    inner: SqliteBackend,
}

#[async_trait]
impl ResourceStorage for LaggingIndex {
    fn backend_name(&self) -> &'static str {
        "lagging-index"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tokio::time::sleep(INDEX_LAG).await;
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
        tokio::time::sleep(INDEX_LAG).await;
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
        tokio::time::sleep(INDEX_LAG).await;
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        tokio::time::sleep(INDEX_LAG).await;
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
impl SearchProvider for LaggingIndex {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.inner.search(tenant, query).await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.inner.search_count(tenant, query).await
    }

    fn search_param_registry(
        &self,
        tenant: &TenantContext,
    ) -> Arc<parking_lot::RwLock<SearchParameterRegistry>> {
        self.inner.search_param_registry(tenant)
    }
}

fn server() -> TestServer {
    let mut primary = sqlite();
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = Arc::new(LaggingIndex { inner: sqlite() });

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Asynchronous)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index as DynSearchProvider);
    let composite = Arc::new(
        CompositeStorage::new(config, backends)
            .expect("composite")
            .with_search_providers(providers)
            .with_full_primary(primary)
            .start_sync_workers(),
    );

    let state = helios_rest::AppState::new(
        composite,
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server")
}

const PROBE_SYSTEM: &str = "urn:zzz:probe";

fn transaction_with_encounter_at(reference: &str) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "resource": {
                "resourceType": "Encounter",
                "status": "finished",
                "class": {"code": "AMB"},
                "serviceProvider": {"reference": reference}
            },
            "request": {"method": "POST", "url": "Encounter"}
        }]
    })
}

/// The exact reproduction from the issue: create the Organization, then
/// immediately post a transaction that references it conditionally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_resolves_conditional_reference_to_a_just_created_resource() {
    let server = server();

    let created = server
        .post("/Organization")
        .json(&json!({
            "resourceType": "Organization",
            "identifier": [{"system": PROBE_SYSTEM, "value": "ORG-PROBE-1047"}],
            "name": "ZZZ Probe Org"
        }))
        .await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );
    let org_id = created.json::<Value>()["id"]
        .as_str()
        .expect("created Organization has an id")
        .to_string();

    let reference = format!("Organization?identifier={PROBE_SYSTEM}|ORG-PROBE-1047");
    let response = server
        .post("/")
        .json(&transaction_with_encounter_at(&reference))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "a transaction referencing a resource the server already acknowledged \
         must not be rejected because the search index has not caught up: {}",
        response.text()
    );
    let bundle = response.json::<Value>();
    assert_eq!(bundle["type"], "transaction-response");
    let entry = &bundle["entry"][0];
    assert_eq!(entry["response"]["status"], "201 Created", "{bundle}");
    let location = entry["response"]["location"]
        .as_str()
        .expect("created entry has a location");
    let encounter_id = location
        .trim_start_matches("Encounter/")
        .split('/')
        .next()
        .expect("Encounter/<id>/_history/<v>");

    let encounter = server.get(&format!("/Encounter/{encounter_id}")).await;
    assert_eq!(encounter.status_code(), StatusCode::OK);
    assert_eq!(
        encounter.json::<Value>()["serviceProvider"]["reference"],
        format!("Organization/{org_id}"),
        "the conditional reference is rewritten to the resolved resource"
    );
}

/// A reference that genuinely matches nothing is still rejected, naming the
/// reference — the behaviour the issue's acceptance criteria keep.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transaction_still_rejects_a_conditional_reference_that_matches_nothing() {
    let server = server();

    let created = server
        .post("/Organization")
        .json(&json!({
            "resourceType": "Organization",
            "identifier": [{"system": PROBE_SYSTEM, "value": "ORG-PROBE-1047"}],
            "name": "ZZZ Probe Org"
        }))
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);

    let reference = format!("Organization?identifier={PROBE_SYSTEM}|ORG-NOBODY");
    let response = server
        .post("/")
        .json(&transaction_with_encounter_at(&reference))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "{}",
        response.text()
    );
    let outcome = response.json::<Value>();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    let text = outcome["issue"][0]["details"]["text"]
        .as_str()
        .or_else(|| outcome["issue"][0]["diagnostics"].as_str())
        .unwrap_or_default();
    assert!(
        text.contains(&reference) && text.contains("matches no existing resource"),
        "diagnostic must name the unresolvable reference: {outcome}"
    );

    let encounters = server.get("/Encounter").await;
    assert_eq!(encounters.status_code(), StatusCode::OK);
    assert!(
        encounters.json::<Value>()["entry"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a declined transaction writes nothing"
    );
}
