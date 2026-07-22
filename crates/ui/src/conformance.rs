//! Conformance data source: how the UI reads SearchParameter and
//! CompartmentDefinition resources.
//!
//! Primary storage is the source of truth (#235/#237/#238): the server seeds
//! the spec resources into storage and serves them at `/SearchParameter` and
//! `/CompartmentDefinition`. This crate reads them from that FHIR API over
//! HTTP rather than from disk, so the UI shows exactly what the server holds.
//!
//! [`ConformanceSource`] abstracts the fetch so tests can inject a
//! [`StaticConformanceSource`] (offline, from the shipped `data/` bundles)
//! instead of standing up a real server.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde_json::Value;

/// Fetches all FHIR resources of a conformance type for a FHIR version, as the
/// raw resource JSON `Value`s (the `entry[].resource` of a searchset Bundle).
#[async_trait]
pub trait ConformanceSource: Send + Sync {
    /// Returns every resource of `resource_type` the server holds for `version`,
    /// or an `Err` message when the fetch fails (the caller degrades the page).
    async fn fetch(&self, resource_type: &str, version: FhirVersion) -> Result<Vec<Value>, String>;
}

/// Reads conformance resources from the server's own FHIR API over HTTP.
///
/// The self-call targets a loopback base URL; credentials (a service token when
/// auth is enabled, nothing when it is not) come from the injected
/// [`OutboundAuthProvider`](helios_auth::outbound::OutboundAuthProvider).
pub(crate) struct HttpConformanceSource {
    client: reqwest::Client,
    base_url: String,
    outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
}

impl HttpConformanceSource {
    pub fn new(
        base_url: String,
        outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            outbound_auth,
        }
    }
}

#[async_trait]
impl ConformanceSource for HttpConformanceSource {
    async fn fetch(
        &self,
        resource_type: &str,
        _version: FhirVersion,
    ) -> Result<Vec<Value>, String> {
        // A single page large enough to hold the whole conformance set (~1.4k
        // SearchParameters per version): the UI needs the full list for its
        // facets and rail, and paginates in-memory. Capped at 10000 — the
        // Elasticsearch max_result_window — so the search also succeeds on
        // backends that delegate search to ES.
        let url = format!("{}/{}?_count=10000", self.base_url, resource_type);
        let request = self
            .client
            .get(&url)
            .header("Accept", "application/fhir+json");
        let request = self
            .outbound_auth
            .authorize(request, &self.base_url)
            .await
            .map_err(|e| format!("outbound auth failed: {e}"))?;
        let response = request
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("{url} returned {}", response.status()));
        }
        let bundle: Value = response
            .json()
            .await
            .map_err(|e| format!("parsing {resource_type} bundle failed: {e}"))?;
        Ok(extract_bundle_resources(&bundle))
    }
}

/// Pulls `entry[].resource` out of a searchset Bundle.
fn extract_bundle_resources(bundle: &Value) -> Vec<Value> {
    bundle
        .get("entry")
        .and_then(|e| e.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("resource").cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// In-memory conformance source for tests: returns whatever resources it was
/// seeded with, keyed by `(resource_type, version)`. Offline and deterministic.
#[doc(hidden)]
pub struct StaticConformanceSource {
    map: HashMap<(String, FhirVersion), Vec<Value>>,
}

impl StaticConformanceSource {
    /// An empty source: every fetch returns no resources (a degraded page).
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Seeds one `(resource_type, version)` slot.
    pub fn with(
        mut self,
        resource_type: &str,
        version: FhirVersion,
        resources: Vec<Value>,
    ) -> Self {
        self.map
            .insert((resource_type.to_string(), version), resources);
        self
    }

    /// Loads the shipped `data/` spec bundles for every enabled version, so a
    /// test exercises the real handlers and view models against real data
    /// without a running server. `data_dir` is the repo `data/` directory.
    pub fn from_data_dir(data_dir: &std::path::Path) -> Self {
        use helios_fhir::compartment::CompartmentDefinitionLoader;
        use helios_fhir::search::SearchParameterLoader;

        let mut source = Self::empty();
        for version in crate::search_params::enabled_versions() {
            if let Ok(params) = SearchParameterLoader::new(version).load_spec_resources(data_dir) {
                source = source.with("SearchParameter", version, params);
            }
            if let Ok(defs) =
                CompartmentDefinitionLoader::new(version).load_spec_resources(data_dir)
            {
                source = source.with("CompartmentDefinition", version, defs);
            }
        }
        source
    }
}

#[async_trait]
impl ConformanceSource for StaticConformanceSource {
    async fn fetch(&self, resource_type: &str, version: FhirVersion) -> Result<Vec<Value>, String> {
        Ok(self
            .map
            .get(&(resource_type.to_string(), version))
            .cloned()
            .unwrap_or_default())
    }
}
