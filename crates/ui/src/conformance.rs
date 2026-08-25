//! Conformance data source: how the UI reads SearchParameter and
//! CompartmentDefinition resources.
//!
//! Primary storage is the source of truth (#235/#237/#238): the server seeds
//! the spec resources into storage and serves them at `/SearchParameter` and
//! `/CompartmentDefinition`. This crate reads them from that FHIR API over
//! HTTP rather than from disk, so the UI shows exactly what the server holds.
//!
//! Storage only ever holds the server's default FHIR version (seeding is
//! keyed on `HFS_DEFAULT_FHIR_VERSION`), so a fetch for any *other* enabled
//! version answers from the shipped `data/` spec bundles instead (#562) —
//! the correct per-version spec set, minus tenant-stored custom resources,
//! which only exist for the seeded default.
//!
//! [`ConformanceSource`] abstracts the fetch so tests can inject a
//! [`StaticConformanceSource`] (offline, from the shipped `data/` bundles)
//! instead of standing up a real server.

use std::collections::HashMap;
use std::path::PathBuf;
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
    async fn fetch(
        &self,
        resource_type: &str,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Vec<Value>, String>;

    /// The server's live CapabilityStatement for `version` and `tenant`
    /// (`GET /metadata`, #653), or an `Err` message when the fetch fails —
    /// the page degrades, it never fabricates capabilities.
    async fn metadata(&self, version: FhirVersion, tenant: &str) -> Result<Value, String> {
        let _ = (version, tenant);
        Err("metadata is not available from this source".to_string())
    }

    /// Runs `$sql-run` with `view_definition` as the inline subject and
    /// returns the output rows (`_format=json`), or an `Err` message the page
    /// shows in place of the results table (#649).
    async fn sql_run(
        &self,
        view_definition: &Value,
        limit: usize,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Vec<Value>, String> {
        let _ = (view_definition, limit, version, tenant);
        Err("$sql-run is not available from this source".to_string())
    }

    /// Creates (`id: None`) or updates one resource through the server's own
    /// FHIR API and returns the stored resource. Backs the ViewDefinition
    /// editor's plain-form save, which must work without JavaScript (#649).
    async fn save_resource(
        &self,
        resource_type: &str,
        id: Option<&str>,
        resource: Value,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Value, String> {
        let _ = (resource_type, id, resource, version, tenant);
        Err("saving is not available from this source".to_string())
    }
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
    /// The version the server seeds into storage (`HFS_DEFAULT_FHIR_VERSION`).
    /// Fetches for this version go over HTTP; any other enabled version
    /// answers from the spec bundles in `data_dir` (#562).
    default_version: FhirVersion,
    data_dir: Option<PathBuf>,
}

impl HttpConformanceSource {
    pub fn new(
        base_url: String,
        outbound_auth: Arc<dyn helios_auth::outbound::OutboundAuthProvider>,
        default_version: FhirVersion,
        data_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            outbound_auth,
            default_version,
            data_dir,
        }
    }

    /// Loads `resource_type` for a non-default `version` from the shipped
    /// `data/` spec bundles. Storage cannot answer these: seeding only writes
    /// the default version, and the FHIR search surface has no per-version
    /// filter, so the bundle is the authoritative set for the version.
    async fn fetch_spec_bundle(
        &self,
        resource_type: &str,
        version: FhirVersion,
    ) -> Result<Vec<Value>, String> {
        let Some(dir) = self.data_dir.clone() else {
            return Err(format!(
                "no {resource_type} data for FHIR {}: storage holds the server default ({}) and no data directory is configured",
                version.as_str(),
                self.default_version.as_str(),
            ));
        };
        let resource_type = resource_type.to_string();
        tokio::task::spawn_blocking(move || match resource_type.as_str() {
            "SearchParameter" => helios_fhir::search::SearchParameterLoader::new(version)
                .load_spec_resources(&dir)
                .map_err(|e| {
                    format!(
                        "loading the FHIR {} SearchParameter bundle failed: {e}",
                        version.as_str()
                    )
                }),
            "CompartmentDefinition" => {
                helios_fhir::compartment::CompartmentDefinitionLoader::new(version)
                    .load_spec_resources(&dir)
                    .map_err(|e| {
                        format!(
                            "loading the FHIR {} CompartmentDefinition bundle failed: {e}",
                            version.as_str()
                        )
                    })
            }
            other => Err(format!("unsupported conformance type {other}")),
        })
        .await
        .map_err(|e| format!("spec bundle load failed: {e}"))?
    }
}

#[async_trait]
impl ConformanceSource for HttpConformanceSource {
    /// `GET /metadata` on the loopback base (#653). `/metadata` composes the
    /// statement fresh from live server state and selects the described
    /// version from the Accept header's `fhirVersion` parameter, so the
    /// sidebar's selection rides the request — no spec-bundle fallback here,
    /// the endpoint itself is version-aware.
    async fn metadata(&self, version: FhirVersion, tenant: &str) -> Result<Value, String> {
        let url = format!("{}/metadata", self.base_url);
        let mut request = self.client.get(&url).header(
            "Accept",
            format!(
                "application/fhir+json; fhirVersion={}",
                version.as_mime_param()
            ),
        );
        if !tenant.is_empty() {
            request = request.header("X-Tenant-ID", tenant);
        }
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
        response
            .json()
            .await
            .map_err(|e| format!("parsing the CapabilityStatement failed: {e}"))
    }

    async fn fetch(
        &self,
        resource_type: &str,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Vec<Value>, String> {
        // Storage only holds the seeded default version; any other enabled
        // version answers from the shipped spec bundles (#562).
        if version != self.default_version {
            return self.fetch_spec_bundle(resource_type, version).await;
        }
        // Ask for everything at once, then follow `next` links for whatever
        // the server's `_count` policy withheld (#460): the request says
        // 10000, but a server capping at 1000 used to silently truncate the
        // registry (1377 R4 SearchParameters served as 1000). The UI needs
        // the full list for its facets and rail, and paginates in-memory.
        let mut url = format!("{}/{}?_count=10000", self.base_url, resource_type);
        let mut resources = Vec::new();
        // Generous page bound — only a runaway self-linking server hits it.
        for _ in 0..100 {
            let mut request = self.client.get(&url).header(
                "Accept",
                format!(
                    "application/fhir+json; fhirVersion={}",
                    version.as_mime_param()
                ),
            );
            // Scope the self-call to the effective tenant (#344); an empty id
            // means the server default and needs no header.
            if !tenant.is_empty() {
                request = request.header("X-Tenant-ID", tenant);
            }
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
                // A failed page fails the fetch: serving a silently partial
                // registry is exactly the bug this loop exists to fix.
                return Err(format!("{url} returned {}", response.status()));
            }
            let bundle: Value = response
                .json()
                .await
                .map_err(|e| format!("parsing {resource_type} bundle failed: {e}"))?;
            resources.extend(extract_bundle_resources(&bundle));
            match next_link(&bundle) {
                // The advertised link carries the server's own idea of its base
                // URL, which need not be the loopback this client targets —
                // keep the path + query, swap in our base.
                Some(next) => url = rebase_link(&next, &self.base_url),
                None => break,
            }
        }
        Ok(resources)
    }

    /// `POST /$sql-run` on the loopback base with the ViewDefinition as the
    /// inline subject (the handler's raw-resource shorthand for a Parameters
    /// body). Storage holds the seeded default version only, so any other
    /// version degrades with a message rather than running against the wrong
    /// data.
    async fn sql_run(
        &self,
        view_definition: &Value,
        limit: usize,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Vec<Value>, String> {
        if version != self.default_version {
            return Err(format!(
                "$sql-run reads stored data, which holds the server default (FHIR {}); switch the sidebar back to run this view",
                self.default_version.as_str(),
            ));
        }
        let url = format!("{}/$sql-run?_format=json&_limit={limit}", self.base_url);
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(view_definition);
        if !tenant.is_empty() {
            request = request.header("X-Tenant-ID", tenant);
        }
        let request = self
            .outbound_auth
            .authorize(request, &self.base_url)
            .await
            .map_err(|e| format!("outbound auth failed: {e}"))?;
        let response = request
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            // $sql-run explains its 4xx in an OperationOutcome; surface its
            // diagnostics so an invalid view reads as more than a status code.
            let detail = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|o| outcome_diagnostics(&o))
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            return Err(format!("$sql-run returned {status}{detail}"));
        }
        response
            .json::<Vec<Value>>()
            .await
            .map_err(|e| format!("parsing $sql-run rows failed: {e}"))
    }

    async fn save_resource(
        &self,
        resource_type: &str,
        id: Option<&str>,
        resource: Value,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Value, String> {
        let url = match id {
            Some(id) => format!("{}/{resource_type}/{id}", self.base_url),
            None => format!("{}/{resource_type}", self.base_url),
        };
        let accept = format!(
            "application/fhir+json; fhirVersion={}",
            version.as_mime_param()
        );
        let mut request = match id {
            Some(_) => self.client.put(&url),
            None => self.client.post(&url),
        }
        .header("Content-Type", accept.clone())
        .header("Accept", accept)
        .json(&resource);
        if !tenant.is_empty() {
            request = request.header("X-Tenant-ID", tenant);
        }
        let request = self
            .outbound_auth
            .authorize(request, &self.base_url)
            .await
            .map_err(|e| format!("outbound auth failed: {e}"))?;
        let response = request
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            let detail = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|o| outcome_diagnostics(&o))
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            return Err(format!("saving to {url} returned {status}{detail}"));
        }
        response
            .json()
            .await
            .map_err(|e| format!("parsing the saved resource failed: {e}"))
    }
}

/// The first issue diagnostics of an OperationOutcome, if that is what this is.
fn outcome_diagnostics(outcome: &Value) -> Option<String> {
    outcome
        .get("issue")?
        .as_array()?
        .iter()
        .find_map(|i| i.get("diagnostics").and_then(Value::as_str))
        .map(String::from)
}

/// The `next` page URL of a searchset Bundle, if any.
fn next_link(bundle: &Value) -> Option<String> {
    bundle
        .get("link")?
        .as_array()?
        .iter()
        .find(|l| l.get("relation").and_then(Value::as_str) == Some("next"))?
        .get("url")?
        .as_str()
        .map(String::from)
}

/// Points a server-advertised link at `base_url`, keeping its path and query.
fn rebase_link(link: &str, base_url: &str) -> String {
    match link.find("://").and_then(|i| link[i + 3..].find('/')) {
        Some(slash) => {
            let i = link.find("://").unwrap() + 3;
            format!("{}{}", base_url, &link[i + slash..])
        }
        None => link.to_string(),
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
    metadata: Option<Value>,
    sql_rows: Option<Result<Vec<Value>, String>>,
}

impl StaticConformanceSource {
    /// An empty source: every fetch returns no resources (a degraded page).
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            metadata: None,
            sql_rows: None,
        }
    }

    /// Seeds the CapabilityStatement `metadata()` answers with (#653).
    pub fn with_metadata(mut self, statement: Value) -> Self {
        self.metadata = Some(statement);
        self
    }

    /// Seeds what `sql_run()` answers (#649): `Ok` rows or the `Err` message.
    pub fn with_sql_run(mut self, outcome: Result<Vec<Value>, String>) -> Self {
        self.sql_rows = Some(outcome);
        self
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
    async fn fetch(
        &self,
        resource_type: &str,
        version: FhirVersion,
        _tenant: &str,
    ) -> Result<Vec<Value>, String> {
        Ok(self
            .map
            .get(&(resource_type.to_string(), version))
            .cloned()
            .unwrap_or_default())
    }

    async fn metadata(&self, _version: FhirVersion, _tenant: &str) -> Result<Value, String> {
        self.metadata
            .clone()
            .ok_or_else(|| "no metadata seeded".to_string())
    }

    async fn sql_run(
        &self,
        _view_definition: &Value,
        limit: usize,
        _version: FhirVersion,
        _tenant: &str,
    ) -> Result<Vec<Value>, String> {
        match &self.sql_rows {
            Some(Ok(rows)) => Ok(rows.iter().take(limit).cloned().collect()),
            Some(Err(e)) => Err(e.clone()),
            None => Err("no $sql-run outcome seeded".to_string()),
        }
    }

    /// Echoes the resource back with an id, so the save handler's redirect
    /// (and a test's assertion on it) has something stable to point at.
    async fn save_resource(
        &self,
        _resource_type: &str,
        id: Option<&str>,
        mut resource: Value,
        _version: FhirVersion,
        _tenant: &str,
    ) -> Result<Value, String> {
        let id = id.unwrap_or("static-created").to_string();
        if let Some(map) = resource.as_object_mut() {
            map.insert("id".to_string(), Value::String(id));
        }
        Ok(resource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_link_finds_the_next_relation() {
        let bundle = serde_json::json!({
            "link": [
                {"relation": "self", "url": "http://s/SearchParameter?_count=1000"},
                {"relation": "next", "url": "http://s/SearchParameter?_count=1000&_offset=1000"}
            ]
        });
        assert_eq!(
            next_link(&bundle).as_deref(),
            Some("http://s/SearchParameter?_count=1000&_offset=1000")
        );
        assert_eq!(next_link(&serde_json::json!({"link": []})), None);
    }

    #[test]
    fn rebase_link_swaps_the_advertised_base_for_ours() {
        assert_eq!(
            rebase_link(
                "http://localhost:8080/SearchParameter?_offset=1000",
                "http://127.0.0.1:9999"
            ),
            "http://127.0.0.1:9999/SearchParameter?_offset=1000"
        );
    }

    /// #562: a fetch for a version other than the server default answers from
    /// the shipped spec bundles without touching HTTP — nothing listens at the
    /// base URL, so reaching for it would fail the fetch.
    #[cfg(feature = "R4B")]
    #[tokio::test]
    async fn non_default_versions_answer_from_the_spec_bundles() {
        let source = HttpConformanceSource::new(
            "http://127.0.0.1:1".to_string(),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            Some(std::path::PathBuf::from("../../data")),
        );
        let defs = source
            .fetch("CompartmentDefinition", FhirVersion::R4B, "")
            .await
            .expect("spec bundle load");
        let patient = defs
            .iter()
            .find(|d| d["code"] == "Patient")
            .expect("patient compartment");
        let codes: Vec<&str> = patient["resource"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["code"].as_str())
            .collect();
        // The set is genuinely the R4B one: Ingredient joined in R4B,
        // EffectEvidenceSynthesis left after R4.
        assert!(codes.contains(&"Ingredient"));
        assert!(!codes.contains(&"EffectEvidenceSynthesis"));
    }

    /// #562: with no data directory a non-default version cannot be served —
    /// the fetch fails loudly (degraded page) instead of silently answering
    /// with the wrong version's set.
    #[cfg(feature = "R4B")]
    #[tokio::test]
    async fn non_default_version_without_a_data_dir_fails_loudly() {
        let source = HttpConformanceSource::new(
            "http://127.0.0.1:1".to_string(),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );
        let err = source
            .fetch("CompartmentDefinition", FhirVersion::R4B, "")
            .await
            .expect_err("no data dir to answer from");
        assert!(err.contains(FhirVersion::R4B.as_str()), "{err}");
    }

    /// #460: a server that caps `_count` answers in pages; the fetch must
    /// follow `next` links and return the union, not the first page.
    /// The self-call also names the requested version on the Accept header
    /// (#562) — the handler rejects a request without it.
    #[tokio::test]
    async fn http_fetch_follows_next_links() {
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::{Router, extract::Query, routing::get};
        use std::collections::HashMap;

        async fn page(
            headers: HeaderMap,
            Query(q): Query<HashMap<String, String>>,
        ) -> axum::response::Response {
            let accept = headers
                .get("accept")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            if !accept.contains("fhirVersion=4.0") {
                return axum::http::StatusCode::NOT_ACCEPTABLE.into_response();
            }
            inner(q).into_response()
        }

        fn inner(q: HashMap<String, String>) -> axum::Json<Value> {
            let offset: usize = q.get("_offset").map(|o| o.parse().unwrap()).unwrap_or(0);
            let mut bundle = serde_json::json!({
                "resourceType": "Bundle",
                "type": "searchset",
                "entry": [{"resource": {"resourceType": "SearchParameter", "id": format!("sp-{offset}")}}]
            });
            if offset == 0 {
                // Advertise the next page under a base URL that is not the
                // one the client dialed, like a server with a configured
                // public base does.
                bundle["link"] = serde_json::json!([
                    {"relation": "next", "url": "http://advertised.invalid/SearchParameter?_offset=1"}
                ]);
            }
            axum::Json(bundle)
        }

        let app = Router::new().route("/SearchParameter", get(page));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );
        let resources = source
            .fetch("SearchParameter", FhirVersion::R4, "")
            .await
            .expect("fetch succeeds");
        let ids: Vec<_> = resources
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["sp-0", "sp-1"]);
    }
}
