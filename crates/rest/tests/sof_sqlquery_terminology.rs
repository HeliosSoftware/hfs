//! Integration tests for terminology dependencies of SQL Query Libraries
//! (SQL on FHIR "Terminology in SQL", HL7/sql-on-fhir#407; HFS #1824): a
//! Library's `relatedArtifact[depends-on]` may name a ValueSet or a
//! ConceptMap, which the runner exposes under its label as a relation with a
//! fixed column set. The spec page's own worked example — the `conditions`
//! view, four Conditions, the cardiovascular value set and the SNOMED CT to
//! ICD-10 map — runs here verbatim, from storage and from `context`, through
//! `$sql-run` and `$sql-export`.

mod sof_sqlquery_terminology_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::SofRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{InMemoryController, InMemorySink};
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
    const PREFER: HeaderName = HeaderName::from_static("prefer");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

    const CONDITIONS_URL: &str = "https://example.org/ViewDefinition/conditions";
    const CVD_URL: &str = "http://example.org/ValueSet/cardiovascular-disease";
    const MAP_URL: &str = "http://example.org/ConceptMap/sct-to-icd10";
    const SCT: &str = "http://snomed.info/sct";
    const ICD10: &str = "http://hl7.org/fhir/sid/icd-10";

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        create_test_server_with_config(ServerConfig::for_testing()).await
    }

    async fn create_test_server_with_config(
        config: ServerConfig,
    ) -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);
        let runner: Arc<dyn SofRunner> = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");
        let sink = InMemorySink::new("http://localhost");
        let controller = InMemoryController::new(Arc::clone(&runner), sink, None);
        let config = ServerConfig {
            base_url: "http://localhost".to_string(),
            ..config
        };
        let state = helios_rest::AppState::new(Arc::clone(&backend), config)
            .with_sof_runner(runner)
            .with_export_controller(Arc::new(controller));
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");
        (server, backend)
    }

    // ---- the spec page's fixtures, verbatim ---------------------------------

    fn conditions_view_definition() -> Value {
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

    fn cvd_value_set() -> Value {
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

    fn sct_to_icd10_concept_map() -> Value {
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

    const CVD_SQL: &str = "SELECT conditions.patient_id, conditions.code
FROM conditions
WHERE EXISTS (
  SELECT 1
  FROM cvd_codes
  WHERE cvd_codes.system = conditions.system
    AND cvd_codes.code = conditions.code
)";

    const ICD10_SQL: &str = "SELECT conditions.patient_id,
       conditions.code,
       sct_to_icd10.target_code,
       sct_to_icd10.relationship
FROM conditions
LEFT JOIN sct_to_icd10
  ON sct_to_icd10.source_system = conditions.system
 AND sct_to_icd10.source_code = conditions.code
 AND (sct_to_icd10.relationship IS NULL
      OR sct_to_icd10.relationship <> 'not-related-to')";

    fn sql_lib(id: &str, name: &str, sql: &str, depends_on: &[(&str, &str)]) -> Value {
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

    fn cvd_query() -> Value {
        sql_lib(
            "CardiovascularDiseasePatientsQuery",
            "CardiovascularDiseasePatientsQuery",
            CVD_SQL,
            &[
                ("conditions", CONDITIONS_URL),
                (
                    "cvd_codes",
                    "http://example.org/ValueSet/cardiovascular-disease|2026",
                ),
            ],
        )
    }

    fn icd10_query() -> Value {
        sql_lib(
            "ConditionsToIcd10Query",
            "ConditionsToIcd10Query",
            ICD10_SQL,
            &[
                ("conditions", CONDITIONS_URL),
                (
                    "sct_to_icd10",
                    "http://example.org/ConceptMap/sct-to-icd10|2026",
                ),
            ],
        )
    }

    async fn seed(backend: &SqliteBackend, resource_type: &str, resource: Value) {
        let id = resource["id"].as_str().expect("fixture id").to_string();
        backend
            .create_or_update(&tenant(), resource_type, &id, resource, FhirVersion::R4)
            .await
            .unwrap_or_else(|e| panic!("seed {resource_type}/{id}: {e}"));
    }

    /// The four Conditions of the spec page's `conditions` table.
    async fn seed_conditions(backend: &SqliteBackend) {
        seed(backend, "ViewDefinition", conditions_view_definition()).await;
        for (patient, system, code) in [
            ("p1", SCT, "22298006"),
            ("p2", ICD10, "I21"),
            ("p3", SCT, "73211009"),
            ("p4", SCT, "102499006"),
        ] {
            seed(
                backend,
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

    fn run_body(library: Value, format: &str, context: Vec<Value>) -> Value {
        let mut params = vec![
            json!({"name": "_format", "valueCode": format}),
            json!({"name": "subjectResource", "resource": library}),
        ];
        for artifact in context {
            params.push(json!({"name": "context", "resource": artifact}));
        }
        json!({"resourceType": "Parameters", "parameter": params})
    }

    async fn post_sql_run(server: &TestServer, body: &Value) -> axum_test::TestResponse {
        server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(body)
            .await
    }

    /// Rows as `(patient_id, code, target_code, relationship)` tuples sorted
    /// by patient, so order is not asserted.
    fn sorted_rows(rows: &Value) -> Vec<(String, String, Option<String>, Option<String>)> {
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

    fn part<'a>(param: &'a Value, name: &str) -> &'a Value {
        param["part"]
            .as_array()
            .expect("part array")
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("part {name} in {param}"))
    }

    // =========================================================================
    // Both upstream examples, from storage
    // =========================================================================

    #[tokio::test]
    async fn cardiovascular_example_returns_p1_and_p2_from_a_stored_value_set() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        seed(&backend, "ValueSet", cvd_value_set()).await;

        let response = post_sql_run(&server, &run_body(cvd_query(), "json", vec![])).await;
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

    #[tokio::test]
    async fn icd10_example_returns_the_four_translated_rows_from_a_stored_concept_map() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        seed(&backend, "ConceptMap", sct_to_icd10_concept_map()).await;

        let response = post_sql_run(&server, &run_body(icd10_query(), "json", vec![])).await;
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
    // Both examples, from `context`
    // =========================================================================

    #[tokio::test]
    async fn both_examples_run_with_the_artifacts_supplied_in_context() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;

        let response = post_sql_run(
            &server,
            &run_body(cvd_query(), "json", vec![cvd_value_set()]),
        )
        .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        assert_eq!(sorted_rows(&response.json()).len(), 2);

        let response = post_sql_run(
            &server,
            &run_body(icd10_query(), "json", vec![sct_to_icd10_concept_map()]),
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
        let stored = backend
            .read(&tenant(), "ValueSet", "cardiovascular-disease")
            .await
            .expect("read");
        assert!(
            stored.is_none(),
            "context artifacts are not written to storage"
        );
    }

    #[tokio::test]
    async fn context_value_set_with_an_enumerated_compose_is_expanded_locally_for_the_request() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
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
        let response =
            post_sql_run(&server, &run_body(cvd_query(), "json", vec![compose_only])).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        assert_eq!(sorted_rows(&response.json()).len(), 2);
    }

    // =========================================================================
    // FHIR output typing: uri / code / boolean columns
    // =========================================================================

    #[tokio::test]
    async fn fhir_format_types_relation_columns_as_uri_code_and_boolean() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let mut vs = cvd_value_set();
        // Mark the ICD-10 member inactive so the boolean column is populated
        // for one row and null for the other.
        vs["expansion"]["contains"][0]["contains"][1]["inactive"] = json!(true);
        seed(&backend, "ValueSet", vs).await;
        seed(&backend, "ConceptMap", sct_to_icd10_concept_map()).await;

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
        let response = post_sql_run(&server, &run_body(library, "fhir", vec![])).await;
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
    // Failures: term-20
    // =========================================================================

    #[tokio::test]
    async fn stored_value_set_without_expansion_is_a_422_invalid_naming_the_label() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let mut vs = cvd_value_set();
        vs.as_object_mut().unwrap().remove("expansion");
        vs["compose"] = json!({"include": [{"system": SCT, "concept": [{"code": "22298006"}]}]});
        seed(&backend, "ValueSet", vs).await;

        let response = post_sql_run(&server, &run_body(cvd_query(), "json", vec![])).await;
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

    #[tokio::test]
    async fn unresolvable_canonical_is_a_404_naming_the_four_kinds() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let response = post_sql_run(&server, &run_body(cvd_query(), "json", vec![])).await;
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

    #[tokio::test]
    async fn a_version_pin_only_matches_that_version() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let mut other_version = cvd_value_set();
        other_version["version"] = json!("2025");
        seed(&backend, "ValueSet", other_version).await;
        let response = post_sql_run(&server, &run_body(cvd_query(), "json", vec![])).await;
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_FOUND,
            "{}",
            response.text()
        );
    }

    #[tokio::test]
    async fn an_unsupported_concept_map_construct_is_a_422_naming_the_path() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let mut map = sct_to_icd10_concept_map();
        map["group"][0]["element"][0]["target"][0]["dependsOn"] =
            json!([{"attribute": "laterality", "value": "left"}]);
        seed(&backend, "ConceptMap", map).await;
        let response = post_sql_run(&server, &run_body(icd10_query(), "json", vec![])).await;
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

    #[tokio::test]
    async fn member_cap_is_a_422_too_costly() {
        let config = ServerConfig {
            sof_terminology_max_expansion_members: 1,
            ..ServerConfig::for_testing()
        };
        let (server, backend) = create_test_server_with_config(config).await;
        seed_conditions(&backend).await;
        seed(&backend, "ValueSet", cvd_value_set()).await;
        let response = post_sql_run(&server, &run_body(cvd_query(), "json", vec![])).await;
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
    // `$sql-export`
    // =========================================================================

    async fn poll_to_manifest(server: &TestServer, status_url: &str) -> Value {
        for _ in 0..40 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let poll = server
                .get(status_url)
                .add_header(X_TENANT_ID, "test-tenant")
                .await;
            match poll.status_code() {
                StatusCode::SEE_OTHER => {
                    let result_url = poll
                        .headers()
                        .get(axum::http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .expect("303 completion response missing Location header")
                        .to_string();
                    let result = server
                        .get(&result_url)
                        .add_header(X_TENANT_ID, "test-tenant")
                        .await;
                    assert_eq!(result.status_code(), StatusCode::OK, "{}", result.text());
                    return result.json::<Value>();
                }
                StatusCode::ACCEPTED => continue,
                other => panic!("unexpected poll status {other}: {}", poll.text()),
            }
        }
        panic!("export did not complete within 2s for {status_url}");
    }

    async fn download_output(server: &TestServer, manifest: &Value, name: &str) -> Vec<Value> {
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
        let download = server
            .get(path)
            .add_header(X_TENANT_ID, "test-tenant")
            .await;
        assert_eq!(download.status_code(), StatusCode::OK);
        download
            .text()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn sql_export_runs_both_examples_sharing_one_resolution_per_artifact() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        seed(&backend, "ValueSet", cvd_value_set()).await;
        seed(&backend, "ConceptMap", sct_to_icd10_concept_map()).await;

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
                    {"name": "subjectResource", "resource": cvd_query()}
                ]},
                {"name": "subject", "part": [
                    {"name": "name", "valueString": "icd10"},
                    {"name": "subjectResource", "resource": icd10_query()}
                ]},
                {"name": "subject", "part": [
                    {"name": "name", "valueString": "count"},
                    {"name": "subjectResource", "resource": cvd_count}
                ]}
            ]
        });
        let submit = server
            .post("/$sql-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
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
        let manifest = poll_to_manifest(&server, &location).await;

        let cvd = download_output(&server, &manifest, "cvd").await;
        assert_eq!(sorted_rows(&Value::Array(cvd)).len(), 2);
        let icd10 = download_output(&server, &manifest, "icd10").await;
        let icd10 = sorted_rows(&Value::Array(icd10));
        assert_eq!(icd10.len(), 4);
        assert_eq!(icd10[0].2.as_deref(), Some("I21"));
        let count = download_output(&server, &manifest, "count").await;
        assert_eq!(count[0]["n"], json!(2));
    }

    #[tokio::test]
    async fn sql_export_rejects_a_terminology_failure_at_kick_off() {
        let (server, backend) = create_test_server().await;
        seed_conditions(&backend).await;
        let mut vs = cvd_value_set();
        vs.as_object_mut().unwrap().remove("expansion");
        seed(&backend, "ValueSet", vs).await;
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subject", "part": [
                {"name": "name", "valueString": "cvd"},
                {"name": "subjectResource", "resource": cvd_query()}
            ]}]
        });
        let submit = server
            .post("/$sql-export")
            .add_header(PREFER, "respond-async")
            .add_header(X_TENANT_ID, "test-tenant")
            .json(&body)
            .await;
        assert_eq!(
            submit.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            submit.text()
        );
        let outcome: Value = submit.json();
        assert_eq!(outcome["issue"][0]["code"], "invalid", "{outcome}");
    }
}
