//! Integration tests for the tenant-maintenance admin API
//! (`GET`/`POST`/`DELETE /admin/tenants`).
//!
//! Exercises the full REST stack against an in-memory SQLite-backed test server
//! with auth disabled (the default), so the admin tier is reachable without a
//! system-context token. The harness mirrors `console_metrics.rs`: it merges the
//! FHIR routes with the admin-tenant router exactly as `build_app` does on the
//! auth-disabled path.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const TENANT_HEADER: HeaderName = HeaderName::from_static("x-tenant-id");

async fn create_test_server() -> TestServer {
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
        default_tenant: "default-tenant".to_string(),
        // These tests assert exact per-tenant resource counts, so don't let
        // provisioning seed the conformance set on top.
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone())
        .merge(helios_rest::routing::admin_tenants::routes(state));
    TestServer::new(router).expect("Failed to create test server")
}

/// Seeds a resource for a specific tenant via the normal REST create path.
async fn seed_for(server: &TestServer, tenant: &str, resource_type: &str) {
    let response = server
        .post(&format!("/{resource_type}"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .add_header(TENANT_HEADER, HeaderValue::from_str(tenant).unwrap())
        .json(&json!({ "resourceType": resource_type }))
        .await;
    response.assert_status(StatusCode::CREATED);
}

fn tenants(body: &Value) -> &Vec<Value> {
    body["tenants"].as_array().expect("tenants array")
}

fn find<'a>(body: &'a Value, id: &str) -> Option<&'a Value> {
    tenants(body).iter().find(|t| t["id"] == id)
}

#[tokio::test]
async fn list_is_empty_before_any_tenant() {
    let server = create_test_server().await;
    let res = server.get("/admin/tenants").await;
    res.assert_status(StatusCode::OK);
    let body = res.json::<Value>();
    assert_eq!(body["tenant_count"], 0);
    assert!(tenants(&body).is_empty());
}

#[tokio::test]
async fn create_then_list_round_trips() {
    let server = create_test_server().await;

    let created = server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme", "display_name": "Acme Health" }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let rec = created.json::<Value>();
    assert_eq!(rec["id"], "acme");
    assert_eq!(rec["display_name"], "Acme Health");
    assert_eq!(rec["registered"], true);
    assert!(rec["created_at"].as_str().is_some_and(|s| !s.is_empty()));

    let list = server.get("/admin/tenants").await.json::<Value>();
    assert_eq!(list["tenant_count"], 1);
    let acme = find(&list, "acme").expect("acme present");
    assert_eq!(acme["display_name"], "Acme Health");
    assert_eq!(acme["resources"], 0);
}

/// The listing reports whether each id satisfies the canonical validator, so an
/// operator can find tenants stranded by the tightening in issue #385.
///
/// Only the `true` side is reachable here, and that is the point: since this
/// change, neither the admin API nor `ResourceStorage::register_tenant` will
/// mint a non-canonical id, and a resource cannot be written under one either
/// (the tenant header is validated). A `canonical: false` row can therefore only
/// come from data that predates the validator — which a fresh test database has
/// none of. The `false` side is covered where it can be constructed, against the
/// unchecked constructor: `TenantId::is_canonical`'s unit tests in
/// `helios-persistence`.
#[tokio::test]
async fn listing_reports_canonicality_so_legacy_ids_can_be_found() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme/research" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "beta", "Patient").await;

    let list = server.get("/admin/tenants").await.json::<Value>();

    // Registered and data-discovered rows both carry the flag.
    assert_eq!(
        find(&list, "acme/research").expect("acme")["canonical"],
        true
    );
    assert_eq!(find(&list, "beta").expect("beta")["canonical"], true);
    // And the summary an operator actually reads.
    assert_eq!(list["non_canonical_count"], 0);
}

#[tokio::test]
async fn duplicate_create_conflicts() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn invalid_ids_are_rejected() {
    let server = create_test_server().await;
    for bad in ["", "has space", "__system__", "bad*char"] {
        let res = server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST);
    }
}

/// Ids naming a control-plane namespace are refused at creation (issue #271).
///
/// Still defence in depth rather than what makes the S3 keyspace safe — that is
/// structural — but it is no longer admin-API-only. Since issue #385 the same
/// rule is applied by `TenantId::parse` at every ingress, including the JWT
/// claim, and again by `ResourceStorage::register_tenant` on all backends.
#[tokio::test]
async fn control_plane_namespace_ids_are_reserved() {
    let server = create_test_server().await;
    for reserved in ["tenants", "resources", "history", "bulk"] {
        let res = server
            .post("/admin/tenants")
            .json(&json!({ "id": reserved }))
            .await;
        res.assert_status(StatusCode::BAD_REQUEST);
    }
}

/// A reserved name is reserved in **every** segment, not just as the whole id
/// (issue #385).
///
/// `acme/resources` passed the old whole-id check. On S3 its objects land under
/// `acme/resources/…`, which is inside the prefix tenant `acme` lists — and that
/// `purge_tenant_data("acme")` deletes. Two tenants, one keyspace, with the
/// deletion crossing the boundary.
#[tokio::test]
async fn reserved_names_are_rejected_in_any_hierarchy_segment() {
    let server = create_test_server().await;
    for bad in [
        "acme/resources",
        "acme/history",
        "acme/bulk",
        "acme/tenants",
        "acme/__system__",
        "acme/../evil",
    ] {
        server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }

    // Only whole segments are reserved — a substring must stay usable.
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme/resources-archive" }))
        .await
        .assert_status(StatusCode::CREATED);
}

/// The length cap is the canonical 64, not the 128 this handler used to apply
/// on its own (issue #385).
///
/// An id in 65..=128 bytes was registrable but unroutable: both the header and
/// URL-prefix validators capped at 64, so it could only ever be reached through
/// the JWT claim — the ingress that validated nothing at all.
#[tokio::test]
async fn ids_longer_than_the_canonical_cap_are_rejected() {
    let server = create_test_server().await;

    server
        .post("/admin/tenants")
        .json(&json!({ "id": "a".repeat(64) }))
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post("/admin/tenants")
        .json(&json!({ "id": "a".repeat(65) }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

/// Malformed hierarchy is rejected: `/` is a separator, so it cannot lead,
/// trail, or repeat.
#[tokio::test]
async fn malformed_hierarchy_is_rejected() {
    let server = create_test_server().await;
    for bad in ["/acme", "acme/", "acme//research"] {
        server
            .post("/admin/tenants")
            .json(&json!({ "id": bad }))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// The rejection body names the reason, so an operator can act on it without
/// reading the source.
#[tokio::test]
async fn rejection_explains_why() {
    let server = create_test_server().await;
    let body = server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme corp" }))
        .await
        .text();
    assert!(
        body.contains("letters, digits"),
        "expected an actionable reason, got: {body}"
    );
}

/// Hierarchical ids stay valid — `TenantId` models `/` as the hierarchy
/// separator (`is_descendant_of`, `parent`, `ancestors`), so tightening the
/// charset here would contradict a shipped feature of the domain type.
///
/// This used to cite `SharedSchemaStrategy` as the second justification; that
/// type was unwired scaffolding and was removed in #370, so the domain model is
/// now the whole reason. Note what the removed code got wrong, because any
/// future per-tenant schema/bucket/database naming scheme must not repeat it:
/// its tenant→identifier mapping was **not injective** — it folded `/` and `-`
/// to `_` and lowercased, so `acme/research` and `acme_research` (and `Acme`
/// and `acme`) collided onto one identifier. Two distinct tenants sharing one
/// container is a cross-tenant data breach. Any such mapping must be injective
/// (percent-encode, as `S3Keyspace::registry_object_id` does) and must not use
/// `DefaultHasher`, whose output is not stable across Rust releases.
#[tokio::test]
async fn hierarchical_ids_are_still_accepted() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme/research" }))
        .await
        .assert_status(StatusCode::CREATED);
}

#[tokio::test]
async fn list_includes_data_only_tenants_with_counts() {
    let server = create_test_server().await;
    // A tenant that has data but was never registered.
    seed_for(&server, "beta", "Patient").await;
    seed_for(&server, "beta", "Observation").await;
    // A registered tenant with data.
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;

    let list = server.get("/admin/tenants").await.json::<Value>();

    let acme = find(&list, "acme").expect("acme");
    assert_eq!(acme["registered"], true);
    assert_eq!(acme["resources"], 1);
    assert!(acme["created_at"].as_str().is_some());

    let beta = find(&list, "beta").expect("beta discovered from data");
    assert_eq!(beta["registered"], false);
    assert_eq!(beta["resources"], 2);
    assert!(beta["created_at"].is_null());
}

#[tokio::test]
async fn delete_deregisters_without_purge_by_default() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;

    let del = server.delete("/admin/tenants/acme").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["deregistered"], true);
    assert_eq!(body["purged"], false);
    assert!(body["resources_removed"].is_null());

    // Deregistered, but the data survives, so it now shows as data-only.
    let list = server.get("/admin/tenants").await.json::<Value>();
    let acme = find(&list, "acme").expect("still discoverable via data");
    assert_eq!(acme["registered"], false);
    assert_eq!(acme["resources"], 1);
}

#[tokio::test]
async fn delete_with_purge_tears_down_data() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;
    seed_for(&server, "acme", "Patient").await;

    let del = server.delete("/admin/tenants/acme?purge=true").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["purged"], true);
    assert_eq!(body["resources_removed"], 2);

    // Gone entirely: no registration and no data.
    let list = server.get("/admin/tenants").await.json::<Value>();
    assert!(find(&list, "acme").is_none());
}

#[tokio::test]
async fn delete_unknown_tenant_is_404() {
    let server = create_test_server().await;
    server
        .delete("/admin/tenants/ghost")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ── #317: the internal system tenant is not addressable from a request ───────

/// Builds the same server as [`create_test_server`] but also hands back the
/// backend, so a test can seed and re-read system-tenant data directly — the
/// REST create path can no longer be used for that, which is the point.
async fn create_test_server_with_backend() -> (TestServer, Arc<SqliteBackend>) {
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
        default_tenant: "default-tenant".to_string(),
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone())
        .merge(helios_rest::routing::admin_tenants::routes(state));
    (
        TestServer::new(router).expect("Failed to create test server"),
        backend,
    )
}

/// Writes a resource into the system tenant the way the audit sink does — in
/// process, with a `TenantContext::system()` — and returns its id.
async fn seed_system_tenant_resource(backend: &Arc<SqliteBackend>) -> String {
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::TenantContext;

    let stored = backend
        .create(
            &TenantContext::system(),
            "AuditEvent",
            json!({ "resourceType": "AuditEvent" }),
            helios_fhir::FhirVersion::R4,
        )
        .await
        .expect("seed system-tenant AuditEvent");
    stored.id().to_string()
}

async fn system_tenant_resource_count(backend: &Arc<SqliteBackend>) -> u64 {
    use helios_persistence::core::ResourceStorage;
    backend
        .count_by_tenant()
        .await
        .expect("count_by_tenant")
        .into_iter()
        .find(|(t, _)| t == helios_persistence::tenant::SYSTEM_TENANT)
        .map(|(_, n)| n)
        .unwrap_or(0)
}

/// The header door. With auth disabled — the default deployment shape — this
/// needed no credentials at all, and returned the cross-tenant AuditEvent trail.
#[tokio::test]
async fn system_tenant_is_not_readable_via_header() {
    let (server, backend) = create_test_server_with_backend().await;
    seed_system_tenant_resource(&backend).await;

    let res = server
        .get("/AuditEvent")
        .add_header(TENANT_HEADER, HeaderValue::from_static("__system__"))
        .await;

    res.assert_status(StatusCode::FORBIDDEN);
    // The data consequence, not just the status: nothing from the shared tenant
    // came back.
    assert!(
        !res.text().contains("AuditEvent"),
        "system-tenant resources must not appear in the response body"
    );
}

/// The destructive half. `delete_tenant_handler` never validated its path id, so
/// this deregistered *and* purged the shared tenant — destroying the audit trail
/// that records the attack.
#[tokio::test]
async fn deleting_the_system_tenant_is_refused_and_purges_nothing() {
    let (server, backend) = create_test_server_with_backend().await;
    let seeded_id = seed_system_tenant_resource(&backend).await;
    let before = system_tenant_resource_count(&backend).await;
    assert!(before > 0, "precondition: system tenant has data");

    // 400, not 404 — validation must run before the existence probe, or a
    // reserved id with data would sail past it into the purge.
    server
        .delete("/admin/tenants/__system__?purge=true")
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Nothing was destroyed.
    assert_eq!(
        system_tenant_resource_count(&backend).await,
        before,
        "a refused delete must not purge any system-tenant data"
    );
    use helios_persistence::core::ResourceStorage;
    // `read` returns `Ok(None)` for a purged resource, so assert on the payload,
    // not merely on `is_ok()`.
    let survivor = backend
        .read(
            &helios_persistence::tenant::TenantContext::system(),
            "AuditEvent",
            &seeded_id,
        )
        .await
        .expect("read must not error");
    assert!(
        survivor.is_some(),
        "the seeded system-tenant resource must still be readable"
    );
}

/// The other reserved ids take the same door.
#[tokio::test]
async fn deleting_a_control_plane_namespace_tenant_is_refused() {
    let server = create_test_server().await;
    for reserved in ["tenants", "resources", "history", "bulk"] {
        server
            .delete(&format!("/admin/tenants/{reserved}?purge=true"))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// The reservation is exact, not a `__` namespace ban: tenant id is a partition
/// key with no rename, so a deployment already holding `__legacy` must keep
/// being able to route to it and, crucially, to delete it.
#[tokio::test]
async fn underscore_prefixed_tenants_remain_usable() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "__legacy" }))
        .await
        .assert_status(StatusCode::CREATED);
    server
        .delete("/admin/tenants/__legacy")
        .await
        .assert_status(StatusCode::OK);
}

// ── #1078: deleting a tenant reaches the write observers ─────────────────────

/// Records the tenant-level write events a delete reports, as
/// `"erased:<tenant>"` / `"removed:<tenant>"`.
#[derive(Default)]
struct TenantEventRecorder {
    events: std::sync::Mutex<Vec<String>>,
}

impl helios_persistence::core::WriteObserver for TenantEventRecorder {
    fn on_write(&self, event: &helios_persistence::core::WriteEvent) {
        use helios_persistence::core::{ErasedScope, WriteEvent};
        let label = match event {
            WriteEvent::Erased {
                tenant,
                scope: ErasedScope::Tenant,
            } => format!("erased:{}", tenant.as_str()),
            WriteEvent::TenantRemoved { tenant } => format!("removed:{}", tenant.as_str()),
            _ => return,
        };
        self.events.lock().unwrap().push(label);
    }
}

/// [`create_test_server`] with the write observers exposed: the recorder and
/// the dashboard counters they feed.
async fn create_observed_test_server() -> (
    TestServer,
    Arc<TenantEventRecorder>,
    Arc<helios_observability::dashboard_counters::DashboardCounters>,
) {
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
        default_tenant: "default-tenant".to_string(),
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };

    let observability = helios_rest::WriteObservability::new();
    let recorder = Arc::new(TenantEventRecorder::default());
    observability
        .observers
        .subscribe(Arc::clone(&recorder) as Arc<dyn helios_persistence::core::WriteObserver>);
    let counters = Arc::clone(&observability.counters);
    let state = helios_rest::AppState::new(Arc::clone(&backend), config)
        .with_write_observability(observability);
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone())
        .merge(helios_rest::routing::admin_tenants::routes(state));
    (
        TestServer::new(router).expect("Failed to create test server"),
        recorder,
        counters,
    )
}

/// Deregistering a tenant reports `TenantRemoved`, which drops the tenant's
/// dashboard counters (#1078); the data itself survives without `purge`.
#[tokio::test]
async fn delete_reports_tenant_removed_and_drops_its_dashboard_counters() {
    let (server, recorder, counters) = create_observed_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;
    assert!(counters.tenants().contains(&"acme".to_string()));

    server
        .delete("/admin/tenants/acme")
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(*recorder.events.lock().unwrap(), vec!["removed:acme"]);
    assert!(!counters.tenants().contains(&"acme".to_string()));
}

/// With `purge`, the tenant's data erasure is reported first, then its
/// removal (#1078).
#[tokio::test]
async fn delete_with_purge_reports_erased_then_tenant_removed() {
    let (server, recorder, counters) = create_observed_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "beta" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "beta", "Patient").await;

    server
        .delete("/admin/tenants/beta?purge=true")
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        *recorder.events.lock().unwrap(),
        vec!["erased:beta", "removed:beta"]
    );
    assert!(!counters.tenants().contains(&"beta".to_string()));
}

/// The counted shape is pinned field-for-field (#1913).
///
/// `GET /admin/tenants` now reads `discover_tenants`, which on SQLite,
/// PostgreSQL and MongoDB is the same grouped live-resource count it used to
/// read through `count_by_tenant`. These backends must keep returning exactly
/// the pre-#1913 payload: a numeric `resources` on every row and none of the
/// presence-only fields (`resources_evidence`, `has_data`) that S3 adds.
#[tokio::test]
async fn counted_backends_keep_the_numeric_payload_unchanged() {
    let server = create_test_server().await;
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme", "display_name": "Acme Health" }))
        .await
        .assert_status(StatusCode::CREATED);
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "empty" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;
    seed_for(&server, "beta", "Patient").await;
    seed_for(&server, "beta", "Observation").await;

    let body = server.get("/admin/tenants").await.json::<Value>();
    let created = |id: &str| find(&body, id).expect("registered row")["created_at"].clone();

    assert_eq!(
        body,
        json!({
            "tenant_count": 3,
            "non_canonical_count": 0,
            "tenants": [
                {
                    "id": "acme",
                    "display_name": "Acme Health",
                    "created_at": created("acme"),
                    "registered": true,
                    "canonical": true,
                    "resources": 1,
                },
                {
                    "id": "empty",
                    "display_name": null,
                    "created_at": created("empty"),
                    "registered": true,
                    "canonical": true,
                    "resources": 0,
                },
                {
                    "id": "beta",
                    "display_name": null,
                    "created_at": null,
                    "registered": false,
                    "canonical": true,
                    "resources": 2,
                },
            ],
        })
    );
}

// ── #1912: DELETE asks only about the tenant being deleted ───────────────────

/// SQLite storage whose cross-tenant reads — `count_by_tenant` and
/// `discover_tenants` — fail and are counted.
///
/// `DELETE /admin/tenants/{id}` used to decide "does this tenant hold data"
/// from the first, so deleting one tenant counted every tenant in the
/// store (#1912). Behind this wrapper, any handler still doing that fails
/// with a 5xx, and `cross_tenant_counts` records the attempt.
///
/// It also counts `purge_tenant_data` calls (`purge_calls`) and can make the
/// next purge fail after it ran (`interrupt_next_purge`), modelling a
/// non-atomic backend (MongoDB, S3, composite) that deleted the resources and
/// then failed on a later step (#2309).
struct NoCrossTenantCount {
    inner: Arc<SqliteBackend>,
    cross_tenant_counts: std::sync::atomic::AtomicUsize,
    purge_calls: std::sync::atomic::AtomicUsize,
    interrupt_next_purge: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl helios_persistence::core::ResourceStorage for NoCrossTenantCount {
    fn backend_name(&self) -> &'static str {
        "no-cross-tenant-count"
    }

    async fn create(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> helios_persistence::error::StorageResult<helios_persistence::types::StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> helios_persistence::error::StorageResult<(helios_persistence::types::StoredResource, bool)>
    {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        resource_type: &str,
        id: &str,
    ) -> helios_persistence::error::StorageResult<Option<helios_persistence::types::StoredResource>>
    {
        self.inner.read(tenant, resource_type, id).await
    }

    async fn update(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        current: &helios_persistence::types::StoredResource,
        resource: Value,
    ) -> helios_persistence::error::StorageResult<helios_persistence::types::StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        resource_type: &str,
        id: &str,
    ) -> helios_persistence::error::StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    async fn count(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        resource_type: Option<&str>,
    ) -> helios_persistence::error::StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }

    async fn count_by_tenant(
        &self,
    ) -> helios_persistence::error::StorageResult<Vec<(String, u64)>> {
        self.cross_tenant_counts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(helios_persistence::error::StorageError::Backend(
            helios_persistence::error::BackendError::Unavailable {
                backend_name: self.backend_name().to_string(),
                message: "the cross-tenant count is off limits to DELETE (#1912)".to_string(),
            },
        ))
    }

    async fn discover_tenants(
        &self,
        _req: &helios_persistence::core::DiscoveryRequest,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::TenantDiscovery> {
        // The listing's cross-tenant read (#1913) is off limits to DELETE too.
        self.cross_tenant_counts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(helios_persistence::error::StorageError::Backend(
            helios_persistence::error::BackendError::Unavailable {
                backend_name: self.backend_name().to_string(),
                message: "cross-tenant discovery is off limits to DELETE (#1912)".to_string(),
            },
        ))
    }

    async fn tenant_has_resources(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
    ) -> helios_persistence::error::StorageResult<bool> {
        self.inner.tenant_has_resources(tenant).await
    }

    fn supports_tenant_registry(&self) -> bool {
        self.inner.supports_tenant_registry()
    }

    async fn list_tenants(
        &self,
    ) -> helios_persistence::error::StorageResult<Vec<helios_persistence::core::TenantRecord>> {
        self.inner.list_tenants().await
    }

    async fn get_tenant(
        &self,
        id: &str,
    ) -> helios_persistence::error::StorageResult<Option<helios_persistence::core::TenantRecord>>
    {
        self.inner.get_tenant(id).await
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::TenantRecord> {
        self.inner.register_tenant(id, display_name).await
    }

    async fn deregister_tenant(&self, id: &str) -> helios_persistence::error::StorageResult<bool> {
        self.inner.deregister_tenant(id).await
    }

    async fn purge_tenant_data(&self, id: &str) -> helios_persistence::error::StorageResult<u64> {
        use std::sync::atomic::Ordering::SeqCst;
        self.purge_calls.fetch_add(1, SeqCst);
        let removed = self.inner.purge_tenant_data(id).await?;
        if self.interrupt_next_purge.swap(false, SeqCst) {
            return Err(helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::Internal {
                    backend_name: self.backend_name().to_string(),
                    message: "purge interrupted after the resources were deleted (#2309)"
                        .to_string(),
                    source: None,
                },
            ));
        }
        Ok(removed)
    }
}

/// The admin-tenant router over [`NoCrossTenantCount`], with the storage
/// handed back so a test can seed tenants and read the call counter.
fn create_scoped_delete_server() -> (TestServer, Arc<NoCrossTenantCount>) {
    let backend = SqliteBackend::in_memory().expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let storage = Arc::new(NoCrossTenantCount {
        inner: Arc::new(backend),
        cross_tenant_counts: std::sync::atomic::AtomicUsize::new(0),
        purge_calls: std::sync::atomic::AtomicUsize::new(0),
        interrupt_next_purge: std::sync::atomic::AtomicBool::new(false),
    });
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default-tenant".to_string(),
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&storage), config);
    let router = helios_rest::routing::admin_tenants::routes(state);
    (
        TestServer::new(router).expect("Failed to create test server"),
        storage,
    )
}

/// Writes `Patient/{id}` into `tenant` straight through storage.
async fn seed_patient(storage: &NoCrossTenantCount, tenant: &str, id: &str) {
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    storage
        .create(
            &TenantContext::new(TenantId::new(tenant), TenantPermissions::full_access()),
            "Patient",
            json!({ "resourceType": "Patient", "id": id }),
            helios_fhir::FhirVersion::R4,
        )
        .await
        .expect("seed patient");
}

fn cross_tenant_counts(storage: &NoCrossTenantCount) -> usize {
    storage
        .cross_tenant_counts
        .load(std::sync::atomic::Ordering::SeqCst)
}

fn purge_calls(storage: &NoCrossTenantCount) -> usize {
    storage
        .purge_calls
        .load(std::sync::atomic::Ordering::SeqCst)
}

/// The data branch: a tenant with data but no registration is deleted (and
/// purged) without the cross-tenant count, and is a 404 once purged.
#[tokio::test]
async fn delete_of_a_data_only_tenant_uses_no_cross_tenant_count() {
    let (server, storage) = create_scoped_delete_server();
    seed_patient(&storage, "beta", "p1").await;
    seed_patient(&storage, "beta", "p2").await;
    // Other tenants' data must not count as beta's, nor be touched.
    seed_patient(&storage, "beta-x", "p1").await;

    let del = server.delete("/admin/tenants/beta").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["id"], "beta");
    assert_eq!(body["deregistered"], false);
    assert_eq!(body["purged"], false);
    assert!(body["resources_removed"].is_null());

    // Still has data, so a purge is still possible.
    let purge = server.delete("/admin/tenants/beta?purge=true").await;
    purge.assert_status(StatusCode::OK);
    let body = purge.json::<Value>();
    assert_eq!(body["deregistered"], false);
    assert_eq!(body["purged"], true);
    assert_eq!(body["resources_removed"], 2);

    // Neither registered nor holding data any more.
    server
        .delete("/admin/tenants/beta")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    // The look-alike tenant kept its data.
    server
        .delete("/admin/tenants/beta-x")
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(cross_tenant_counts(&storage), 0);
}

/// The no-data branches: a registered empty tenant is deregistered, while an
/// unregistered tenant holding nothing — never seen, or only delete tombstones
/// on SQLite, whose discovery counts live resources — is a 404. These are the
/// no-purge 404 branches: with `?purge=true` the same ids answer 200 (see the
/// #2309 tests below).
#[tokio::test]
async fn delete_without_data_uses_no_cross_tenant_count() {
    let (server, storage) = create_scoped_delete_server();
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);

    let del = server.delete("/admin/tenants/acme").await;
    del.assert_status(StatusCode::OK);
    let body = del.json::<Value>();
    assert_eq!(body["deregistered"], true);
    assert_eq!(body["purged"], false);
    server
        .delete("/admin/tenants/acme")
        .await
        .assert_status(StatusCode::NOT_FOUND);

    seed_patient(&storage, "tomb", "p1").await;
    {
        use helios_persistence::core::ResourceStorage;
        use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
        storage
            .delete(
                &TenantContext::new(TenantId::new("tomb"), TenantPermissions::full_access()),
                "Patient",
                "p1",
            )
            .await
            .expect("delete Patient/p1");
    }
    server
        .delete("/admin/tenants/tomb")
        .await
        .assert_status(StatusCode::NOT_FOUND);

    server
        .delete("/admin/tenants/ghost")
        .await
        .assert_status(StatusCode::NOT_FOUND);

    assert_eq!(cross_tenant_counts(&storage), 0);
}

// ── #2309: a purge never answers 404, so a retry finishes it ─────────────────

/// A deregistered tenant holding only delete tombstones and history — the
/// natural leftover on SQLite and PostgreSQL, where `tenant_has_resources`
/// counts live rows only — is purged by `?purge=true` instead of answering 404.
#[tokio::test]
async fn purge_reaches_a_deregistered_tenant_that_holds_only_tombstones() {
    use helios_persistence::core::{InstanceHistoryProvider, ResourceStorage};
    use helios_persistence::error::{ResourceError, StorageError};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    let (server, storage) = create_scoped_delete_server();
    let ctx = TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access());
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_patient(&storage, "acme", "p1").await;
    storage
        .delete(&ctx, "Patient", "p1")
        .await
        .expect("delete Patient/p1");

    // Only a tombstone and its history are left.
    assert!(matches!(
        storage.read(&ctx, "Patient", "p1").await,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
    assert!(
        storage
            .inner
            .history_instance_count(&ctx, "Patient", "p1")
            .await
            .expect("history count")
            > 0
    );

    // The no-purge rule is unchanged: the first call deregisters, the second 404s.
    let del = server.delete("/admin/tenants/acme").await;
    del.assert_status(StatusCode::OK);
    assert_eq!(del.json::<Value>()["deregistered"], true);
    server
        .delete("/admin/tenants/acme")
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // With purge the leftovers are reached.
    let purge = server.delete("/admin/tenants/acme?purge=true").await;
    purge.assert_status(StatusCode::OK);
    assert_eq!(
        purge.json::<Value>(),
        json!({
            "id": "acme",
            "deregistered": false,
            "purged": true,
            "resources_removed": 1,
        })
    );
    assert!(
        storage
            .read(&ctx, "Patient", "p1")
            .await
            .expect("read after purge")
            .is_none()
    );
    assert_eq!(
        storage
            .inner
            .history_instance_count(&ctx, "Patient", "p1")
            .await
            .expect("history count"),
        0
    );
    assert_eq!(purge_calls(&storage), 1);

    // Purging again removes nothing and is still a 200.
    let again = server.delete("/admin/tenants/acme?purge=true").await;
    again.assert_status(StatusCode::OK);
    assert_eq!(again.json::<Value>()["resources_removed"], 0);
    assert_eq!(cross_tenant_counts(&storage), 0);
}

/// The issue's sequence: the purge removes the resources and then fails, the
/// tenant is deregistered with no live resource, and the retry is neither a 404
/// nor skipped.
#[tokio::test]
async fn a_purge_that_failed_after_its_resources_were_deleted_can_be_retried() {
    use std::sync::atomic::Ordering::SeqCst;

    let (server, storage) = create_scoped_delete_server();
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_patient(&storage, "acme", "p1").await;
    seed_patient(&storage, "acme", "p2").await;
    storage.interrupt_next_purge.store(true, SeqCst);

    // The deregister ran and the inner purge removed the rows; then it failed.
    let first = server.delete("/admin/tenants/acme?purge=true").await;
    assert!(
        first.status_code().is_server_error(),
        "the interrupted purge must fail the request, got {}",
        first.status_code()
    );

    // The trap: without purge the tenant looks gone.
    server
        .delete("/admin/tenants/acme")
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // The retry reaches the purge and succeeds. The resources were already
    // deleted, so it reports 0 removed.
    let retry = server.delete("/admin/tenants/acme?purge=true").await;
    retry.assert_status(StatusCode::OK);
    assert_eq!(
        retry.json::<Value>(),
        json!({
            "id": "acme",
            "deregistered": false,
            "purged": true,
            "resources_removed": 0,
        })
    );
    assert_eq!(purge_calls(&storage), 2);
    assert_eq!(cross_tenant_counts(&storage), 0);
}

/// An id that was never seen is purged to nothing: 200, `deregistered: false`,
/// 0 removed. Without `purge` it is still a 404.
#[tokio::test]
async fn purge_of_a_tenant_that_never_existed_is_200_with_nothing_removed() {
    let (server, storage) = create_scoped_delete_server();

    let purge = server.delete("/admin/tenants/ghost?purge=true").await;
    purge.assert_status(StatusCode::OK);
    assert_eq!(
        purge.json::<Value>(),
        json!({
            "id": "ghost",
            "deregistered": false,
            "purged": true,
            "resources_removed": 0,
        })
    );
    assert_eq!(purge_calls(&storage), 1);

    server
        .delete("/admin/tenants/ghost")
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert_eq!(cross_tenant_counts(&storage), 0);
}

/// With the existence probe gone from the purge path, validation is the only
/// REST-layer guard: a reserved id is a 400 and never reaches the purge.
#[tokio::test]
async fn reserved_ids_with_purge_are_400_and_never_reach_the_purge() {
    let (server, storage) = create_scoped_delete_server();
    for reserved in ["__system__", "tenants", "resources", "history", "bulk"] {
        server
            .delete(&format!("/admin/tenants/{reserved}?purge=true"))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
    assert_eq!(purge_calls(&storage), 0);
}

/// Purging an id the server does not know still reports the tenant-level
/// events, in order, and creates no dashboard-counter entry for it: every
/// observer treats an unknown tenant as a no-op.
#[tokio::test]
async fn purge_of_an_unknown_tenant_reports_erased_then_removed() {
    let (server, recorder, counters) = create_observed_test_server().await;

    server
        .delete("/admin/tenants/ghost?purge=true")
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        *recorder.events.lock().unwrap(),
        vec!["erased:ghost", "removed:ghost"]
    );
    assert!(!counters.tenants().contains(&"ghost".to_string()));
}

/// An output store over a real local-fs store whose tenant-wide delete can be
/// made to fail (#2279).
struct FlakyOutputStore {
    inner: helios_persistence::backends::local_fs::LocalFsOutputStore,
    fail_tenant_delete: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl helios_persistence::core::ExportOutputStore for FlakyOutputStore {
    async fn open_writer(
        &self,
        key: &helios_persistence::core::ExportPartKey,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::ExportPartWriter> {
        self.inner.open_writer(key).await
    }
    async fn finalize_part(
        &self,
        key: &helios_persistence::core::ExportPartKey,
        writer: helios_persistence::core::ExportPartWriter,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::FinalizedPart> {
        self.inner.finalize_part(key, writer).await
    }
    async fn download_url(
        &self,
        key: &helios_persistence::core::ExportPartKey,
        ttl: std::time::Duration,
    ) -> helios_persistence::error::StorageResult<helios_persistence::core::DownloadUrl> {
        self.inner.download_url(key, ttl).await
    }
    async fn open_reader(
        &self,
        key: &helios_persistence::core::ExportPartKey,
    ) -> helios_persistence::error::StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>>
    {
        self.inner.open_reader(key).await
    }
    async fn delete_job_outputs(
        &self,
        tenant: &helios_persistence::tenant::TenantContext,
        job_id: &helios_persistence::core::ExportJobId,
    ) -> helios_persistence::error::StorageResult<()> {
        self.inner.delete_job_outputs(tenant, job_id).await
    }
    async fn delete_tenant_outputs(
        &self,
        tenant_id: &str,
    ) -> helios_persistence::error::StorageResult<()> {
        if self
            .fail_tenant_delete
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::Internal {
                    backend_name: "test".to_string(),
                    message: "output store down".to_string(),
                    source: None,
                },
            ));
        }
        self.inner.delete_tenant_outputs(tenant_id).await
    }
}

/// A purge deletes the tenant's bulk output files before its rows (#2279): a
/// failing output store fails the request with every row still in place, and
/// the retry then removes both. Rows-first, the failure would have left files
/// whose job rows were gone, which no expiry sweep can find.
#[tokio::test]
async fn delete_with_purge_removes_output_files_first() {
    use helios_persistence::core::{ExportJobId, ExportOutputStore, ExportPartKey};
    use std::sync::atomic::Ordering;

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
    .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(FlakyOutputStore {
        inner: helios_persistence::backends::local_fs::LocalFsOutputStore::new(
            tmp.path(),
            "http://localhost:8080",
        ),
        fail_tenant_delete: true.into(),
    });
    let key = ExportPartKey::output("acme", ExportJobId::new(), "Patient", 0, 1);
    let mut writer = store.open_writer(&key).await.unwrap();
    writer.write_line("{}").await.unwrap();
    store.finalize_part(&key, writer).await.unwrap();

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default-tenant".to_string(),
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_export(
        backend.clone() as Arc<dyn helios_persistence::core::BulkExportJobStore>,
        store.clone() as Arc<dyn ExportOutputStore>,
        Arc::new(helios_rest::bulk_export_auth::BearerScopeAuth),
    );
    let router = helios_rest::routing::fhir_routes::create_routes(state.clone())
        .merge(helios_rest::routing::admin_tenants::routes(state));
    let server = TestServer::new(router).unwrap();
    server
        .post("/admin/tenants")
        .json(&json!({ "id": "acme" }))
        .await
        .assert_status(StatusCode::CREATED);
    seed_for(&server, "acme", "Patient").await;

    let failed = server.delete("/admin/tenants/acme?purge=true").await;
    assert!(
        !failed.status_code().is_success(),
        "a failed file delete must fail the purge, got {}",
        failed.status_code()
    );
    assert!(tmp.path().join("acme").exists());
    let list = server.get("/admin/tenants").await.json::<Value>();
    assert!(
        find(&list, "acme").is_some(),
        "the rows must survive a failed file delete"
    );

    store.fail_tenant_delete.store(false, Ordering::SeqCst);
    let retried = server.delete("/admin/tenants/acme?purge=true").await;
    retried.assert_status(StatusCode::OK);
    assert_eq!(retried.json::<Value>()["resources_removed"], 1);
    assert!(!tmp.path().join("acme").exists());
}
