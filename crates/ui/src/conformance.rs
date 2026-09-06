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
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde_json::Value;

/// Who is asking on the loopback self-call that backs `$sql-export` (#833):
/// the effective tenant, and — when the browser sent one — the
/// `Authorization` header verbatim.
///
/// Built once per request from the incoming headers and the tenant the
/// request already resolved to (the crate's `RequestTenant` extractor), and
/// threaded unchanged through the four `$sql-export` methods of
/// [`ConformanceSource`]. It never decodes, validates, or re-derives a
/// token — it only carries what the browser sent so the self-call can
/// forward it verbatim, keeping kick-off and polling under the same identity
/// the async FHIR pattern and SMART scopes assume.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The tenant the request resolved to; sent as `X-Tenant-ID` unless
    /// empty (the server-default tenant needs no header, same rule as every
    /// other self-call in this module).
    pub tenant: String,
    /// The `Authorization` header value the browser sent, if any. When
    /// present it is forwarded verbatim to the self-call in place of the
    /// [`OutboundAuthProvider`](helios_auth::outbound::OutboundAuthProvider)'s
    /// credentials.
    pub authorization: Option<String>,
}

impl Caller {
    /// Builds a [`Caller`] from a request's headers and its resolved tenant.
    ///
    /// Reads only the `Authorization` header (case-insensitive, per
    /// `HeaderMap`) and copies its value as-is — no parsing, no scheme
    /// check, no decoding. An absent or non-UTF-8 header yields `None`,
    /// which falls back to the server's outbound credentials.
    pub fn from_request(headers: &axum::http::HeaderMap, tenant: &str) -> Self {
        Self {
            tenant: tenant.to_string(),
            authorization: headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(String::from),
        }
    }
}

/// A `$sql-export` request, as the UI builds it, before it becomes the
/// operation's `Parameters` body (#836/#837): every checked subject, the
/// output format, the job-wide filters (patients, groups, since, the CSV
/// header switch, a tracking id — #836), and, per subject, the parameter
/// values a SQL Query needs to run (#837). This is the single shape both
/// [`ConformanceSource`] implementations accept and [`crate::sql_export`]
/// builds from a persisted job record — see [`sql_export_parameters_body`]
/// for how it turns into the request the server actually receives.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlExportRequest {
    pub subjects: Vec<SqlExportSubject>,
    /// `ndjson` | `csv` | `json` | `parquet`.
    pub format: String,
    /// Only sent when `Some` — a `csv`-only switch the caller decides
    /// whether to include (#836); this type does not enforce that rule
    /// itself.
    pub header: Option<bool>,
    /// An RFC 3339 instant already resolved from whatever preset/custom input
    /// the caller offered — never a raw preset name (#836).
    pub since: Option<String>,
    /// `Patient/{id}` references, in submission order (#836).
    pub patients: Vec<String>,
    /// `Group/{id}` references, in submission order (#836).
    pub groups: Vec<String>,
    pub client_tracking_id: Option<String>,
}

/// One subject of a [`SqlExportRequest`]: the output name `$sql-export` will
/// use, the `ViewDefinition/{id}` or `Library/{id}` it runs, and — for a
/// parameterized SQL Query — the values to supply for its
/// `Library.parameter` declarations (#837). Empty `parameters` for a
/// ViewDefinition, a SQL View, or a SQL Query the caller left unparameterized.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlExportSubject {
    pub name: String,
    pub reference: String,
    pub parameters: Vec<SqlExportParameter>,
}

/// One supplied value for a subject's `Library.parameter` (#837): the
/// declared name (without its leading `:`), the FHIR type code from
/// `Library.parameter.type`, and the value text as the user entered it —
/// already validated by the time it reaches this type, so
/// [`sql_export_parameters_body`] only has to type it, never reject it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SqlExportParameter {
    pub name: String,
    pub type_code: String,
    pub value: String,
}

/// Builds the `$sql-export` request body — a `Parameters` resource — from a
/// [`SqlExportRequest`]. The single source of truth for that body: both
/// [`HttpConformanceSource`]'s self-call and [`StaticConformanceSource`]'s
/// recorded [`RecordedExportCall::body`] go through this function, so a test
/// asserting on the mock's recorded body is asserting on exactly what the
/// real self-call would have sent.
///
/// Parameter order (also the order the operation's own `$sql-export`
/// definition documents): `_format` always first, then `header` and `_since`
/// when the request carries them, then `clientTrackingId` when it is set and
/// not empty, then one `patient` per [`SqlExportRequest::patients`] and one
/// `group` per [`SqlExportRequest::groups`] (in submission order), and
/// finally one `subject` per [`SqlExportSubject`] — its `name`/
/// `subjectReference` parts, plus, only when the subject carries parameters,
/// a nested `parameters` part wrapping a `Parameters` resource of typed
/// `value[x]` entries (see [`typed_parameter_value`] for the type table).
/// `source` never appears: this UI never offers it (out of scope, the server
/// rejects it with a 400).
pub fn sql_export_parameters_body(request: &SqlExportRequest) -> Value {
    let mut params: Vec<Value> = vec![serde_json::json!({
        "name": "_format", "valueCode": request.format,
    })];
    if let Some(header) = request.header {
        params.push(serde_json::json!({ "name": "header", "valueBoolean": header }));
    }
    if let Some(since) = &request.since {
        params.push(serde_json::json!({ "name": "_since", "valueInstant": since }));
    }
    if let Some(tracking_id) = request
        .client_tracking_id
        .as_deref()
        .filter(|t| !t.is_empty())
    {
        params.push(serde_json::json!({ "name": "clientTrackingId", "valueString": tracking_id }));
    }
    for patient in &request.patients {
        params.push(serde_json::json!({
            "name": "patient",
            "valueReference": { "reference": patient },
        }));
    }
    for group in &request.groups {
        params.push(serde_json::json!({
            "name": "group",
            "valueReference": { "reference": group },
        }));
    }
    for subject in &request.subjects {
        let mut part = vec![
            serde_json::json!({ "name": "name", "valueString": subject.name }),
            serde_json::json!({
                "name": "subjectReference",
                "valueReference": { "reference": subject.reference },
            }),
        ];
        if !subject.parameters.is_empty() {
            let parameter: Vec<Value> = subject
                .parameters
                .iter()
                .map(|p| {
                    let (key, value) = typed_parameter_value(&p.type_code, &p.value);
                    let mut entry = serde_json::Map::new();
                    entry.insert("name".to_string(), Value::String(p.name.clone()));
                    entry.insert(key.to_string(), value);
                    Value::Object(entry)
                })
                .collect();
            part.push(serde_json::json!({
                "name": "parameters",
                "resource": { "resourceType": "Parameters", "parameter": parameter },
            }));
        }
        params.push(serde_json::json!({ "name": "subject", "part": part }));
    }
    serde_json::json!({ "resourceType": "Parameters", "parameter": params })
}

/// The `value[x]` key and JSON-typed value for one subject parameter, from
/// its declared `Library.parameter.type` and the raw text the user entered
/// (#837) — the same type table `helios_sof::sqlquery::bind`'s
/// `value_x_suffix_for` uses to bind a supplied value back on the server, so
/// a value this function types one way is the value the binder expects.
///
/// A value that fails to parse for a numeric or boolean type (e.g. `"abc"`
/// for `integer`) is never rejected here — the UI validates a value before
/// it ever reaches this function — it falls back to `valueString` so the
/// server's own binder is the one that reports the error, with its own
/// message.
fn typed_parameter_value(type_code: &str, value: &str) -> (&'static str, Value) {
    match type_code {
        "boolean" => match value.parse::<bool>() {
            Ok(b) => ("valueBoolean", Value::Bool(b)),
            Err(_) => ("valueString", Value::String(value.to_string())),
        },
        "integer" | "positiveInt" | "unsignedInt" => match value.parse::<i64>() {
            Ok(n) => ("valueInteger", Value::Number(n.into())),
            Err(_) => ("valueString", Value::String(value.to_string())),
        },
        // Represented as a JSON string even on success: a 64-bit integer can
        // exceed the precision JSON numbers guarantee.
        "integer64" => ("valueInteger64", Value::String(value.to_string())),
        "decimal" => match value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
        {
            Some(n) => ("valueDecimal", Value::Number(n)),
            None => ("valueString", Value::String(value.to_string())),
        },
        "date" => ("valueDate", Value::String(value.to_string())),
        "dateTime" => ("valueDateTime", Value::String(value.to_string())),
        "instant" => ("valueInstant", Value::String(value.to_string())),
        "time" => ("valueTime", Value::String(value.to_string())),
        "code" => ("valueCode", Value::String(value.to_string())),
        "id" => ("valueId", Value::String(value.to_string())),
        "uri" => ("valueUri", Value::String(value.to_string())),
        "url" => ("valueUrl", Value::String(value.to_string())),
        "canonical" => ("valueCanonical", Value::String(value.to_string())),
        "markdown" => ("valueMarkdown", Value::String(value.to_string())),
        "oid" => ("valueOid", Value::String(value.to_string())),
        "uuid" => ("valueUuid", Value::String(value.to_string())),
        "base64Binary" => ("valueBase64Binary", Value::String(value.to_string())),
        _ => ("valueString", Value::String(value.to_string())),
    }
}

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

    /// Submits a `$sql-export` job — subjects, format, job-wide filters, and
    /// per-subject parameter values, per [`SqlExportRequest`] (#836/#837) —
    /// and returns the job id from its `Content-Location` (#649).
    ///
    /// `caller` carries the tenant and, when the browser sent one, the
    /// `Authorization` to run the job under (#833) — see [`Caller`].
    async fn sql_export_start(
        &self,
        request: &SqlExportRequest,
        caller: &Caller,
    ) -> Result<String, String> {
        let _ = (request, caller);
        Err("$sql-export is not available from this source".to_string())
    }

    /// Polls a job's status URL (#649), under the same [`Caller`] identity
    /// the kick-off used (#833).
    async fn sql_export_status(&self, job_id: &str, caller: &Caller) -> SqlExportStatus {
        let _ = (job_id, caller);
        SqlExportStatus::Unknown
    }

    /// Cancels a job; `Ok` when the server accepted the cancellation (#649).
    async fn sql_export_cancel(&self, job_id: &str, caller: &Caller) -> Result<(), String> {
        let _ = (job_id, caller);
        Err("$sql-export is not available from this source".to_string())
    }

    /// The completion manifest `Parameters` of a finished job (#649).
    async fn sql_export_manifest(&self, job_id: &str, caller: &Caller) -> Result<Value, String> {
        let _ = (job_id, caller);
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
    /// Anything else: a transport failure (connection refused, timeout, …)
    /// or a response that is not 202/303/404 — most notably 401/403 (the
    /// forwarded credential was rejected) or a 5xx. The job is *not* known
    /// to be gone, unlike [`Unknown`](Self::Unknown); callers should keep
    /// treating it as in progress and retry the poll (#833). The message is
    /// short and names the cause, e.g. `"status poll answered 401"`.
    Unavailable(String),
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

    /// A request with the caller's tenant and credentials applied — the one
    /// place that decides between the two credential sources for the
    /// `$sql-export` self-calls (#833).
    ///
    /// `X-Tenant-ID` follows the same rule as [`authorized`](Self::authorized)
    /// (omitted when the tenant is empty). Credentials: `caller.authorization`
    /// wins when present, forwarded verbatim; otherwise the configured
    /// [`OutboundAuthProvider`](helios_auth::outbound::OutboundAuthProvider)
    /// is consulted, exactly as every other self-call in this module does.
    async fn authorized_for(
        &self,
        request: reqwest::RequestBuilder,
        caller: &Caller,
    ) -> Result<reqwest::RequestBuilder, String> {
        let request = if caller.tenant.is_empty() {
            request
        } else {
            request.header("X-Tenant-ID", &caller.tenant)
        };
        match &caller.authorization {
            Some(authorization) => Ok(request.header("Authorization", authorization)),
            None => self
                .outbound_auth
                .authorize(request, &self.base_url)
                .await
                .map_err(|e| format!("outbound auth failed: {e}")),
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
        // One more than the page: whether a further page exists is read off
        // the overflow row, not only the Bundle's `next` link — MongoDB
        // issues no page cursors (and so no `next` link) for a search sorted
        // by a search parameter, and the rails always sort by `name`.
        query.push(("_count".to_string(), (count + 1).to_string()));
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
        let mut resources = extract_bundle_resources(&bundle);
        let overflow = resources.len() > count;
        resources.truncate(count);
        Ok(SearchPage {
            resources,
            has_next: overflow || next_link(&bundle).is_some(),
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
        request: &SqlExportRequest,
        caller: &Caller,
    ) -> Result<String, String> {
        self.export_start(request, caller).await
    }

    async fn sql_export_status(&self, job_id: &str, caller: &Caller) -> SqlExportStatus {
        self.export_status(job_id, caller).await
    }

    async fn sql_export_cancel(&self, job_id: &str, caller: &Caller) -> Result<(), String> {
        self.export_cancel(job_id, caller).await
    }

    async fn sql_export_manifest(&self, job_id: &str, caller: &Caller) -> Result<Value, String> {
        self.export_manifest(job_id, caller).await
    }
}

impl HttpConformanceSource {
    /// `POST {base}/$sql-export` with the body [`sql_export_parameters_body`]
    /// builds from `request` — the single place that constructs it, so this
    /// self-call and [`StaticConformanceSource`]'s recorded body never drift
    /// apart (#836/#837).
    async fn export_start(
        &self,
        request: &SqlExportRequest,
        caller: &Caller,
    ) -> Result<String, String> {
        let body = sql_export_parameters_body(request);
        let url = format!("{}/$sql-export", self.base_url);
        let http_request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Prefer", "respond-async")
            .json(&body);
        let response = self
            .authorized_for(http_request, caller)
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

    /// Polls a job's status. `202`/`303`/`404` map to `Running`/`Done`/
    /// `Unknown` as before; every other outcome — a transport failure or any
    /// other status (401/403/5xx…) — maps to `Unavailable` rather than being
    /// folded into `Unknown`, so a rejected credential does not read as "the
    /// server forgot this job" (#833).
    async fn export_status(&self, job_id: &str, caller: &Caller) -> SqlExportStatus {
        let url = format!("{}/export/{job_id}/status", self.base_url);
        let request = match self.authorized_for(self.client.get(&url), caller).await {
            Ok(r) => r,
            Err(e) => return SqlExportStatus::Unavailable(e),
        };
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                match status.as_u16() {
                    202 => SqlExportStatus::Running(
                        response
                            .headers()
                            .get("x-progress")
                            .and_then(|v| v.to_str().ok())
                            .map(String::from),
                    ),
                    303 => SqlExportStatus::Done,
                    404 => SqlExportStatus::Unknown,
                    _ => SqlExportStatus::Unavailable(format!("status poll answered {status}")),
                }
            }
            Err(e) => SqlExportStatus::Unavailable(format!("status poll request failed: {e}")),
        }
    }

    async fn export_cancel(&self, job_id: &str, caller: &Caller) -> Result<(), String> {
        let url = format!("{}/export/{job_id}/status", self.base_url);
        let response = self
            .authorized_for(self.client.delete(&url), caller)
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

    async fn export_manifest(&self, job_id: &str, caller: &Caller) -> Result<Value, String> {
        let url = format!("{}/export/{job_id}/result", self.base_url);
        let response = self
            .authorized_for(self.client.get(&url), caller)
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

/// One `$sql-export` operation as [`StaticConformanceSource`] received it —
/// which method, the [`Caller`] it was called with (#833), and — for
/// `"start"` only — the `(output name, reference)` subjects submitted, so a
/// test can assert on the output names a kick-off actually sent (#833)
/// without standing up a real server. Empty for every other operation,
/// which does not carry subjects.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedExportCall {
    /// `"start"`, `"status"`, `"cancel"`, or `"manifest"`.
    pub operation: &'static str,
    pub caller: Caller,
    pub subjects: Vec<(String, String)>,
    /// The exact `Parameters` body [`sql_export_parameters_body`] built for
    /// this call — `Some` only for `"start"` (#836/#837), so a test can
    /// assert on job-wide filters and subject parameters the same way it
    /// already asserts on `subjects`, without standing up a real server.
    pub body: Option<Value>,
}

/// In-memory conformance source for tests: returns whatever resources it was
/// seeded with, keyed by `(resource_type, version)`. Offline and deterministic.
///
/// `export_calls` is behind an `Arc<Mutex<_>>` so the log survives the clone
/// that a test typically keeps for itself before moving the other clone into
/// an `Arc<dyn ConformanceSource>` for the app under test (#833) — both see
/// the same recorded calls.
#[doc(hidden)]
#[derive(Clone)]
pub struct StaticConformanceSource {
    map: HashMap<(String, FhirVersion), Vec<Value>>,
    fetch_errors: HashMap<(String, FhirVersion), String>,
    metadata: Option<Value>,
    sql_rows: Option<Result<Vec<Value>, String>>,
    export_status: SqlExportStatus,
    export_manifest: Option<Result<Value, String>>,
    export_calls: Arc<Mutex<Vec<RecordedExportCall>>>,
    saved_resources: Arc<Mutex<Vec<Value>>>,
    sql_run_calls: Arc<Mutex<Vec<Value>>>,
}

impl StaticConformanceSource {
    /// An empty source: every fetch returns no resources (a degraded page).
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            fetch_errors: HashMap::new(),
            metadata: None,
            sql_rows: None,
            export_status: SqlExportStatus::Unknown,
            export_manifest: None,
            export_calls: Arc::new(Mutex::new(Vec::new())),
            saved_resources: Arc::new(Mutex::new(Vec::new())),
            sql_run_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The resources `save_resource` has received so far, in call order —
    /// for a test to assert on what a Save actually merged (with an id
    /// already assigned), since this source does not persist them anywhere
    /// `fetch()` would read back from.
    pub fn saved_resources(&self) -> Vec<Value> {
        self.saved_resources
            .lock()
            .expect("saved_resources mutex is never held across a panic")
            .clone()
    }

    /// The documents `sql_run` has received so far, in call order — for a
    /// test to assert on exactly what a `$sql-run` call carried (e.g. that
    /// `embed_sql` reached it), since [`Self::with_sql_run`] only seeds the
    /// *outcome*, blind to the input.
    pub fn sql_run_calls(&self) -> Vec<Value> {
        self.sql_run_calls
            .lock()
            .expect("sql_run_calls mutex is never held across a panic")
            .clone()
    }

    /// The `$sql-export` calls this source has received so far, in call
    /// order — for tests to assert on which [`Caller`] reached which
    /// operation (#833).
    pub fn export_calls(&self) -> Vec<RecordedExportCall> {
        self.export_calls
            .lock()
            .expect("export_calls mutex is never held across a panic")
            .clone()
    }

    /// Records one `$sql-export` call, shared across every clone of this
    /// source (see the struct docs). `subjects` and `body` are only ever
    /// non-empty/`Some` for `"start"`.
    fn record_export_call(
        &self,
        operation: &'static str,
        caller: &Caller,
        subjects: &[(String, String)],
        body: Option<Value>,
    ) {
        self.export_calls
            .lock()
            .expect("export_calls mutex is never held across a panic")
            .push(RecordedExportCall {
                operation,
                caller: caller.clone(),
                subjects: subjects.to_vec(),
                body,
            });
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

    /// Makes `fetch()` fail for one `(resource_type, version)` slot instead
    /// of returning its (possibly empty) resource list — exercising a
    /// degraded fetch (#834), the kind the SQL Export builder reports as
    /// "the library list could not be loaded" rather than as an actually
    /// empty store.
    pub fn with_fetch_error(
        mut self,
        resource_type: &str,
        version: FhirVersion,
        message: impl Into<String>,
    ) -> Self {
        self.fetch_errors
            .insert((resource_type.to_string(), version), message.into());
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
        if let Some(message) = self.fetch_errors.get(&(resource_type.to_string(), version)) {
            return Err(message.clone());
        }
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
        view_definition: &Value,
        limit: usize,
        _version: FhirVersion,
        _tenant: &str,
    ) -> Result<Vec<Value>, String> {
        self.sql_run_calls
            .lock()
            .expect("sql_run_calls mutex is never held across a panic")
            .push(view_definition.clone());
        match &self.sql_rows {
            Some(Ok(rows)) => Ok(rows.iter().take(limit).cloned().collect()),
            Some(Err(e)) => Err(e.clone()),
            None => Err("no $sql-run outcome seeded".to_string()),
        }
    }

    /// Records the call and the exact body [`sql_export_parameters_body`]
    /// built for it (#836/#837), and always answers with a fixed job id — the
    /// static double never actually runs anything, so a test drives the
    /// resulting job's lifecycle through `with_export_status`/
    /// `with_export_manifest` instead.
    async fn sql_export_start(
        &self,
        request: &SqlExportRequest,
        caller: &Caller,
    ) -> Result<String, String> {
        let subjects: Vec<(String, String)> = request
            .subjects
            .iter()
            .map(|s| (s.name.clone(), s.reference.clone()))
            .collect();
        let body = sql_export_parameters_body(request);
        self.record_export_call("start", caller, &subjects, Some(body));
        Ok("static-job".to_string())
    }

    async fn sql_export_status(&self, _job_id: &str, caller: &Caller) -> SqlExportStatus {
        self.record_export_call("status", caller, &[], None);
        self.export_status.clone()
    }

    async fn sql_export_cancel(&self, _job_id: &str, caller: &Caller) -> Result<(), String> {
        self.record_export_call("cancel", caller, &[], None);
        Ok(())
    }

    async fn sql_export_manifest(&self, _job_id: &str, caller: &Caller) -> Result<Value, String> {
        self.record_export_call("manifest", caller, &[], None);
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
    /// Also records the exact resource that was posted (post-id) in
    /// [`Self::saved_resources`], for a test that needs to assert on the
    /// document a save actually merged and would have persisted — #840's
    /// own Details/SQL fusion, say — without a real backend to read it back
    /// from.
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
        self.saved_resources
            .lock()
            .expect("saved_resources mutex is never held across a panic")
            .push(resource.clone());
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
    /// `_count` (one over the page, so a further page shows up as an
    /// overflow row even from a server that issues no `next` link — MongoDB
    /// on a parameter sort) and `_offset`, and derives `has_next` from that
    /// overflow or the response Bundle's `next` link (present or absent).
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
        assert_eq!(first.resources[0]["_count"], "51");
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

    /// #833: `Caller::from_request` copies the `Authorization` header
    /// verbatim (case-insensitively, per `HeaderMap`) and never touches it
    /// otherwise; a request with none yields `None`.
    #[test]
    fn caller_from_request_reads_authorization_verbatim() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("Authorization", "Bearer user-token".parse().unwrap());
        let caller = Caller::from_request(&headers, "clinic-a");
        assert_eq!(caller.tenant, "clinic-a");
        assert_eq!(caller.authorization.as_deref(), Some("Bearer user-token"));

        let none = Caller::from_request(&axum::http::HeaderMap::new(), "clinic-a");
        assert_eq!(none.authorization, None);
    }

    /// #833: the one credentials rule every `$sql-export` self-call shares —
    /// `caller.authorization` wins and is forwarded verbatim, even when the
    /// outbound provider would supply a different (service) credential;
    /// without one, the outbound provider is consulted as every other
    /// self-call does. `X-Tenant-ID` follows the caller's tenant, omitted
    /// when it is empty.
    #[tokio::test]
    async fn export_credentials_prefer_the_caller_over_the_outbound_provider() {
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::routing::post;

        async fn start(headers: HeaderMap) -> axum::response::Response {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>");
            let tenant = headers
                .get("x-tenant-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("<none>");
            // Echo what was received back as the job id, via Content-Location,
            // so the test can assert on it through the public API rather than
            // shared mutable state.
            (
                axum::http::StatusCode::ACCEPTED,
                [(
                    "content-location",
                    format!("/export/{auth}::{tenant}/status"),
                )],
                "",
            )
                .into_response()
        }

        let app = axum::Router::new().route("/$sql-export", post(start));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            Arc::new(helios_auth::outbound::StaticBearerOutboundAuthProvider::new("service-token")),
            FhirVersion::R4,
            None,
        );

        let request = SqlExportRequest {
            format: "ndjson".to_string(),
            ..Default::default()
        };

        // The caller's own bearer wins over the outbound provider.
        let with_bearer = Caller {
            tenant: "clinic-a".to_string(),
            authorization: Some("Bearer user-token".to_string()),
        };
        let job = source
            .sql_export_start(&request, &with_bearer)
            .await
            .expect("202 carries a job id");
        assert_eq!(job, "Bearer user-token::clinic-a");

        // No caller bearer: falls back to the outbound provider's credential.
        let no_bearer = Caller {
            tenant: "clinic-a".to_string(),
            authorization: None,
        };
        let job = source
            .sql_export_start(&request, &no_bearer)
            .await
            .expect("202 carries a job id");
        assert_eq!(job, "Bearer service-token::clinic-a");

        // An empty tenant omits X-Tenant-ID, same rule as every other
        // self-call in this module.
        let no_tenant = Caller {
            tenant: String::new(),
            authorization: Some("Bearer user-token".to_string()),
        };
        let job = source
            .sql_export_start(&request, &no_tenant)
            .await
            .expect("202 carries a job id");
        assert_eq!(job, "Bearer user-token::<none>");
    }

    /// #833: `export_status` maps every self-call outcome to the right
    /// variant — `202`/`303`/`404` unchanged, and everything else (401, 500,
    /// …) to `Unavailable` with a short cause naming the status, rather than
    /// being folded into `Unknown` — a job the server still knows about must
    /// not read as "the server forgot this job".
    #[tokio::test]
    async fn export_status_maps_every_http_outcome() {
        use axum::extract::Path;
        use axum::http::StatusCode as AxStatus;
        use axum::response::IntoResponse;
        use axum::routing::get;

        async fn status(Path(id): Path<String>) -> axum::response::Response {
            match id.as_str() {
                "running" => (AxStatus::ACCEPTED, [("x-progress", "42")], "").into_response(),
                "done" => AxStatus::SEE_OTHER.into_response(),
                "gone" => AxStatus::NOT_FOUND.into_response(),
                "denied" => AxStatus::UNAUTHORIZED.into_response(),
                "broken" => AxStatus::INTERNAL_SERVER_ERROR.into_response(),
                other => panic!("unexpected job id {other}"),
            }
        }

        let app = axum::Router::new().route("/export/{id}/status", get(status));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let source = HttpConformanceSource::new(
            format!("http://{addr}"),
            Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );
        let caller = Caller::default();

        assert_eq!(
            source.sql_export_status("running", &caller).await,
            SqlExportStatus::Running(Some("42".to_string()))
        );
        assert_eq!(
            source.sql_export_status("done", &caller).await,
            SqlExportStatus::Done
        );
        assert_eq!(
            source.sql_export_status("gone", &caller).await,
            SqlExportStatus::Unknown
        );
        match source.sql_export_status("denied", &caller).await {
            SqlExportStatus::Unavailable(message) => assert!(message.contains("401"), "{message}"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        match source.sql_export_status("broken", &caller).await {
            SqlExportStatus::Unavailable(message) => assert!(message.contains("500"), "{message}"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    /// #833: a self-call that cannot even reach the server (connection
    /// refused) also maps to `Unavailable`, never `Unknown` — the server has
    /// said nothing about the job, so it must not read as "gone".
    #[tokio::test]
    async fn export_status_maps_a_transport_failure_to_unavailable() {
        let source = HttpConformanceSource::new(
            "http://127.0.0.1:1".to_string(),
            Arc::new(helios_auth::outbound::NoOpOutboundAuthProvider),
            FhirVersion::R4,
            None,
        );
        match source.sql_export_status("any", &Caller::default()).await {
            SqlExportStatus::Unavailable(_) => {}
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    /// #833: [`StaticConformanceSource`] records every `$sql-export` call it
    /// receives — which operation and which [`Caller`] — and the log is
    /// shared across clones, so a test can keep one clone for itself while
    /// handing another to the app under test.
    #[tokio::test]
    async fn static_source_records_export_calls_across_clones() {
        let source = StaticConformanceSource::empty();
        let recorder = source.clone();

        let caller = Caller {
            tenant: "clinic-a".to_string(),
            authorization: Some("Bearer user-token".to_string()),
        };
        source
            .sql_export_start(
                &SqlExportRequest {
                    format: "csv".to_string(),
                    ..Default::default()
                },
                &caller,
            )
            .await
            .expect("static source always accepts a start");
        let _ = source.sql_export_status("job-1", &caller).await;
        let _ = source.sql_export_cancel("job-1", &caller).await;
        let _ = source.sql_export_manifest("job-1", &caller).await;

        let calls = recorder.export_calls();
        let operations: Vec<&str> = calls.iter().map(|c| c.operation).collect();
        assert_eq!(operations, vec!["start", "status", "cancel", "manifest"]);
        assert!(calls.iter().all(|c| c.caller == caller));
    }

    // -------------------------------------------------------------------
    // sql_export_parameters_body (#836/#837)
    // -------------------------------------------------------------------

    /// A request carrying only the format and one plain subject produces
    /// exactly `_format` + `subject`, with no filter/parameters parts sneaking
    /// in — the shape a bare kick-off from today's builder still sends.
    #[test]
    fn body_of_a_minimal_request_has_only_format_and_subject() {
        let request = SqlExportRequest {
            subjects: vec![SqlExportSubject {
                name: "patients_flat".to_string(),
                reference: "ViewDefinition/vd1".to_string(),
                parameters: vec![],
            }],
            format: "csv".to_string(),
            ..Default::default()
        };
        assert_eq!(
            sql_export_parameters_body(&request),
            serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "csv"},
                    {"name": "subject", "part": [
                        {"name": "name", "valueString": "patients_flat"},
                        {
                            "name": "subjectReference",
                            "valueReference": {"reference": "ViewDefinition/vd1"},
                        },
                    ]},
                ],
            })
        );
    }

    /// Every optional field present at once — header, since, a tracking id,
    /// two patients, one group, and a subject with typed parameters — lands
    /// in the documented order, each parameter's `value[x]` typed by
    /// `typed_parameter_value`.
    #[test]
    fn body_of_a_full_request_carries_every_optional_field_in_order() {
        let request = SqlExportRequest {
            subjects: vec![SqlExportSubject {
                name: "ward_counts".to_string(),
                reference: "Library/q1".to_string(),
                parameters: vec![
                    SqlExportParameter {
                        name: "limit".to_string(),
                        type_code: "integer".to_string(),
                        value: "50".to_string(),
                    },
                    SqlExportParameter {
                        name: "ward".to_string(),
                        type_code: "string".to_string(),
                        value: "west".to_string(),
                    },
                ],
            }],
            format: "csv".to_string(),
            header: Some(true),
            since: Some("2026-01-01T00:00:00Z".to_string()),
            patients: vec!["Patient/p1".to_string(), "Patient/p2".to_string()],
            groups: vec!["Group/g1".to_string()],
            client_tracking_id: Some("trk-1".to_string()),
        };
        assert_eq!(
            sql_export_parameters_body(&request),
            serde_json::json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "csv"},
                    {"name": "header", "valueBoolean": true},
                    {"name": "_since", "valueInstant": "2026-01-01T00:00:00Z"},
                    {"name": "clientTrackingId", "valueString": "trk-1"},
                    {"name": "patient", "valueReference": {"reference": "Patient/p1"}},
                    {"name": "patient", "valueReference": {"reference": "Patient/p2"}},
                    {"name": "group", "valueReference": {"reference": "Group/g1"}},
                    {"name": "subject", "part": [
                        {"name": "name", "valueString": "ward_counts"},
                        {
                            "name": "subjectReference",
                            "valueReference": {"reference": "Library/q1"},
                        },
                        {"name": "parameters", "resource": {
                            "resourceType": "Parameters",
                            "parameter": [
                                {"name": "limit", "valueInteger": 50},
                                {"name": "ward", "valueString": "west"},
                            ],
                        }},
                    ]},
                ],
            })
        );
    }

    /// `header: None` never appears — unlike `since`/`clientTrackingId`,
    /// which this test does not set either, there is no "empty means absent"
    /// text value to compare against for a bool.
    #[test]
    fn body_omits_header_when_it_is_none() {
        let request = SqlExportRequest {
            format: "ndjson".to_string(),
            ..Default::default()
        };
        let names: Vec<String> = sql_export_parameters_body(&request)["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["_format"]);
    }

    /// An empty (but `Some`) tracking id is treated the same as `None`: this
    /// function only ever emits `clientTrackingId` when it is set *and* not
    /// empty.
    #[test]
    fn body_omits_an_empty_client_tracking_id() {
        let request = SqlExportRequest {
            format: "ndjson".to_string(),
            client_tracking_id: Some(String::new()),
            ..Default::default()
        };
        let names: Vec<String> = sql_export_parameters_body(&request)["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["_format"]);
    }

    /// A subject with no parameters gets no nested `parameters` part, even
    /// when other subjects in the same request do.
    #[test]
    fn body_omits_the_parameters_part_for_a_subject_with_none() {
        let request = SqlExportRequest {
            subjects: vec![
                SqlExportSubject {
                    name: "patients".to_string(),
                    reference: "ViewDefinition/vd1".to_string(),
                    parameters: vec![],
                },
                SqlExportSubject {
                    name: "ward_counts".to_string(),
                    reference: "Library/q1".to_string(),
                    parameters: vec![SqlExportParameter {
                        name: "ward".to_string(),
                        type_code: "string".to_string(),
                        value: "west".to_string(),
                    }],
                },
            ],
            format: "ndjson".to_string(),
            ..Default::default()
        };
        let body = sql_export_parameters_body(&request);
        let subjects = body["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == "subject");
        for subject in subjects {
            let parts = subject["part"].as_array().unwrap();
            let has_parameters = parts.iter().any(|p| p["name"] == "parameters");
            let name = parts[0]["valueString"].as_str().unwrap();
            assert_eq!(has_parameters, name == "ward_counts", "{name}");
        }
    }

    /// #837: `typed_parameter_value` matches
    /// `helios_sof::sqlquery::bind::value_x_suffix_for`'s type table exactly
    /// — every declared type gets its own `value[x]` key with a JSON-typed
    /// value, and a value that fails to parse for a numeric/boolean type
    /// falls back to `valueString` rather than being rejected here.
    #[test]
    fn typed_parameter_value_covers_every_declared_type_and_falls_back_to_string() {
        let cases: Vec<(&str, &str, &str, Value)> = vec![
            ("boolean", "true", "valueBoolean", Value::Bool(true)),
            ("integer", "42", "valueInteger", serde_json::json!(42)),
            ("positiveInt", "7", "valueInteger", serde_json::json!(7)),
            ("unsignedInt", "0", "valueInteger", serde_json::json!(0)),
            (
                "integer64",
                "9223372036854775807",
                "valueInteger64",
                serde_json::json!("9223372036854775807"),
            ),
            ("decimal", "3.5", "valueDecimal", serde_json::json!(3.5)),
            (
                "date",
                "2026-01-01",
                "valueDate",
                serde_json::json!("2026-01-01"),
            ),
            (
                "dateTime",
                "2026-01-01T00:00:00Z",
                "valueDateTime",
                serde_json::json!("2026-01-01T00:00:00Z"),
            ),
            (
                "instant",
                "2026-01-01T00:00:00Z",
                "valueInstant",
                serde_json::json!("2026-01-01T00:00:00Z"),
            ),
            (
                "time",
                "09:00:00",
                "valueTime",
                serde_json::json!("09:00:00"),
            ),
            ("code", "final", "valueCode", serde_json::json!("final")),
            ("id", "abc-1", "valueId", serde_json::json!("abc-1")),
            ("uri", "urn:x", "valueUri", serde_json::json!("urn:x")),
            ("url", "http://x", "valueUrl", serde_json::json!("http://x")),
            (
                "canonical",
                "http://x|1",
                "valueCanonical",
                serde_json::json!("http://x|1"),
            ),
            (
                "markdown",
                "**b**",
                "valueMarkdown",
                serde_json::json!("**b**"),
            ),
            ("oid", "1.2.3", "valueOid", serde_json::json!("1.2.3")),
            (
                "uuid",
                "3f9c9a2e-8f2a-4e7a-9d38-2a7a6b6a2b11",
                "valueUuid",
                serde_json::json!("3f9c9a2e-8f2a-4e7a-9d38-2a7a6b6a2b11"),
            ),
            (
                "base64Binary",
                "Zm9v",
                "valueBase64Binary",
                serde_json::json!("Zm9v"),
            ),
            ("string", "west", "valueString", serde_json::json!("west")),
            (
                "some-unrecognized-type",
                "x",
                "valueString",
                serde_json::json!("x"),
            ),
            // Not convertible for the declared type — falls back to
            // `valueString` rather than being rejected here.
            ("integer", "abc", "valueString", serde_json::json!("abc")),
            ("boolean", "yes", "valueString", serde_json::json!("yes")),
            ("decimal", "abc", "valueString", serde_json::json!("abc")),
        ];
        for (type_code, value, expected_key, expected_value) in cases {
            let (key, json_value) = typed_parameter_value(type_code, value);
            assert_eq!(key, expected_key, "type_code={type_code} value={value}");
            assert_eq!(
                json_value, expected_value,
                "type_code={type_code} value={value}"
            );
        }
    }
}
