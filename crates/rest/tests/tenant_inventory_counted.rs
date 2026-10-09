//! Counted backends keep their exact pre-#1913 payloads.
//!
//! `GET /admin/tenants` and `GET /console/metrics/tenants` now read
//! `ResourceStorage::discover_tenants` instead of `count_by_tenant`. On
//! SQLite, PostgreSQL and MongoDB discovery is that same grouped live count,
//! so the responses must not change by a single byte. Each test seeds a
//! registered tenant with data, a registered empty tenant, a data-only tenant
//! and a tenant whose only resource was deleted, then compares the response
//! body byte for byte with an oracle: the pre-#1913 handler logic, copied
//! verbatim, fed from `list_tenants` + `count_by_tenant` on the same store.
//!
//! PostgreSQL and MongoDB run in testcontainers (Docker), like the other
//! backend-specific rest suites; MongoDB honours `HFS_TEST_MONGODB_URL` and
//! skips cleanly when Docker is unavailable.

#![cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{SYSTEM_TENANT, TenantContext, TenantId, TenantPermissions};
use helios_rest::{AppState, ServerConfig};
use serde_json::{Value, json};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// The pre-#1913 `GET /admin/tenants` body, verbatim.
async fn admin_oracle<S: ResourceStorage>(storage: &S) -> Value {
    let registered = storage.list_tenants().await.unwrap();
    let counts: HashMap<String, u64> = storage
        .count_by_tenant()
        .await
        .unwrap()
        .into_iter()
        .collect();
    let is_canonical = |id: &str| TenantId::new(id).is_canonical();

    let mut seen = std::collections::HashSet::new();
    let mut tenants = Vec::new();
    for rec in &registered {
        seen.insert(rec.id.clone());
        tenants.push(json!({
            "id": rec.id,
            "display_name": rec.display_name,
            "created_at": rec.created_at,
            "registered": true,
            "canonical": is_canonical(&rec.id),
            "resources": counts.get(&rec.id).copied().unwrap_or(0),
        }));
    }
    let mut discovered: Vec<(&String, &u64)> = counts
        .iter()
        .filter(|(id, _)| id.as_str() != SYSTEM_TENANT && !seen.contains(id.as_str()))
        .collect();
    discovered.sort_by(|a, b| a.0.cmp(b.0));
    for (id, n) in discovered {
        tenants.push(json!({
            "id": id,
            "display_name": null,
            "created_at": null,
            "registered": false,
            "canonical": is_canonical(id),
            "resources": n,
        }));
    }
    let non_canonical = tenants
        .iter()
        .filter(|t| t["canonical"] == json!(false))
        .count();
    json!({
        "tenant_count": tenants.len(),
        "non_canonical_count": non_canonical,
        "tenants": tenants,
    })
}

/// The pre-#1913 `GET /console/metrics/tenants` body, verbatim, minus the
/// clock: `generated_at` is taken from the response under test. No traffic is
/// recorded in this binary, so the traffic columns are zero on both sides.
async fn console_oracle<S: ResourceStorage>(storage: &S, generated_at: &Value) -> Value {
    let window = 3600;
    let mut resource_counts = storage.count_by_tenant().await.unwrap();
    resource_counts.retain(|(tenant, _)| !TenantId::is_reserved(tenant.as_str()));
    resource_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let traffic = helios_observability::reqlog::per_tenant(window);
    let traffic_by_tenant: HashMap<&str, &helios_observability::reqlog::TenantTraffic> =
        traffic.iter().map(|t| (t.tenant.as_str(), t)).collect();

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut tenants = Vec::new();
    for (tenant, resources) in &resource_counts {
        seen.insert(tenant.as_str());
        let tr = traffic_by_tenant.get(tenant.as_str());
        tenants.push(json!({
            "tenant": tenant,
            "resources": resources,
            "requests_per_second": tr.map(|t| t.requests_per_second).unwrap_or(0.0),
            "p95_ms": tr.map(|t| t.p95_ms).unwrap_or(0.0),
            "error_rate": tr.map(|t| t.error_rate).unwrap_or(0.0),
        }));
    }
    for t in &traffic {
        if !seen.contains(t.tenant.as_str()) {
            tenants.push(json!({
                "tenant": t.tenant,
                "resources": 0,
                "requests_per_second": t.requests_per_second,
                "p95_ms": t.p95_ms,
                "error_rate": t.error_rate,
            }));
        }
    }

    json!({
        "instance": helios_observability::uptime::instance_id(),
        "resources_scope": if storage.is_cluster_shared() { "cluster" } else { "single-instance" },
        "traffic_scope": "single-instance",
        "generated_at": generated_at,
        "window_seconds": window,
        "tenant_count": tenants.len(),
        "tenants": tenants,
    })
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

async fn create_patient<S: ResourceStorage>(storage: &S, tenant_id: &str) -> String {
    storage
        .create(
            &tenant(tenant_id),
            "Patient",
            json!({ "resourceType": "Patient" }),
            FhirVersion::default(),
        )
        .await
        .expect("create Patient")
        .id()
        .to_string()
}

/// Seeds the mixed roster, then compares both endpoints byte for byte with
/// the pre-#1913 oracle and sanity-checks that the counts are real.
async fn assert_counted_payloads_unchanged<S>(storage: Arc<S>)
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let run = uuid::Uuid::new_v4().simple().to_string();
    let reg = format!("reg-{run}");
    let empty = format!("empty-{run}");
    let data = format!("data-{run}");
    let gone = format!("gone-{run}");

    storage
        .register_tenant(&reg, Some("Registered"))
        .await
        .unwrap();
    storage.register_tenant(&empty, None).await.unwrap();
    create_patient(storage.as_ref(), &reg).await;
    create_patient(storage.as_ref(), &reg).await;
    create_patient(storage.as_ref(), &data).await;
    let deleted = create_patient(storage.as_ref(), &gone).await;
    storage
        .delete(&tenant(&gone), "Patient", &deleted)
        .await
        .unwrap();

    let config = ServerConfig {
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };
    let state = AppState::new(Arc::clone(&storage), config);
    let server = TestServer::new(
        helios_rest::routing::admin_tenants::routes(state.clone())
            .merge(helios_rest::routing::console_metrics::admin_routes(state)),
    )
    .expect("test server");

    let admin = server.get("/admin/tenants").await;
    admin.assert_status_ok();
    let expected = serde_json::to_string(&admin_oracle(storage.as_ref()).await).unwrap();
    assert_eq!(admin.text(), expected);

    let console = server.get("/console/metrics/tenants").await;
    console.assert_status_ok();
    let generated_at = console.json::<Value>()["generated_at"].clone();
    let expected =
        serde_json::to_string(&console_oracle(storage.as_ref(), &generated_at).await).unwrap();
    assert_eq!(console.text(), expected);

    // The payloads are not vacuously equal: the counts are there.
    let body = admin.json::<Value>();
    let row = |id: &str| {
        body["tenants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .cloned()
    };
    assert_eq!(row(&reg).unwrap()["resources"], 2);
    assert_eq!(row(&empty).unwrap()["resources"], 0);
    assert_eq!(row(&data).unwrap()["resources"], 1);
    assert!(
        row(&gone).is_none(),
        "a deleted-only tenant has no live data"
    );
    assert!(body.get("resources_evidence").is_none());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_payloads_are_unchanged() {
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir()),
            ..Default::default()
        },
    )
    .expect("SQLite backend");
    backend.init_schema().expect("schema");
    assert_counted_payloads_unchanged(Arc::new(backend)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_payloads_are_unchanged() {
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
    // Not shared: one PostgreSQL test per binary, so the container is dropped
    // (and removed) when this test returns.
    let container = Postgres::default()
        .with_tag("16-alpine")
        .with_label("github.run_id", &run_id)
        .start()
        .await
        .expect("failed to start PostgreSQL container");
    let backend = PostgresBackend::new(PostgresConfig {
        host: container.get_host().await.unwrap().to_string(),
        port: container.get_host_port_ipv4(5432).await.unwrap(),
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(data_dir()),
        ..Default::default()
    })
    .await
    .expect("PostgresBackend");
    backend.init_schema().await.expect("schema");
    assert_counted_payloads_unchanged(Arc::new(backend)).await;
}

#[cfg(feature = "mongodb")]
#[tokio::test]
async fn mongodb_payloads_are_unchanged() {
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::Backend;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;

    // Honour an instance the caller already has, like the other MongoDB suites.
    let external = std::env::var("HFS_TEST_MONGODB_URL")
        .ok()
        .filter(|url| !url.trim().is_empty());
    let mut _container = None;
    let connection_string = match external {
        Some(url) => url,
        None => {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let Ok(container) = Mongo::default()
                .with_label("github.run_id", &run_id)
                .with_startup_timeout(std::time::Duration::from_secs(120))
                .start()
                .await
            else {
                eprintln!("Skipping mongodb_payloads_are_unchanged (requires Docker)");
                return;
            };
            let host = container.get_host().await.unwrap().to_string();
            let port = container.get_host_port_ipv4(27017).await.unwrap();
            _container = Some(container);
            format!("mongodb://{host}:{port}")
        }
    };
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string,
        database_name: format!("inventory_{}", uuid::Uuid::new_v4().simple()),
        data_dir: Some(data_dir()),
        ..Default::default()
    })
    .expect("MongoBackend");
    backend.initialize().await.expect("schema");
    assert_counted_payloads_unchanged(Arc::new(backend)).await;
}
