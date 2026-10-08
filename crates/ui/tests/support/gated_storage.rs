//! [`GatedStorage`]: a delegating [`ResourceStorage`] whose cross-tenant
//! count and discovery can be held behind a gate, counted, and failed on
//! their own (#1851, reused by #1849).
//!
//! Every call reaches the wrapped store unchanged, except
//! [`ResourceStorage::discover_tenants`] and
//! [`ResourceStorage::count_by_tenant`], the two cross-tenant scans:
//!
//! - each call is counted ([`GatedStorage::discover_calls`],
//!   [`GatedStorage::count_by_tenant_calls`]);
//! - the call reads the wrapped store **first**, then waits at the gate while
//!   it is held ([`GatedStorage::hold`] / [`GatedStorage::release`]). A held
//!   answer is therefore a snapshot of the store as it was when the call
//!   began, delivered late: exactly a slow scan that started before a
//!   mutation. [`GatedStorage::wait_until_waiting`] waits until a call is
//!   parked there;
//! - [`GatedStorage::fail_counts`] makes both fail while the registry
//!   (`list_tenants`, `get_tenant`, ...) stays healthy: a count-only outage.
//!
//! Included on its own (`#[path = "support/gated_storage.rs"] mod
//! gated_storage;`), like `html.rs`, so a binary that does not need it does
//! not compile it.

// Each test binary includes this module and uses a different subset of it.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::core::{DiscoveryRequest, ResourceStorage, TenantDiscovery, TenantRecord};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use serde_json::Value;
use tokio::sync::watch;

/// Bounds every wait in this module, so a broken test fails instead of
/// hanging. Never an assertion about elapsed time.
const HANG_GUARD: Duration = Duration::from_secs(10);

/// A delegating store with a gated, counted, failable cross-tenant scan.
/// See the [module documentation](self).
pub struct GatedStorage {
    inner: Arc<dyn ResourceStorage>,
    /// `true` while the gate is held.
    held: watch::Sender<bool>,
    /// How many scans are parked at the gate right now.
    waiting: watch::Sender<usize>,
    discover_calls: AtomicUsize,
    count_by_tenant_calls: AtomicUsize,
    fail_counts: AtomicBool,
}

impl GatedStorage {
    /// Wraps `inner`, with the gate open and nothing failing.
    pub fn new(inner: Arc<dyn ResourceStorage>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            held: watch::Sender::new(false),
            waiting: watch::Sender::new(0),
            discover_calls: AtomicUsize::new(0),
            count_by_tenant_calls: AtomicUsize::new(0),
            fail_counts: AtomicBool::new(false),
        })
    }

    /// Holds every later (and parked) scan at the gate.
    pub fn hold(&self) {
        self.held.send_replace(true);
    }

    /// Opens the gate: parked scans return their answers.
    pub fn release(&self) {
        self.held.send_replace(false);
    }

    /// Makes `discover_tenants` and `count_by_tenant` fail (or work again),
    /// leaving every registry call healthy.
    pub fn fail_counts(&self, fail: bool) {
        self.fail_counts.store(fail, Ordering::SeqCst);
    }

    /// `discover_tenants` calls so far.
    pub fn discover_calls(&self) -> usize {
        self.discover_calls.load(Ordering::SeqCst)
    }

    /// `count_by_tenant` calls so far.
    pub fn count_by_tenant_calls(&self) -> usize {
        self.count_by_tenant_calls.load(Ordering::SeqCst)
    }

    /// Waits until `n` scans are parked at the gate (panics after the hang
    /// guard).
    pub async fn wait_until_waiting(&self, n: usize) {
        let mut rx = self.waiting.subscribe();
        tokio::time::timeout(HANG_GUARD, rx.wait_for(|waiting| *waiting >= n))
            .await
            .unwrap_or_else(|_| panic!("{n} scan(s) never reached the gate"))
            .expect("the gate's sender lives as long as the storage");
    }

    /// Waits until `discover_tenants` has been called at least `n` times
    /// (panics after the hang guard).
    pub async fn wait_for_discover_calls(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + HANG_GUARD;
        while self.discover_calls() < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "discover_tenants was called {} time(s), never {n}",
                self.discover_calls()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Parks at the gate while it is held. The answer was read before, so a
    /// held call delivers a snapshot from before whatever happens meanwhile.
    async fn gate(&self) {
        let mut held = self.held.subscribe();
        if !*held.borrow_and_update() {
            return;
        }
        self.waiting.send_modify(|waiting| *waiting += 1);
        // No hang guard here: a test that never releases fails at its own
        // timeout, and a test that drops the inventory leaves this parked
        // until the runtime shuts down.
        let _ = held.wait_for(|held| !*held).await;
        self.waiting.send_modify(|waiting| *waiting -= 1);
    }

    fn count_failure(&self) -> StorageError {
        StorageError::Backend(BackendError::Unavailable {
            backend_name: "gated".to_string(),
            message: "count-only failure injected by GatedStorage".to_string(),
        })
    }
}

#[async_trait]
impl ResourceStorage for GatedStorage {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn discover_tenants(&self, req: &DiscoveryRequest) -> StorageResult<TenantDiscovery> {
        self.discover_calls.fetch_add(1, Ordering::SeqCst);
        let answer = if self.fail_counts.load(Ordering::SeqCst) {
            Err(self.count_failure())
        } else {
            self.inner.discover_tenants(req).await
        };
        self.gate().await;
        answer
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.count_by_tenant_calls.fetch_add(1, Ordering::SeqCst);
        let answer = if self.fail_counts.load(Ordering::SeqCst) {
            Err(self.count_failure())
        } else {
            self.inner.count_by_tenant().await
        };
        self.gate().await;
        answer
    }

    // ---- everything else delegates unchanged ------------------------------

    fn is_cluster_shared(&self) -> bool {
        self.inner.is_cluster_shared()
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        self.inner.readiness_check().await
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

    async fn create_many(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resources: Vec<Value>,
        fhir_version: FhirVersion,
    ) -> Vec<StorageResult<StoredResource>> {
        self.inner
            .create_many(tenant, resource_type, resources, fhir_version)
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

    fn bulk_write_concurrency(&self) -> usize {
        self.inner.bulk_write_concurrency()
    }

    fn supports_tenant_registry(&self) -> bool {
        self.inner.supports_tenant_registry()
    }

    async fn list_tenants(&self) -> StorageResult<Vec<TenantRecord>> {
        self.inner.list_tenants().await
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<TenantRecord>> {
        self.inner.get_tenant(id).await
    }

    fn ensure_canonical_tenant_id(&self, id: &str) -> StorageResult<()> {
        self.inner.ensure_canonical_tenant_id(id)
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<TenantRecord> {
        self.inner.register_tenant(id, display_name).await
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        self.inner.deregister_tenant(id).await
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        self.inner.purge_tenant_data(id).await
    }
}
