//! SQL query ValueSet/ConceptMap dependencies (#1824, #2185), end to end on
//! any backend: a SQLQuery/SQLView Library write has the terminology server
//! expand its terminology dependencies into the primary backend's terminology
//! store, `$sql-run` and `$sql-export` join them by label, and the last
//! Library referencing a relation drops it on delete or on an update that
//! removes the dependency.
//!
//! Each backend's test binary includes this by `#[path]` and instantiates the
//! scenarios with [`sof_terminology_e2e_suite!`], handing it a factory that
//! builds a [`Harness`] over its storage (see [`harness`]). Every harness
//! runs under its own tenant, so scenarios sharing one database or bucket
//! stay apart.

#![allow(dead_code)]

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use futures::future::BoxFuture;
use helios_fhir::FhirVersion;
use helios_persistence::core::history::{
    InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use helios_persistence::core::{
    BundleProvider, ConditionalStorage, ExportDataProvider, GroupExportProvider, IncludeProvider,
    PatientExportProvider, ResourceStorage, RevincludeProvider, SearchProvider,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
const VD_URL: &str = "http://example.org/ViewDefinition/patients";
const VS_URL: &str = "http://example.org/ValueSet/cardio";
const CM_URL: &str = "http://example.org/ConceptMap/to-icd";

/// A server over one backend, the backend itself for seeding and inspecting
/// the terminology store, and the tenant every request runs under.
pub struct Harness {
    pub server: TestServer,
    pub storage: Arc<dyn ResourceStorage>,
    pub tenant: String,
}

/// Builds a [`Harness`] whose server points at `terminology` (the stub's
/// base URL), or `None` when the backend's infrastructure is unavailable.
pub type Factory = fn(Option<String>) -> BoxFuture<'static, Option<Harness>>;

/// The workspace `data/` directory: the spec search parameters, so a stored
/// ValueSet is findable by `url` as in a real deployment.
pub fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// Wires `storage` the way `hfs` does for SQL on FHIR: its own runner, an
/// in-memory export controller, and `terminology` as the terminology server.
pub fn harness<S>(storage: Arc<S>, terminology: Option<String>) -> Harness
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
    let runner = storage.sof_runner().expect("a SOF runner");
    assert!(
        storage.sof_terminology_store().is_some(),
        "every backend with a SOF runner persists terminology (#2185)"
    );
    let mut config = ServerConfig::for_testing();
    config.terminology_server = terminology;
    config.base_url = "http://localhost".to_string();
    let sink = helios_rest::export::InMemorySink::new("http://localhost");
    let controller = helios_rest::export::InMemoryController::new(Arc::clone(&runner), sink, None);
    let state = helios_rest::AppState::new(Arc::clone(&storage), config)
        .with_sof_runner(runner)
        .with_export_controller(Arc::new(controller));
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("failed to create test server");
    Harness {
        server,
        storage,
        tenant: format!("softerm-{}", uuid::Uuid::new_v4().simple()),
    }
}

impl Harness {
    fn tenant(&self) -> TenantContext {
        TenantContext::new(
            TenantId::new(&self.tenant),
            TenantPermissions::full_access(),
        )
    }

    fn with_headers(&self, request: axum_test::TestRequest) -> axum_test::TestRequest {
        request
            .add_header(X_TENANT_ID, HeaderValue::from_str(&self.tenant).unwrap())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
    }

    /// Three patients and the ViewDefinition the queries select from.
    pub async fn seed(&self) {
        for (id, family) in [("p1", "Smith"), ("p2", "Jones"), ("p3", "Brown")] {
            self.storage
                .create(
                    &self.tenant(),
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
                    FhirVersion::R4,
                )
                .await
                .expect("seed patient");
        }
        self.storage
            .create_or_update(
                &self.tenant(),
                "ViewDefinition",
                "patients",
                json!({
                    "resourceType": "ViewDefinition",
                    "id": "patients",
                    "url": VD_URL,
                    "status": "active",
                    "resource": "Patient",
                    "select": [{"column": [
                        {"path": "id", "name": "patient_id", "type": "string"},
                        {"path": "name.family", "name": "family", "type": "string"}
                    ]}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed view definition");
    }

    async fn put_library(&self, lib: &Value) -> axum_test::TestResponse {
        let id = lib["id"].as_str().expect("library id");
        self.with_headers(self.server.put(&format!("/Library/{id}")))
            .json(lib)
            .await
    }

    async fn run(&self, subject: Value) -> axum_test::TestResponse {
        self.with_headers(self.server.post("/$sql-run"))
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "json"},
                    {"name": "subjectResource", "resource": subject}
                ]
            }))
            .await
    }

    async fn run_canonical(&self, url: &str) -> axum_test::TestResponse {
        self.with_headers(self.server.post("/$sql-run"))
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "json"},
                    {"name": "subjectCanonical", "valueCanonical": url}
                ]
            }))
            .await
    }

    /// Rows the backend's terminology store holds for `url`.
    async fn stored_rows(&self, url: &str) -> Option<usize> {
        self.storage
            .sof_terminology_store()
            .expect("terminology store")
            .load(&self.tenant(), url)
            .await
            .expect("load")
            .map(|r| r.rows.len())
    }

    async fn store_value_set(&self, url: &str) {
        self.storage
            .create(
                &self.tenant(),
                "ValueSet",
                json!({"resourceType": "ValueSet", "url": url, "status": "active"}),
                FhirVersion::R4,
            )
            .await
            .expect("seed value set");
    }
}

/// A terminology server stub: `VS_URL` expands (nested, with an abstract
/// parent) to the patient ids `p1` and `p3`; `CM_URL` maps `p1` and marks
/// `p2` as having no map.
async fn terminology_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$expand"))
        .and(wiremock::matchers::body_partial_json(
            json!({"parameter": [{"name": "url", "valueUri": VS_URL}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "url": VS_URL,
            "expansion": {"total": 3, "contains": [
                {"system": "http://ids", "code": "group", "abstract": true, "contains": [
                    {"system": "http://ids", "code": "p1", "display": "One"},
                    {"system": "http://ids", "code": "p3", "inactive": true}
                ]}
            ]}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$expand"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ConceptMap"))
        .and(query_param("url", CM_URL))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Bundle",
            "entry": [{"resource": {
                "resourceType": "ConceptMap",
                "url": CM_URL,
                "group": [{
                    "source": "http://ids",
                    "target": "http://icd|2024",
                    "element": [
                        {"code": "p1", "target": [
                            {"code": "I10", "relationship": "equivalent"}
                        ]},
                        {"code": "p2", "noMap": true}
                    ]
                }]
            }}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ConceptMap"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"resourceType": "Bundle", "entry": []})),
        )
        .mount(&server)
        .await;
    server
}

fn sql_lib(id: &str, url: Option<&str>, sql: &str, depends_on: &[(&str, &str)]) -> Value {
    let mut lib = json!({
        "resourceType": "Library",
        "id": id,
        "status": "active",
        "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
        "content": [{"contentType": "application/sql", "data": B64.encode(sql)}],
        "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
            "type": "depends-on", "label": label, "resource": target
        })).collect::<Vec<_>>(),
    });
    if let Some(url) = url {
        lib["url"] = json!(url);
    }
    lib
}

const SEMI_JOIN: &str = "SELECT p.patient_id FROM pts p WHERE EXISTS \
     (SELECT 1 FROM cardio c WHERE c.code = p.patient_id) ORDER BY p.patient_id";

/// Builds and seeds a harness, or `None` to skip.
async fn start(factory: Factory, terminology: Option<&MockServer>) -> Option<Harness> {
    let h = factory(terminology.map(MockServer::uri)).await?;
    h.seed().await;
    Some(h)
}

pub async fn value_set_dependency_is_materialized_on_write_and_dropped_on_delete(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let lib = sql_lib(
        "cardio-patients",
        Some("http://example.org/Library/cardio-patients"),
        SEMI_JOIN,
        &[("pts", VD_URL), ("cardio", VS_URL)],
    );

    let created = h.with_headers(h.server.post("/Library")).json(&lib).await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"]
        .as_str()
        .expect("assigned id")
        .to_string();
    assert_eq!(h.stored_rows(VS_URL).await, Some(2), "abstract row skipped");

    // The query reads the persisted relation: the terminology server is off
    // the query path from here on.
    ts.reset().await;
    let response = h
        .run_canonical("http://example.org/Library/cardio-patients")
        .await;
    response.assert_status(StatusCode::OK);
    assert_eq!(
        response.json::<Value>(),
        json!([{"patient_id": "p1"}, {"patient_id": "p3"}])
    );

    h.with_headers(h.server.delete(&format!("/Library/{id}")))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(h.stored_rows(VS_URL).await, None);
}

pub async fn shared_table_survives_until_its_last_reference_goes(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let deps = [("pts", VD_URL), ("cardio", VS_URL)];
    h.put_library(&sql_lib("a", None, SEMI_JOIN, &deps))
        .await
        .assert_status(StatusCode::CREATED);
    h.put_library(&sql_lib("b", None, SEMI_JOIN, &deps))
        .await
        .assert_status(StatusCode::CREATED);

    h.with_headers(h.server.delete("/Library/a"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(h.stored_rows(VS_URL).await, Some(2), "b still refers");

    // Updating b to drop the dependency releases the last reference.
    h.put_library(&sql_lib(
        "b",
        None,
        "SELECT patient_id FROM pts",
        &[("pts", VD_URL)],
    ))
    .await
    .assert_status(StatusCode::OK);
    assert_eq!(h.stored_rows(VS_URL).await, None);
}

pub async fn concept_map_dependency_left_joins_with_no_map_rows(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let lib = sql_lib(
        "to-icd",
        Some("http://example.org/Library/to-icd"),
        "SELECT p.patient_id, m.target_system, m.target_version, m.target_code, \
                m.relationship \
           FROM pts p LEFT JOIN icd m ON m.source_code = p.patient_id \
          ORDER BY p.patient_id",
        &[("pts", VD_URL), ("icd", CM_URL)],
    );
    h.put_library(&lib).await.assert_status(StatusCode::CREATED);
    assert_eq!(h.stored_rows(CM_URL).await, Some(2));

    let response = h.run_canonical("http://example.org/Library/to-icd").await;
    response.assert_status(StatusCode::OK);
    assert_eq!(
        response.json::<Value>(),
        json!([
            {"patient_id": "p1", "target_system": "http://icd", "target_version": "2024",
             "target_code": "I10", "relationship": "equivalent"},
            {"patient_id": "p2", "target_system": "http://icd", "target_version": "2024",
             "target_code": null, "relationship": null},
            {"patient_id": "p3", "target_system": null, "target_version": null,
             "target_code": null, "relationship": null}
        ])
    );
}

pub async fn inline_library_is_expanded_for_the_request_only(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let response = h
        .run(sql_lib(
            "inline",
            None,
            SEMI_JOIN,
            &[("pts", VD_URL), ("cardio", VS_URL)],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    assert_eq!(
        response.json::<Value>(),
        json!([{"patient_id": "p1"}, {"patient_id": "p3"}])
    );
    assert_eq!(h.stored_rows(VS_URL).await, None, "nothing persisted");
}

pub async fn value_set_the_terminology_server_cannot_expand_rejects_the_write(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let unknown = "http://example.org/ValueSet/unknown-to-ts";
    h.store_value_set(unknown).await;

    let response = h
        .put_library(&sql_lib(
            "bad",
            None,
            SEMI_JOIN,
            &[("pts", VD_URL), ("cardio", unknown)],
        ))
        .await;
    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        response
            .text()
            .contains("terminology server has no ValueSet")
    );
    h.with_headers(h.server.get("/Library/bad"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

pub async fn no_terminology_server_rejects_a_value_set_dependency(factory: Factory) {
    let Some(h) = start(factory, None).await else {
        return;
    };
    h.store_value_set(VS_URL).await;
    let response = h
        .put_library(&sql_lib(
            "bad",
            None,
            SEMI_JOIN,
            &[("pts", VD_URL), ("cardio", VS_URL)],
        ))
        .await;
    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.text().contains("HFS_TERMINOLOGY_SERVER"));
}

pub async fn unsupported_concept_map_rejects_the_write(factory: Factory) {
    let ts = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$expand"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&ts)
        .await;
    Mock::given(method("GET"))
        .and(path("/ConceptMap"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Bundle",
            "entry": [{"resource": {"resourceType": "ConceptMap", "group": [{
                "source": "http://a",
                "element": [{"code": "1", "target": [
                    {"code": "x", "relationship": "equivalent", "dependsOn": [{}]}
                ]}]
            }]}}]
        })))
        .mount(&ts)
        .await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let response = h
        .put_library(&sql_lib(
            "bad",
            None,
            "SELECT * FROM icd",
            &[("icd", CM_URL)],
        ))
        .await;
    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.text().contains("dependsOn"));
}

/// `$sql-export` reads the persisted relation too: the terminology server is
/// gone by the time the job runs.
pub async fn sql_export_reads_the_persisted_value_set_table(factory: Factory) {
    let ts = terminology_server().await;
    let Some(h) = start(factory, Some(&ts)).await else {
        return;
    };
    let deps = [("pts", VD_URL), ("cardio", VS_URL)];
    h.put_library(&sql_lib("stored", None, SEMI_JOIN, &deps))
        .await
        .assert_status(StatusCode::CREATED);
    ts.reset().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "subject", "part": [
            {"name": "name", "valueString": "cardio"},
            {"name": "subjectResource", "resource": sql_lib("inline", None, SEMI_JOIN, &deps)}
        ]}]
    });
    let submit = h
        .with_headers(h.server.post("/$sql-export"))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("respond-async"),
        )
        .json(&body)
        .await;
    submit.assert_status(StatusCode::ACCEPTED);
    let status_url = submit
        .headers()
        .get("content-location")
        .and_then(|v| v.to_str().ok())
        .expect("content-location")
        .to_string();

    let mut manifest = None;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let poll = h.with_headers(h.server.get(&status_url)).await;
        match poll.status_code() {
            StatusCode::ACCEPTED => continue,
            StatusCode::SEE_OTHER => {
                let result_url = poll
                    .headers()
                    .get(axum::http::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .expect("location")
                    .to_string();
                manifest = Some(
                    h.with_headers(h.server.get(&result_url))
                        .await
                        .json::<Value>(),
                );
                break;
            }
            other => panic!("unexpected poll status {other}: {}", poll.text()),
        }
    }
    let manifest = manifest.expect("export finished");
    let file_url = manifest["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "output")
        .and_then(|o| o["part"].as_array())
        .and_then(|parts| parts.iter().find(|p| p["name"] == "location"))
        .and_then(|p| p["valueUri"].as_str())
        .expect("output location")
        .to_string();
    let download = h
        .with_headers(
            h.server
                .get(file_url.strip_prefix("http://localhost").unwrap()),
        )
        .await;
    download.assert_status(StatusCode::OK);
    let ids: Vec<Value> = download
        .text()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).unwrap()["patient_id"].clone())
        .collect();
    assert_eq!(ids, vec![json!("p1"), json!("p3")]);
}

/// Instantiates every scenario as a `#[tokio::test]` over `$factory`, an
/// `async fn(Option<String>) -> Option<Harness>`.
#[macro_export]
macro_rules! sof_terminology_e2e_suite {
    ($factory:path) => {
        $crate::sof_terminology_e2e_suite!(@each $factory;
            value_set_dependency_is_materialized_on_write_and_dropped_on_delete,
            shared_table_survives_until_its_last_reference_goes,
            concept_map_dependency_left_joins_with_no_map_rows,
            inline_library_is_expanded_for_the_request_only,
            value_set_the_terminology_server_cannot_expand_rejects_the_write,
            no_terminology_server_rejects_a_value_set_dependency,
            unsupported_concept_map_rejects_the_write,
            sql_export_reads_the_persisted_value_set_table,
        );
    };
    (@each $factory:path; $($name:ident),* $(,)?) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn $name() {
                $crate::sof_terminology_e2e::$name(|terminology| {
                    Box::pin($factory(terminology))
                })
                .await;
            }
        )*
    };
}
