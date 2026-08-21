//! Backend-agnostic tenant-id fidelity suite (issue #447).
//!
//! Issue #447 is an S3 defect: `S3Keyspace::with_tenant_prefix` derived a
//! tenant's data-key prefix with `trim_matches('/')`, so `acme`, `/acme`,
//! `acme/` and `//acme` all resolved to one prefix while the tenant registry
//! held them as four separate tenants. The fix lives in `s3/keyspace.rs` and is
//! proven there, both as properties over an adversarial id corpus and end to end
//! through the mock S3 client.
//!
//! This suite exists because the defect is an instance of a *class*: **a backend
//! that derives a storage location from the tenant id rather than matching on
//! the id itself owes an injective derivation.** Two backends derive; three do
//! not:
//!
//! | backend | tenant scoping | injective? |
//! |---|---|---|
//! | SQLite | `tenant_id TEXT` in the composite primary key, bound and compared with `=` (BINARY collation) | identity |
//! | PostgreSQL | `tenant_id TEXT` in the composite primary key, bound and compared with `=` | identity |
//! | MongoDB | `"tenant_id": <id>` exact BSON match, default (case-sensitive) collation | identity |
//! | S3 | **derives a key prefix** | issue #447 — fixed in `s3/keyspace.rs` |
//! | Elasticsearch | **derives an index name** (`to_lowercase`) | issue #384 — fixed in PR #446 |
//!
//! For the three "identity" rows that claim is a code reading, and a code
//! reading is exactly what let #384 and #447 sit undiscovered in the two rows
//! that derive. So the claim is asserted instead: these scenarios drive the id
//! shapes that S3 collapsed through each backend's real storage path and check
//! that nothing commingles. They are cheap, they run against SQLite in ordinary
//! CI, and they run against PostgreSQL and MongoDB wherever those containers are
//! available.
//!
//! Included by `#[path]` into each backend's test binary rather than living in
//! `tests/common/`, which no test target declares and cargo therefore never
//! compiles (issue #306) — the same arrangement as
//! `transactions/if_match_suite.rs`.
//!
//! ## Shared-database backends
//!
//! PostgreSQL and MongoDB run the whole binary against one container database,
//! so every scenario takes a `base` id that the caller must make unique. All the
//! variant ids are derived from it, keeping runs from colliding while preserving
//! the exact *shape* relationships (padding, nesting) that are under test.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{PurgableStorage, ResourceStorage};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

/// The id shapes that S3's derivation used to collapse onto `base`, plus `base`
/// itself. Distinct strings, therefore distinct tenants — on every backend.
fn variants(base: &str) -> Vec<String> {
    vec![
        base.to_string(),
        format!("/{base}"),
        format!("{base}/"),
        format!("//{base}"),
        // Not a padding case: this one nested *inside* the prefix `base` sweeps
        // and lists, which is the second defect in the same derivation.
        format!("{base}/resources"),
    ]
}

fn ctx(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

/// Distinct tenant ids must never share stored data.
///
/// Writes one uniquely-named Patient per variant tenant, then checks every
/// tenant sees exactly its own — no commingled reads, no inflated counts.
pub async fn distinct_tenant_ids_never_share_data<S>(backend: &S, base: &str)
where
    S: ResourceStorage,
{
    let ids = variants(base);

    for (i, id) in ids.iter().enumerate() {
        backend
            .create(
                &ctx(id),
                "Patient",
                json!({"resourceType": "Patient", "id": format!("p{i}")}),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create in tenant {id:?} failed: {e}"));
    }

    for (i, id) in ids.iter().enumerate() {
        let tenant = ctx(id);

        assert!(
            backend
                .read(&tenant, "Patient", &format!("p{i}"))
                .await
                .unwrap()
                .is_some(),
            "tenant {id:?} must see its own resource"
        );

        for (j, other) in ids.iter().enumerate() {
            if i == j {
                continue;
            }
            assert!(
                backend
                    .read(&tenant, "Patient", &format!("p{j}"))
                    .await
                    .unwrap()
                    .is_none(),
                "tenant {id:?} must not see p{j}, which belongs to {other:?}"
            );
        }

        assert_eq!(
            backend.count(&tenant, None).await.unwrap(),
            1,
            "tenant {id:?} must count only its own resource"
        );
    }
}

/// Purging one tenant must not touch another whose id merely *looks* like it.
///
/// This is issue #447's headline consequence stated backend-agnostically: on S3
/// it destroyed the bare tenant's resources and version history while leaving
/// its registry record behind, and reported success to the caller.
pub async fn purging_one_tenant_leaves_the_look_alikes_intact<S>(backend: &S, base: &str)
where
    S: ResourceStorage + PurgableStorage,
{
    let ids = variants(base);

    for (i, id) in ids.iter().enumerate() {
        backend
            .create(
                &ctx(id),
                "Patient",
                json!({"resourceType": "Patient", "id": format!("p{i}")}),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create in tenant {id:?} failed: {e}"));
    }

    // Purge every variant *except* the bare id, one at a time.
    for id in ids.iter().skip(1) {
        backend
            .purge_tenant_data(id)
            .await
            .unwrap_or_else(|e| panic!("purge of tenant {id:?} failed: {e}"));

        assert!(
            backend
                .read(&ctx(base), "Patient", "p0")
                .await
                .unwrap()
                .is_some(),
            "purging {id:?} must not delete tenant {base:?}'s resource"
        );
    }

    assert_eq!(
        backend.count(&ctx(base), None).await.unwrap(),
        1,
        "tenant {base:?} must be untouched by every look-alike purge"
    );

    // And the purges did do their own job.
    for (i, id) in ids.iter().enumerate().skip(1) {
        assert!(
            backend
                .read(&ctx(id), "Patient", &format!("p{i}"))
                .await
                .unwrap()
                .is_none(),
            "tenant {id:?}'s own resource should have been purged"
        );
    }
}
