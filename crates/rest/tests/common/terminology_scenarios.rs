//! The "Terminology in SQL" scenarios (#1824), shared by the per-backend
//! suites: `sof_terminology.rs` (SQLite) and its `_postgres`, `_mongodb`,
//! `_sqlite_elasticsearch` and `_s3` twins.
//!
//! Two groups, one body each, run by every backend through the same
//! [`Driver`]:
//!
//! - the **query path**: a SQL Query Library's `relatedArtifact[depends-on]`
//!   names a ValueSet or ConceptMap, which `$sql-run` / `$sql-export` expose
//!   as a relation (the spec page's worked example, verbatim);
//! - the **write path**: a Library write expands the ValueSets it depends on
//!   into storage (T1), a ValueSet write some Library depends on gains its
//!   expansion in the candidate (T2), with the terminology server as the
//!   engine when configured and local enumerated expansion otherwise;
//!   strict / lenient / off modes, SMART scopes, the write-free
//!   preconditions, batch and transaction Bundles over the bundle overlay.
//!
//! Nothing here names a backend. A suite implements [`Driver::server`] over
//! its backend with [`build`] (the router's own bounds, so any backend that
//! can serve HFS can serve the scenarios), says whether it supports atomic
//! transactions (standalone S3 does not: those scenarios pin the `501`) and
//! whether it has a search index (standalone S3 resolves canonicals by scan),
//! and expands [`terminology_suite!`] over its factory. Every server gets a
//! tenant of its own, so suites sharing one container never see each other.
//!
//! The terminology server is a mock `axum` app served over a real socket by
//! `axum_test`, so HFS's HTTP client talks to it exactly as it would to HTS.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_test::{TestResponse, TestServer};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::Utc;
use futures::StreamExt;
use helios_auth::{Principal, ScopeSet};
use helios_fhir::FhirVersion;
use helios_persistence::composite::{DynSearchProvider, DynStorage};
use helios_persistence::core::sof_runner::SofRunner;
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, ExportDataProvider, GroupExportProvider, IncludeProvider,
    InstanceHistoryProvider, PatientExportProvider, ResourceStorage, RevincludeProvider,
    SearchProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::export::{InMemoryController, InMemorySink};
use serde_json::{Value, json};

pub const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
pub const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
pub const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
pub const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
pub const PREFER: HeaderName = HeaderName::from_static("prefer");
pub const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
pub const TAG_SYSTEM: &str = "http://heliossoftware.com/fhir/CodeSystem/sof-terminology";

pub const CONDITIONS_URL: &str = "https://example.org/ViewDefinition/conditions";
pub const CVD_URL: &str = "http://example.org/ValueSet/cardiovascular-disease";
pub const MAP_URL: &str = "http://example.org/ConceptMap/sct-to-icd10";
pub const SCT: &str = "http://snomed.info/sct";
pub const ICD10: &str = "http://hl7.org/fhir/sid/icd-10";

// =========================================================================
// Mock terminology server
// =========================================================================

#[derive(Default)]
pub struct Mock {
    pub value_sets: Vec<Value>,
    pub concept_maps: Vec<Value>,
    pub expand_calls: usize,
    /// Definition searches (`GET /ValueSet?url=`, `GET /ConceptMap?url=`)
    /// the server answered, by resource type.
    pub search_calls: Vec<String>,
    pub fail_expand: bool,
}

pub type Shared = Arc<Mutex<Mock>>;

/// Expands the inline `valueSet` from its enumerated compose, as a real
/// server would, stamping an identifier that proves the server did it.
async fn mock_expand(
    State(mock): State<Shared>,
    Json(params): Json<Value>,
) -> axum::response::Response {
    let mut m = mock.lock().unwrap();
    m.expand_calls += 1;
    if m.fail_expand {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"resourceType": "OperationOutcome", "issue": [{"severity": "error", "code": "exception", "diagnostics": "boom"}]})),
        )
            .into_response();
    }
    let value_set = params["parameter"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == "valueSet"))
        .and_then(|p| p.get("resource"))
        .cloned()
        .unwrap_or(json!({}));
    let mut contains = Vec::new();
    if let Some(includes) = value_set
        .pointer("/compose/include")
        .and_then(Value::as_array)
    {
        for include in includes {
            let system = include["system"].as_str().unwrap_or("");
            for concept in include["concept"].as_array().cloned().unwrap_or_default() {
                let mut entry = json!({"system": system, "code": concept["code"]});
                if let Some(display) = concept.get("display") {
                    entry["display"] = display.clone();
                }
                if concept["code"] == "I21" {
                    entry["inactive"] = json!(true);
                }
                contains.push(entry);
            }
        }
    }
    let mut expanded = value_set;
    expanded["expansion"] = json!({
        "identifier": "urn:uuid:hts-mock",
        "timestamp": "2026-10-14T00:00:00Z",
        "total": contains.len(),
        "contains": contains
    });
    Json(expanded).into_response()
}

async fn mock_search(
    State(mock): State<Shared>,
    axum::extract::Path(resource_type): axum::extract::Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let mut m = mock.lock().unwrap();
    m.search_calls.push(resource_type.clone());
    let pool = if resource_type == "ValueSet" {
        &m.value_sets
    } else {
        &m.concept_maps
    };
    let entries: Vec<Value> = pool
        .iter()
        .filter(|r| {
            query.get("url").is_none_or(|u| r["url"] == *u)
                && query.get("version").is_none_or(|v| r["version"] == *v)
        })
        .map(|r| json!({"resource": r}))
        .collect();
    Json(json!({"resourceType": "Bundle", "type": "searchset", "entry": entries}))
}

/// A mock HTS over a real socket. Returns the server (keep it alive), its
/// base URL for `HFS_TERMINOLOGY_SERVER`, and the shared state.
pub async fn mock_hts() -> (TestServer, String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Mock::default()));
    let app = Router::new()
        .route("/ValueSet/$expand", post(mock_expand))
        .route("/{resource_type}", get(mock_search))
        .with_state(Arc::clone(&shared));
    let server = TestServer::builder()
        .http_transport()
        .build(app)
        .expect("mock terminology server");
    let url = server
        .server_address()
        .expect("http transport has an address")
        .to_string()
        .trim_end_matches('/')
        .to_string();
    (server, url, shared)
}

// =========================================================================
// Driver and server
// =========================================================================

/// What a backend suite supplies: a server over a fresh tenant, and the two
/// capabilities the scenarios branch on.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Builds an HFS over this backend with `config`, under a tenant no other
    /// server shares; `scopes` installs a principal carrying them.
    async fn server(&self, config: ServerConfig, scopes: Option<&str>) -> Hfs;
}

/// One HFS test server over one backend and one tenant.
pub struct Hfs {
    pub server: TestServer,
    pub storage: DynStorage,
    pub search: DynSearchProvider,
    pub tenant: TenantContext,
    pub tenant_id: String,
    /// `false` where a transaction Bundle is `501` (standalone S3).
    pub supports_transactions: bool,
    /// `false` where `GET /Type?url=` is unsupported and canonicals resolve
    /// by scan (standalone S3).
    pub supports_search: bool,
    /// `false` where a conditional entry inside a transaction Bundle is
    /// `501` because the search index lives on a secondary (the composites
    /// with offloaded search, `batch.rs`).
    pub conditional_in_transaction: bool,
}

/// The server configuration the scenarios run under: materialization
/// `mode` (`strict` / `lenient` / `off`), an optional terminology server.
pub fn config(mode: &str, hts: Option<&str>) -> ServerConfig {
    ServerConfig {
        base_url: "http://localhost".to_string(),
        sof_terminology_materialize: mode.to_string(),
        terminology_server: hts.map(str::to_string),
        ..ServerConfig::for_testing()
    }
}

/// A tenant id no other server has: short, lowercase, well inside the
/// 64-byte cap every backend accepts.
pub fn fresh_tenant_id() -> String {
    format!("t{}", uuid::Uuid::new_v4().simple())
}

/// Builds the routes over `backend` for a fresh tenant, with the router's own
/// bounds, so whatever `hfs` can serve, this can. Export jobs run in process
/// over the backend's SQL-on-FHIR runner.
pub async fn build<S>(
    backend: Arc<S>,
    config: ServerConfig,
    scopes: Option<&str>,
    supports_transactions: bool,
    supports_search: bool,
) -> Hfs
where
    S: ResourceStorage
        + ConditionalStorage
        + SearchProvider
        + IncludeProvider
        + RevincludeProvider
        + InstanceHistoryProvider
        + TypeHistoryProvider
        + SystemHistoryProvider
        + BundleProvider
        + ExportDataProvider
        + PatientExportProvider
        + GroupExportProvider
        + Send
        + Sync
        + 'static,
{
    let tenant_id = fresh_tenant_id();
    let tenant = TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
    let runner: Arc<dyn SofRunner> = backend
        .sof_runner()
        .expect("the backend must provide a SQL-on-FHIR runner");
    let sink = InMemorySink::new("http://localhost");
    let controller = InMemoryController::new(Arc::clone(&runner), sink, None);
    let state = helios_rest::AppState::new(Arc::clone(&backend), config)
        .with_sof_runner(runner)
        .with_export_controller(Arc::new(controller));
    let mut app = helios_rest::routing::fhir_routes::create_routes(state);
    if let Some(scopes) = scopes {
        let principal = Principal {
            subject: "client-1".to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: Some(tenant_id.clone()),
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
            ..Default::default()
        };
        app = app.layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: Next| {
                let principal = principal.clone();
                async move {
                    request.extensions_mut().insert(principal);
                    next.run(request).await
                }
            },
        ));
    }
    let server = TestServer::new(app).expect("failed to create test server");
    Hfs {
        server,
        storage: backend.clone() as DynStorage,
        search: backend as DynSearchProvider,
        tenant,
        tenant_id,
        supports_transactions,
        supports_search,
        conditional_in_transaction: true,
    }
}

impl Hfs {
    fn tenant_header(&self) -> HeaderValue {
        HeaderValue::from_str(&self.tenant_id).expect("tenant id is a header value")
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.server
            .get(path)
            .add_header(X_TENANT_ID, self.tenant_header())
            .await
    }

    pub async fn put(&self, path: &str, body: &Value) -> TestResponse {
        self.put_with(path, body, &[]).await
    }

    /// `PUT` with extra headers (`If-Match`, ...).
    pub async fn put_with(
        &self,
        path: &str,
        body: &Value,
        headers: &[(HeaderName, HeaderValue)],
    ) -> TestResponse {
        let mut request = self
            .server
            .put(path)
            .add_header(X_TENANT_ID, self.tenant_header())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            );
        for (name, value) in headers {
            request = request.add_header(name.clone(), value.clone());
        }
        request.json(body).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> TestResponse {
        self.post_with(path, body, &[]).await
    }

    /// `POST` with extra headers (`If-None-Exist`, `Prefer`, ...).
    pub async fn post_with(
        &self,
        path: &str,
        body: &Value,
        headers: &[(HeaderName, HeaderValue)],
    ) -> TestResponse {
        let mut request = self
            .server
            .post(path)
            .add_header(X_TENANT_ID, self.tenant_header())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            );
        for (name, value) in headers {
            request = request.add_header(name.clone(), value.clone());
        }
        request.json(body).await
    }

    /// A JSON Patch (`application/json-patch+json`) `PATCH`.
    pub async fn json_patch(&self, path: &str, patch: &Value) -> TestResponse {
        self.server
            .patch(path)
            .add_header(X_TENANT_ID, self.tenant_header())
            .bytes(serde_json::to_vec(patch).unwrap().into())
            .content_type("application/json-patch+json")
            .await
    }

    /// Posts a transaction Bundle. Where the backend has no atomic
    /// transactions the answer is pinned to `501` and `None` is returned, so
    /// the scenario stops there.
    pub async fn post_transaction(&self, bundle: &Value) -> Option<TestResponse> {
        let response = self.post("/", bundle).await;
        if self.supports_transactions {
            return Some(response);
        }
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_IMPLEMENTED,
            "a backend without atomic transactions answers 501: {}",
            response.text()
        );
        None
    }

    /// The stored resource's `(versionId, content)`, read from storage.
    pub async fn stored(&self, resource_type: &str, id: &str) -> Option<(String, Value)> {
        self.storage
            .read(&self.tenant, resource_type, id)
            .await
            .expect("read")
            .map(|s| (s.version_id().to_string(), s.content().clone()))
    }

    /// Writes a fixture straight to storage, then makes it visible to the
    /// search index: a seed is set-up, never the write under test.
    pub async fn seed(&self, resource_type: &str, resource: Value) {
        let id = resource["id"].as_str().expect("fixture id").to_string();
        self.storage
            .create_or_update(&self.tenant, resource_type, &id, resource, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("seed {resource_type}/{id}: {e}"));
        self.search
            .ensure_writes_visible(&self.tenant, &[resource_type])
            .await
            .unwrap_or_else(|e| panic!("make {resource_type} visible: {e}"));
    }

    /// The spec page's `conditions` view and its four Conditions.
    pub async fn seed_conditions(&self) {
        self.seed("ViewDefinition", conditions_view_definition())
            .await;
        for (patient, system, code) in [
            ("p1", SCT, "22298006"),
            ("p2", ICD10, "I21"),
            ("p3", SCT, "73211009"),
            ("p4", SCT, "102499006"),
        ] {
            self.seed(
                "Condition",
                json!({
                    "resourceType": "Condition",
                    "id": format!("c-{patient}"),
                    "subject": {"reference": format!("Patient/{patient}")},
                    "code": {"coding": [{"system": system, "code": code}]}
                }),
            )
            .await;
        }
    }

    /// Runs `library` inline through `$sql-run` and returns status and body.
    pub async fn run(&self, library: &Value) -> (StatusCode, Value) {
        let response = self
            .post("/$sql-run", &run_body(library.clone(), "json", vec![]))
            .await;
        let status = response.status_code();
        (status, response.json())
    }

    /// Every live resource of `resource_type` whose `url` is `url`: by search
    /// where there is one, by scan otherwise (the resolver's own fallback).
    /// A verification read, so the index is made current first: what it
    /// checks is what the server stored, not when the index saw it.
    pub async fn find_by_url(&self, resource_type: &str, url: &str) -> Vec<Value> {
        if self.supports_search {
            self.search
                .ensure_writes_visible(&self.tenant, &[resource_type])
                .await
                .unwrap_or_else(|e| panic!("make {resource_type} visible: {e}"));
            let bundle: Value = self
                .get(&format!("/{resource_type}?url={url}"))
                .await
                .json();
            return bundle["entry"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|e| e["resource"].clone())
                        .filter(|r| r["resourceType"] == resource_type)
                        .collect()
                })
                .unwrap_or_default();
        }
        let scan = self
            .storage
            .resource_scan()
            .expect("a backend without search offers a scan");
        let mut stream = scan
            .scan_resources(&self.tenant, resource_type)
            .await
            .expect("scan");
        let mut found = Vec::new();
        while let Some(item) = stream.next().await {
            let resource = item.expect("scanned resource");
            if resource["url"] == url {
                found.push(resource);
            }
        }
        found
    }

    /// Every live Library of the tenant (see [`Self::find_by_url`]).
    pub async fn all_libraries(&self) -> Vec<Value> {
        if self.supports_search {
            self.search
                .ensure_writes_visible(&self.tenant, &["Library"])
                .await
                .unwrap_or_else(|e| panic!("make Library visible: {e}"));
            let bundle: Value = self.get("/Library").await.json();
            return bundle["entry"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|e| e["resource"].clone())
                        .filter(|r| r["resourceType"] == "Library")
                        .collect()
                })
                .unwrap_or_default();
        }
        let scan = self
            .storage
            .resource_scan()
            .expect("a backend without search offers a scan");
        let mut stream = scan
            .scan_resources(&self.tenant, "Library")
            .await
            .expect("scan");
        let mut found = Vec::new();
        while let Some(item) = stream.next().await {
            found.push(item.expect("scanned resource"));
        }
        found
    }
}

// =========================================================================
// Fixtures: the spec page's, verbatim, and the write-path ones
// =========================================================================

pub fn conditions_view_definition() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "id": "conditions",
        "url": CONDITIONS_URL,
        "status": "active",
        "resource": "Condition",
        "select": [{"column": [
            {"path": "subject.getReferenceKey(Patient)", "name": "patient_id", "type": "string"},
            {"path": "code.coding.first().system", "name": "system", "type": "uri"},
            {"path": "code.coding.first().version", "name": "version", "type": "string"},
            {"path": "code.coding.first().code", "name": "code", "type": "code"}
        ]}]
    })
}

pub fn cvd_value_set() -> Value {
    json!({
        "resourceType": "ValueSet",
        "id": "cardiovascular-disease",
        "url": CVD_URL,
        "version": "2026",
        "name": "CardiovascularDisease",
        "status": "active",
        "expansion": {
            "identifier": "urn:uuid:635ad7e4-4f98-48ef-a3c2-1e3af6cc03ec",
            "timestamp": "2026-09-16T00:00:00Z",
            "total": 3,
            "parameter": [
                {"name": "version", "valueUri": "http://snomed.info/sct|http://snomed.info/sct/900000000000207008/version/20260201"},
                {"name": "version", "valueUri": "http://hl7.org/fhir/sid/icd-10|2019"}
            ],
            "contains": [{
                "abstract": true,
                "display": "Ischaemic heart disease",
                "contains": [
                    {"system": SCT, "version": "http://snomed.info/sct/900000000000207008/version/20260201",
                     "code": "22298006", "display": "Myocardial infarction"},
                    {"system": ICD10, "version": "2019", "code": "I21", "display": "Acute myocardial infarction"}
                ]
            }]
        }
    })
}

pub fn sct_to_icd10_concept_map() -> Value {
    json!({
        "resourceType": "ConceptMap",
        "id": "sct-to-icd10",
        "url": MAP_URL,
        "version": "2026",
        "name": "SnomedCtToIcd10",
        "status": "active",
        "group": [{
            "source": SCT,
            "target": "http://hl7.org/fhir/sid/icd-10|2019",
            "element": [
                {"code": "22298006", "display": "Myocardial infarction",
                 "target": [{"code": "I21", "display": "Acute myocardial infarction", "relationship": "equivalent"}]},
                {"code": "73211009", "display": "Diabetes mellitus",
                 "target": [{"code": "E14", "display": "Unspecified diabetes mellitus",
                             "relationship": "source-is-broader-than-target",
                             "comment": "The source covers every type of diabetes; ICD-10 classifies the types to E10-E13 and the unspecified remainder to E14"}]},
                {"code": "102499006", "display": "Fit and well", "noMap": true,
                 "comment": "A finding of health has no counterpart in a classification of disease"}
            ]
        }]
    })
}

pub const SPEC_CVD_SQL: &str = "SELECT conditions.patient_id, conditions.code
FROM conditions
WHERE EXISTS (
  SELECT 1
  FROM cvd_codes
  WHERE cvd_codes.system = conditions.system
    AND cvd_codes.code = conditions.code
)";

pub const SPEC_ICD10_SQL: &str = "SELECT conditions.patient_id,
       conditions.code,
       sct_to_icd10.target_code,
       sct_to_icd10.relationship
FROM conditions
LEFT JOIN sct_to_icd10
  ON sct_to_icd10.source_system = conditions.system
 AND sct_to_icd10.source_code = conditions.code
 AND (sct_to_icd10.relationship IS NULL
      OR sct_to_icd10.relationship <> 'not-related-to')";

/// A `sql-query` Library.
pub fn sql_lib(id: &str, name: &str, sql: &str, depends_on: &[(&str, &str)]) -> Value {
    json!({
        "resourceType": "Library",
        "id": id,
        "name": name,
        "status": "active",
        "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
        "content": [{"contentType": "application/sql", "data": B64.encode(sql.as_bytes())}],
        "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
            "type": "depends-on", "label": label, "resource": target
        })).collect::<Vec<_>>()
    })
}

/// The spec page's cardiovascular query, pinned to the 2026 ValueSet.
pub fn spec_cvd_query() -> Value {
    sql_lib(
        "CardiovascularDiseasePatientsQuery",
        "CardiovascularDiseasePatientsQuery",
        SPEC_CVD_SQL,
        &[
            ("conditions", CONDITIONS_URL),
            (
                "cvd_codes",
                "http://example.org/ValueSet/cardiovascular-disease|2026",
            ),
        ],
    )
}

/// The spec page's ICD-10 translation query, pinned to the 2026 map.
pub fn spec_icd10_query() -> Value {
    sql_lib(
        "ConditionsToIcd10Query",
        "ConditionsToIcd10Query",
        SPEC_ICD10_SQL,
        &[
            ("conditions", CONDITIONS_URL),
            (
                "sct_to_icd10",
                "http://example.org/ConceptMap/sct-to-icd10|2026",
            ),
        ],
    )
}

pub fn run_body(library: Value, format: &str, context: Vec<Value>) -> Value {
    let mut params = vec![
        json!({"name": "_format", "valueCode": format}),
        json!({"name": "subjectResource", "resource": library}),
    ];
    for artifact in context {
        params.push(json!({"name": "context", "resource": artifact}));
    }
    json!({"resourceType": "Parameters", "parameter": params})
}

/// Rows as `(patient_id, code, target_code, relationship)` tuples sorted by
/// patient, so order is not asserted.
pub fn sorted_rows(rows: &Value) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut out: Vec<_> = rows
        .as_array()
        .expect("json array")
        .iter()
        .map(|r| {
            (
                r["patient_id"].as_str().unwrap().to_string(),
                r["code"].as_str().unwrap().to_string(),
                r.get("target_code")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                r.get("relationship")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
        })
        .collect();
    out.sort();
    out
}

pub fn part<'a>(param: &'a Value, name: &str) -> &'a Value {
    param["part"]
        .as_array()
        .expect("part array")
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("part {name} in {param}"))
}

pub fn compose_only_vs(id: &str, url: &str, version: Option<&str>) -> Value {
    let mut vs = json!({
        "resourceType": "ValueSet", "id": id, "url": url, "status": "active",
        "compose": {"include": [
            {"system": SCT, "concept": [{"code": "22298006", "display": "Myocardial infarction"}]},
            {"system": ICD10, "concept": [{"code": "I21"}]}
        ]}
    });
    if let Some(v) = version {
        vs["version"] = json!(v);
    }
    vs
}

pub fn filter_vs(id: &str, url: &str) -> Value {
    json!({
        "resourceType": "ValueSet", "id": id, "url": url, "status": "active",
        "compose": {"include": [{"system": SCT, "filter": [{"property": "concept", "op": "is-a", "value": "49601007"}]}]}
    })
}

/// A Library of `type_code` (`sql-query` / `sql-view`).
pub fn library(id: &str, type_code: &str, sql: &str, depends_on: &[(&str, &str)]) -> Value {
    json!({
        "resourceType": "Library", "id": id, "name": id, "status": "active",
        "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": type_code}]},
        "content": [{"contentType": "application/sql", "data": B64.encode(sql.as_bytes())}],
        "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
            "type": "depends-on", "label": label, "resource": target
        })).collect::<Vec<_>>()
    })
}

pub const CVD_SQL: &str = "SELECT conditions.patient_id, conditions.code FROM conditions \
    WHERE EXISTS (SELECT 1 FROM cvd_codes WHERE cvd_codes.system = conditions.system \
    AND cvd_codes.code = conditions.code)";

/// A cardiovascular query depending on `dependency` under the label `cvd_codes`.
pub fn cvd_query(id: &str, dependency: &str) -> Value {
    library(
        id,
        "sql-query",
        CVD_SQL,
        &[("conditions", CONDITIONS_URL), ("cvd_codes", dependency)],
    )
}

pub fn expansion_of(resource: &Value) -> Option<&Value> {
    resource.get("expansion")
}

pub fn has_tag(resource: &Value, code: &str) -> bool {
    resource
        .pointer("/meta/tag")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter()
                .any(|t| t["system"] == TAG_SYSTEM && t["code"] == code)
        })
}

pub fn issue_code(body: &Value) -> String {
    body["issue"][0]["code"].as_str().unwrap_or("").to_string()
}

pub fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

pub fn entry(method: &str, url: &str, resource: Option<Value>) -> Value {
    let mut e = json!({"request": {"method": method, "url": url}});
    if let Some(resource) = resource {
        e["resource"] = resource;
    }
    e
}

// =========================================================================
// Query path: both upstream examples, from storage
// =========================================================================

pub async fn cardiovascular_example_returns_p1_and_p2_from_a_stored_value_set(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", cvd_value_set()).await;

    let response = hfs
        .post("/$sql-run", &run_body(spec_cvd_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let rows: Value = response.json();
    assert_eq!(
        sorted_rows(&rows),
        vec![
            ("p1".into(), "22298006".into(), None, None),
            ("p2".into(), "I21".into(), None, None),
        ]
    );
}

pub async fn icd10_example_returns_the_four_translated_rows_from_a_stored_concept_map(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ConceptMap", sct_to_icd10_concept_map()).await;

    let response = hfs
        .post("/$sql-run", &run_body(spec_icd10_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let rows: Value = response.json();
    assert_eq!(
        sorted_rows(&rows),
        vec![
            (
                "p1".into(),
                "22298006".into(),
                Some("I21".into()),
                Some("equivalent".into())
            ),
            ("p2".into(), "I21".into(), None, None),
            (
                "p3".into(),
                "73211009".into(),
                Some("E14".into()),
                Some("source-is-broader-than-target".into())
            ),
            ("p4".into(), "102499006".into(), None, None),
        ]
    );
}

// =========================================================================
// Query path: both examples, from `context`
// =========================================================================

pub async fn both_examples_run_with_the_artifacts_supplied_in_context(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;

    let response = hfs
        .post(
            "/$sql-run",
            &run_body(spec_cvd_query(), "json", vec![cvd_value_set()]),
        )
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(sorted_rows(&response.json()).len(), 2);

    let response = hfs
        .post(
            "/$sql-run",
            &run_body(spec_icd10_query(), "json", vec![sct_to_icd10_concept_map()]),
        )
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let rows = sorted_rows(&response.json());
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[2].2.as_deref(), Some("E14"));

    // A `context` ValueSet is never persisted (D7).
    let stored = hfs
        .storage
        .read(&hfs.tenant, "ValueSet", "cardiovascular-disease")
        .await
        .expect("read");
    assert!(
        stored.is_none(),
        "context artifacts are not written to storage"
    );
}

pub async fn context_value_set_with_an_enumerated_compose_is_expanded_locally_for_the_request(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let compose_only = json!({
        "resourceType": "ValueSet",
        "url": CVD_URL,
        "version": "2026",
        "status": "active",
        "compose": {"include": [
            {"system": SCT, "concept": [{"code": "22298006"}]},
            {"system": ICD10, "concept": [{"code": "I21"}]}
        ]}
    });
    let response = hfs
        .post(
            "/$sql-run",
            &run_body(spec_cvd_query(), "json", vec![compose_only]),
        )
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(sorted_rows(&response.json()).len(), 2);
}

// =========================================================================
// Query path: FHIR output typing
// =========================================================================

pub async fn fhir_format_types_relation_columns_as_uri_code_and_boolean(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut vs = cvd_value_set();
    // Mark the ICD-10 member inactive so the boolean column is populated for
    // one row and null for the other.
    vs["expansion"]["contains"][0]["contains"][1]["inactive"] = json!(true);
    hfs.seed("ValueSet", vs).await;
    hfs.seed("ConceptMap", sct_to_icd10_concept_map()).await;

    let library = sql_lib(
        "typed",
        "typed",
        "SELECT cvd_codes.system, cvd_codes.code, cvd_codes.inactive, sct_to_icd10.relationship \
         FROM cvd_codes LEFT JOIN sct_to_icd10 ON sct_to_icd10.target_code = cvd_codes.code \
         ORDER BY cvd_codes.code",
        &[
            (
                "cvd_codes",
                "http://example.org/ValueSet/cardiovascular-disease|2026",
            ),
            ("sct_to_icd10", MAP_URL),
        ],
    );
    let response = hfs
        .post("/$sql-run", &run_body(library, "fhir", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    let rows = body["parameter"].as_array().expect("parameter array");
    assert_eq!(rows.len(), 2, "{body}");

    // ORDER BY code: 22298006 (SNOMED, active, unmapped as a target) first.
    let sct_row = &rows[0];
    assert_eq!(part(sct_row, "system")["valueUri"], json!(SCT));
    assert_eq!(part(sct_row, "code")["valueCode"], json!("22298006"));
    assert!(
        sct_row["part"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "inactive"),
        "a null inactive is omitted: {sct_row}"
    );

    let icd_row = &rows[1];
    assert_eq!(part(icd_row, "system")["valueUri"], json!(ICD10));
    assert_eq!(part(icd_row, "inactive")["valueBoolean"], json!(true));
    assert_eq!(
        part(icd_row, "relationship")["valueCode"],
        json!("equivalent")
    );
}

// =========================================================================
// Query path: failures (term-20)
// =========================================================================

pub async fn stored_value_set_without_expansion_is_a_422_invalid_naming_the_label(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut vs = cvd_value_set();
    vs.as_object_mut().unwrap().remove("expansion");
    vs["compose"] = json!({"include": [{"system": SCT, "concept": [{"code": "22298006"}]}]});
    hfs.seed("ValueSet", vs).await;

    let response = hfs
        .post("/$sql-run", &run_body(spec_cvd_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    let issue = &outcome["issue"][0];
    assert_eq!(issue["code"], "invalid", "{outcome}");
    let diagnostics = issue["diagnostics"].as_str().unwrap();
    assert!(
        diagnostics.contains("dependency 'cvd_codes'"),
        "{diagnostics}"
    );
    assert!(diagnostics.contains("has no expansion"), "{diagnostics}");
    assert_eq!(
        issue["expression"][0],
        "relatedArtifact.where(label='cvd_codes')"
    );
}

pub async fn unresolvable_canonical_is_a_404_naming_the_four_kinds(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let response = hfs
        .post("/$sql-run", &run_body(spec_cvd_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
    let text = response.text();
    assert!(
        text.contains("ViewDefinition, Library, ValueSet or ConceptMap"),
        "{text}"
    );
    assert!(
        text.contains("http://example.org/ValueSet/cardiovascular-disease|2026"),
        "{text}"
    );
}

pub async fn a_version_pin_only_matches_that_version(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut other_version = cvd_value_set();
    other_version["version"] = json!("2025");
    hfs.seed("ValueSet", other_version).await;
    let response = hfs
        .post("/$sql-run", &run_body(spec_cvd_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
}

pub async fn an_unsupported_concept_map_construct_is_a_422_naming_the_path(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut map = sct_to_icd10_concept_map();
    map["group"][0]["element"][0]["target"][0]["dependsOn"] =
        json!([{"attribute": "laterality", "value": "left"}]);
    hfs.seed("ConceptMap", map).await;
    let response = hfs
        .post("/$sql-run", &run_body(spec_icd10_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    let diagnostics = outcome["issue"][0]["diagnostics"].as_str().unwrap();
    assert!(
        diagnostics.contains("group[0].element[0].target[0] has dependsOn"),
        "{diagnostics}"
    );
}

pub async fn member_cap_is_a_422_too_costly(d: &dyn Driver) {
    let hfs = d
        .server(
            ServerConfig {
                sof_terminology_max_expansion_members: 1,
                ..config("strict", None)
            },
            None,
        )
        .await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", cvd_value_set()).await;
    let response = hfs
        .post("/$sql-run", &run_body(spec_cvd_query(), "json", vec![]))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["issue"][0]["code"], "too-costly", "{outcome}");
}

// =========================================================================
// Query path: `$sql-export`
// =========================================================================

pub async fn poll_to_manifest(hfs: &Hfs, status_url: &str) -> Value {
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let poll = hfs.get(status_url).await;
        match poll.status_code() {
            StatusCode::SEE_OTHER => {
                let result_url = poll
                    .headers()
                    .get(axum::http::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .expect("303 completion response missing Location header")
                    .to_string();
                let result = hfs.get(&result_url).await;
                assert_eq!(result.status_code(), StatusCode::OK, "{}", result.text());
                return result.json::<Value>();
            }
            StatusCode::ACCEPTED => continue,
            other => panic!("unexpected poll status {other}: {}", poll.text()),
        }
    }
    panic!("export did not complete within 5s for {status_url}");
}

pub async fn download_output(hfs: &Hfs, manifest: &Value, name: &str) -> Vec<Value> {
    let output = manifest["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == "output")
        .find(|o| part(o, "name")["valueString"] == name)
        .unwrap_or_else(|| panic!("output {name} in {manifest}"));
    let file_url = part(output, "location")["valueUri"]
        .as_str()
        .unwrap()
        .to_string();
    let path = file_url.strip_prefix("http://localhost").unwrap();
    let download = hfs.get(path).await;
    assert_eq!(download.status_code(), StatusCode::OK);
    download
        .text()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Kicks off `$sql-export` with `Prefer: respond-async` and returns the
/// response.
pub async fn submit_export(hfs: &Hfs, body: &Value) -> TestResponse {
    hfs.post_with(
        "/$sql-export",
        body,
        &[(PREFER, HeaderValue::from_static("respond-async"))],
    )
    .await
}

pub async fn sql_export_runs_both_examples_sharing_one_resolution_per_artifact(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", cvd_value_set()).await;
    hfs.seed("ConceptMap", sct_to_icd10_concept_map()).await;

    // Two subjects depend on the same ValueSet: one resolution serves both.
    let cvd_count = sql_lib(
        "cvd-count",
        "cvd-count",
        "SELECT COUNT(*) AS n FROM cvd_codes",
        &[(
            "cvd_codes",
            "http://example.org/ValueSet/cardiovascular-disease|2026",
        )],
    );
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "subject", "part": [
                {"name": "name", "valueString": "cvd"},
                {"name": "subjectResource", "resource": spec_cvd_query()}
            ]},
            {"name": "subject", "part": [
                {"name": "name", "valueString": "icd10"},
                {"name": "subjectResource", "resource": spec_icd10_query()}
            ]},
            {"name": "subject", "part": [
                {"name": "name", "valueString": "count"},
                {"name": "subjectResource", "resource": cvd_count}
            ]}
        ]
    });
    let submit = submit_export(&hfs, &body).await;
    assert_eq!(
        submit.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        submit.text()
    );
    let location = submit
        .headers()
        .get("content-location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let manifest = poll_to_manifest(&hfs, &location).await;

    let cvd = download_output(&hfs, &manifest, "cvd").await;
    assert_eq!(sorted_rows(&Value::Array(cvd)).len(), 2);
    let icd10 = download_output(&hfs, &manifest, "icd10").await;
    let icd10 = sorted_rows(&Value::Array(icd10));
    assert_eq!(icd10.len(), 4);
    assert_eq!(icd10[0].2.as_deref(), Some("I21"));
    let count = download_output(&hfs, &manifest, "count").await;
    assert_eq!(count[0]["n"], json!(2));
}

pub async fn sql_export_rejects_a_terminology_failure_at_kick_off(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut vs = cvd_value_set();
    vs.as_object_mut().unwrap().remove("expansion");
    hfs.seed("ValueSet", vs).await;
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "subject", "part": [
            {"name": "name", "valueString": "cvd"},
            {"name": "subjectResource", "resource": spec_cvd_query()}
        ]}]
    });
    let submit = submit_export(&hfs, &body).await;
    assert_eq!(
        submit.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        submit.text()
    );
    let outcome: Value = submit.json();
    assert_eq!(outcome["issue"][0]["code"], "invalid", "{outcome}");
}

// =========================================================================
// Write path: T1 and T2 on the direct paths, local expansion (no server)
// =========================================================================

pub async fn put_library_expands_its_stored_enumerated_value_set_locally_and_tags_it(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let vs = hfs
        .put("/ValueSet/cvd", &compose_only_vs("cvd", CVD_URL, None))
        .await;
    assert_eq!(vs.status_code(), StatusCode::CREATED, "{}", vs.text());
    let (v1, before) = hfs.stored("ValueSet", "cvd").await.unwrap();
    assert!(
        expansion_of(&before).is_none(),
        "no Library depends on it yet"
    );

    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(lib.status_code(), StatusCode::CREATED, "{}", lib.text());

    let (v2, after) = hfs.stored("ValueSet", "cvd").await.unwrap();
    assert_ne!(v1, v2, "the expansion is a new history version");
    let expansion = expansion_of(&after).expect("expansion materialized");
    assert_eq!(expansion["total"], 2);
    assert!(
        expansion["identifier"]
            .as_str()
            .unwrap()
            .starts_with("urn:uuid:")
    );
    assert!(expansion.get("timestamp").is_some());
    assert!(has_tag(&after, "materialized-expansion"), "{after}");
    assert_eq!(
        after["compose"], before["compose"],
        "the definition is kept"
    );

    let (status, rows) = hfs.run(&cvd_query("inline", CVD_URL)).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 2);
}

pub async fn post_value_set_after_the_library_is_transformed_in_the_candidate_and_if_match_still_works(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    // The Library first: its dependency is unresolvable, which a write never
    // validated, so it is stored as is.
    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(lib.status_code(), StatusCode::CREATED, "{}", lib.text());

    let posted = hfs
        .post("/ValueSet", &compose_only_vs("ignored", CVD_URL, None))
        .await;
    assert_eq!(
        posted.status_code(),
        StatusCode::CREATED,
        "{}",
        posted.text()
    );
    let body: Value = posted.json();
    assert!(
        expansion_of(&body).is_some(),
        "T2 transformed the candidate: {body}"
    );
    assert!(has_tag(&body, "materialized-expansion"));
    assert_eq!(body["meta"]["versionId"], "1", "one write, not two");
    let id = body["id"].as_str().unwrap().to_string();
    let etag = posted
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // A later PUT with the returned ETag is a normal update.
    let mut again = body.clone();
    again["title"] = json!("renamed");
    let put = hfs
        .put_with(
            &format!("/ValueSet/{id}"),
            &again,
            &[(IF_MATCH, HeaderValue::from_str(&etag).unwrap())],
        )
        .await;
    assert_eq!(put.status_code(), StatusCode::OK, "{}", put.text());

    let (status, rows) = hfs.run(&cvd_query("inline", CVD_URL)).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 2);
}

pub async fn value_set_nobody_depends_on_is_stored_as_sent(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    let posted = hfs
        .post(
            "/ValueSet",
            &compose_only_vs("x", "http://example.org/other", None),
        )
        .await;
    assert_eq!(posted.status_code(), StatusCode::CREATED);
    let body: Value = posted.json();
    assert!(expansion_of(&body).is_none());
    assert!(!has_tag(&body, "materialized-expansion"));
}

pub async fn unexpandable_dependency_is_a_422_in_strict_a_warning_in_lenient_and_ignored_when_off(
    d: &dyn Driver,
) {
    for (mode, expected_status) in [
        ("strict", StatusCode::UNPROCESSABLE_ENTITY),
        ("lenient", StatusCode::CREATED),
        ("off", StatusCode::CREATED),
    ] {
        let hfs = d.server(config(mode, None), None).await;
        hfs.seed_conditions().await;
        hfs.seed("ValueSet", filter_vs("cvd", CVD_URL)).await;
        let (_, before) = hfs.stored("ValueSet", "cvd").await.unwrap();

        let lib = hfs
            .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
            .await;
        assert_eq!(lib.status_code(), expected_status, "{mode}: {}", lib.text());
        let (_, after) = hfs.stored("ValueSet", "cvd").await.unwrap();
        assert_eq!(after, before, "{mode}: the ValueSet is untouched");
        let stored_lib = hfs.stored("Library", "cvd-q").await;
        if mode == "strict" {
            let body: Value = lib.json();
            assert_eq!(issue_code(&body), "invalid", "{body}");
            let text = body["issue"][0]["diagnostics"].as_str().unwrap();
            assert!(text.contains("HFS_TERMINOLOGY_SERVER"), "{text}");
            assert!(stored_lib.is_none(), "nothing written");
        } else {
            assert!(stored_lib.is_some(), "{mode}: the Library is stored");
            let (status, body) = hfs.run(&cvd_query("inline", CVD_URL)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{mode}: {body}");
        }
    }
}

pub async fn member_cap_rejects_the_write_with_nothing_written(d: &dyn Driver) {
    let hfs = d
        .server(
            ServerConfig {
                sof_terminology_max_expansion_members: 1,
                ..config("strict", None)
            },
            None,
        )
        .await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(
        lib.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        lib.text()
    );
    assert_eq!(issue_code(&lib.json()), "too-costly");
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());
    assert!(hfs.stored("Library", "cvd-q").await.is_none());
}

pub async fn transitive_dependencies_through_a_sqlview_are_materialized_and_cycles_are_rejected(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let view_url = "http://example.org/Library/cvd-view";
    let mut view = library(
        "cvd-view",
        "sql-view",
        "SELECT system, code FROM cvd_codes",
        &[("cvd_codes", CVD_URL)],
    );
    view["url"] = json!(view_url);
    hfs.seed("Library", view).await;
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());

    let query = library(
        "q",
        "sql-query",
        "SELECT conditions.patient_id FROM conditions JOIN cvd_view ON cvd_view.code = conditions.code",
        &[("conditions", CONDITIONS_URL), ("cvd_view", view_url)],
    );
    let put = hfs.put("/Library/q", &query).await;
    assert_eq!(put.status_code(), StatusCode::CREATED, "{}", put.text());
    assert!(
        expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some(),
        "reached at depth 2"
    );
    let (status, rows) = hfs.run(&query).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 2);

    // A cycle S1 -> S2 -> S1 reached from a query is rejected, nothing written.
    let s1 = "http://example.org/Library/s1";
    let s2 = "http://example.org/Library/s2";
    let mut lib1 = library("s1", "sql-view", "SELECT * FROM t", &[("t", s2)]);
    lib1["url"] = json!(s1);
    let mut lib2 = library("s2", "sql-view", "SELECT * FROM t", &[("t", s1)]);
    lib2["url"] = json!(s2);
    hfs.seed("Library", lib1).await;
    hfs.seed("Library", lib2).await;
    let cyclic = library("cyc", "sql-query", "SELECT * FROM t", &[("t", s1)]);
    let put = hfs.put("/Library/cyc", &cyclic).await;
    assert_eq!(put.status_code(), StatusCode::BAD_REQUEST, "{}", put.text());
    assert!(
        put.text().contains("dependency cycle detected"),
        "{}",
        put.text()
    );
    assert!(hfs.stored("Library", "cyc").await.is_none());
}

pub async fn patch_adding_a_dependency_materializes_it(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    hfs.seed("Library", library("q", "sql-query", "SELECT 1 AS one", &[]))
        .await;
    let patch = json!([
        {"op": "add", "path": "/relatedArtifact", "value": [
            {"type": "depends-on", "label": "cvd_codes", "resource": CVD_URL}
        ]},
        {"op": "replace", "path": "/content/0/data", "value": B64.encode("SELECT code FROM cvd_codes")}
    ]);
    let response = hfs.json_patch("/Library/q", &patch).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
}

// =========================================================================
// Write path: the terminology server as engine and source of definitions
// =========================================================================

pub async fn with_a_terminology_server_every_expansion_comes_from_it(d: &dyn Driver) {
    let (_mock, url, shared) = mock_hts().await;
    let hfs = d.server(config("strict", Some(&url)), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(lib.status_code(), StatusCode::CREATED, "{}", lib.text());
    let (_, after) = hfs.stored("ValueSet", "cvd").await.unwrap();
    let expansion = expansion_of(&after).unwrap();
    assert_eq!(expansion["identifier"], "urn:uuid:hts-mock", "{after}");
    assert_eq!(shared.lock().unwrap().expand_calls, 1);
    // The server asserted inactivity, which a local expansion never does.
    assert_eq!(expansion["contains"][1]["inactive"], json!(true));
    assert!(has_tag(&after, "materialized-expansion"));
}

pub async fn a_definition_only_the_terminology_server_holds_is_fetched_stored_and_expanded(
    d: &dyn Driver,
) {
    let (_mock, url, shared) = mock_hts().await;
    shared
        .lock()
        .unwrap()
        .value_sets
        .push(compose_only_vs("remote", CVD_URL, Some("2026")));
    let hfs = d.server(config("strict", Some(&url)), None).await;
    hfs.seed_conditions().await;
    let pinned = format!("{CVD_URL}|2026");
    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", &pinned))
        .await;
    assert_eq!(lib.status_code(), StatusCode::CREATED, "{}", lib.text());

    let entries = hfs.find_by_url("ValueSet", CVD_URL).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    let vs = &entries[0];
    assert_eq!(vs["version"], "2026");
    assert_ne!(vs["id"], "remote", "the create assigned its own id");
    assert!(expansion_of(vs).is_some());
    assert!(has_tag(vs, "fetched-from-terminology-server"), "{vs}");

    let (status, rows) = hfs.run(&cvd_query("inline", &pinned)).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 2);
}

pub async fn a_failing_terminology_server_fails_the_write_with_exception_and_writes_nothing(
    d: &dyn Driver,
) {
    let (_mock, url, shared) = mock_hts().await;
    shared.lock().unwrap().fail_expand = true;
    let hfs = d.server(config("strict", Some(&url)), None).await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let (v1, before) = hfs.stored("ValueSet", "cvd").await.unwrap();
    let lib = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(
        lib.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        lib.text()
    );
    assert_eq!(issue_code(&lib.json()), "exception");
    let (v2, after) = hfs.stored("ValueSet", "cvd").await.unwrap();
    assert_eq!((v1, before), (v2, after));
    assert!(hfs.stored("Library", "cvd-q").await.is_none());
}

pub async fn an_unreachable_terminology_server_is_an_exception_in_strict_and_a_stored_library_in_lenient(
    d: &dyn Driver,
) {
    for (mode, status) in [
        ("strict", StatusCode::UNPROCESSABLE_ENTITY),
        ("lenient", StatusCode::CREATED),
    ] {
        let hfs = d
            .server(config(mode, Some("http://127.0.0.1:9")), None)
            .await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let lib = hfs
            .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
            .await;
        assert_eq!(lib.status_code(), status, "{mode}: {}", lib.text());
        assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());
    }
}

// =========================================================================
// Write path: write-free preconditions come first (D13, R3-2, R4-3)
// =========================================================================

pub async fn stale_if_match_is_a_412_before_anything_is_touched(d: &dyn Driver) {
    let hfs = d
        .server(config("strict", Some("http://127.0.0.1:9")), None)
        .await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    hfs.seed(
        "Library",
        library("cvd-q", "sql-query", "SELECT 1 AS one", &[]),
    )
    .await;
    let before = hfs.stored("ValueSet", "cvd").await.unwrap();
    let put = hfs
        .put_with(
            "/Library/cvd-q",
            &cvd_query("cvd-q", CVD_URL),
            &[(IF_MATCH, HeaderValue::from_static("W/\"999\""))],
        )
        .await;
    assert_eq!(
        put.status_code(),
        StatusCode::PRECONDITION_FAILED,
        "{}",
        put.text()
    );
    assert_eq!(
        hfs.stored("ValueSet", "cvd").await.unwrap(),
        before,
        "byte-identical"
    );
}

pub async fn conditional_create_that_matches_returns_the_existing_library_without_touching_anything(
    d: &dyn Driver,
) {
    let hfs = d
        .server(config("strict", Some("http://127.0.0.1:9")), None)
        .await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let mut existing = cvd_query("cvd-q", CVD_URL);
    existing["url"] = json!("http://example.org/Library/cvd-q");
    hfs.seed("Library", existing.clone()).await;
    let before = hfs.stored("ValueSet", "cvd").await.unwrap();
    let post = hfs
        .post_with(
            "/Library",
            &existing,
            &[(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )],
        )
        .await;
    if !hfs.supports_search {
        // No pre-search, so the match is decided inside the create and
        // phase A runs before it (D13): with the terminology server
        // unreachable the write fails there, and nothing is touched. The
        // `url` criterion could not have been evaluated on this backend
        // anyway (`identifier` and `_id` only).
        assert_eq!(
            post.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            post.text()
        );
        assert_eq!(issue_code(&post.json()), "exception");
        assert_eq!(hfs.stored("ValueSet", "cvd").await.unwrap(), before);
        assert_eq!(hfs.all_libraries().await.len(), 1, "no duplicate");
        return;
    }
    assert_eq!(post.status_code(), StatusCode::OK, "{}", post.text());
    assert_eq!(hfs.stored("ValueSet", "cvd").await.unwrap(), before);
}

pub async fn conditional_create_without_a_match_and_an_unexpandable_dependency_writes_nothing(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed("ValueSet", filter_vs("cvd", CVD_URL)).await;
    let mut lib = cvd_query("cvd-q", CVD_URL);
    lib["url"] = json!("http://example.org/Library/cvd-q");
    let post = hfs
        .post_with(
            "/Library",
            &lib,
            &[(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )],
        )
        .await;
    assert_eq!(
        post.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        post.text()
    );
    let libraries = hfs.all_libraries().await;
    assert!(libraries.is_empty(), "{libraries:?}");
}

pub async fn conditional_create_without_a_match_materializes_after_the_committed_create(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let mut lib = cvd_query("cvd-q", CVD_URL);
    lib["url"] = json!("http://example.org/Library/cvd-q");
    let post = hfs
        .post_with(
            "/Library",
            &lib,
            &[(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )],
        )
        .await;
    if !hfs.supports_search {
        // A backend without a search index evaluates `identifier` and `_id`
        // criteria only; `url` is refused before anything is written.
        assert_eq!(
            post.status_code(),
            StatusCode::BAD_REQUEST,
            "{}",
            post.text()
        );
        assert!(
            post.text().contains("`url` cannot be evaluated"),
            "{}",
            post.text()
        );
        assert!(hfs.all_libraries().await.is_empty());
        assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());
        return;
    }
    assert_eq!(post.status_code(), StatusCode::CREATED, "{}", post.text());
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
}

// =========================================================================
// Write path: authorization (D12)
// =========================================================================

pub async fn materialization_needs_the_callers_value_set_update_scope(d: &dyn Driver) {
    // Library-only scopes: strict refuses with 403 naming ValueSet.
    let hfs = d
        .server(
            config("strict", None),
            Some("user/Library.cud user/ValueSet.r"),
        )
        .await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let put = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(put.status_code(), StatusCode::FORBIDDEN, "{}", put.text());
    assert!(put.text().contains("ValueSet"), "{}", put.text());
    assert!(hfs.stored("Library", "cvd-q").await.is_none());
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());

    // Lenient: the Library is stored, the ValueSet left alone.
    let hfs = d
        .server(
            config("lenient", None),
            Some("user/Library.cud user/ValueSet.r"),
        )
        .await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let put = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(put.status_code(), StatusCode::CREATED, "{}", put.text());
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());

    // Wildcard write scopes: materialized.
    let hfs = d.server(config("strict", None), Some("user/*.cruds")).await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let put = hfs
        .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
        .await;
    assert_eq!(put.status_code(), StatusCode::CREATED, "{}", put.text());
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
}

// =========================================================================
// Write path: Bundles
// =========================================================================

pub async fn batch_entries_materialize_independently(d: &dyn Driver) {
    // Batch entries are independent ("the server may process the entries in
    // any order") and run concurrently up to `HFS_BATCH_MAX_CONCURRENCY`, so
    // a Library entry resolving a ValueSet an earlier entry writes is a race
    // on any backend with real concurrency (PostgreSQL loses it). One entry
    // at a time makes the order the scenario relies on real; what it tests
    // is that one entry's failure leaves the others alone.
    let hfs = d
        .server(
            ServerConfig {
                batch_max_concurrency: 1,
                ..config("strict", None)
            },
            None,
        )
        .await;
    hfs.seed_conditions().await;
    let body = bundle(
        "batch",
        vec![
            entry(
                "PUT",
                "ValueSet/cvd",
                Some(compose_only_vs("cvd", CVD_URL, None)),
            ),
            entry("POST", "Library", Some(cvd_query("cvd-q", CVD_URL))),
            // Depends on an unexpandable stored ValueSet: fails alone.
            entry(
                "PUT",
                "ValueSet/bad",
                Some(filter_vs("bad", "http://example.org/ValueSet/bad")),
            ),
            entry(
                "POST",
                "Library",
                Some(cvd_query("bad-q", "http://example.org/ValueSet/bad")),
            ),
        ],
    );
    let response = hfs.post("/", &body).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let out: Value = response.json();
    let statuses: Vec<&str> = out["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["response"]["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "201 Created",
            "201 Created",
            "201 Created",
            "422 Unprocessable Entity"
        ],
        "{out}"
    );
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
    assert!(expansion_of(&hfs.stored("ValueSet", "bad").await.unwrap().1).is_none());
}

pub async fn transaction_with_value_set_and_library_commits_once_in_either_order(d: &dyn Driver) {
    for reversed in [false, true] {
        let hfs = d.server(config("strict", None), None).await;
        hfs.seed_conditions().await;
        let mut entries = vec![
            entry(
                "POST",
                "ValueSet",
                Some(compose_only_vs("cvd", CVD_URL, None)),
            ),
            entry("PUT", "Library/cvd-q", Some(cvd_query("cvd-q", CVD_URL))),
        ];
        if reversed {
            entries.reverse();
        }
        let Some(response) = hfs.post_transaction(&bundle("transaction", entries)).await else {
            return;
        };
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "reversed={reversed}: {}",
            response.text()
        );
        let out: Value = response.json();
        assert_eq!(
            out["entry"].as_array().unwrap().len(),
            2,
            "exactly the request's entries"
        );
        let vs_entry = out["entry"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["resource"]["resourceType"] == "ValueSet")
            .unwrap();
        assert!(
            expansion_of(&vs_entry["resource"]).is_some(),
            "reversed={reversed}: {out}"
        );
        assert_eq!(vs_entry["resource"]["meta"]["versionId"], "1", "one write");
        let (status, rows) = hfs.run(&cvd_query("inline", CVD_URL)).await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 2);
    }
}

pub async fn transaction_needing_a_side_write_is_refused_before_persistence(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    let body = bundle(
        "transaction",
        vec![entry(
            "PUT",
            "Library/cvd-q",
            Some(cvd_query("cvd-q", CVD_URL)),
        )],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let text = response.text();
    assert!(
        text.contains("include the ValueSet in the transaction, or store it first"),
        "{text}"
    );
    assert!(hfs.stored("Library", "cvd-q").await.is_none());
    assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());
}

pub async fn transaction_with_an_unresolvable_dependency_and_no_terminology_server_commits(
    d: &dyn Driver,
) {
    // Nothing could supply the dependency, and a Library write never
    // validated its dependencies (D4): the Library is stored as sent.
    let hfs = d.server(config("strict", None), None).await;
    let body = bundle(
        "transaction",
        vec![entry(
            "PUT",
            "Library/cvd-q",
            Some(cvd_query("cvd-q", CVD_URL)),
        )],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(hfs.stored("Library", "cvd-q").await.is_some());
}

pub async fn transaction_deleting_a_value_set_makes_it_unresolvable_for_a_later_library(
    d: &dyn Driver,
) {
    // The deleted ValueSet is never resurrected: with a terminology server
    // that holds the same canonical, the Library's dependency is found only
    // there, which a transaction cannot store, so the bundle is refused;
    // without one, nothing can supply it and the Library is stored as sent,
    // with the ValueSet gone.
    let (_mock, url, shared) = mock_hts().await;
    shared
        .lock()
        .unwrap()
        .value_sets
        .push(compose_only_vs("remote", CVD_URL, None));
    for hts in [Some(url.as_str()), None] {
        let hfs = d.server(config("strict", hts), None).await;
        hfs.seed_conditions().await;
        let mut expanded = compose_only_vs("cvd", CVD_URL, None);
        expanded["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}]});
        hfs.seed("ValueSet", expanded).await;
        let body = bundle(
            "transaction",
            vec![
                entry("DELETE", "ValueSet/cvd", None),
                entry("POST", "Library", Some(cvd_query("cvd-q", CVD_URL))),
            ],
        );
        let Some(response) = hfs.post_transaction(&body).await else {
            return;
        };
        if hts.is_some() {
            assert_eq!(
                response.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{}",
                response.text()
            );
            assert!(
                response
                    .text()
                    .contains("the terminology server holds this ValueSet"),
                "{}",
                response.text()
            );
            assert!(
                hfs.stored("ValueSet", "cvd").await.is_some(),
                "nothing committed"
            );
        } else {
            assert_eq!(
                response.status_code(),
                StatusCode::OK,
                "{}",
                response.text()
            );
            let gone = hfs.storage.read(&hfs.tenant, "ValueSet", "cvd").await;
            assert!(
                !matches!(gone, Ok(Some(_))),
                "the delete committed: {gone:?}"
            );
            let (status, _) = hfs.run(&cvd_query("inline", CVD_URL)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "no resurrection");
        }
    }
}

pub async fn conditional_post_of_a_value_set_cannot_be_combined_with_materialization(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed("ValueSet", compose_only_vs("x", CVD_URL, None))
        .await;
    let (v_before, before) = hfs.stored("ValueSet", "x").await.unwrap();
    let mut submitted = compose_only_vs("y", CVD_URL, None);
    submitted["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}]});
    let mut conditional = entry("POST", "ValueSet", Some(submitted.clone()));
    conditional["request"]["ifNoneExist"] = json!(format!("url={CVD_URL}"));
    let with_library = bundle(
        "transaction",
        vec![
            conditional.clone(),
            entry("POST", "Library", Some(cvd_query("cvd-q", CVD_URL))),
        ],
    );
    let Some(response) = hfs.post_transaction(&with_library).await else {
        return;
    };
    if !hfs.conditional_in_transaction {
        // The composite refuses any conditional entry inside a transaction
        // before the terminology pre-pass sees it; nothing is written.
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_IMPLEMENTED,
            "{}",
            response.text()
        );
        assert!(
            response.text().contains("conditional interaction"),
            "{}",
            response.text()
        );
        assert_eq!(
            hfs.stored("ValueSet", "x").await.unwrap(),
            (v_before, before)
        );
        return;
    }
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    assert!(
        response
            .text()
            .contains("conditional operations on ValueSet, ConceptMap or Library"),
        "{}",
        response.text()
    );
    assert_eq!(
        hfs.stored("ValueSet", "x").await.unwrap(),
        (v_before, before.clone())
    );

    // Without the Library nothing resolves, so the bundle commits as before.
    let alone = bundle("transaction", vec![conditional]);
    let response = hfs.post("/", &alone).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );

    // Lenient: the bundle commits, the stored ValueSet stays unexpanded.
    let hfs = d.server(config("lenient", None), None).await;
    hfs.seed("ValueSet", compose_only_vs("x", CVD_URL, None))
        .await;
    let response = hfs.post("/", &with_library).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(expansion_of(&hfs.stored("ValueSet", "x").await.unwrap().1).is_none());
}

fn fhir_patch_adding_cvd_dependency() -> Value {
    json!({"resourceType": "Parameters", "parameter": [
        {"name": "operation", "part": [
            {"name": "type", "valueCode": "add"},
            {"name": "path", "valueString": "Library"},
            {"name": "name", "valueString": "relatedArtifact"},
            {"name": "value", "valueRelatedArtifact": {"type": "depends-on", "label": "cvd_codes", "resource": CVD_URL}}
        ]}
    ]})
}

pub async fn transaction_patch_adding_a_dependency_on_an_in_bundle_compose_only_value_set_commits(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    hfs.seed("Library", library("q", "sql-query", "SELECT 1 AS one", &[]))
        .await;
    let body = bundle(
        "transaction",
        vec![
            entry(
                "POST",
                "ValueSet",
                Some(compose_only_vs("cvd", CVD_URL, None)),
            ),
            entry(
                "PATCH",
                "Library/q",
                Some(fhir_patch_adding_cvd_dependency()),
            ),
        ],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let out: Value = response.json();
    let vs = out["entry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["resource"]["resourceType"] == "ValueSet")
        .unwrap();
    assert!(
        expansion_of(&vs["resource"]).is_some(),
        "discovered from the provisional PATCH: {out}"
    );
    let (_, patched) = hfs.stored("Library", "q").await.unwrap();
    assert_eq!(patched["relatedArtifact"][0]["label"], "cvd_codes");
}

pub async fn transaction_patch_declaring_an_unprepared_dependency_rolls_back(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
        .await;
    hfs.seed("Library", library("q", "sql-query", "SELECT 1 AS one", &[]))
        .await;
    let body = bundle(
        "transaction",
        vec![entry(
            "PATCH",
            "Library/q",
            Some(fhir_patch_adding_cvd_dependency()),
        )],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        response.text()
    );
    let (_, lib) = hfs.stored("Library", "q").await.unwrap();
    assert_eq!(lib["relatedArtifact"], json!([]), "rolled back: {lib}");
}

pub async fn two_in_bundle_versions_of_one_url_are_two_candidates(d: &dyn Driver) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut v1 = compose_only_vs("v1", CVD_URL, Some("1"));
    v1["compose"]["include"][1]["concept"] = json!([]);
    v1["compose"]["include"].as_array_mut().unwrap().pop();
    let v2 = compose_only_vs("v2", CVD_URL, Some("2"));
    let body = bundle(
        "transaction",
        vec![
            entry("PUT", "ValueSet/v1", Some(v1)),
            entry("PUT", "ValueSet/v2", Some(v2)),
            entry(
                "PUT",
                "Library/q1",
                Some(cvd_query("q1", &format!("{CVD_URL}|1"))),
            ),
            entry(
                "PUT",
                "Library/q2",
                Some(cvd_query("q2", &format!("{CVD_URL}|2"))),
            ),
        ],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let (s1, rows1) = hfs.run(&cvd_query("i1", &format!("{CVD_URL}|1"))).await;
    let (s2, rows2) = hfs.run(&cvd_query("i2", &format!("{CVD_URL}|2"))).await;
    assert_eq!(
        (s1, s2),
        (StatusCode::OK, StatusCode::OK),
        "{rows1} {rows2}"
    );
    assert_eq!(rows1.as_array().unwrap().len(), 1, "{rows1}");
    assert_eq!(rows2.as_array().unwrap().len(), 2, "{rows2}");
}

pub async fn deleting_the_newest_stored_version_in_the_bundle_resolves_an_unpinned_reference_to_the_survivor(
    d: &dyn Driver,
) {
    let hfs = d.server(config("strict", None), None).await;
    hfs.seed_conditions().await;
    let mut v1 = compose_only_vs("v1", CVD_URL, Some("1"));
    v1["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}]});
    hfs.seed("ValueSet", v1).await;
    let mut v2 = compose_only_vs("v2", CVD_URL, Some("2"));
    v2["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}, {"system": ICD10, "code": "I21"}]});
    hfs.seed("ValueSet", v2).await;
    let body = bundle(
        "transaction",
        vec![
            entry("DELETE", "ValueSet/v2", None),
            entry("PUT", "Library/q", Some(cvd_query("q", CVD_URL))),
        ],
    );
    let Some(response) = hfs.post_transaction(&body).await else {
        return;
    };
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let (status, rows) = hfs.run(&cvd_query("i", CVD_URL)).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 1, "v1 survives: {rows}");
}

// =========================================================================
// The suite
// =========================================================================

/// Expands one `#[tokio::test]` per scenario over `$factory`, an
/// `async fn(&str) -> Option<D>` (`D: Driver`) taking the scenario name;
/// `None` skips the scenario (no Docker). The explicit-list arm lets a suite
/// run a subset, for scenarios it marks `#[ignore]` by hand.
#[macro_export]
macro_rules! terminology_suite {
    ($factory:path) => {
        $crate::terminology_suite!(@each $factory;
            cardiovascular_example_returns_p1_and_p2_from_a_stored_value_set,
            icd10_example_returns_the_four_translated_rows_from_a_stored_concept_map,
            both_examples_run_with_the_artifacts_supplied_in_context,
            context_value_set_with_an_enumerated_compose_is_expanded_locally_for_the_request,
            fhir_format_types_relation_columns_as_uri_code_and_boolean,
            stored_value_set_without_expansion_is_a_422_invalid_naming_the_label,
            unresolvable_canonical_is_a_404_naming_the_four_kinds,
            a_version_pin_only_matches_that_version,
            an_unsupported_concept_map_construct_is_a_422_naming_the_path,
            member_cap_is_a_422_too_costly,
            sql_export_runs_both_examples_sharing_one_resolution_per_artifact,
            sql_export_rejects_a_terminology_failure_at_kick_off,
            put_library_expands_its_stored_enumerated_value_set_locally_and_tags_it,
            post_value_set_after_the_library_is_transformed_in_the_candidate_and_if_match_still_works,
            value_set_nobody_depends_on_is_stored_as_sent,
            unexpandable_dependency_is_a_422_in_strict_a_warning_in_lenient_and_ignored_when_off,
            member_cap_rejects_the_write_with_nothing_written,
            transitive_dependencies_through_a_sqlview_are_materialized_and_cycles_are_rejected,
            patch_adding_a_dependency_materializes_it,
            with_a_terminology_server_every_expansion_comes_from_it,
            a_definition_only_the_terminology_server_holds_is_fetched_stored_and_expanded,
            a_failing_terminology_server_fails_the_write_with_exception_and_writes_nothing,
            an_unreachable_terminology_server_is_an_exception_in_strict_and_a_stored_library_in_lenient,
            stale_if_match_is_a_412_before_anything_is_touched,
            conditional_create_that_matches_returns_the_existing_library_without_touching_anything,
            conditional_create_without_a_match_and_an_unexpandable_dependency_writes_nothing,
            conditional_create_without_a_match_materializes_after_the_committed_create,
            materialization_needs_the_callers_value_set_update_scope,
            batch_entries_materialize_independently,
            transaction_with_value_set_and_library_commits_once_in_either_order,
            transaction_needing_a_side_write_is_refused_before_persistence,
            transaction_with_an_unresolvable_dependency_and_no_terminology_server_commits,
            transaction_deleting_a_value_set_makes_it_unresolvable_for_a_later_library,
            conditional_post_of_a_value_set_cannot_be_combined_with_materialization,
            transaction_patch_adding_a_dependency_on_an_in_bundle_compose_only_value_set_commits,
            transaction_patch_declaring_an_unprepared_dependency_rolls_back,
            two_in_bundle_versions_of_one_url_are_two_candidates,
            deleting_the_newest_stored_version_in_the_bundle_resolves_an_unpinned_reference_to_the_survivor
        );
    };
    (@each $factory:path; $($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                let Some(driver) = $factory(stringify!($name)).await else {
                    eprintln!(concat!("Skipping ", stringify!($name), " (requires Docker)"));
                    return;
                };
                $crate::terminology_scenarios::$name(&driver).await;
            }
        )*
    };
    // Scenarios a suite knows to fail, kept as `#[ignore]` with the reason so
    // `--ignored` runs them and the reason is on the test.
    (@ignored $factory:path; $reason:literal; $($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = $reason]
            async fn $name() {
                let Some(driver) = $factory(stringify!($name)).await else {
                    eprintln!(concat!("Skipping ", stringify!($name), " (requires Docker)"));
                    return;
                };
                $crate::terminology_scenarios::$name(&driver).await;
            }
        )*
    };
}
