//! `GET /admin/tenants` and `GET /console/metrics/tenants` on a backend whose
//! tenant discovery reports **presence, not counts** (#1913).
//!
//! On S3, `count_by_tenant` lists every current pointer of every tenant, so on
//! a large store these two admin endpoints timed out. They now read
//! `ResourceStorage::discover_tenants`, whose S3 cost is bounded by tenant
//! groups. This suite drives both endpoints through the real routers against a
//! presence-only test double:
//!
//! - `count_by_tenant` is never called (the double counts the calls and would
//!   leak an impossible number into the response if it were);
//! - presence rows carry `resources: null` plus `has_data`, never a zero or a
//!   pointer count, and the response names its evidence in
//!   `resources_evidence`;
//! - an `Unsupported` backend (S3 bucket-per-tenant) reads as unknown, not
//!   empty;
//! - a budget-limited (`Partial`) discovery is resumed to completion, and one
//!   that cannot be resumed leaves absent tenants unknown.
//!
//! The counted shape on SQLite/PostgreSQL/MongoDB is pinned in
//! `admin_tenants.rs` and `console_metrics.rs`; real S3 is covered by
//! `tenant_inventory_minio.rs`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::core::{
    DiscoveredTenant, DiscoveryCoverage, DiscoveryCursor, DiscoveryRequest, PresenceBasis,
    ResourceStorage, TenantDataEvidence, TenantDiscovery, TenantRecord,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_rest::{AppState, ServerConfig};
use serde_json::Value;

/// Registry plus scripted discovery; every FHIR CRUD method is unused here.
struct PresenceOnly {
    registered: Vec<TenantRecord>,
    /// One answer per `discover_tenants` call, in order. Calls past the end
    /// repeat the last answer.
    pages: Vec<TenantDiscovery>,
    discover_requests: Mutex<Vec<DiscoveryRequest>>,
    count_by_tenant_calls: AtomicUsize,
}

impl PresenceOnly {
    fn new(registered: &[&str], pages: Vec<TenantDiscovery>) -> Arc<Self> {
        Arc::new(Self {
            registered: registered
                .iter()
                .map(|id| TenantRecord {
                    id: (*id).to_string(),
                    display_name: None,
                    created_at: "2026-10-01T00:00:00Z".to_string(),
                })
                .collect(),
            pages,
            discover_requests: Mutex::new(Vec::new()),
            count_by_tenant_calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ResourceStorage for PresenceOnly {
    fn backend_name(&self) -> &'static str {
        "presence-only"
    }

    fn is_cluster_shared(&self) -> bool {
        true
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.count_by_tenant_calls.fetch_add(1, Ordering::SeqCst);
        // An impossible figure: if a handler ever used it, it would show.
        Ok(vec![("acme".to_string(), 999_999)])
    }

    async fn discover_tenants(&self, req: &DiscoveryRequest) -> StorageResult<TenantDiscovery> {
        let mut requests = self.discover_requests.lock().unwrap();
        let call = requests.len();
        requests.push(req.clone());
        Ok(self.pages[call.min(self.pages.len() - 1)].clone())
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<TenantRecord>> {
        Ok(self.registered.clone())
    }

    async fn create(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        unimplemented!()
    }

    async fn create_or_update(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        unimplemented!()
    }

    async fn read(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        unimplemented!()
    }

    async fn update(
        &self,
        _tenant: &TenantContext,
        _current: &StoredResource,
        _resource: Value,
    ) -> StorageResult<StoredResource> {
        unimplemented!()
    }

    async fn delete(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<()> {
        unimplemented!()
    }

    async fn count(
        &self,
        _tenant: &TenantContext,
        _resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        unimplemented!()
    }
}

fn present(id: &str) -> DiscoveredTenant {
    DiscoveredTenant {
        id: id.to_string(),
        evidence: TenantDataEvidence::Present {
            basis: PresenceBasis::ResourceObjects,
        },
    }
}

fn complete(ids: &[&str]) -> TenantDiscovery {
    TenantDiscovery {
        tenants: ids.iter().map(|id| present(id)).collect(),
        coverage: DiscoveryCoverage::Complete,
    }
}

fn partial(ids: &[&str], resume: Option<&str>) -> TenantDiscovery {
    TenantDiscovery {
        tenants: ids.iter().map(|id| present(id)).collect(),
        coverage: DiscoveryCoverage::Partial {
            resume: resume.map(DiscoveryCursor::new),
        },
    }
}

/// Both cross-tenant routers, merged with no auth layer as `build_app` does
/// when auth is disabled.
fn server(storage: Arc<PresenceOnly>) -> TestServer {
    let state = AppState::new(storage, ServerConfig::for_testing());
    let router = helios_rest::routing::admin_tenants::routes(state.clone())
        .merge(helios_rest::routing::console_metrics::admin_routes(state));
    TestServer::new(router).expect("test server")
}

fn row<'a>(body: &'a Value, key: &str, id: &str) -> &'a Value {
    body["tenants"]
        .as_array()
        .expect("tenants array")
        .iter()
        .find(|t| t[key] == id)
        .unwrap_or_else(|| panic!("row {id} missing from {body:#}"))
}

fn ids(body: &Value, key: &str) -> Vec<String> {
    body["tenants"]
        .as_array()
        .expect("tenants array")
        .iter()
        .map(|t| t[key].as_str().unwrap().to_string())
        .collect()
}

// ---- GET /admin/tenants ------------------------------------------------------

#[tokio::test]
async fn admin_list_reports_presence_without_counting() {
    let storage = PresenceOnly::new(
        &["acme", "empty"],
        vec![complete(&["acme", "orphan", "__system__"])],
    );
    let server = server(Arc::clone(&storage));

    let res = server.get("/admin/tenants").await;
    res.assert_status_ok();
    let body = res.json::<Value>();

    assert_eq!(storage.count_by_tenant_calls.load(Ordering::SeqCst), 0);
    assert_eq!(body["resources_evidence"], "presence");
    assert_eq!(body["tenant_count"], 3);
    assert_eq!(body["non_canonical_count"], 0);
    // Registered rows first, then data-only rows; the system tenant never.
    assert_eq!(ids(&body, "id"), ["acme", "empty", "orphan"]);

    let acme = row(&body, "id", "acme");
    assert_eq!(acme["registered"], true);
    assert_eq!(acme["resources"], Value::Null);
    assert_eq!(acme["has_data"], true);

    // Complete presence coverage: a registered tenant that was not found holds
    // no resource objects at all. Still not a number in this response.
    let empty = row(&body, "id", "empty");
    assert_eq!(empty["resources"], Value::Null);
    assert_eq!(empty["has_data"], false);

    // A data-only tenant (e.g. deregistered, or tombstones only) stays visible.
    let orphan = row(&body, "id", "orphan");
    assert_eq!(orphan["registered"], false);
    assert_eq!(orphan["created_at"], Value::Null);
    assert_eq!(orphan["canonical"], true);
    assert_eq!(orphan["resources"], Value::Null);
    assert_eq!(orphan["has_data"], true);
}

/// S3 bucket-per-tenant: discovery is a capability boundary. The registry is
/// still listed, but nothing about data is claimed, and in particular no row
/// reads as empty.
#[tokio::test]
async fn admin_list_on_unsupported_discovery_is_unknown_not_empty() {
    let storage = PresenceOnly::new(
        &["acme"],
        vec![TenantDiscovery::unsupported(
            "s3-bucket-per-tenant-discovery",
        )],
    );
    let server = server(Arc::clone(&storage));

    let body = server.get("/admin/tenants").await.json::<Value>();

    assert_eq!(storage.count_by_tenant_calls.load(Ordering::SeqCst), 0);
    assert_eq!(body["resources_evidence"], "unsupported");
    assert_eq!(body["tenant_count"], 1);
    let acme = row(&body, "id", "acme");
    assert_eq!(acme["resources"], Value::Null);
    assert_eq!(acme["has_data"], Value::Null);
}

#[tokio::test]
async fn admin_list_resumes_a_partial_discovery_to_completion() {
    let storage = PresenceOnly::new(
        &["zulu"],
        vec![
            partial(&["alpha"], Some("alpha/")),
            complete(&["bravo", "alpha"]),
        ],
    );
    let server = server(Arc::clone(&storage));

    let body = server.get("/admin/tenants").await.json::<Value>();

    let requests = storage.discover_requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], DiscoveryRequest::default());
    assert_eq!(
        requests[1].resume.as_ref().map(DiscoveryCursor::as_str),
        Some("alpha/")
    );
    assert_eq!(body["resources_evidence"], "presence");
    // A tenant seen in two slices is listed once.
    assert_eq!(ids(&body, "id"), ["zulu", "alpha", "bravo"]);
    assert_eq!(row(&body, "id", "zulu")["has_data"], false);
}

#[tokio::test]
async fn admin_list_with_unresumable_partial_leaves_absent_tenants_unknown() {
    let storage = PresenceOnly::new(&["zulu"], vec![partial(&["alpha"], None)]);
    let server = server(Arc::clone(&storage));

    let body = server.get("/admin/tenants").await.json::<Value>();

    assert_eq!(storage.discover_requests.lock().unwrap().len(), 1);
    assert_eq!(body["resources_evidence"], "presence");
    assert_eq!(row(&body, "id", "alpha")["has_data"], true);
    // Not found in an incomplete slice: unknown, never "no data".
    assert_eq!(row(&body, "id", "zulu")["has_data"], Value::Null);
}

// ---- GET /console/metrics/tenants ---------------------------------------------

#[tokio::test]
async fn console_tenants_reports_presence_without_counting() {
    // Unique ids: the request log the traffic columns read is process-global.
    let busy = "presence-console-busy";
    let idle = "presence-console-idle";
    let ghost = "presence-console-ghost";
    for _ in 0..3 {
        helios_observability::reqlog::record(200, 0.010, busy);
        helios_observability::reqlog::record(200, 0.010, ghost);
    }
    let storage = PresenceOnly::new(&[], vec![complete(&[idle, busy, "__system__"])]);
    let server = server(Arc::clone(&storage));

    let res = server.get("/console/metrics/tenants").await;
    res.assert_status_ok();
    let body = res.json::<Value>();

    assert_eq!(storage.count_by_tenant_calls.load(Ordering::SeqCst), 0);
    assert_eq!(body["resources_evidence"], "presence");
    assert_eq!(body["resources_scope"], "cluster");
    let listed = ids(&body, "tenant");
    assert!(!listed.iter().any(|t| t == "__system__"), "{listed:?}");
    // Data holders first (by id, there is no count to rank by), then
    // traffic-only tenants.
    let pos = |id: &str| listed.iter().position(|t| t == id).unwrap();
    assert!(pos(busy) < pos(idle), "{listed:?}");
    assert!(pos(idle) < pos(ghost), "{listed:?}");

    let busy_row = row(&body, "tenant", busy);
    assert_eq!(busy_row["resources"], Value::Null);
    assert_eq!(busy_row["has_data"], true);
    assert!(busy_row["requests_per_second"].as_f64().unwrap() > 0.0);

    let idle_row = row(&body, "tenant", idle);
    assert_eq!(idle_row["resources"], Value::Null);
    assert_eq!(idle_row["has_data"], true);
    assert_eq!(idle_row["requests_per_second"], 0.0);

    // Seen only in traffic, and complete discovery did not find it.
    let ghost_row = row(&body, "tenant", ghost);
    assert_eq!(ghost_row["resources"], Value::Null);
    assert_eq!(ghost_row["has_data"], false);
}

#[tokio::test]
async fn console_tenants_on_unsupported_discovery_is_unknown_not_empty() {
    let ghost = "presence-console-unsupported-ghost";
    helios_observability::reqlog::record(200, 0.010, ghost);
    let storage = PresenceOnly::new(
        &["acme"],
        vec![TenantDiscovery::unsupported(
            "s3-bucket-per-tenant-discovery",
        )],
    );
    let server = server(Arc::clone(&storage));

    let body = server.get("/console/metrics/tenants").await.json::<Value>();

    assert_eq!(storage.count_by_tenant_calls.load(Ordering::SeqCst), 0);
    assert_eq!(body["resources_evidence"], "unsupported");
    let ghost_row = row(&body, "tenant", ghost);
    assert_eq!(ghost_row["resources"], Value::Null);
    assert_eq!(ghost_row["has_data"], Value::Null);
}
