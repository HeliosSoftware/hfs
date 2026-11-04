//! Backend-agnostic suite for the SQL-on-FHIR terminology store (#1824, #2185).
//!
//! Every backend with a `sof_runner()` must also offer a
//! `sof_terminology_store()` with the same contract: one relation per (tenant,
//! canonical), shared and reference-counted across Libraries, dropped with its
//! last reference, isolated per tenant, and removed by a tenant purge.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `tenant_existence_suite.rs`. PostgreSQL and MongoDB share one database
//! across the binary, so `base` must be unique per run; every tenant id here
//! is derived from it.
//!
//! `residue` counts what the backend still stores for a tenant (registry
//! entries, row chunks, generation objects, physical tables). It is how the
//! suite checks that a released relation is actually deleted rather than just
//! unreachable; a backend that cannot count from outside the crate passes
//! `None` and checks that in its own unit tests.

#![allow(dead_code)]

use std::sync::Arc;

use futures::future::BoxFuture;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::sof_terminology::SofTerminologyStore;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_sof::sqlquery::{TerminologyKind, TerminologyRelation};
use serde_json::json;

/// Counts the terminology records a backend still holds for a tenant id.
pub type Residue<'a> = Option<&'a (dyn Fn(String) -> BoxFuture<'static, usize> + Sync)>;

fn ctx(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn vs(codes: &[&str]) -> TerminologyRelation {
    TerminologyRelation {
        kind: TerminologyKind::ValueSet,
        rows: codes
            .iter()
            .map(|c| {
                json!({"system": "http://s", "version": null, "code": c,
                       "display": null, "inactive": c.starts_with('x')})
            })
            .collect(),
    }
}

fn store<S: ResourceStorage>(backend: &S) -> Arc<dyn SofTerminologyStore> {
    backend
        .sof_terminology_store()
        .expect("a backend with a SOF runner must have a terminology store")
}

async fn assert_residue(residue: Residue<'_>, tenant: &str, expected: usize) {
    if let Some(residue) = residue {
        assert_eq!(
            residue(tenant.to_string()).await,
            expected,
            "residue of {tenant}"
        );
    }
}

/// A relation shared by two Libraries survives the first release, is
/// refreshed by the latest write, and is deleted with the last reference.
pub async fn shared_relation_is_dropped_with_its_last_reference<S: ResourceStorage>(
    backend: &S,
    base: &str,
    residue: Residue<'_>,
) {
    let store = store(backend);
    let tenant = format!("{base}-shared");
    let t = ctx(&tenant);
    let url = "http://example.org/vs";

    store
        .set_library_dependencies(&t, "lib-a", vec![(url.into(), vs(&["a", "xb"]))])
        .await
        .unwrap();
    store
        .set_library_dependencies(&t, "lib-b", vec![(url.into(), vs(&["a", "xb", "c"]))])
        .await
        .unwrap();
    assert_eq!(
        store.load(&t, url).await.unwrap(),
        Some(vs(&["a", "xb", "c"]))
    );

    store
        .set_library_dependencies(&t, "lib-a", vec![])
        .await
        .unwrap();
    assert!(
        store.load(&t, url).await.unwrap().is_some(),
        "lib-b still refers"
    );

    store
        .set_library_dependencies(&t, "lib-b", vec![])
        .await
        .unwrap();
    assert!(store.load(&t, url).await.unwrap().is_none());
    assert_residue(residue, &tenant, 0).await;
}

/// An update that drops one of a Library's dependencies releases it.
pub async fn dropping_a_dependency_on_update_releases_it<S: ResourceStorage>(
    backend: &S,
    base: &str,
) {
    let store = store(backend);
    let t = ctx(&format!("{base}-update"));
    store
        .set_library_dependencies(
            &t,
            "lib",
            vec![
                ("http://vs/1".into(), vs(&["a"])),
                ("http://vs/2".into(), vs(&["b"])),
            ],
        )
        .await
        .unwrap();
    store
        .set_library_dependencies(&t, "lib", vec![("http://vs/2".into(), vs(&["b"]))])
        .await
        .unwrap();
    assert!(store.load(&t, "http://vs/1").await.unwrap().is_none());
    assert_eq!(
        store.load(&t, "http://vs/2").await.unwrap(),
        Some(vs(&["b"]))
    );
}

/// Tenants never see, or release, each other's relations.
pub async fn tenants_are_isolated<S: ResourceStorage>(backend: &S, base: &str) {
    let store = store(backend);
    let a = ctx(&format!("{base}-iso-a"));
    let b = ctx(&format!("{base}-iso-b"));
    let url = "http://example.org/vs";
    store
        .set_library_dependencies(&a, "lib", vec![(url.into(), vs(&["a"]))])
        .await
        .unwrap();
    assert!(store.load(&b, url).await.unwrap().is_none());
    // Releasing the same Library id in another tenant leaves tenant a's alone.
    store
        .set_library_dependencies(&b, "lib", vec![])
        .await
        .unwrap();
    assert_eq!(store.load(&a, url).await.unwrap(), Some(vs(&["a"])));
}

/// ConceptMap rows, nulls included, round-trip exactly.
pub async fn concept_map_rows_round_trip<S: ResourceStorage>(backend: &S, base: &str) {
    let store = store(backend);
    let t = ctx(&format!("{base}-cm"));
    let rel = TerminologyRelation {
        kind: TerminologyKind::ConceptMap,
        rows: vec![
            json!({
                "source_system": "http://a", "source_version": null, "source_code": "1",
                "source_display": "One", "target_system": "http://b", "target_version": "2",
                "target_code": "I10", "target_display": null, "relationship": "equivalent"
            }),
            json!({
                "source_system": "http://a", "source_version": null, "source_code": "2",
                "source_display": null, "target_system": null, "target_version": null,
                "target_code": null, "target_display": null, "relationship": null
            }),
        ],
    };
    store
        .set_library_dependencies(&t, "lib", vec![("http://cm".into(), rel.clone())])
        .await
        .unwrap();
    assert_eq!(store.load(&t, "http://cm").await.unwrap(), Some(rel));
}

/// A relation larger than one storage chunk keeps every row, in order; an
/// empty one keeps its kind.
pub async fn large_and_empty_relations_round_trip<S: ResourceStorage>(backend: &S, base: &str) {
    let store = store(backend);
    let t = ctx(&format!("{base}-size"));
    let codes: Vec<String> = (0..2_500).map(|i| format!("c{i:05}")).collect();
    let refs: Vec<&str> = codes.iter().map(String::as_str).collect();
    let large = vs(&refs);
    let empty = vs(&[]);
    store
        .set_library_dependencies(
            &t,
            "lib",
            vec![
                ("http://large".into(), large.clone()),
                ("http://empty".into(), empty.clone()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(store.load(&t, "http://large").await.unwrap(), Some(large));
    assert_eq!(store.load(&t, "http://empty").await.unwrap(), Some(empty));
}

/// Concurrent Library writes on one canonical lose no reference: after all of
/// them the relation is served, and after all release it, it is gone.
pub async fn concurrent_writers_keep_every_reference<S: ResourceStorage>(
    backend: &S,
    base: &str,
    residue: Residue<'_>,
) {
    let store = store(backend);
    let tenant = format!("{base}-race");
    let url = "http://example.org/vs";
    let writers = 6;

    let tasks: Vec<_> = (0..writers)
        .map(|i| {
            let store = Arc::clone(&store);
            let t = ctx(&tenant);
            tokio::spawn(async move {
                store
                    .set_library_dependencies(
                        &t,
                        &format!("lib-{i}"),
                        vec![(url.into(), vs(&["a"]))],
                    )
                    .await
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let t = ctx(&tenant);
    assert_eq!(store.load(&t, url).await.unwrap(), Some(vs(&["a"])));

    // Release all but one concurrently: the survivor's reference must hold.
    let tasks: Vec<_> = (1..writers)
        .map(|i| {
            let store = Arc::clone(&store);
            let t = ctx(&tenant);
            tokio::spawn(async move {
                store
                    .set_library_dependencies(&t, &format!("lib-{i}"), vec![])
                    .await
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert!(
        store.load(&t, url).await.unwrap().is_some(),
        "lib-0 still refers"
    );

    store
        .set_library_dependencies(&t, "lib-0", vec![])
        .await
        .unwrap();
    assert!(store.load(&t, url).await.unwrap().is_none());
    assert_residue(residue, &tenant, 0).await;
}

/// A tenant purge removes the tenant's relations and nobody else's.
pub async fn purge_removes_the_tenants_relations<S: ResourceStorage>(
    backend: &S,
    base: &str,
    residue: Residue<'_>,
) {
    let store = store(backend);
    let purged = format!("{base}-purged");
    let kept = format!("{base}-kept");
    for tenant in [&purged, &kept] {
        store
            .set_library_dependencies(&ctx(tenant), "lib", vec![("http://vs".into(), vs(&["a"]))])
            .await
            .unwrap();
    }
    backend.purge_tenant_data(&purged).await.unwrap();
    assert!(
        store
            .load(&ctx(&purged), "http://vs")
            .await
            .unwrap()
            .is_none()
    );
    assert_residue(residue, &purged, 0).await;
    assert!(
        store
            .load(&ctx(&kept), "http://vs")
            .await
            .unwrap()
            .is_some()
    );

    // The purged tenant starts again from nothing.
    store
        .set_library_dependencies(&ctx(&purged), "lib", vec![("http://vs".into(), vs(&["b"]))])
        .await
        .unwrap();
    assert_eq!(
        store.load(&ctx(&purged), "http://vs").await.unwrap(),
        Some(vs(&["b"]))
    );
}

/// Runs every scenario.
pub async fn run_all<S: ResourceStorage>(backend: &S, base: &str, residue: Residue<'_>) {
    shared_relation_is_dropped_with_its_last_reference(backend, base, residue).await;
    dropping_a_dependency_on_update_releases_it(backend, base).await;
    tenants_are_isolated(backend, base).await;
    concept_map_rows_round_trip(backend, base).await;
    large_and_empty_relations_round_trip(backend, base).await;
    concurrent_writers_keep_every_reference(backend, base, residue).await;
    purge_removes_the_tenants_relations(backend, base, residue).await;
}
