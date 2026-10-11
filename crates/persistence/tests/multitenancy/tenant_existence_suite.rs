//! Backend-agnostic suite for `ResourceStorage::tenant_has_resources` (#1912).
//!
//! `DELETE /admin/tenants/{id}` used to learn whether a tenant still held data
//! from the cross-tenant `count_by_tenant` aggregate. It now asks the
//! tenant-scoped `tenant_has_resources`, whose contract is that it gives the
//! same answer as before: `true` exactly when `count_by_tenant` lists the
//! tenant with a non-zero count. These scenarios check that claim for the four
//! tenant states the admin API distinguishes — registered but empty, data
//! only, tombstones only, data purged — plus look-alike ids that must not
//! borrow each other's data.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `tenant_id_fidelity_suite.rs`. PostgreSQL and MongoDB share one database
//! across the binary, so `base` must be unique per run; every id here is
//! derived from it.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn ctx(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

async fn create_patient<S: ResourceStorage>(backend: &S, tenant: &str, id: &str) {
    backend
        .create(
            &ctx(tenant),
            "Patient",
            json!({"resourceType": "Patient", "id": id}),
            FhirVersion::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("create Patient/{id} in {tenant}: {e}"));
}

/// What the old handler computed: whether the cross-tenant aggregate lists
/// `tenant` with a non-zero count.
async fn discovered<S: ResourceStorage>(backend: &S, tenant: &str) -> bool {
    backend
        .count_by_tenant()
        .await
        .expect("count_by_tenant")
        .into_iter()
        .any(|(t, n)| t == tenant && n > 0)
}

async fn has<S: ResourceStorage>(backend: &S, tenant: &str) -> bool {
    backend
        .tenant_has_resources(&ctx(tenant))
        .await
        .unwrap_or_else(|e| panic!("tenant_has_resources({tenant}): {e}"))
}

/// Asserts `tenant_has_resources(tenant) == expected`, and that this is the
/// answer `count_by_tenant` gave the handler before #1912.
async fn assert_answer<S: ResourceStorage>(backend: &S, tenant: &str, expected: bool) {
    assert_eq!(
        has(backend, tenant).await,
        expected,
        "tenant_has_resources({tenant})"
    );
    assert_eq!(
        discovered(backend, tenant).await,
        expected,
        "count_by_tenant membership for {tenant} — the pre-#1912 answer"
    );
}

/// The four tenant states the admin API distinguishes, answered as before.
///
/// `tombstones_are_data` is the backend's documented tombstone semantics: the
/// SQL and document backends count live rows only (`false`); S3 counts current
/// pointers, delete tombstones included (`true`).
pub async fn tenant_has_resources_matches_discovery<S: ResourceStorage>(
    backend: &S,
    base: &str,
    tombstones_are_data: bool,
) {
    // Registered, never written to.
    let registered_empty = format!("{base}-reg-empty");
    backend
        .register_tenant(&registered_empty, None)
        .await
        .expect("register empty tenant");
    assert_answer(backend, &registered_empty, false).await;

    // Data, never registered.
    let data_only = format!("{base}-data");
    create_patient(backend, &data_only, "p1").await;
    create_patient(backend, &data_only, "p2").await;
    assert_answer(backend, &data_only, true).await;

    // Registered with data, then deregistered: the data keeps it present.
    let deregistered = format!("{base}-dereg");
    backend
        .register_tenant(&deregistered, None)
        .await
        .expect("register tenant");
    create_patient(backend, &deregistered, "p1").await;
    assert!(
        backend
            .deregister_tenant(&deregistered)
            .await
            .expect("deregister")
    );
    assert_answer(backend, &deregistered, true).await;

    // Every resource deleted: only tombstones remain.
    let tombstones = format!("{base}-tomb");
    create_patient(backend, &tombstones, "p1").await;
    backend
        .delete(&ctx(&tombstones), "Patient", "p1")
        .await
        .expect("delete Patient/p1");
    assert_answer(backend, &tombstones, tombstones_are_data).await;

    // Data purged.
    let purged = format!("{base}-purged");
    create_patient(backend, &purged, "p1").await;
    assert_answer(backend, &purged, true).await;
    backend
        .purge_tenant_data(&purged)
        .await
        .expect("purge tenant data");
    assert_answer(backend, &purged, false).await;

    // Never seen at all.
    assert_answer(backend, &format!("{base}-unknown"), false).await;

    // A look-alike whose id extends a populated tenant's id is its own tenant.
    assert_answer(backend, &format!("{base}-dat"), false).await;
    assert_answer(backend, &format!("{base}-data-x"), false).await;
}

/// A child tenant's data is not its parent's (hierarchical ids, #447).
///
/// Only `tenant_has_resources` is asserted for the child: S3's
/// `count_by_tenant` discovers first-segment prefixes only (a known #1672
/// limitation), so it never listed a nested tenant, while the tenant-scoped
/// probe reads the child's own prefix.
pub async fn child_data_is_not_the_parents<S: ResourceStorage>(backend: &S, base: &str) {
    let parent = format!("{base}-parent");
    let child = format!("{parent}/child");
    create_patient(backend, &child, "p1").await;

    assert!(has(backend, &child).await, "the child holds its own data");
    assert_answer(backend, &parent, false).await;
}
