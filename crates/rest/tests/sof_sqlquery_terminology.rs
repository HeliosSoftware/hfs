//! SQL query ValueSet/ConceptMap dependencies (#1824), end to end: a
//! SQLQuery/SQLView Library write has the terminology server expand its
//! terminology dependencies into tables in the primary (SQLite) backend,
//! `$sql-run` joins them by label, and the last Library referencing a table
//! drops it on delete or on an update that removes the dependency.

mod sof_sqlquery_terminology_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
    const VD_URL: &str = "http://example.org/ViewDefinition/patients";
    const VS_URL: &str = "http://example.org/ValueSet/cardio";
    const CM_URL: &str = "http://example.org/ConceptMap/to-icd";

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn server_with(terminology: Option<&MockServer>) -> (TestServer, Arc<SqliteBackend>) {
        // The workspace `data/` directory: the spec search parameters, so a
        // stored ValueSet is findable by `url` as in a real deployment.
        let config = SqliteBackendConfig {
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", config)
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);
        let runner = backend.sof_runner().expect("sqlite SOF runner");

        let mut config = ServerConfig::for_testing();
        config.terminology_server = terminology.map(MockServer::uri);
        let sink = helios_rest::export::InMemorySink::new("http://localhost");
        let controller =
            helios_rest::export::InMemoryController::new(Arc::clone(&runner), sink, None);
        config.base_url = "http://localhost".to_string();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_sof_runner(runner)
            .with_export_controller(Arc::new(controller));
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        for (id, family) in [("p1", "Smith"), ("p2", "Jones"), ("p3", "Brown")] {
            backend
                .create(
                    &tenant(),
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
                    FhirVersion::R4,
                )
                .await
                .expect("seed patient");
        }
        backend
            .create_or_update(
                &tenant(),
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
        (server, backend)
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

    fn with_headers(request: axum_test::TestRequest) -> axum_test::TestRequest {
        request
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
    }

    async fn put_library(server: &TestServer, lib: &Value) -> axum_test::TestResponse {
        let id = lib["id"].as_str().expect("library id");
        with_headers(server.put(&format!("/Library/{id}")))
            .json(lib)
            .await
    }

    async fn run(server: &TestServer, subject: Value) -> axum_test::TestResponse {
        with_headers(server.post("/$sql-run"))
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "json"},
                    {"name": "subjectResource", "resource": subject}
                ]
            }))
            .await
    }

    async fn run_canonical(server: &TestServer, url: &str) -> axum_test::TestResponse {
        with_headers(server.post("/$sql-run"))
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "json"},
                    {"name": "subjectCanonical", "valueCanonical": url}
                ]
            }))
            .await
    }

    async fn stored_rows(backend: &SqliteBackend, url: &str) -> Option<usize> {
        backend
            .sof_terminology_store()
            .expect("sqlite terminology store")
            .load(&tenant(), url)
            .await
            .expect("load")
            .map(|r| r.rows.len())
    }

    #[tokio::test]
    async fn value_set_dependency_is_materialized_on_write_and_dropped_on_delete() {
        let ts = terminology_server().await;
        let (server, backend) = server_with(Some(&ts)).await;
        let lib = sql_lib(
            "cardio-patients",
            Some("http://example.org/Library/cardio-patients"),
            SEMI_JOIN,
            &[("pts", VD_URL), ("cardio", VS_URL)],
        );

        with_headers(server.post("/Library"))
            .json(&lib)
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(
            stored_rows(&backend, VS_URL).await,
            Some(2),
            "abstract row skipped"
        );

        // The query reads the persisted table: the terminology server is off
        // the query path from here on.
        ts.reset().await;
        let response = run_canonical(&server, "http://example.org/Library/cardio-patients").await;
        response.assert_status(StatusCode::OK);
        assert_eq!(
            response.json::<Value>(),
            json!([{"patient_id": "p1"}, {"patient_id": "p3"}])
        );

        let id = library_id(&server, "http://example.org/Library/cardio-patients").await;
        with_headers(server.delete(&format!("/Library/{id}")))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(stored_rows(&backend, VS_URL).await, None);
    }

    /// The id the server assigned a POSTed Library, found by its canonical.
    async fn library_id(server: &TestServer, url: &str) -> String {
        let bundle: Value = with_headers(server.get(&format!("/Library?url={url}")))
            .await
            .json();
        bundle["entry"][0]["resource"]["id"]
            .as_str()
            .expect("library id")
            .to_string()
    }

    #[tokio::test]
    async fn shared_table_survives_until_its_last_reference_goes() {
        let ts = terminology_server().await;
        let (server, backend) = server_with(Some(&ts)).await;
        let deps = [("pts", VD_URL), ("cardio", VS_URL)];
        put_library(&server, &sql_lib("a", None, SEMI_JOIN, &deps))
            .await
            .assert_status(StatusCode::CREATED);
        put_library(&server, &sql_lib("b", None, SEMI_JOIN, &deps))
            .await
            .assert_status(StatusCode::CREATED);

        with_headers(server.delete("/Library/a"))
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            stored_rows(&backend, VS_URL).await,
            Some(2),
            "b still refers"
        );

        // Updating b to drop the dependency releases the last reference.
        put_library(
            &server,
            &sql_lib("b", None, "SELECT patient_id FROM pts", &[("pts", VD_URL)]),
        )
        .await
        .assert_status(StatusCode::OK);
        assert_eq!(stored_rows(&backend, VS_URL).await, None);
    }

    #[tokio::test]
    async fn concept_map_dependency_left_joins_with_no_map_rows() {
        let ts = terminology_server().await;
        let (server, backend) = server_with(Some(&ts)).await;
        let lib = sql_lib(
            "to-icd",
            Some("http://example.org/Library/to-icd"),
            "SELECT p.patient_id, m.target_system, m.target_version, m.target_code, \
                    m.relationship \
               FROM pts p LEFT JOIN icd m ON m.source_code = p.patient_id \
              ORDER BY p.patient_id",
            &[("pts", VD_URL), ("icd", CM_URL)],
        );
        put_library(&server, &lib)
            .await
            .assert_status(StatusCode::CREATED);
        assert_eq!(stored_rows(&backend, CM_URL).await, Some(2));

        let response = run_canonical(&server, "http://example.org/Library/to-icd").await;
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

    #[tokio::test]
    async fn inline_library_is_expanded_for_the_request_only() {
        let ts = terminology_server().await;
        let (server, backend) = server_with(Some(&ts)).await;
        let response = run(
            &server,
            sql_lib(
                "inline",
                None,
                SEMI_JOIN,
                &[("pts", VD_URL), ("cardio", VS_URL)],
            ),
        )
        .await;
        response.assert_status(StatusCode::OK);
        assert_eq!(
            response.json::<Value>(),
            json!([{"patient_id": "p1"}, {"patient_id": "p3"}])
        );
        assert_eq!(
            stored_rows(&backend, VS_URL).await,
            None,
            "nothing persisted"
        );
    }

    async fn store_value_set(backend: &SqliteBackend, url: &str) {
        backend
            .create(
                &tenant(),
                "ValueSet",
                json!({"resourceType": "ValueSet", "url": url, "status": "active"}),
                FhirVersion::R4,
            )
            .await
            .expect("seed value set");
    }

    #[tokio::test]
    async fn value_set_the_terminology_server_cannot_expand_rejects_the_write() {
        let ts = terminology_server().await;
        let (server, backend) = server_with(Some(&ts)).await;
        let unknown = "http://example.org/ValueSet/unknown-to-ts";
        store_value_set(&backend, unknown).await;

        let response = put_library(
            &server,
            &sql_lib(
                "bad",
                None,
                SEMI_JOIN,
                &[("pts", VD_URL), ("cardio", unknown)],
            ),
        )
        .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            response
                .text()
                .contains("terminology server has no ValueSet")
        );
        with_headers(server.get("/Library/bad"))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn no_terminology_server_rejects_a_value_set_dependency() {
        let (server, backend) = server_with(None).await;
        store_value_set(&backend, VS_URL).await;
        let response = put_library(
            &server,
            &sql_lib(
                "bad",
                None,
                SEMI_JOIN,
                &[("pts", VD_URL), ("cardio", VS_URL)],
            ),
        )
        .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.text().contains("HFS_TERMINOLOGY_SERVER"));
    }

    #[tokio::test]
    async fn unsupported_concept_map_rejects_the_write() {
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
        let (server, _backend) = server_with(Some(&ts)).await;
        let response = put_library(
            &server,
            &sql_lib("bad", None, "SELECT * FROM icd", &[("icd", CM_URL)]),
        )
        .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.text().contains("dependsOn"));
    }

    /// `$sql-export` reads the persisted table too: the terminology server is
    /// gone by the time the job runs.
    #[tokio::test]
    async fn sql_export_reads_the_persisted_value_set_table() {
        let ts = terminology_server().await;
        let (server, _backend) = server_with(Some(&ts)).await;
        let deps = [("pts", VD_URL), ("cardio", VS_URL)];
        put_library(&server, &sql_lib("stored", None, SEMI_JOIN, &deps))
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
        let submit = with_headers(server.post("/$sql-export"))
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
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let poll = with_headers(server.get(&status_url)).await;
            match poll.status_code() {
                StatusCode::ACCEPTED => continue,
                StatusCode::SEE_OTHER => {
                    let result_url = poll
                        .headers()
                        .get(axum::http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .expect("location")
                        .to_string();
                    manifest = Some(with_headers(server.get(&result_url)).await.json::<Value>());
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
        let download =
            with_headers(server.get(file_url.strip_prefix("http://localhost").unwrap())).await;
        download.assert_status(StatusCode::OK);
        let ids: Vec<Value> = download
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap()["patient_id"].clone())
            .collect();
        assert_eq!(ids, vec![json!("p1"), json!("p3")]);
    }
}
