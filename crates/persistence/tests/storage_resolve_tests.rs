//! Integration tests for storage-backed FHIRPath `resolve()`
//! ([`helios_persistence::sof::reference_resolver`]).
//!
//! Exercises the [`StorageReferenceResolver`] contract against a real
//! `SqliteBackend`, covering the acceptance criteria from issue #167:
//! stored-resource hit, cross-tenant miss, not-found fallback, and FHIR-version
//! match.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::sof::reference_resolver::{
    StorageBackedResolver, StorageReferenceResolver,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::json;

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
        .await;

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
        .await;

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
        .await;
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
        .await;
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
        .await;
    assert_eq!(matched.len(), 1);
}
