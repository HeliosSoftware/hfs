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

    /// Submits a `$sql-export` job over `(output name, reference)` subjects
    /// and returns the job id from its `Content-Location` (#649).
    async fn sql_export_start(
        &self,
        subjects: &[(String, String)],
        format: &str,
        tenant: &str,
    ) -> Result<String, String> {
        let _ = (subjects, format, tenant);
        Err("$sql-export is not available from this source".to_string())
    }

    /// Polls a job's status URL (#649).
    async fn sql_export_status(&self, job_id: &str, tenant: &str) -> SqlExportStatus {
        let _ = (job_id, tenant);
        SqlExportStatus::Unknown
    }

    /// Cancels a job; `Ok` when the server accepted the cancellation (#649).
    async fn sql_export_cancel(&self, job_id: &str, tenant: &str) -> Result<(), String> {
        let _ = (job_id, tenant);
        Err("$sql-export is not available from this source".to_string())
    }

    /// The completion manifest `Parameters` of a finished job (#649).
    async fn sql_export_manifest(&self, job_id: &str, tenant: &str) -> Result<Value, String> {
        let _ = (job_id, tenant);
        Err("$sql-export is not available from this source".to_string())
    }

    /// Runs `GET {type}?{params}&_count={count}&_offset={offset}` and returns
    /// the resulting page plus whether the server advertised a `next` link.
    /// Unlike [`fetch`](ConformanceSource::fetch), which pulls the whole
    /// collection into memory, this issues exactly one request per page so a
    /// page can filter and paginate server-side (#741).
    async fn search_page(
        &self,
        resource_type: &str,
        params: &[(String, String)],
        count: usize,
        offset: usize,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<SearchPage, String> {
        let _ = (resource_type, params, count, offset, version, tenant);
        Err("search is not available from this source".to_string())
    }

    /// `GET /{resource_type}/{id}`, or an `Err` message when it fails —
    /// including a 404, surfaced as text rather than `Option` so the caller
    /// can show the reason without a second request (#741).
    async fn read_resource(
        &self,
        resource_type: &str,
        id: &str,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Value, String> {
        let _ = (resource_type, id, version, tenant);
        Err("reading a resource by id is not available from this source".to_string())
    }
}

/// One page of a server-side search (#741): the resources that page holds,
/// plus whether the server advertised a `next` link — the UI uses this to
/// decide whether to render a "next" control without fetching an extra page
/// just to find out.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    pub resources: Vec<Value>,
    pub has_next: bool,
}

/// What a `$sql-export` status poll answered (#649).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlExportStatus {
    /// `202` — still running, with the `X-Progress` header when sent.
    Running(Option<String>),
    /// `303` — finished (the manifest tells success from failure).
    Done,
    /// `404` — unknown, cancelled, or reclaimed.
    Unknown,
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
            // Redirects stay manual: the async export pattern's status poll
            // answers `303 See Other` when a job finishes, and following it
            // would make "done" indistinguishable from "running" (#649). No
            // other self-call relies on HTTP redirects.
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client builds"),
            base_url: base_url.trim_end_matches('/').to_string(),
            outbound_auth,
            default_version,
            data_dir,
        }
    }

    /// A request with the tenant header and outbound auth applied.
    async fn authorized(
        &self,
        request: reqwest::RequestBuilder,
        tenant: &str,
    ) -> Result<reqwest::RequestBuilder, String> {
        let request = if tenant.is_empty() {
            request
        } else {
            request.header("X-Tenant-ID", tenant)
        };
        self.outbound_auth
            .authorize(request, &self.base_url)
            .await
            .map_err(|e| format!("outbound auth failed: {e}"))
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
        // The Parameters envelope, not the raw-resource shorthand: the
        // shorthand is ViewDefinition-only, while `subjectResource` carries
        // Library subjects (SQL Queries / SQL Views) just the same.
        let body = serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [{ "name": "subjectResource", "resource": view_definition }],
        });
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body);
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

    /// `GET {base}/{resource_type}?{params}&_count={count}&_offset={offset}`
    /// on the loopback base, a single request per page. Storage holds the
    /// seeded default version only, so a non-default version degrades the
    /// same way `sql_run` does rather than answering from the spec bundles —
    /// those bundles have no search index to page over, and ViewDefinitions
    /// (the first caller of this method) only exist in storage anyway.
    async fn search_page(
        &self,
        resource_type: &str,
        params: &[(String, String)],
        count: usize,
        offset: usize,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<SearchPage, String> {
        if version != self.default_version {
            return Err(format!(
                "search reads stored data, which holds the server default (FHIR {}); switch the sidebar back to search this type",
                self.default_version.as_str(),
            ));
        }
        let url = format!("{}/{resource_type}", self.base_url);
        let mut query: Vec<(String, String)> = params.to_vec();
        query.push(("_count".to_string(), count.to_string()));
        query.push(("_offset".to_string(), offset.to_string()));
        let request = self.client.get(&url).query(&query).header(
            "Accept",
            format!(
                "application/fhir+json; fhirVersion={}",
                version.as_mime_param()
            ),
        );
        let request = self.authorized(request, tenant).await?;
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
            .map_err(|e| format!("parsing {resource_type} search results failed: {e}"))?;
        Ok(SearchPage {
            resources: extract_bundle_resources(&bundle),
            has_next: next_link(&bundle).is_some(),
        })
    }

    /// `GET {base}/{resource_type}/{id}` on the loopback base. Same
    /// non-default-version degradation as [`search_page`](Self::search_page):
    /// storage only holds the server default.
    async fn read_resource(
        &self,
        resource_type: &str,
        id: &str,
        version: FhirVersion,
        tenant: &str,
    ) -> Result<Value, String> {
        if version != self.default_version {
            return Err(format!(
                "reading {resource_type}/{id} reads stored data, which holds the server default (FHIR {}); switch the sidebar back to read this resource",
                self.default_version.as_str(),
            ));
        }
        let url = format!("{}/{resource_type}/{id}", self.base_url);
        let request = self.client.get(&url).header(
            "Accept",
            format!(
                "application/fhir+json; fhirVersion={}",
                version.as_mime_param()
            ),
        );
        let request = self.authorized(request, tenant).await?;
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
            .map_err(|e| format!("parsing {resource_type}/{id} failed: {e}"))
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

    async fn sql_export_start(
        &self,
        subjects: &[(String, String)],
        format: &str,
        tenant: &str,
    ) -> Result<String, String> {
        self.export_start(subjects, format, tenant).await
    }

    async fn sql_export_status(&self, job_id: &str, tenant: &str) -> SqlExportStatus {
        self.export_status(job_id, tenant).await
    }

    async fn sql_export_cancel(&self, job_id: &str, tenant: &str) -> Result<(), String> {
        self.export_cancel(job_id, tenant).await
    }

    async fn sql_export_manifest(&self, job_id: &str, tenant: &str) -> Result<Value, String> {
        self.export_manifest(job_id, tenant).await
    }
}

impl HttpConformanceSource {
    async fn export_start(
        &self,
        subjects: &[(String, String)],
        format: &str,
        tenant: &str,
    ) -> Result<String, String> {
        let mut params: Vec<Value> = vec![serde_json::json!({
            "name": "_format", "valueCode": format,
        })];
        for (name, reference) in subjects {
            params.push(serde_json::json!({
                "name": "subject",
                "part": [
                    { "name": "name", "valueString": name },
                    { "name": "subjectReference", "valueReference": { "reference": reference } },
                ],
            }));
        }
        let url = format!("{}/$sql-export", self.base_url);
        let request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Prefer", "respond-async")
            .json(&serde_json::json!({ "resourceType": "Parameters", "parameter": params }));
        let response = self
            .authorized(request, tenant)
            .await?
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        let status = response.status();
        if status != reqwest::StatusCode::ACCEPTED {
            let detail = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|o| outcome_diagnostics(&o))
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            return Err(format!("$sql-export returned {status}{detail}"));
        }
        // `Content-Location: …/export/{job-id}/status` — the id is the
        // second-to-last path segment.
        response
            .headers()
            .get("content-location")
            .and_then(|v| v.to_str().ok())
            .and_then(|loc| {
                let mut segments = loc.trim_end_matches('/').rsplit('/');
                (segments.next() == Some("status"))
                    .then(|| segments.next())
                    .flatten()
                    .map(String::from)
            })
            .ok_or_else(|| "the 202 carried no job id in Content-Location".to_string())
    }

    async fn export_status(&self, job_id: &str, tenant: &str) -> SqlExportStatus {
        let url = format!("{}/export/{job_id}/status", self.base_url);
        let request = match self.authorized(self.client.get(&url), tenant).await {
            Ok(r) => r,
            Err(_) => return SqlExportStatus::Unknown,
        };
        match request.send().await {
            Ok(response) => match response.status().as_u16() {
                202 => SqlExportStatus::Running(
                    response
                        .headers()
                        .get("x-progress")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from),
                ),
                303 => SqlExportStatus::Done,
                _ => SqlExportStatus::Unknown,
            },
            Err(_) => SqlExportStatus::Unknown,
        }
    }

    async fn export_cancel(&self, job_id: &str, tenant: &str) -> Result<(), String> {
        let url = format!("{}/export/{job_id}/status", self.base_url);
        let response = self
            .authorized(self.client.delete(&url), tenant)
            .await?
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("cancelling returned {}", response.status()))
        }
    }

    async fn export_manifest(&self, job_id: &str, tenant: &str) -> Result<Value, String> {
        let url = format!("{}/export/{job_id}/result", self.base_url);
        let response = self
            .authorized(self.client.get(&url), tenant)
            .await?
            .send()
            .await
            .map_err(|e| format!("request to {url} failed: {e}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("parsing the manifest failed: {e}"))?;
        if !status.is_success() {
            let detail = outcome_diagnostics(&body)
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            return Err(format!("the result endpoint returned {status}{detail}"));
        }
        Ok(body)
    }
}

/// The first issue explanation of an OperationOutcome, if that is what this
/// is — `diagnostics` when present, else the issue's `details.text`.
fn outcome_diagnostics(outcome: &Value) -> Option<String> {
    outcome.get("issue")?.as_array()?.iter().find_map(|i| {
        i.get("diagnostics")
            .and_then(Value::as_str)
            .or_else(|| {
                i.get("details")
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
            })
            .map(String::from)
    })
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
    export_status: SqlExportStatus,
    export_manifest: Option<Result<Value, String>>,
}

impl StaticConformanceSource {
    /// An empty source: every fetch returns no resources (a degraded page).
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            metadata: None,
            sql_rows: None,
            export_status: SqlExportStatus::Unknown,
            export_manifest: None,
        }
    }

    /// Seeds what `sql_export_status()` answers (#649).
    pub fn with_export_status(mut self, status: SqlExportStatus) -> Self {
        self.export_status = status;
        self
    }

    /// Seeds what `sql_export_manifest()` answers (#649).
    pub fn with_export_manifest(mut self, outcome: Result<Value, String>) -> Self {
        self.export_manifest = Some(outcome);
        self
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

    async fn sql_export_start(
        &self,
        _subjects: &[(String, String)],
        _format: &str,
        _tenant: &str,
    ) -> Result<String, String> {
        Ok("static-job".to_string())
    }

    async fn sql_export_status(&self, _job_id: &str, _tenant: &str) -> SqlExportStatus {
        self.export_status.clone()
    }

    async fn sql_export_cancel(&self, _job_id: &str, _tenant: &str) -> Result<(), String> {
        Ok(())
    }

    async fn sql_export_manifest(&self, _job_id: &str, _tenant: &str) -> Result<Value, String> {
        self.export_manifest
            .clone()
            .unwrap_or_else(|| Err("no manifest seeded".to_string()))
    }

    /// Filters and paginates the seeded resources in memory. Implements the
    /// subset of search semantics the View Definitions page actually sends
    /// (#741): `name:contains` (case-insensitive substring on `name`) and
    /// `_sort=name` (also the default with no `_sort` param, matching the
    /// server's own default order). Other params are accepted but ignored —
    /// this is a test double standing in for the real search engine, not a
    /// reimplementation of it.
    async fn search_page(
        &self,
        resource_type: &str,
        params: &[(String, String)],
        count: usize,
        offset: usize,
        version: FhirVersion,
        _tenant: &str,
    ) -> Result<SearchPage, String> {
        let mut resources = self
            .map
            .get(&(resource_type.to_string(), version))
            .cloned()
            .unwrap_or_default();

        if let Some((_, needle)) = params.iter().find(|(name, _)| name == "name:contains") {
            let needle = needle.to_lowercase();
            resources.retain(|r| {
                r.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.to_lowercase().contains(&needle))
            });
        }

        let sort = params
            .iter()
            .find(|(name, _)| name == "_sort")
            .map(|(_, value)| value.as_str())
            .unwrap_or("name");
        if sort == "name" {
            resources.sort_by(|a, b| {
                let name_of = |r: &Value| {
                    r.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                name_of(a).cmp(&name_of(b))
            });
        }

        let total = resources.len();
        let page: Vec<Value> = resources.into_iter().skip(offset).take(count).collect();
        let has_next = offset + page.len() < total;
        Ok(SearchPage {
            resources: page,
            has_next,
        })
    }

    /// The seeded resource with matching `id`, or an `Err` when none exists —
    /// the same failure the HTTP source reports for a 404 (#741).
    async fn read_resource(
        &self,
        resource_type: &str,
        id: &str,
        version: FhirVersion,
        _tenant: &str,
    ) -> Result<Value, String> {
        self.map
            .get(&(resource_type.to_string(), version))
            .and_then(|resources| {
                resources
                    .iter()
                    .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            })
            .cloned()
            .ok_or_else(|| format!("no {resource_type} with id {id}"))
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

    /// #741: `search_page` issues one request with the given params plus
    /// `_count`/`_offset`, and derives `has_next` from the response Bundle's
    /// `next` link (present or absent).
    #[tokio::test]
    async fn http_search_page_sends_params_count_offset_and_reports_has_next() {
        use axum::extract::Query;
        use axum::{Router, routing::get};
        use std::collections::HashMap;

        async fn page(Query(q): Query<HashMap<String, String>>) -> axum::Json<Value> {
            // Echo the received query back as the entry resource, so the
            // test can assert on exactly what the client sent.
            let mut echo = serde_json::json!({ "resourceType": "QueryEcho" });
            if let Some(map) = echo.as_object_mut() {
                for (k, v) in &q {
                    map.insert(k.clone(), Value::String(v.clone()));
                }
            }
            let mut bundle = serde_json::json!({
                "resourceType": "Bundle",
                "type": "searchset",
                "entry": [{"resource": echo}]
            });
            // Only the first page (offset 0) advertises a next link.
            if q.get("_offset").map(String::as_str) == Some("0") {
                bundle["link"] = serde_json::json!([
                    {"relation": "next", "url": "http://x/ViewDefinition?_offset=50"}
                ]);
            }
            axum::Json(bundle)
        }

        let app = Router::new().route("/ViewDefinition", get(page));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );

        let first = source
            .search_page(
                "ViewDefinition",
                &[("name:contains".to_string(), "pat".to_string())],
                50,
                0,
                FhirVersion::R4,
                "",
            )
            .await
            .expect("search succeeds");
        assert_eq!(first.resources[0]["name:contains"], "pat");
        assert_eq!(first.resources[0]["_count"], "50");
        assert_eq!(first.resources[0]["_offset"], "0");
        assert!(first.has_next, "server advertised a next link");

        let second = source
            .search_page("ViewDefinition", &[], 50, 50, FhirVersion::R4, "")
            .await
            .expect("search succeeds");
        assert!(!second.has_next, "no next link on the last page");
    }

    /// #741: the tenant header rides `search_page` and `read_resource` the
    /// same way it does the other self-calls — sent when the tenant is not
    /// empty, absent otherwise.
    #[tokio::test]
    async fn http_search_page_sends_the_tenant_header_only_when_present() {
        use axum::extract::Path;
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::{Router, routing::get};

        fn echo_tenant(headers: &HeaderMap) -> axum::Json<Value> {
            let tenant = headers
                .get("x-tenant-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let mut resource =
                serde_json::json!({ "resourceType": "ViewDefinition", "id": "vd-1" });
            if let Some(t) = tenant {
                resource["tenant"] = Value::String(t);
            }
            axum::Json(resource)
        }

        async fn get_one(headers: HeaderMap, Path(_id): Path<String>) -> axum::response::Response {
            echo_tenant(&headers).into_response()
        }

        let app = Router::new().route("/ViewDefinition/{id}", get(get_one));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );

        let with_tenant = source
            .read_resource("ViewDefinition", "vd-1", FhirVersion::R4, "clinic-a")
            .await
            .expect("read succeeds");
        assert_eq!(with_tenant["tenant"], "clinic-a");

        let without_tenant = source
            .read_resource("ViewDefinition", "vd-1", FhirVersion::R4, "")
            .await
            .expect("read succeeds");
        assert!(
            without_tenant.get("tenant").is_none(),
            "no X-Tenant-ID sent for an empty tenant"
        );
    }

    /// #741: a non-default FHIR version degrades with `Err` for both new
    /// methods, the same as `sql_run` — storage only holds the server
    /// default, and there is no search index or single-resource lookup over
    /// the spec bundles to fall back to.
    #[cfg(feature = "R4B")]
    #[tokio::test]
    async fn non_default_version_degrades_search_and_read() {
        let source = HttpConformanceSource::new(
            "http://127.0.0.1:1".to_string(),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );

        let err = source
            .search_page("ViewDefinition", &[], 50, 0, FhirVersion::R4B, "")
            .await
            .expect_err("non-default version has no stored data to search");
        assert!(err.contains(FhirVersion::R4.as_str()), "{err}");

        let err = source
            .read_resource("ViewDefinition", "vd-1", FhirVersion::R4B, "")
            .await
            .expect_err("non-default version has no stored data to read");
        assert!(err.contains(FhirVersion::R4.as_str()), "{err}");
    }

    /// #741: `read_resource` surfaces both success and a 404 as the caller
    /// expects — the same distinction the View Definitions detail view needs
    /// to render "not found" rather than a generic error.
    #[tokio::test]
    async fn http_read_resource_succeeds_and_reports_404() {
        use axum::extract::Path;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::{Router, routing::get};

        async fn get_one(Path(id): Path<String>) -> axum::response::Response {
            if id == "vd-1" {
                axum::Json(serde_json::json!({
                    "resourceType": "ViewDefinition",
                    "id": "vd-1",
                    "name": "Patients"
                }))
                .into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }

        let app = Router::new().route("/ViewDefinition/{id}", get(get_one));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            std::sync::Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );

        let found = source
            .read_resource("ViewDefinition", "vd-1", FhirVersion::R4, "")
            .await
            .expect("resource exists");
        assert_eq!(found["name"], "Patients");

        let missing = source
            .read_resource("ViewDefinition", "missing", FhirVersion::R4, "")
            .await
            .expect_err("404 surfaces as an Err");
        assert!(missing.contains("404"), "{missing}");
    }

    /// #741: the static double's `name:contains` filter is case-insensitive,
    /// the default order is ascending by `name`, and `has_next` is correct on
    /// the exact boundary (total is a multiple of the page size).
    #[tokio::test]
    async fn static_search_page_filters_sorts_and_paginates() {
        fn vd(id: &str, name: &str) -> Value {
            serde_json::json!({ "resourceType": "ViewDefinition", "id": id, "name": name })
        }

        let source = StaticConformanceSource::empty().with(
            "ViewDefinition",
            FhirVersion::default(),
            vec![
                vd("vd-3", "Charlie"),
                vd("vd-1", "alpha-patients"),
                vd("vd-2", "Bravo"),
                vd("vd-4", "another-alpha"),
            ],
        );

        // `name:contains` is case-insensitive on a substring.
        let filtered = source
            .search_page(
                "ViewDefinition",
                &[("name:contains".to_string(), "ALPHA".to_string())],
                50,
                0,
                FhirVersion::default(),
                "",
            )
            .await
            .expect("search succeeds");
        let names: Vec<_> = filtered
            .resources
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["alpha-patients", "another-alpha"]);
        assert!(!filtered.has_next);

        // Default order (no `_sort`) is ascending by name.
        let sorted = source
            .search_page("ViewDefinition", &[], 50, 0, FhirVersion::default(), "")
            .await
            .expect("search succeeds");
        let names: Vec<_> = sorted
            .resources
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["Bravo", "Charlie", "alpha-patients", "another-alpha"]
        );

        // Pagination on the exact boundary: 4 resources, page size 2 — the
        // first page has a next, the second does not.
        let page1 = source
            .search_page("ViewDefinition", &[], 2, 0, FhirVersion::default(), "")
            .await
            .expect("search succeeds");
        assert_eq!(page1.resources.len(), 2);
        assert!(page1.has_next, "two more resources remain");

        let page2 = source
            .search_page("ViewDefinition", &[], 2, 2, FhirVersion::default(), "")
            .await
            .expect("search succeeds");
        assert_eq!(page2.resources.len(), 2);
        assert!(!page2.has_next, "exactly consumed the total");
    }

    /// #741: the static double's `read_resource` finds the seeded resource by
    /// id, or reports an `Err` when none matches.
    #[tokio::test]
    async fn static_read_resource_finds_by_id_or_errs() {
        let source = StaticConformanceSource::empty().with(
            "ViewDefinition",
            FhirVersion::default(),
            vec![serde_json::json!({
                "resourceType": "ViewDefinition",
                "id": "vd-1",
                "name": "Patients"
            })],
        );

        let found = source
            .read_resource("ViewDefinition", "vd-1", FhirVersion::default(), "")
            .await
            .expect("resource exists");
        assert_eq!(found["name"], "Patients");

        let missing = source
            .read_resource("ViewDefinition", "missing", FhirVersion::default(), "")
            .await
            .expect_err("no such id");
        assert!(missing.contains("missing"), "{missing}");
    }
}
