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

    let mut outcome = SeedOutcome {
        created: 0,
        existing: 0,
        failed: 0,
    };
    for resource in resources {
        match storage
            .create(&tenant, "SearchParameter", resource, fhir_version)
            .await
        {
            Ok(_) => outcome.created += 1,
            Err(StorageError::Resource(ResourceError::AlreadyExists { .. })) => {
                outcome.existing += 1;
            }
            Err(e) => {
                outcome.failed += 1;
                tracing::warn!("SearchParameter seeding: create failed: {e}");
            }
        }
    }
    tracing::info!(
        created = outcome.created,
        existing = outcome.existing,
        failed = outcome.failed,
        tenant = %tenant_id,
        "Seeded spec SearchParameters into storage"
    );
    Ok(outcome)
}
