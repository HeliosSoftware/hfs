//! The Tenants page over the stores HFS actually mounts it on (#1849).
//!
//! On a search composite (`*-elasticsearch`) the UI holds the composite, so a
//! purge fans out to the search index; its cross-tenant reads must still come
//! from the primary alone. And every mounted router keeps its own inventory:
//! two stores never share counts, however their backends are named.
//!
//! The search secondary here is a double that would answer every
//! cross-tenant read with a tenant only it knows, so an answer that came from
//! it shows on the page. Waits poll the count status the way the page does;
//! nothing here measures time.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{
    BackendKind, CountBasis, DiscoveryRequest, ResourceStorage, TenantDiscovery, TenantRecord,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[path = "support/gated_storage.rs"]
mod gated_storage;
#[path = "support/html.rs"]
mod html;

use gated_storage::GatedStorage;
use html::Dom;

/// A search secondary that records every call it gets and would answer the
/// cross-tenant reads with `from-the-index`, a tenant no primary holds.
#[derive(Default)]
struct IndexDouble {
    calls: Mutex<Vec<String>>,
}

impl IndexDouble {
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ResourceStorage for IndexDouble {
    fn backend_name(&self) -> &'static str {
        "index-double"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.record("create");
        Ok(StoredResource::new(
            resource_type,
            "x",
            tenant.tenant_id().clone(),
            resource,
            fhir_version,
        ))
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.record("create_or_update");
        Ok((
            StoredResource::new(
                resource_type,
                id,
                tenant.tenant_id().clone(),
                resource,
                fhir_version,
            ),
            true,
        ))
    }

    async fn read(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        Ok(None)
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        Ok(StoredResource::new(
            current.resource_type(),
            current.id(),
            tenant.tenant_id().clone(),
            resource,
            current.fhir_version(),
        ))
    }

    async fn delete(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
    ) -> StorageResult<()> {
        Ok(())
    }

    async fn count(
        &self,
        _tenant: &TenantContext,
        _resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        Ok(99)
    }

    fn supports_type_counts(&self) -> bool {
        true
    }

    fn type_count_basis(&self) -> Option<CountBasis> {
        Some(CountBasis::IndexedLiveDocuments)
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.record("count_by_tenant");
        Ok(vec![("from-the-index".to_string(), 99)])
    }

    async fn discover_tenants(&self, _req: &DiscoveryRequest) -> StorageResult<TenantDiscovery> {
        self.record("discover_tenants");
        Ok(TenantDiscovery::from_grouped_counts(
            vec![("from-the-index".to_string(), 99)],
            CountBasis::IndexedLiveDocuments,
        ))
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<TenantRecord>> {
        self.record("list_tenants");
        Ok(vec![TenantRecord {
            id: "from-the-index".to_string(),
            display_name: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }])
    }

    async fn get_tenant(&self, _id: &str) -> StorageResult<Option<TenantRecord>> {
        self.record("get_tenant");
        Ok(None)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        self.record(format!("purge {id}"));
        Ok(99)
    }
}

fn sqlite() -> Arc<SqliteBackend> {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

/// SQLite primary with `index` as its Elasticsearch-role search secondary.
fn composite(primary: &Arc<SqliteBackend>, index: &Arc<IndexDouble>) -> Arc<CompositeStorage> {
    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()
        .expect("composite config");
    let mut backends = std::collections::HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("es".to_string(), index.clone() as DynStorage);
    Arc::new(CompositeStorage::new(config, backends).expect("composite"))
}

fn app(store: Arc<dyn ResourceStorage>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch::default(),
        Some(store),
        None,
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty()),
        FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn send(router: &Router, request: Request<Body>) -> (StatusCode, String) {
    let res = router.clone().oneshot(request).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn get(router: &Router, uri: &str) -> (StatusCode, String) {
    send(router, Request::get(uri).body(Body::empty()).unwrap()).await
}

fn counts_state(html: &str) -> String {
    Dom::page(html)
        .one("#tenant-counts-status [data-counts-state]")
        .attr("data-counts-state")
        .expect("the status carries its state")
        .to_string()
}

/// Polls the rows fragment until the counts settle; panics after ~10s.
async fn wait_counts_settled(router: &Router) -> String {
    for _ in 0..200 {
        let (status, html) = get(router, "/ui/tenants/rows").await;
        assert_eq!(status, StatusCode::OK);
        if !matches!(counts_state(&html).as_str(), "pending" | "refreshing") {
            return html;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the counts did not settle in time");
}

fn row_ids(html: &str) -> Vec<String> {
    Dom::page(html)
        .all(".tenant-id__slug")
        .iter()
        .map(|slug| slug.text())
        .collect()
}

fn cell_state(html: &str, id: &str) -> Option<String> {
    Dom::page(html)
        .all("tbody tr")
        .into_iter()
        .find(|row| row.all(".tenant-id__slug").iter().any(|s| s.text() == id))
        .map(|row| {
            row.one("td.col-num")
                .attr("data-count-state")
                .unwrap_or("none")
                .to_string()
        })
}

async fn seed_patient(store: &dyn ResourceStorage, tenant: &str) {
    store
        .create(
            &TenantContext::new(TenantId::new(tenant), TenantPermissions::full_access()),
            "Patient",
            serde_json::json!({"resourceType": "Patient"}),
            FhirVersion::R4,
        )
        .await
        .expect("seed a patient");
}

/// Over a search composite the roster, the counts and the data-only tenants
/// are the primary's: the index, which would claim a tenant of its own, is
/// never asked for any of them. Purging a data-only tenant from the page
/// reaches the index too, so its search documents go with it.
#[tokio::test]
async fn the_page_over_a_search_composite_reads_the_primary_and_purges_both() {
    let primary = sqlite();
    let index = Arc::new(IndexDouble::default());
    primary.register_tenant("acme", None).await.unwrap();
    primary.register_tenant("empty", None).await.unwrap();
    seed_patient(primary.as_ref(), "acme").await;
    seed_patient(primary.as_ref(), "northwind").await;
    let router = app(composite(&primary, &index));

    let settled = wait_counts_settled(&router).await;
    assert_eq!(counts_state(&settled), "ready");
    assert_eq!(row_ids(&settled), ["acme", "empty", "northwind"]);
    assert_eq!(cell_state(&settled, "acme").as_deref(), Some("number"));
    assert_eq!(cell_state(&settled, "empty").as_deref(), Some("zero"));
    assert_eq!(cell_state(&settled, "northwind").as_deref(), Some("number"));
    let (_, page) = get(&router, "/ui/tenants").await;
    assert!(!page.contains("from-the-index"), "{page}");
    assert!(
        index.calls().is_empty(),
        "the index answered a tenant read: {:?}",
        index.calls()
    );

    let (status, purged) = send(
        &router,
        Request::delete("/ui/tenants/northwind?purge=true")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!row_ids(&purged).contains(&"northwind".to_string()));
    assert_eq!(index.calls(), ["purge northwind"]);
    let after = wait_counts_settled(&router).await;
    assert_eq!(row_ids(&after), ["acme", "empty"]);
    assert_eq!(index.calls(), ["purge northwind"], "and nothing else");
}

/// Two mounted routers over two stores keep their own inventories: each
/// store is scanned once for its own page, never for the other's, and
/// neither page shows the other's tenants.
#[tokio::test]
async fn two_mounts_keep_their_own_inventories() {
    let (first, second) = (sqlite(), sqlite());
    seed_patient(first.as_ref(), "only-in-first").await;
    seed_patient(second.as_ref(), "only-in-second").await;
    let first_gate = GatedStorage::new(first.clone() as Arc<dyn ResourceStorage>);
    let second_gate = GatedStorage::new(second.clone() as Arc<dyn ResourceStorage>);
    let first_app = app(first_gate.clone() as Arc<dyn ResourceStorage>);
    let second_app = app(second_gate.clone() as Arc<dyn ResourceStorage>);

    let first_rows = wait_counts_settled(&first_app).await;
    assert_eq!(row_ids(&first_rows), ["only-in-first"]);
    assert_eq!(first_gate.discover_calls(), 1);
    assert_eq!(second_gate.discover_calls(), 0, "the other mount is idle");

    let second_rows = wait_counts_settled(&second_app).await;
    assert_eq!(row_ids(&second_rows), ["only-in-second"]);
    assert_eq!(second_gate.discover_calls(), 1);

    // Both warm: further visits to either reuse their own result.
    for router in [&first_app, &second_app] {
        let (_, page) = get(router, "/ui/tenants").await;
        assert_eq!(counts_state(&page), "ready");
    }
    assert_eq!(first_gate.discover_calls(), 1);
    assert_eq!(second_gate.discover_calls(), 1);
}
