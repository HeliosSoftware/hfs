//! Integration tests for storage-backed FHIRPath `resolve()`
//! ([`helios_persistence::sof::reference_resolver`]).
//!
//! Exercises the [`StorageReferenceResolver`] contract against a real
//! `SqliteBackend`, covering the acceptance criteria from issue #167:
//! stored-resource hit, cross-tenant miss, not-found fallback, and FHIR-version
//! match.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::sof_runner::SofError;
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use helios_persistence::sof::reference_resolver::{
    StorageBackedResolver, StorageReferenceResolver,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use serde_json::{Value, json};

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

/// A fresh in-memory backend with the schema initialised.
fn backend() -> Arc<SqliteBackend> {
    let backend = SqliteBackend::with_config(":memory:", SqliteBackendConfig::default())
        .expect("create in-memory SqliteBackend");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
}

fn resolver(storage: Arc<SqliteBackend>) -> StorageBackedResolver {
    StorageBackedResolver::new(storage, StorageBackedResolver::DEFAULT_MAX_FANOUT)
}

/// A relative `Type/id` reference is dereferenced to the stored resource,
/// scoped to the owning tenant.
#[tokio::test]
async fn resolves_stored_resource_for_owning_tenant() {
    let backend = backend();
    let t = tenant("clinic-a");
    backend
        .create(
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": "123", "active": true}),
            FhirVersion::R4,
        )
        .await
        .unwrap();

    let resolved = resolver(backend.clone())
        .resolve(
            &t,
            FhirVersion::R4,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect("resolve");

    assert_eq!(resolved.len(), 1, "expected the stored Patient to resolve");
    assert_eq!(resolved[0]["resourceType"], "Patient");
    assert_eq!(resolved[0]["id"], "123");
}

/// A resource stored under one tenant MUST NOT resolve for another tenant.
#[tokio::test]
async fn does_not_resolve_across_tenants() {
    let backend = backend();
    backend
        .create(
            &tenant("clinic-a"),
            "Patient",
            json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::R4,
        )
        .await
        .unwrap();

    // A different tenant must see nothing.
    let resolved = resolver(backend.clone())
        .resolve(
            &tenant("clinic-b"),
            FhirVersion::R4,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect("resolve");

    assert!(
        resolved.is_empty(),
        "cross-tenant resolution must return nothing, got {resolved:?}"
    );
}

/// A reference to a resource that does not exist resolves to nothing (the caller
/// then falls back to the engine's typed-stub / empty semantics).
#[tokio::test]
async fn missing_reference_resolves_to_nothing() {
    let backend = backend();
    let resolved = resolver(backend.clone())
        .resolve(
            &tenant("clinic-a"),
            FhirVersion::R4,
            &[("Patient".to_string(), "does-not-exist".to_string())],
        )
        .await
        .expect("resolve");
    assert!(resolved.is_empty());
}

/// Only resources whose stored FHIR version matches the evaluation version are
/// returned, so the engine never mixes versions.
#[cfg(feature = "R4B")]
#[tokio::test]
async fn resolves_only_matching_fhir_version() {
    let backend = backend();
    let t = tenant("clinic-a");
    backend
        .create(
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::R4,
        )
        .await
        .unwrap();

    // Same tenant + id, but the evaluation is for a different version → no match.
    let mismatched = resolver(backend.clone())
        .resolve(
            &t,
            FhirVersion::R4B,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect("resolve");
    assert!(
        mismatched.is_empty(),
        "version mismatch must not resolve, got {mismatched:?}"
    );

    // The matching version resolves.
    let matched = resolver(backend.clone())
        .resolve(
            &t,
            FhirVersion::R4,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect("resolve");
    assert_eq!(matched.len(), 1);
}

/// #1870: more distinct references than the cap fail before any read, instead
/// of leaving the excess unresolved.
#[tokio::test]
async fn over_the_cap_is_an_error_not_a_silent_drop() {
    let backend = backend();
    let refs = [
        ("Patient".to_string(), "1".to_string()),
        ("Patient".to_string(), "2".to_string()),
    ];
    let err = StorageBackedResolver::new(backend, 1)
        .resolve(&tenant("clinic-a"), FhirVersion::R4, &refs)
        .await
        .expect_err("two references over a cap of one");
    assert!(matches!(err, SofError::ResolutionLimit(_)), "got {err:?}");
}

/// A storage whose `read_batch` fails its first `failures` calls.
struct FlakyStorage {
    inner: Arc<SqliteBackend>,
    failures: usize,
    calls: AtomicUsize,
}

impl FlakyStorage {
    fn new(inner: Arc<SqliteBackend>, failures: usize) -> Arc<Self> {
        Arc::new(Self {
            inner,
            failures,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ResourceStorage for FlakyStorage {
    fn backend_name(&self) -> &'static str {
        "flaky"
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

    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            return Err(StorageError::Backend(BackendError::Unavailable {
                backend_name: "flaky".to_string(),
                message: "connection reset".to_string(),
            }));
        }
        self.inner.read_batch(tenant, resource_type, ids).await
    }
}

async fn seeded_patient() -> (Arc<SqliteBackend>, TenantContext) {
    let backend = backend();
    let t = tenant("clinic-a");
    backend
        .create(
            &t,
            "Patient",
            json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::R4,
        )
        .await
        .unwrap();
    (backend, t)
}

/// #1870: one failed batch read is retried, and the retry's result is used.
#[tokio::test]
async fn a_failed_read_is_retried_once() {
    let (backend, t) = seeded_patient().await;
    let storage = FlakyStorage::new(backend, 1);
    let resolved = StorageBackedResolver::new(storage.clone(), 10)
        .resolve(
            &t,
            FhirVersion::R4,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect("the retry succeeds");
    assert_eq!(resolved.len(), 1);
    assert_eq!(storage.calls.load(Ordering::SeqCst), 2);
}

/// #1870: a read that fails twice fails the resolution, naming the type.
#[tokio::test]
async fn a_read_that_fails_twice_is_an_error() {
    let (backend, t) = seeded_patient().await;
    let storage = FlakyStorage::new(backend, 2);
    let err = StorageBackedResolver::new(storage.clone(), 10)
        .resolve(
            &t,
            FhirVersion::R4,
            &[("Patient".to_string(), "123".to_string())],
        )
        .await
        .expect_err("both reads fail");
    assert!(
        matches!(&err, SofError::Storage(m) if m.contains("Patient")),
        "got {err:?}"
    );
    assert_eq!(storage.calls.load(Ordering::SeqCst), 2);
}
