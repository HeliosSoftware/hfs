//! Backend readiness / error-contract tests (issue #286).
//!
//! This file is the home of the reusable "unreachable backend" construction:
//! [`UnavailableBackend`], a [`ResourceStorage`] whose readiness probe fails
//! deterministically with no network or timing dependency. REST-layer tests that
//! need "storage is down" (readiness → 503, request → 503) reuse this shape.
//!
//! It also pins the positive control: a live in-memory SQLite backend reports
//! ready, exercising the `ResourceStorage::readiness_check` → `Backend::health_check`
//! delegation added for this issue.

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::{Backend, ResourceStorage};
use helios_persistence::error::{BackendError, StorageResult};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use serde_json::Value;

/// A `ResourceStorage` that is always unreachable — its readiness probe returns
/// [`BackendError::Unavailable`]. The deterministic, no-network stand-in for a
/// down backend.
pub struct UnavailableBackend;

#[async_trait]
impl ResourceStorage for UnavailableBackend {
    fn backend_name(&self) -> &'static str {
        "unavailable-test-backend"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        Err(BackendError::Unavailable {
            backend_name: "unavailable-test-backend".to_string(),
            message: "backend is down".to_string(),
        })
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

#[tokio::test]
async fn unavailable_backend_readiness_check_reports_unavailable() {
    let backend = UnavailableBackend;
    let err = backend
        .readiness_check()
        .await
        .expect_err("an unreachable backend must fail its readiness probe");
    assert!(
        matches!(err, BackendError::Unavailable { .. }),
        "expected Unavailable, got {err:?}"
    );
}

#[tokio::test]
async fn live_sqlite_readiness_and_health_check_ok() {
    // A reachable backend's readiness probe (and the Backend::health_check it
    // delegates to) must succeed — the positive control for the readiness fix.
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    Backend::health_check(&backend)
        .await
        .expect("health_check ok on a live db");
    ResourceStorage::readiness_check(&backend)
        .await
        .expect("readiness_check ok on a live db");
}
