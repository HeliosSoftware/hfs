//! CompositeStorage behavior tests for S3 + Elasticsearch pairing.

#![cfg(all(feature = "s3", feature = "elasticsearch"))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::composite::{CompositeConfig, CompositeStorage};
use helios_persistence::core::{BackendKind, ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    Page, PageInfo, SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::{Value, json};

#[derive(Debug)]
struct MockBackend {
    name: &'static str,
    fail_search: bool,
    count_value: u64,
    search_calls: AtomicUsize,
    search_count_calls: AtomicUsize,
}

impl MockBackend {
    fn new(name: &'static str, fail_search: bool, count_value: u64) -> Self {
        Self {
            name,
            fail_search,
            count_value,
            search_calls: AtomicUsize::new(0),
            search_count_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ResourceStorage for MockBackend {
    fn backend_name(&self) -> &'static str {
        self.name
    }

    async fn create(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: self.name.to_string(),
            capability: "create".to_string(),
        }))
    }

    async fn create_or_update(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _id: &str,
        _resource: Value,
        _fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: self.name.to_string(),
            capability: "create_or_update".to_string(),
        }))
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
        _tenant: &TenantContext,
        _current: &StoredResource,
        _resource: Value,
    ) -> StorageResult<StoredResource> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: self.name.to_string(),
            capability: "update".to_string(),
        }))
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
        Ok(0)
    }
}

#[async_trait]
impl SearchProvider for MockBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.search_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_search {
            return Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.name.to_string(),
                capability: "search".to_string(),
            }));
        }

        let resource = StoredResource::new(
            "Patient",
            "p-1",
            tenant.tenant_id().clone(),
            json!({"resourceType":"Patient","id":"p-1"}),
            FhirVersion::default(),
        );
        Ok(SearchResult::new(Page::new(
            vec![resource],
            PageInfo::end(),
        )))
    }

    async fn search_count(
        &self,
        _tenant: &TenantContext,
        _query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.search_count_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_search {
            return Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.name.to_string(),
                capability: "search_count".to_string(),
            }));
        }
        Ok(self.count_value)
    }
}

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("tenant-a"), TenantPermissions::full_access())
}

fn build_composite(primary: Arc<MockBackend>, es: Arc<MockBackend>) -> CompositeStorage {
    let config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()
        .expect("valid composite config");

    let mut backends = HashMap::new();
    backends.insert(
        "s3".to_string(),
        primary.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "s3".to_string(),
        primary as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es as helios_persistence::composite::DynSearchProvider,
    );

    CompositeStorage::new(config, backends)
        .expect("composite creation")
        .with_search_providers(search_providers)
}

#[tokio::test]
async fn s3_primary_routes_full_text_search_to_es_only() {
    let primary = Arc::new(MockBackend::new("s3", true, 0));
    let es = Arc::new(MockBackend::new("es", false, 5));
    let composite = build_composite(primary.clone(), es.clone());

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("smith")],
        chain: vec![],
        components: vec![],
    });

    let result = composite.search(&tenant(), &query).await;
    assert!(result.is_ok(), "search should be served by ES: {result:?}");
    assert_eq!(primary.search_calls.load(Ordering::SeqCst), 0);
    assert_eq!(es.search_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn s3_primary_routes_search_count_to_es_only() {
    let primary = Arc::new(MockBackend::new("s3", true, 0));
    let es = Arc::new(MockBackend::new("es", false, 17));
    let composite = build_composite(primary.clone(), es.clone());

    let query = SearchQuery::new("Patient");
    let count = composite
        .search_count(&tenant(), &query)
        .await
        .expect("count result");

    assert_eq!(count, 17);
    assert_eq!(primary.search_count_calls.load(Ordering::SeqCst), 0);
    assert_eq!(es.search_count_calls.load(Ordering::SeqCst), 1);
}
