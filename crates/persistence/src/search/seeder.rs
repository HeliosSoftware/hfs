//! Seeds storage with the FHIR spec's SearchParameter resources (#235).
//!
//! Storage — not the in-memory registry — is the source of truth for
//! SearchParameters. On startup each primary backend seeds its store from the
//! same spec bundle the registry loads, so `GET /SearchParameter` discovers
//! the parameters the server actually resolves searches against, and any node
//! in a cluster boots into the same set.
//!
//! Seeding is idempotent and safe under concurrent multi-node boots: every
//! spec resource carries its bundle id (`Patient-name`, `Resource-id`, …), so
//! a second writer's `create` fails with `AlreadyExists` and is treated as
//! "already seeded". Existing resources are never updated or clobbered.
//!
//! Resources are seeded verbatim — including the spec's `status: draft`, which
//! the registry deliberately promotes to active only when loading (see
//! `SearchParameterLoader::load_from_spec_file`). The registry keeps loading
//! spec definitions from the bundled file as `Embedded`; the stored copies
//! exist for API discovery and cluster-wide consistency, not as a second
//! registration path — the stored-parameter refresh skips them (draft, and
//! their canonical URLs are already registered).

use std::future::Future;
use std::path::Path;

use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::ResourceStorage;
use crate::error::{ResourceError, StorageError, StorageResult};
use crate::search::loader::SearchParameterLoader;
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

/// Outcome of a seeding pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedOutcome {
    /// Resources newly written by this pass.
    pub created: usize,
    /// Resources already present (same id), left untouched.
    pub existing: usize,
    /// Resources that failed to write and were skipped (logged).
    pub failed: usize,
}

/// Writes every resource of `resource_type` for `tenant`, fanning creates out
/// at the backend's [`bulk_write_concurrency`] so latency-bound backends (one
/// PUT per resource on S3, one round trip per row on networked databases) pay
/// round-trips-divided-by-fanout rather than their sum. `AlreadyExists` counts
/// as already-seeded. Any other error is retried twice with a short backoff —
/// shared-cache SQLite surfaces reader/writer overlap as an immediate `table
/// is locked` error rather than waiting — before counting as failed.
///
/// [`bulk_write_concurrency`]: ResourceStorage::bulk_write_concurrency
async fn create_all<S>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &'static str,
    resources: Vec<Value>,
    fhir_version: FhirVersion,
) -> SeedOutcome
where
    S: ResourceStorage + ?Sized,
{
    let concurrency = storage.bulk_write_concurrency().clamp(1, 32);
    write_all(
        concurrency,
        resource_type,
        resources,
        |resource| async move {
            storage
                .create(tenant, resource_type, resource, fhir_version)
                .await
                .map(|_| ())
        },
    )
    .await
}

/// The fanout/retry core of [`create_all`], parameterized over the write so it
/// can be exercised without a storage backend.
async fn write_all<F, Fut>(
    concurrency: usize,
    resource_type: &'static str,
    resources: Vec<Value>,
    create: F,
) -> SeedOutcome
where
    F: Fn(Value) -> Fut,
    Fut: Future<Output = StorageResult<()>>,
{
    use futures::stream::{self, StreamExt};

    enum Wrote {
        Created,
        Existing,
        Failed,
    }

    let create = &create;
    stream::iter(resources)
        .map(|resource| async move {
            let mut attempt: u32 = 0;
            loop {
                match create(resource.clone()).await {
                    Ok(()) => return Wrote::Created,
                    Err(StorageError::Resource(ResourceError::AlreadyExists { .. })) => {
                        return Wrote::Existing;
                    }
                    Err(_) if attempt < 2 => {
                        attempt += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(25 * attempt as u64))
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!("{resource_type} seeding: create failed: {e}");
                        return Wrote::Failed;
                    }
                }
            }
        })
        .buffer_unordered(concurrency.max(1))
        .fold(
            SeedOutcome {
                created: 0,
                existing: 0,
                failed: 0,
            },
            |mut outcome, wrote| async move {
                match wrote {
                    Wrote::Created => outcome.created += 1,
                    Wrote::Existing => outcome.existing += 1,
                    Wrote::Failed => outcome.failed += 1,
                }
                outcome
            },
        )
        .await
}

/// Seeds `storage` with the spec SearchParameter bundle for `fhir_version`,
/// plus the embedded fallback parameters, under `tenant_id`.
///
/// The tenant is the server's default tenant: searches are tenant-scoped, so
/// seeding anywhere else would leave `GET /SearchParameter` empty for the
/// common single-tenant deployment. Non-default tenants do not see the seeded
/// resources via the API (the in-memory registry still resolves searches for
/// every tenant); revisit if shared-resource search lands.
///
/// Fast path: when the tenant already holds at least as many SearchParameters
/// as the spec set, the pass is skipped entirely — one `count` per boot. A
/// partial set (interrupted seed, or user-POSTed parameters predating this
/// feature) is completed resource-by-resource, skipping whatever exists.
pub async fn seed_spec_search_parameters<S>(
    storage: &S,
    fhir_version: FhirVersion,
    data_dir: &Path,
    tenant_id: &str,
) -> StorageResult<SeedOutcome>
where
    S: ResourceStorage + ?Sized,
{
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    let loader = SearchParameterLoader::new(fhir_version);

    let mut resources: Vec<Value> = match loader.load_spec_resources(data_dir) {
        Ok(resources) => resources,
        Err(e) => {
            // No spec file is a supported minimal deployment (the registry
            // falls back the same way); seed only the embedded fallbacks.
            tracing::warn!("SearchParameter seeding: no spec bundle loaded: {e}");
            Vec::new()
        }
    };
    if let Ok(fallbacks) = loader.load_embedded() {
        resources.extend(
            fallbacks
                .iter()
                .map(SearchParameterLoader::definition_to_fhir_resource),
        );
    }
    // Drop entries whose id duplicates one already in the set. Several embedded
    // fallbacks (`Resource-id`, `Library-url`, …) share an id with a spec
    // bundle entry, so their `create` always fails `AlreadyExists` and never
    // adds a row. Left in, they inflate `resources.len()` above the count the
    // store can ever reach, so `present >= resources.len()` below would never
    // hold and every boot would re-run the full create scan instead of taking
    // the single-`count` fast path.
    let mut seen_ids = std::collections::HashSet::new();
    resources.retain(|resource| {
        resource
            .get("id")
            .and_then(|id| id.as_str())
            .is_none_or(|id| seen_ids.insert(id.to_string()))
    });
    if resources.is_empty() {
        return Ok(SeedOutcome {
            created: 0,
            existing: 0,
            failed: 0,
        });
    }

    let present = storage.count(&tenant, Some("SearchParameter")).await?;
    if present as usize >= resources.len() {
        return Ok(SeedOutcome {
            created: 0,
            existing: resources.len(),
            failed: 0,
        });
    }

    let outcome = create_all(storage, &tenant, "SearchParameter", resources, fhir_version).await;
    tracing::info!(
        created = outcome.created,
        existing = outcome.existing,
        failed = outcome.failed,
        tenant = %tenant_id,
        "Seeded spec SearchParameters into storage"
    );
    Ok(outcome)
}

/// Seeds one tenant with both spec conformance resource sets — SearchParameters
/// (#235) and CompartmentDefinitions (#237/#238) — from `data_dir`. Best-effort
/// per set: an error in one is logged and does not block the other. Returns the
/// combined outcome. Idempotent; safe to call at startup and on tenant
/// provisioning.
pub async fn seed_tenant_conformance<S>(
    storage: &S,
    fhir_version: FhirVersion,
    data_dir: &Path,
    tenant_id: &str,
) -> SeedOutcome
where
    S: ResourceStorage + ?Sized,
{
    let mut total = SeedOutcome {
        created: 0,
        existing: 0,
        failed: 0,
    };
    let mut add = |r: StorageResult<SeedOutcome>, kind: &str| match r {
        Ok(o) => {
            total.created += o.created;
            total.existing += o.existing;
            total.failed += o.failed;
        }
        Err(e) => tracing::warn!(tenant = %tenant_id, "{kind} seeding failed: {e}"),
    };
    add(
        seed_spec_search_parameters(storage, fhir_version, data_dir, tenant_id).await,
        "SearchParameter",
    );
    add(
        seed_spec_compartment_definitions(storage, fhir_version, data_dir, tenant_id).await,
        "CompartmentDefinition",
    );
    total
}

/// Seeds `storage` with the spec CompartmentDefinition bundle for
/// `fhir_version`, under `tenant_id`.
///
/// The compartment analogue of [`seed_spec_search_parameters`]: storage — not
/// the codegen'd membership table — is what `GET /CompartmentDefinition` and
/// the web UI read. Compartment *search* membership is unaffected (it still
/// resolves through `helios_fhir::get_compartment_params`); these stored copies
/// exist only for API discovery.
///
/// Same idempotency contract: every definition carries its bundle id
/// (`patient`, `encounter`, …), so a concurrent second writer's `create` fails
/// `AlreadyExists` and is treated as already-seeded; existing resources are
/// never clobbered. When the tenant already holds at least the bundle's count,
/// the pass short-circuits to a single `count`.
pub async fn seed_spec_compartment_definitions<S>(
    storage: &S,
    fhir_version: FhirVersion,
    data_dir: &Path,
    tenant_id: &str,
) -> StorageResult<SeedOutcome>
where
    S: ResourceStorage + ?Sized,
{
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    let loader = helios_fhir::compartment::CompartmentDefinitionLoader::new(fhir_version);

    let resources: Vec<Value> = match loader.load_spec_resources(data_dir) {
        Ok(resources) => resources,
        Err(e) => {
            // No bundle is a supported minimal deployment; nothing to seed.
            tracing::warn!("CompartmentDefinition seeding: no spec bundle loaded: {e}");
            Vec::new()
        }
    };
    if resources.is_empty() {
        return Ok(SeedOutcome {
            created: 0,
            existing: 0,
            failed: 0,
        });
    }

    let present = storage
        .count(&tenant, Some("CompartmentDefinition"))
        .await?;
    if present as usize >= resources.len() {
        return Ok(SeedOutcome {
            created: 0,
            existing: resources.len(),
            failed: 0,
        });
    }

    let outcome = create_all(
        storage,
        &tenant,
        "CompartmentDefinition",
        resources,
        fhir_version,
    )
    .await;
    tracing::info!(
        created = outcome.created,
        existing = outcome.existing,
        failed = outcome.failed,
        tenant = %tenant_id,
        "Seeded spec CompartmentDefinitions into storage"
    );
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;
    use crate::error::BackendError;

    fn resources(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "id": format!("r{i}") })).collect()
    }

    fn transient() -> StorageError {
        StorageError::Backend(BackendError::Internal {
            backend_name: "test".to_string(),
            message: "table is locked".to_string(),
            source: None,
        })
    }

    #[tokio::test(start_paused = true)]
    async fn counts_created_and_existing() {
        let calls = AtomicUsize::new(0);
        let outcome = write_all(4, "SearchParameter", resources(5), |resource| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let dup = resource["id"] == json!("r0");
            async move {
                let _ = n;
                if dup {
                    Err(StorageError::Resource(ResourceError::AlreadyExists {
                        resource_type: "SearchParameter".to_string(),
                        id: "r0".to_string(),
                    }))
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert_eq!(outcome.created, 4);
        assert_eq!(outcome.existing, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            5,
            "AlreadyExists is not retried"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn transient_failures_are_retried_then_succeed() {
        let calls = AtomicUsize::new(0);
        let outcome = write_all(1, "SearchParameter", resources(1), |_| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(transient())
                } else {
                    Ok(())
                }
            }
        })
        .await;
        assert_eq!(outcome.created, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_failures_count_after_three_attempts() {
        let calls = AtomicUsize::new(0);
        let outcome = write_all(2, "CompartmentDefinition", resources(2), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { Err(transient()) }
        })
        .await;
        assert_eq!(outcome.created, 0);
        assert_eq!(outcome.failed, 2);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            6,
            "initial try + two retries each"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn zero_concurrency_is_clamped_not_deadlocked() {
        let outcome = write_all(0, "SearchParameter", resources(3), |_| async { Ok(()) }).await;
        assert_eq!(outcome.created, 3);
    }
}
