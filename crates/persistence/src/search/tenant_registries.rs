//! Per-tenant SearchParameter registries.
//!
//! Search-parameter resolution is tenant-scoped: `acme1` may POST a custom
//! `SearchParameter` (or override a standard one via `(base, code)` shadowing,
//! see #239/#242) that `acme2` must not see, and the FHIR search API must
//! behave differently per tenant id.
//!
//! The model is **shared base + per-tenant overlay**:
//! - a single **base** [`SearchParameterRegistry`] holds the params that are
//!   identical for every tenant — the embedded fallbacks, the spec bundle, and
//!   any custom-directory params — loaded once at backend construction;
//! - each tenant's registry is `base.clone()` plus that tenant's **stored**
//!   (POSTed) active params, registered on top so they shadow the base by
//!   precedence. Cloning the base is cheap (both indexes hold
//!   `Arc<SearchParameterDefinition>`).
//!
//! Per-tenant registries are built lazily on first use and cached for the
//! process lifetime; a tenant's registry is dropped (rebuilt on next access)
//! when that tenant writes a `SearchParameter` or the TTL refresh fires. The
//! **loader** closure — supplied by the primary backend — reads a tenant's
//! stored params from storage; it captures the connection pool, not the
//! backend, so an Elasticsearch backend that *shares* this container resolves
//! per-tenant params through the primary's storage without its own DB handle.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::search::{SearchParameterDefinition, SearchParameterRegistry};

/// Loads a tenant's stored (POSTed) active SearchParameter definitions from
/// storage. Returns an empty vec when the tenant has none (or the backend has
/// no store, e.g. S3).
pub type StoredParamLoader = Arc<dyn Fn(&str) -> Vec<SearchParameterDefinition> + Send + Sync>;

/// A shared base registry plus lazily-built, cached per-tenant registries.
///
/// Backends hold `Arc<TenantSearchRegistries>` in place of the former single
/// `Arc<RwLock<SearchParameterRegistry>>`. [`for_tenant`](Self::for_tenant) is
/// the per-tenant analogue of what `SearchProvider::search_param_registry`
/// returns.
pub struct TenantSearchRegistries {
    /// Params identical for every tenant (embedded + spec + custom). Populated
    /// at construction via [`base`](Self::base); read-mostly thereafter.
    base: Arc<RwLock<SearchParameterRegistry>>,
    /// Per-tenant registries (`base` clone + tenant stored overlay), keyed by
    /// tenant id. Built on first access, dropped on invalidation.
    per_tenant: RwLock<HashMap<String, Arc<RwLock<SearchParameterRegistry>>>>,
    /// Reads a tenant's stored active params from storage.
    loader: StoredParamLoader,
}

impl TenantSearchRegistries {
    /// Creates a container over an empty base and the given stored-param loader.
    /// Callers populate the base via [`base`](Self::base) immediately after.
    pub fn new(loader: StoredParamLoader) -> Self {
        Self {
            base: Arc::new(RwLock::new(SearchParameterRegistry::new())),
            per_tenant: RwLock::new(HashMap::new()),
            loader,
        }
    }

    /// Creates a container whose tenants never have stored overlays — every
    /// tenant sees exactly the base. Used by backends without a store (S3).
    pub fn base_only() -> Self {
        Self::new(Arc::new(|_tenant: &str| Vec::new()))
    }

    /// The shared base registry. Construction-time loading (embedded/spec/custom)
    /// registers into this; any change here invalidates the per-tenant cache so
    /// the overlays pick it up.
    pub fn base(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        &self.base
    }

    /// Returns the registry for `tenant_id`, building and caching it on first
    /// use: a clone of the base with the tenant's stored active params overlaid.
    pub fn for_tenant(&self, tenant_id: &str) -> Arc<RwLock<SearchParameterRegistry>> {
        if let Some(reg) = self.per_tenant.read().get(tenant_id) {
            return reg.clone();
        }
        // Build outside the map lock. A concurrent builder for the same tenant
        // is harmless — the last writer wins and both hold equivalent content.
        let mut reg = self.base.read().clone();
        for def in (self.loader)(tenant_id) {
            // A stored param may legitimately shadow a base spec param; ignore
            // duplicate-url rejections (already-registered canonical URLs).
            let _ = reg.register(def);
        }
        let arc = Arc::new(RwLock::new(reg));
        self.per_tenant
            .write()
            .insert(tenant_id.to_string(), arc.clone());
        arc
    }

    /// Whether creating this `SearchParameter` resource can change a tenant's
    /// overlay. Only `status: active` params are registered into per-tenant
    /// registries, so a create of any other status need not invalidate the
    /// cache. Neither can an active param whose canonical `url` is already
    /// registered in the shared base: the overlay rebuild ignores duplicate
    /// URLs, so registering it is a no-op. That second check is what keeps
    /// spec seeding from triggering an O(n²) rebuild storm — every seeded copy
    /// is in the base by construction, and R6's bundle ships them as `active`
    /// (R4's are all `draft`), so the status check alone does not cover it
    /// (#667). Update/delete are rare and invalidate unconditionally.
    pub fn create_affects_overlay(&self, resource: &serde_json::Value) -> bool {
        if resource.get("status").and_then(|s| s.as_str()) != Some("active") {
            return false;
        }
        match resource.get("url").and_then(|u| u.as_str()) {
            Some(url) => self.base.read().get_by_url(url).is_none(),
            None => true,
        }
    }

    /// Drops a tenant's cached registry so the next access rebuilds it from
    /// storage. Called when that tenant writes a `SearchParameter`.
    pub fn invalidate(&self, tenant_id: &str) {
        self.per_tenant.write().remove(tenant_id);
    }

    /// Drops every cached per-tenant registry (the TTL refresh: storage is the
    /// source of truth, so each tenant's overlay is re-read on next access).
    pub fn invalidate_all(&self) {
        self.per_tenant.write().clear();
    }

    /// Number of currently-cached tenant registries (for diagnostics/tests).
    pub fn cached_tenant_count(&self) -> usize {
        self.per_tenant.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchParameterSource, SearchParameterStatus};
    use crate::types::SearchParamType;

    fn def(
        url: &str,
        base: &str,
        code: &str,
        source: SearchParameterSource,
    ) -> SearchParameterDefinition {
        SearchParameterDefinition::new(url, code, SearchParamType::String, format!("{base}.{code}"))
            .with_base([base])
            .with_source(source)
            .with_status(SearchParameterStatus::Active)
    }

    fn registries_with(
        stored: HashMap<String, Vec<SearchParameterDefinition>>,
    ) -> TenantSearchRegistries {
        let stored = Arc::new(stored);
        let regs = TenantSearchRegistries::new(Arc::new(move |t: &str| {
            stored.get(t).cloned().unwrap_or_default()
        }));
        // Base: one standard param present for all tenants.
        regs.base()
            .write()
            .register(def(
                "http://hl7.org/fhir/SearchParameter/Patient-name",
                "Patient",
                "name",
                SearchParameterSource::Embedded,
            ))
            .unwrap();
        regs
    }

    #[test]
    fn every_tenant_sees_the_shared_base() {
        let regs = registries_with(HashMap::new());
        for t in ["acme1", "acme2", "default"] {
            let reg = regs.for_tenant(t);
            assert!(
                reg.read().get_param("Patient", "name").is_some(),
                "{t} missing base param"
            );
        }
    }

    #[test]
    fn a_tenants_stored_param_is_isolated() {
        let mut stored = HashMap::new();
        stored.insert(
            "acme1".to_string(),
            vec![def(
                "http://acme.health/fhir/SearchParameter/patient-nickname",
                "Patient",
                "nickname",
                SearchParameterSource::Stored,
            )],
        );
        let regs = registries_with(stored);

        assert!(
            regs.for_tenant("acme1")
                .read()
                .get_param("Patient", "nickname")
                .is_some()
        );
        assert!(
            regs.for_tenant("acme2")
                .read()
                .get_param("Patient", "nickname")
                .is_none()
        );
        // Both still have the shared base param.
        assert!(
            regs.for_tenant("acme2")
                .read()
                .get_param("Patient", "name")
                .is_some()
        );
    }

    #[test]
    fn create_affects_overlay_requires_active_status_and_a_url_new_to_the_base() {
        let regs = registries_with(HashMap::new());
        let param = |status: &str, url: &str| {
            serde_json::json!({
                "resourceType": "SearchParameter",
                "status": status,
                "url": url,
                "code": "name",
                "base": ["Patient"],
            })
        };

        // The R4 spec-bundle shape: seeded copies are draft.
        assert!(!regs.create_affects_overlay(&param(
            "draft",
            "http://acme.health/fhir/SearchParameter/patient-nickname"
        )));
        // The R6 spec-bundle shape (#667): seeded copies are active, but their
        // canonical URL is already registered in the base.
        assert!(!regs.create_affects_overlay(&param(
            "active",
            "http://hl7.org/fhir/SearchParameter/Patient-name"
        )));
        // A user-POSTed active param with a new URL changes the overlay.
        assert!(regs.create_affects_overlay(&param(
            "active",
            "http://acme.health/fhir/SearchParameter/patient-nickname"
        )));
        // No URL at all: invalidate rather than guess.
        assert!(regs.create_affects_overlay(&serde_json::json!({
            "resourceType": "SearchParameter",
            "status": "active",
        })));
    }

    #[test]
    fn invalidate_rebuilds_from_loader() {
        // A mutable-ish loader via Arc<RwLock<..>> to simulate a storage change.
        let store: Arc<RwLock<HashMap<String, Vec<SearchParameterDefinition>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let store2 = store.clone();
        let regs = TenantSearchRegistries::new(Arc::new(move |t: &str| {
            store2.read().get(t).cloned().unwrap_or_default()
        }));
        regs.base()
            .write()
            .register(def(
                "http://hl7.org/fhir/SearchParameter/Patient-name",
                "Patient",
                "name",
                SearchParameterSource::Embedded,
            ))
            .unwrap();

        assert!(
            regs.for_tenant("acme1")
                .read()
                .get_param("Patient", "nickname")
                .is_none()
        );
        // Storage gains a param for acme1; visible only after invalidation.
        store.write().insert(
            "acme1".to_string(),
            vec![def(
                "http://acme.health/fhir/SearchParameter/patient-nickname",
                "Patient",
                "nickname",
                SearchParameterSource::Stored,
            )],
        );
        assert!(
            regs.for_tenant("acme1")
                .read()
                .get_param("Patient", "nickname")
                .is_none(),
            "cached"
        );
        regs.invalidate("acme1");
        assert!(
            regs.for_tenant("acme1")
                .read()
                .get_param("Patient", "nickname")
                .is_some(),
            "rebuilt"
        );
    }
}
