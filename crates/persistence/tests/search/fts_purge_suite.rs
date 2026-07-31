//! Backend-agnostic full-text purge-completeness suite (issue #386).
//!
//! Every scenario is generic over the storage traits, so the *same* assertions
//! run on SQLite and PostgreSQL rather than being retyped per engine. Like
//! `transactions/if_match_suite.rs`, this file is `#[path]`-included by each
//! backend's test binary rather than living in `tests/common/`, which no test
//! target declares and cargo therefore never compiles (issue #306).
//!
//! ## Why these tests assert on raw rows, not on search results
//!
//! The obvious regression test — "purge a resource, then check `_text` finds
//! nothing" — **passes on the unfixed code**, and is therefore worthless.
//!
//! SQLite compiles `_text`/`_content` into a sub-filter:
//! `resource_id IN (SELECT resource_id FROM resource_fts WHERE tenant_id = ?1
//! AND … MATCH ?n)`, AND-ed onto an outer query that selects from
//! `search_index`. Purge *does* clear `search_index`, so an orphaned
//! `resource_fts` row is unreachable through search while the resource's
//! narrative and complete serialized body are still on disk.
//!
//! So each scenario asserts two independent things:
//!
//!  1. **Residency** — a direct `SELECT COUNT(*) FROM resource_fts` via
//!     [`FtsProbe`], which bypasses every query path and answers "is the PHI
//!     still there", not "can a search reach it".
//!  2. **Exploitability** — the id-reuse oracle. Purge a resource, create a new
//!     one reusing the same logical id, and the *old* resource's text becomes
//!     matchable again and is attributed to the new resource. This is the
//!     user-visible harm in the issue, and unlike (1) it fails through the
//!     public search API.
//!
//! Every scenario also opens with a **positive control** asserting the row was
//! indexed in the first place. Without it, a broken indexer, a typo'd tenant, a
//! missing feature, or a renamed parameter would sail straight through the
//! post-conditions and the test would pass while proving nothing.
//!
//! ## Shared-database backends
//!
//! The PostgreSQL suite runs every scenario against one long-lived container
//! database, so scenarios must not collide. Each takes its own
//! [`TenantContext`]; callers must pass a **distinct tenant per scenario**.
//!
//! A distinct tenant is not enough on its own, because [`FtsProbe::
//! fts_rows_containing`] is deliberately *not* tenant-scoped — that is the whole
//! point of it. Scenarios run concurrently under `cargo test`, and several end
//! with a live indexed row on purpose (the reindex scenarios, the bystander
//! tenant), so a term shared across scenarios makes the unqualified count
//! non-zero for reasons that have nothing to do with the purge under test.
//! [`planted_term`] therefore derives the token from the tenant id: the probe
//! stays global — it still catches a surviving row under *any* tenant, type or
//! id — while only ever counting rows this scenario planted.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{PurgableStorage, ResourceStorage, SearchProvider};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// Stem of the token planted in narrative text. Rare and unambiguous, so a
/// match can only come from a resource this suite created, never from seeded
/// conformance data or another test's fixtures.
const PLANTED_STEM: &str = "Zebracrossingdiagnosis";

/// Stem of the second planted token, for the resource that must survive.
const BYSTANDER_STEM: &str = "Quokkaflavoured";

/// The token this scenario plants, unique to `tenant`.
///
/// See the module docs: the residency probe counts rows across *all* tenants, so
/// the token — not the tenant predicate — is what keeps concurrent scenarios on
/// a shared database from reading as each other's leaks.
pub fn planted_term(tenant: &TenantContext) -> String {
    scoped_term(PLANTED_STEM, tenant)
}

/// The bystander token for `tenant`, unique on the same basis as
/// [`planted_term`].
pub fn bystander_term(tenant: &TenantContext) -> String {
    scoped_term(BYSTANDER_STEM, tenant)
}

/// Appends the tenant id to `stem` as a single indexable word.
///
/// Non-alphanumerics are dropped rather than kept: both engines' tokenizers
/// split on `-` and `_`, which would turn one rare token into several common
/// ones and break the exact-match assertions. What remains is one long word that
/// FTS5 and `to_tsvector('english', …)` both keep intact, and that a `LIKE
/// '%…%'` residency probe matches literally.
fn scoped_term(stem: &str, tenant: &TenantContext) -> String {
    let suffix: String = tenant
        .tenant_id()
        .as_str()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("{stem}{suffix}")
}

/// Reads physical `resource_fts` rows, bypassing every query path.
///
/// This is what distinguishes "the PHI is gone" from "a search cannot currently
/// reach the PHI". Both backends can implement it from an integration-test
/// target without any production API change: PostgreSQL exposes
/// `#[doc(hidden)] pub get_client`, and SQLite can be pointed at a file-backed
/// database that the test opens a second `rusqlite` connection onto.
#[async_trait::async_trait]
pub trait FtsProbe {
    /// Number of `resource_fts` rows held for `tenant_id`.
    async fn fts_row_count(&self, tenant_id: &str) -> u64;

    /// Number of `resource_fts` rows anywhere whose stored text contains
    /// `needle`, **with no tenant predicate**.
    ///
    /// The unqualified form is what catches a `DELETE` whose `WHERE` clause is
    /// wrong in a way a tenant-scoped count would hide.
    async fn fts_rows_containing(&self, needle: &str) -> u64;
}

/// A Patient carrying `term` in its generated narrative, which is what
/// `_text` indexes.
pub fn patient_with_narrative(id: &str, term: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "text": {
            "status": "generated",
            "div": format!(
                "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Assessment: {term}.</p></div>"
            )
        },
        "name": [{"family": "Purgetest"}]
    })
}

/// A Patient with no trace of the planted term, for id-reuse scenarios.
pub fn unrelated_patient(id: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "text": {
            "status": "generated",
            "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Routine review.</p></div>"
        },
        "name": [{"family": "Replacement"}]
    })
}

fn text_query(term: &str) -> SearchQuery {
    SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::Special,
        modifier: None,
        values: vec![SearchValue::eq(term)],
        chain: vec![],
        components: vec![],
    })
}

fn content_query(term: &str) -> SearchQuery {
    SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_content".to_string(),
        param_type: SearchParamType::Special,
        modifier: None,
        values: vec![SearchValue::eq(term)],
        chain: vec![],
        components: vec![],
    })
}

async fn text_hits<B>(backend: &B, tenant: &TenantContext, term: &str) -> Vec<String>
where
    B: SearchProvider,
{
    backend
        .search(tenant, &text_query(term))
        .await
        .expect("_text search should succeed")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

async fn content_hits<B>(backend: &B, tenant: &TenantContext, term: &str) -> Vec<String>
where
    B: SearchProvider,
{
    backend
        .search(tenant, &content_query(term))
        .await
        .expect("_content search should succeed")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// Seeds one Patient carrying this tenant's [`planted_term`] and asserts the
/// full-text pipeline actually indexed it. Every scenario starts here; if this
/// fails, nothing after it would have meant anything.
async fn seed_and_control<B, P>(backend: &B, probe: &P, tenant: &TenantContext, id: &str)
where
    B: ResourceStorage + SearchProvider,
    P: FtsProbe,
{
    let term = planted_term(tenant);
    backend
        .create(
            tenant,
            "Patient",
            patient_with_narrative(id, &term),
            FhirVersion::default(),
        )
        .await
        .expect("seed create should succeed");

    let rows = probe.fts_row_count(tenant.tenant_id().as_str()).await;
    assert_eq!(
        rows, 1,
        "POSITIVE CONTROL: the seeded resource must be present in resource_fts \
         before the purge, or every assertion below is vacuous"
    );
    assert_eq!(
        text_hits(backend, tenant, &term).await,
        vec![id.to_string()],
        "POSITIVE CONTROL: _text must find the seeded narrative before the purge"
    );
}

// ===========================================================================
// Residency: the purge paths must remove the physical rows
// ===========================================================================

/// Single-resource `$purge` removes the resource's full-text row.
pub async fn purge_removes_fts_rows<B, P>(backend: &B, probe: &P, tenant: &TenantContext)
where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, tenant, "p1").await;

    backend
        .purge(tenant, "Patient", "p1")
        .await
        .expect("purge should succeed");

    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        0,
        "purge must delete the resource's resource_fts row; a surviving row keeps \
         the narrative and the full serialized body on disk after the API \
         reported the resource was removed"
    );
    assert_eq!(
        probe.fts_rows_containing(&planted_term(tenant)).await,
        0,
        "no resource_fts row anywhere may still contain the purged text"
    );
}

/// Type-level `$purge` removes every matching resource's full-text row.
pub async fn purge_all_removes_fts_rows<B, P>(backend: &B, probe: &P, tenant: &TenantContext)
where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, tenant, "p1").await;

    backend
        .purge_all(tenant, "Patient")
        .await
        .expect("purge_all should succeed");

    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        0,
        "purge_all must delete the type's resource_fts rows"
    );
    assert_eq!(probe.fts_rows_containing(&planted_term(tenant)).await, 0);
}

/// Tenant purge removes every one of the tenant's full-text rows, across types.
pub async fn purge_tenant_data_removes_fts_rows<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
) where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, tenant, "p1").await;
    let term = planted_term(tenant);

    backend
        .create(
            tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": {"text": "vitals"},
                "text": {
                    "status": "generated",
                    "div": format!(
                        "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>{term}</p></div>"
                    )
                }
            }),
            FhirVersion::default(),
        )
        .await
        .expect("second seed should succeed");

    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        2,
        "POSITIVE CONTROL: both seeded resources must be indexed"
    );

    backend
        .purge_tenant_data(tenant.tenant_id().as_str())
        .await
        .expect("purge_tenant_data should succeed");

    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        0,
        "purge_tenant_data must delete every one of the tenant's resource_fts \
         rows, across all resource types"
    );
    assert_eq!(probe.fts_rows_containing(&term).await, 0);
}

/// The purge must not be over-broad: another tenant's rows survive untouched.
///
/// A `DELETE FROM resource_fts` that lost its tenant predicate would satisfy
/// every other assertion in this suite. This is the only scenario that catches
/// it.
pub async fn purge_tenant_data_leaves_other_tenants_intact<B, P>(
    backend: &B,
    probe: &P,
    victim: &TenantContext,
    bystander: &TenantContext,
) where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, victim, "p1").await;

    backend
        .create(
            bystander,
            "Patient",
            patient_with_narrative("p1", &bystander_term(bystander)),
            FhirVersion::default(),
        )
        .await
        .expect("bystander seed should succeed");
    assert_eq!(
        probe.fts_row_count(bystander.tenant_id().as_str()).await,
        1,
        "POSITIVE CONTROL: the bystander tenant must be indexed too"
    );

    backend
        .purge_tenant_data(victim.tenant_id().as_str())
        .await
        .expect("purge_tenant_data should succeed");

    assert_eq!(
        probe.fts_row_count(victim.tenant_id().as_str()).await,
        0,
        "the purged tenant's rows must be gone"
    );
    assert_eq!(
        probe.fts_row_count(bystander.tenant_id().as_str()).await,
        1,
        "purging one tenant must not touch another tenant's resource_fts rows"
    );
    assert_eq!(
        text_hits(backend, bystander, &bystander_term(bystander)).await,
        vec!["p1".to_string()],
        "the bystander tenant's full-text search must still work after the purge"
    );
}

// ===========================================================================
// Exploitability: the id-reuse oracle
// ===========================================================================

/// Purging a resource and then reusing its logical id must not resurrect the
/// purged narrative.
///
/// This is the harm the issue describes, and it fails through the public search
/// API on unfixed code: the orphaned `resource_fts` row is matched by
/// `_text`, and because the sub-select intersects on `resource_id` alone, the
/// hit is attributed to the *new* resource.
pub async fn reuse_after_purge_does_not_resurrect_narrative<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
) where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, tenant, "p1").await;

    backend
        .purge(tenant, "Patient", "p1")
        .await
        .expect("purge should succeed");

    // Same logical id — the ordinary case for client-assigned ids, fixtures and
    // reload workflows.
    backend
        .create(
            tenant,
            "Patient",
            unrelated_patient("p1"),
            FhirVersion::default(),
        )
        .await
        .expect("re-create should succeed");

    assert_eq!(
        text_hits(backend, tenant, &planted_term(tenant)).await,
        Vec::<String>::new(),
        "the purged resource's narrative must not be matchable through a \
         re-created resource that merely reuses its id"
    );
    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        1,
        "only the replacement resource may be indexed; a count of 2 means the \
         purged generation's row is still on disk"
    );
}

/// Same oracle, reached through a tenant purge and tenant-id reuse — the
/// scenario named in the issue.
pub async fn tenant_reuse_does_not_resurrect_narrative<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
) where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    seed_and_control(backend, probe, tenant, "p1").await;

    backend
        .purge_tenant_data(tenant.tenant_id().as_str())
        .await
        .expect("purge_tenant_data should succeed");

    // A tenant of the same id is provisioned again and stores its own data.
    backend
        .create(
            tenant,
            "Patient",
            unrelated_patient("p1"),
            FhirVersion::default(),
        )
        .await
        .expect("re-create under the reused tenant id should succeed");

    assert_eq!(
        text_hits(backend, tenant, &planted_term(tenant)).await,
        Vec::<String>::new(),
        "a re-created tenant must not be able to surface the previous tenant's \
         purged content"
    );
    assert_eq!(
        content_hits(backend, tenant, &planted_term(tenant)).await,
        Vec::<String>::new(),
        "_content must be clean too, not only _text"
    );
    assert_eq!(probe.fts_row_count(tenant.tenant_id().as_str()).await, 1);
}

/// Repeated create/purge cycles must not accumulate orphaned rows.
///
/// Guards against unbounded growth of the FTS table — every purged generation
/// leaving a row behind is both a leak and a slow, permanent index bloat.
pub async fn repeated_purge_and_recreate_does_not_grow_fts<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
) where
    B: ResourceStorage + SearchProvider + PurgableStorage,
    P: FtsProbe,
{
    for cycle in 0..5 {
        backend
            .create(
                tenant,
                "Patient",
                patient_with_narrative("p1", &planted_term(tenant)),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create on cycle {cycle} should succeed: {e}"));

        assert_eq!(
            probe.fts_row_count(tenant.tenant_id().as_str()).await,
            1,
            "POSITIVE CONTROL on cycle {cycle}: exactly one row should be indexed"
        );

        backend
            .purge(tenant, "Patient", "p1")
            .await
            .unwrap_or_else(|e| panic!("purge on cycle {cycle} should succeed: {e}"));

        assert_eq!(
            probe.fts_row_count(tenant.tenant_id().as_str()).await,
            0,
            "cycle {cycle} left an orphaned resource_fts row behind"
        );
    }
}

// ===========================================================================
// $reindex must REBUILD full-text search, not destroy it
// ===========================================================================

/// Drives a reindex job to completion, panicking on failure or timeout.
async fn run_reindex_to_completion(
    reindex: &helios_persistence::search::ReindexOperation,
    tenant: &TenantContext,
    request: helios_persistence::search::ReindexRequest,
) {
    use helios_persistence::search::ReindexStatus;

    let job_id = reindex
        .start(tenant.clone(), request, None)
        .await
        .expect("reindex should start");

    for _ in 0..200 {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let progress = reindex
            .get_progress(&job_id)
            .await
            .expect("progress should be readable");
        match progress.status {
            ReindexStatus::Completed => return,
            ReindexStatus::Failed => {
                panic!("reindex failed: {:?}", progress.error_message)
            }
            _ => {}
        }
    }
    panic!("reindex timed out");
}

/// `$reindex` must leave `_text`/`_content` working.
///
/// On unfixed code this fails on **both** backends and in **both** modes:
/// `run_reindex` calls `delete_search_entries` for every resource
/// unconditionally — which does drop the `resource_fts` row — while
/// `write_search_entries` never put it back. So the documented recovery
/// operation silently disabled full-text search until each resource happened to
/// be written again. `clear_existing` was never the discriminator.
pub async fn reindex_preserves_full_text_search<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
    reindex: &helios_persistence::search::ReindexOperation,
    clear_existing: bool,
) where
    B: ResourceStorage + SearchProvider,
    P: FtsProbe,
{
    use helios_persistence::search::ReindexRequest;

    seed_and_control(backend, probe, tenant, "p1").await;

    let mut request = ReindexRequest::for_types(vec!["Patient"]);
    if clear_existing {
        request = request.clear_existing();
    }
    run_reindex_to_completion(reindex, tenant, request).await;

    assert_eq!(
        probe.fts_row_count(tenant.tenant_id().as_str()).await,
        1,
        "$reindex (clear_existing={clear_existing}) must rebuild the resource's \
         resource_fts row, not drop it"
    );
    assert_eq!(
        text_hits(backend, tenant, &planted_term(tenant)).await,
        vec!["p1".to_string()],
        "_text must still work after $reindex (clear_existing={clear_existing})"
    );
    assert_eq!(
        content_hits(backend, tenant, &planted_term(tenant)).await,
        vec!["p1".to_string()],
        "_content must still work after $reindex (clear_existing={clear_existing})"
    );
}

/// A reindex must not duplicate full-text rows.
///
/// SQLite's `index_fts_content` is a bare INSERT with no delete-first; it is
/// only safe because `run_reindex` deletes the resource's entries immediately
/// beforehand. This pins that ordering — if it is ever changed, the row count
/// doubles here rather than silently bloating production indexes.
pub async fn repeated_reindex_does_not_duplicate_fts_rows<B, P>(
    backend: &B,
    probe: &P,
    tenant: &TenantContext,
    reindex: &helios_persistence::search::ReindexOperation,
) where
    B: ResourceStorage + SearchProvider,
    P: FtsProbe,
{
    use helios_persistence::search::ReindexRequest;

    seed_and_control(backend, probe, tenant, "p1").await;

    for pass in 0..3 {
        run_reindex_to_completion(reindex, tenant, ReindexRequest::for_types(vec!["Patient"]))
            .await;
        assert_eq!(
            probe.fts_row_count(tenant.tenant_id().as_str()).await,
            1,
            "reindex pass {pass} must leave exactly one resource_fts row"
        );
    }

    assert_eq!(
        text_hits(backend, tenant, &planted_term(tenant)).await,
        vec!["p1".to_string()],
        "_text must still return exactly one hit after repeated reindexing"
    );
}
