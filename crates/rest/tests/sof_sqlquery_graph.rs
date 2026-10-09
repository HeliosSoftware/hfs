//! Integration tests for the two-phase dependency-graph resolver (design
//! #567): SQLQuery -> SQLView chains, diamond dependencies, cycles, the
//! fixed depth limit, the SQLView `parameter 0..0` profile rule, and
//! aggregated Phase 1 errors — exercised end to end through `$sql-run`.

mod sof_sqlquery_graph_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use futures::TryStreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";

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

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) {
        let p = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": family}],
        });
        backend
            .create(&tenant(), "Patient", p, FhirVersion::R4)
            .await
            .expect("seed patient");
    }

    /// Seeds a ViewDefinition flattening Patient to (`patient_id`, `family`)
    /// under `url`, returning that canonical URL.
    async fn seed_view_definition(backend: &SqliteBackend, id: &str, url: &str) -> String {
        let vd = json!({
            "resourceType": "ViewDefinition",
            "id": id,
            "url": url,
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "name.family", "name": "family", "type": "string"}
                ]
            }]
        });
        backend
            .create_or_update(&tenant(), "ViewDefinition", id, vd, FhirVersion::R4)
            .await
            .expect("seed view definition");
        url.to_string()
    }

    /// Builds a spec-conforming SQLQuery or SQLView Library.
    fn sql_lib(
        id: &str,
        url: Option<&str>,
        type_code: &str,
        sql: &str,
        depends_on: &[(&str, &str)],
        parameters: Vec<Value>,
    ) -> Value {
        let data = B64.encode(sql.as_bytes());
        let mut lib = json!({
            "resourceType": "Library",
            "id": id,
            "status": "active",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": type_code}]},
            "content": [{ "contentType": "application/sql", "data": data }],
            "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
                "type": "depends-on",
                "label": label,
                "resource": target
            })).collect::<Vec<_>>(),
        });
        if let Some(u) = url {
            lib["url"] = json!(u);
        }
        if !parameters.is_empty() {
            lib["parameter"] = json!(parameters);
        }
        lib
    }

    async fn seed_library(backend: &SqliteBackend, id: &str, lib: Value) {
        backend
            .create_or_update(&tenant(), "Library", id, lib, FhirVersion::R4)
            .await
            .expect("seed library");
    }

    fn run_body_inline(library: Value, format: &str) -> Value {
        json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": format},
                {"name": "subjectResource", "resource": library}
            ]
        })
    }

    /// Like [`run_body_inline`] but also attaches one `context` parameter per
    /// entry in `context`.
    fn run_body_with_context(library: Value, format: &str, context: Vec<Value>) -> Value {
        let mut body = run_body_inline(library, format);
        let params = body["parameter"].as_array_mut().expect("parameter array");
        for artifact in context {
            params.push(json!({"name": "context", "resource": artifact}));
        }
        body
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

    // =========================================================================
    // Chain of 3 levels: SQLQuery -> SQLView -> SQLView -> ViewDefinition
    // =========================================================================

    #[tokio::test]
    async fn three_level_chain_resolves_correctly() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let leaf_url = seed_view_definition(&backend, "leaf", "http://example.org/leaf").await;
        let inner_url = "http://example.org/inner";
        let mid_url = "http://example.org/mid";

        seed_library(
            &backend,
            "inner",
            sql_lib(
                "inner",
                Some(inner_url),
                "sql-view",
                "SELECT * FROM leaf_t",
                &[("leaf_t", &leaf_url)],
                vec![],
            ),
        )
        .await;
        seed_library(
            &backend,
            "mid",
            sql_lib(
                "mid",
                Some(mid_url),
                "sql-view",
                "SELECT * FROM inner_t",
                &[("inner_t", inner_url)],
                vec![],
            ),
        )
        .await;

        let subject = sql_lib(
            "subject",
            None,
            "sql-query",
            "SELECT patient_id, family FROM mid_t ORDER BY patient_id",
            &[("mid_t", mid_url)],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["patient_id"], json!("p1"));
        assert_eq!(rows[0]["family"], json!("Smith"));
        assert_eq!(rows[1]["patient_id"], json!("p2"));
        assert_eq!(rows[1]["family"], json!("Jones"));
    }

    // =========================================================================
    // Diamond: a shared dependency materialized once, two labels
    // =========================================================================

    #[tokio::test]
    async fn diamond_dependency_is_materialized_once_and_both_labels_work() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let shared_url =
            seed_view_definition(&backend, "shared", "http://example.org/shared").await;
        let a_url = "http://example.org/branch-a";
        let b_url = "http://example.org/branch-b";

        seed_library(
            &backend,
            "branch-a",
            sql_lib(
                "branch-a",
                Some(a_url),
                "sql-view",
                "SELECT * FROM s",
                &[("s", &shared_url)],
                vec![],
            ),
        )
        .await;
        seed_library(
            &backend,
            "branch-b",
            sql_lib(
                "branch-b",
                Some(b_url),
                "sql-view",
                "SELECT * FROM s",
                &[("s", &shared_url)],
                vec![],
            ),
        )
        .await;

        let subject = sql_lib(
            "subject",
            None,
            "sql-query",
            "SELECT (SELECT COUNT(*) FROM a_t) AS ca, (SELECT COUNT(*) FROM b_t) AS cb",
            &[("a_t", a_url), ("b_t", b_url)],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 1);
        // Both labels see the same underlying (once-materialized) shared
        // dependency: each independently counts all seeded patients.
        assert_eq!(rows[0]["ca"], json!(2));
        assert_eq!(rows[0]["cb"], json!(2));
    }

    /// Two labels declared by the same consumer both target the same
    /// ViewDefinition, so the subject's `JOIN` reads the shared node twice
    /// under two different names. Each label is a view over the same
    /// underlying table, not a copy, so both sides see identical rows —
    /// checked with a self-join on `patient_id` and by asserting every
    /// value, not just row counts.
    #[tokio::test]
    async fn two_labels_on_one_dependency_return_identical_rows_and_types() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;
        seed_patient(&backend, "p3", "Brown").await;

        let shared_url =
            seed_view_definition(&backend, "shared", "http://example.org/two-labels-shared").await;

        let subject = sql_lib(
            "two-labels-subject",
            None,
            "sql-query",
            "SELECT f.patient_id AS pid, s.family AS fam FROM first f \
             JOIN second s ON s.patient_id = f.patient_id ORDER BY pid",
            &[("first", &shared_url), ("second", &shared_url)],
            vec![],
        );

        let json_response = post_sql_run(&server, &run_body_inline(subject.clone(), "json")).await;
        json_response.assert_status(StatusCode::OK);
        let rows: Value = json_response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0]["pid"], json!("p1"));
        assert_eq!(rows[0]["fam"], json!("Smith"));
        assert_eq!(rows[1]["pid"], json!("p2"));
        assert_eq!(rows[1]["fam"], json!("Jones"));
        assert_eq!(rows[2]["pid"], json!("p3"));
        assert_eq!(rows[2]["fam"], json!("Brown"));

        let fhir_response = post_sql_run(&server, &run_body_inline(subject, "fhir")).await;
        fhir_response.assert_status(StatusCode::OK);
        let v: Value = fhir_response.json();
        assert_eq!(v["resourceType"], json!("Parameters"));
        let parameters = v["parameter"].as_array().expect("parameter array");
        assert_eq!(parameters.len(), 3, "{parameters:?}");
        let expected = [("p1", "Smith"), ("p2", "Jones"), ("p3", "Brown")];
        for (param, (pid, fam)) in parameters.iter().zip(expected) {
            let parts = param["part"].as_array().expect("part array");
            let pid_part = parts
                .iter()
                .find(|p| p["name"] == "pid")
                .expect("pid part present");
            assert_eq!(pid_part["valueString"], json!(pid), "{pid_part}");
            let fam_part = parts
                .iter()
                .find(|p| p["name"] == "fam")
                .expect("fam part present");
            assert_eq!(fam_part["valueString"], json!(fam), "{fam_part}");
        }
    }

    // =========================================================================
    // Cycle detection
    // =========================================================================

    #[tokio::test]
    async fn cycle_returns_400_with_the_full_path_in_diagnostics() {
        let (server, backend) = create_test_server().await;

        let a_url = "http://example.org/cycle-a";
        let b_url = "http://example.org/cycle-b";

        seed_library(
            &backend,
            "cycle-b",
            sql_lib(
                "cycle-b",
                Some(b_url),
                "sql-view",
                "SELECT * FROM a_t",
                &[("a_t", a_url)],
                vec![],
            ),
        )
        .await;

        // The subject itself (A) closes the cycle back on itself via B.
        let subject = sql_lib(
            "cycle-a",
            Some(a_url),
            "sql-view",
            "SELECT * FROM b_t",
            &[("b_t", b_url)],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body.to_string();
        assert!(text.contains("cycle"), "{text}");
        assert!(text.contains(a_url), "{text}");
        assert!(text.contains(b_url), "{text}");
    }

    // =========================================================================
    // Depth limit: 16 succeeds, 17 is rejected
    // =========================================================================

    #[tokio::test]
    async fn depth_seventeen_returns_400_naming_the_limit() {
        let (server, backend) = create_test_server().await;

        let leaf_url =
            seed_view_definition(&backend, "depth-leaf", "http://example.org/depth-leaf").await;

        let levels = 17;
        let mut next_target = leaf_url;
        for i in (1..=levels).rev() {
            let id = format!("depth-{i}");
            let url = format!("http://example.org/depth-{i}");
            seed_library(
                &backend,
                &id,
                sql_lib(
                    &id,
                    Some(&url),
                    "sql-view",
                    "SELECT * FROM t",
                    &[("t", &next_target)],
                    vec![],
                ),
            )
            .await;
            next_target = url;
        }

        let subject = sql_lib(
            "depth-subject",
            None,
            "sql-query",
            "SELECT * FROM t",
            &[("t", &next_target)],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body.to_string();
        assert!(text.contains("16"), "{text}");
    }

    // =========================================================================
    // SQLView subject declaring parameters violates `parameter 0..0`
    // =========================================================================

    #[tokio::test]
    async fn sqlview_subject_with_parameters_returns_400() {
        let (server, _backend) = create_test_server().await;
        let subject = sql_lib(
            "bad-sqlview",
            Some("http://example.org/bad-sqlview"),
            "sql-view",
            "SELECT 1 AS n",
            &[],
            vec![json!({"name": "p", "use": "in", "type": "string"})],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body.to_string();
        assert!(text.contains("parameter"), "{text}");
    }

    // =========================================================================
    // Aggregated Phase 1 errors: unresolved URL AND a cycle, both reported
    // =========================================================================

    #[tokio::test]
    async fn aggregated_errors_report_both_an_unresolved_url_and_a_cycle() {
        let (server, backend) = create_test_server().await;

        let a_url = "http://example.org/agg-a";
        seed_library(
            &backend,
            "agg-a",
            sql_lib(
                "agg-a",
                Some(a_url),
                "sql-view",
                "SELECT * FROM a_t",
                &[("a_t", a_url)],
                vec![],
            ),
        )
        .await;

        let subject = sql_lib(
            "agg-subject",
            None,
            "sql-query",
            "SELECT * FROM a_t, missing_t",
            &[
                ("a_t", a_url),
                ("missing_t", "http://example.org/agg-missing"),
            ],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let issues = body["issue"].as_array().expect("issue array");
        assert_eq!(issues.len(), 2, "{body}");
        let text = body.to_string();
        assert!(text.contains("cycle"), "{text}");
        assert!(text.contains("agg-missing"), "{text}");
    }

    // =========================================================================
    // `context` semantics (design #568): server-first order, transitive
    // matching, uniform validation, and degenerate entries.
    // =========================================================================

    /// A `context` entry can satisfy a dependency reached at any depth: the
    /// subject's direct dependency ("mid") is stored, but "mid"'s own
    /// dependency ("inner") is available only through `context`.
    #[tokio::test]
    async fn context_entry_satisfies_a_dependency_reached_through_an_intermediate_sqlview() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let leaf_url =
            seed_view_definition(&backend, "ctx-leaf", "http://example.org/ctx-leaf").await;
        let inner_url = "http://example.org/ctx-inner";
        let mid_url = "http://example.org/ctx-mid";

        let inner_lib = sql_lib(
            "ctx-inner",
            Some(inner_url),
            "sql-view",
            "SELECT * FROM leaf_t",
            &[("leaf_t", &leaf_url)],
            vec![],
        );
        // "inner" is deliberately *not* stored: it only exists via `context`.
        seed_library(
            &backend,
            "ctx-mid",
            sql_lib(
                "ctx-mid",
                Some(mid_url),
                "sql-view",
                "SELECT * FROM inner_t",
                &[("inner_t", inner_url)],
                vec![],
            ),
        )
        .await;

        let subject = sql_lib(
            "ctx-subject",
            None,
            "sql-query",
            "SELECT patient_id, family FROM mid_t ORDER BY patient_id",
            &[("mid_t", mid_url)],
            vec![],
        );
        let body = run_body_with_context(subject, "json", vec![inner_lib]);
        let response = post_sql_run(&server, &body).await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["patient_id"], json!("p1"));
        assert_eq!(rows[1]["patient_id"], json!("p2"));
    }

    /// A dependency unresolved by both storage and `context` is a 404 naming
    /// the URL that could not be resolved.
    #[tokio::test]
    async fn unresolvable_dependency_without_a_context_entry_returns_404_naming_the_url() {
        let (server, _backend) = create_test_server().await;

        let subject = sql_lib(
            "ctx-undefined-subject",
            None,
            "sql-query",
            "SELECT * FROM t",
            &[("t", "http://example.org/ctx-undefined")],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::NOT_FOUND);
        let body: Value = response.json();
        let text = body.to_string();
        assert!(text.contains("http://example.org/ctx-undefined"), "{text}");
    }

    /// Server-first order: a `context` entry that duplicates a URL the
    /// server already resolves is ignored. Proven here by making the
    /// `context` copy structurally invalid as a dependency (a SQLQuery
    /// Library, which may never be a dependency) — if the resolver used it
    /// the request would fail with 400; instead the stored ViewDefinition
    /// wins and the query succeeds.
    #[tokio::test]
    async fn context_entry_duplicating_a_stored_artifact_is_ignored_server_wins() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let dup_url = "http://example.org/ctx-dup";
        seed_view_definition(&backend, "ctx-dup-vd", dup_url).await;

        let bad_context_entry = sql_lib(
            "ctx-dup-bad",
            Some(dup_url),
            "sql-query",
            "SELECT 1",
            &[],
            vec![],
        );

        let subject = sql_lib(
            "ctx-dup-subject",
            None,
            "sql-query",
            "SELECT patient_id, family FROM t ORDER BY patient_id",
            &[("t", dup_url)],
            vec![],
        );
        let body = run_body_with_context(subject, "json", vec![bad_context_entry]);
        let response = post_sql_run(&server, &body).await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 2, "{rows:?}");
    }

    /// A `context` artifact passes through exactly the same validation as a
    /// stored one: a SQLView Library declaring `parameter` violates the
    /// profile's `parameter 0..0` constraint whether it came from storage or
    /// `context`.
    #[tokio::test]
    async fn context_sqlview_with_parameters_returns_400() {
        let (server, _backend) = create_test_server().await;

        let bad_url = "http://example.org/ctx-bad-sqlview";
        let bad_context_entry = sql_lib(
            "ctx-bad-sqlview",
            Some(bad_url),
            "sql-view",
            "SELECT 1 AS n",
            &[],
            vec![json!({"name": "p", "use": "in", "type": "string"})],
        );

        let subject = sql_lib(
            "ctx-bad-sqlview-subject",
            None,
            "sql-query",
            "SELECT * FROM t",
            &[("t", bad_url)],
            vec![],
        );
        let body = run_body_with_context(subject, "json", vec![bad_context_entry]);
        let response = post_sql_run(&server, &body).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let body: Value = response.json();
        let text = body.to_string();
        assert!(text.contains("parameter"), "{text}");
    }

    /// Worked example (mirrors operations-common.html#context-example): a
    /// client holds a ViewDefinition the server does not, and supplies it
    /// inline via `context` so the subject's SQLQuery can run against it.
    #[tokio::test]
    async fn context_worked_example_satisfies_the_subjects_direct_dependency() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let vd_url = "http://example.org/ctx-example-vd";
        let vd = json!({
            "resourceType": "ViewDefinition",
            "url": vd_url,
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"path": "id", "name": "patient_id", "type": "string"},
                    {"path": "name.family", "name": "family", "type": "string"}
                ]
            }]
        });

        let subject = sql_lib(
            "ctx-example-subject",
            None,
            "sql-query",
            "SELECT patient_id, family FROM t ORDER BY patient_id",
            &[("t", vd_url)],
            vec![],
        );
        let body = run_body_with_context(subject, "json", vec![vd]);
        let response = post_sql_run(&server, &body).await;
        response.assert_status(StatusCode::OK);
        let rows: Value = response.json();
        let rows = rows.as_array().expect("json array");
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0]["family"], json!("Smith"));
        assert_eq!(rows[1]["family"], json!("Jones"));
    }

    // =========================================================================
    // Per-dependency row cap on an interior SQLView (#1473)
    // =========================================================================

    /// A SQLView whose result is larger than the per-dependency cap fails the
    /// request with a 422 naming the SQLView, the label it is selected by,
    /// the cap and the setting. Its own input (3 rows) is within the cap; the
    /// SQLView's cross join (9 rows) is not.
    #[tokio::test]
    async fn sqlview_over_the_source_row_cap_returns_422_naming_sqlview_and_setting() {
        let mut config = ServerConfig::for_testing();
        config.sof_sqlquery_max_source_rows_per_vd = 5;
        let (server, backend) = create_test_server_with_config(config).await;
        for (id, family) in [("p1", "Smith"), ("p2", "Jones"), ("p3", "Brown")] {
            seed_patient(&backend, id, family).await;
        }
        let leaf_url = seed_view_definition(&backend, "leaf", "http://example.org/leaf").await;
        let view_url = "http://example.org/patient-pairs";
        let mut view = sql_lib(
            "patient-pairs",
            Some(view_url),
            "sql-view",
            "SELECT a.patient_id AS patient_id FROM l a, l b",
            &[("l", &leaf_url)],
            vec![],
        );
        view["name"] = json!("patient_pairs");
        seed_library(&backend, "patient-pairs", view).await;

        let subject = sql_lib(
            "subject",
            None,
            "sql-query",
            "SELECT patient_id FROM pairs WHERE patient_id = 'p1'",
            &[("pairs", view_url)],
            vec![],
        );
        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;

        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        let diagnostics = outcome["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default();
        for needle in [
            "dependency 'pairs'",
            "(SQL View patient_pairs)",
            "exceeds 5-row limit",
            "HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD",
        ] {
            assert!(
                diagnostics.contains(needle),
                "missing {needle:?} in: {diagnostics}"
            );
        }
        assert!(
            !diagnostics.contains("add a WHERE/LIMIT clause"),
            "{diagnostics}"
        );
    }

    // =========================================================================
    // Bound on the number of `context` entries (#1705)
    // =========================================================================

    /// 257 `context` entries are rejected with a 400 naming the limit; 256
    /// pass the check and the request runs.
    #[tokio::test]
    async fn more_than_256_context_entries_return_400_naming_the_limit() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        let vd_url = seed_view_definition(&backend, "ctx-limit-vd", "http://example.org/vd").await;

        let context = |n: usize| -> Vec<Value> {
            (0..n)
                .map(|i| {
                    json!({
                        "resourceType": "ViewDefinition",
                        "url": format!("http://example.org/ctx/{i}"),
                        "status": "active",
                        "resource": "Patient",
                        "select": [{"column": [{"path": "id", "name": "patient_id"}]}]
                    })
                })
                .collect()
        };
        let subject = || {
            sql_lib(
                "ctx-limit-subject",
                None,
                "sql-query",
                "SELECT patient_id FROM t",
                &[("t", vd_url.as_str())],
                vec![],
            )
        };

        let response = post_sql_run(
            &server,
            &run_body_with_context(subject(), "json", context(257)),
        )
        .await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let outcome: Value = response.json();
        let text = outcome.to_string();
        assert!(text.contains("256"), "{text}");
        assert!(text.contains("context"), "{text}");

        let response = post_sql_run(
            &server,
            &run_body_with_context(subject(), "json", context(256)),
        )
        .await;
        response.assert_status(StatusCode::OK);
    }

    /// A Library declaring more than 4 x `HFS_SOF_SQLQUERY_MAX_VDS`
    /// depends-on entries is a 400 before any of them is fetched (the urls
    /// here resolve to nothing, so a fetch would be a 404).
    #[tokio::test]
    async fn more_than_four_times_max_vds_depends_on_entries_return_400() {
        let config = ServerConfig {
            sof_sqlquery_max_vds: 1,
            ..ServerConfig::for_testing()
        };
        let (server, _backend) = create_test_server_with_config(config).await;
        let urls: Vec<String> = (0..5)
            .map(|i| format!("http://example.org/none/{i}"))
            .collect();
        let labels: Vec<String> = (0..5).map(|i| format!("t{i}")).collect();
        let depends_on: Vec<(&str, &str)> = labels
            .iter()
            .zip(&urls)
            .map(|(l, u)| (l.as_str(), u.as_str()))
            .collect();
        let subject = sql_lib(
            "depends-on-limit",
            None,
            "sql-query",
            "SELECT * FROM t0",
            &depends_on,
            vec![],
        );

        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let text = response.text();
        assert!(text.contains("depends-on"), "{text}");
        assert!(text.contains("declares 5"), "{text}");
        assert!(text.contains("at most 4"), "{text}");
    }

    // =========================================================================
    // Node limit stops the walk (#1804)
    // =========================================================================

    /// `HFS_SOF_SQLQUERY_MAX_VDS` stops the dependency walk at the first node
    /// past it, before anything further is fetched (#1804): with a limit of 1,
    /// the second ViewDefinition is that node, so the third, unresolvable URL is
    /// never fetched. The request is the node-limit 422, not a 404 naming it.
    #[tokio::test]
    async fn max_vds_stops_the_walk_before_fetching_further_dependencies() {
        let config = ServerConfig {
            sof_sqlquery_max_vds: 1,
            ..ServerConfig::for_testing()
        };
        let (server, backend) = create_test_server_with_config(config).await;
        let a_url =
            seed_view_definition(&backend, "max-vds-a", "http://example.org/max-vds/a").await;
        let b_url =
            seed_view_definition(&backend, "max-vds-b", "http://example.org/max-vds/b").await;
        let missing_url = "http://example.org/max-vds/missing";
        let subject = sql_lib(
            "max-vds-subject",
            None,
            "sql-query",
            "SELECT * FROM a_t",
            &[
                ("a_t", a_url.as_str()),
                ("b_t", b_url.as_str()),
                ("missing_t", missing_url),
            ],
            vec![],
        );

        let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let text = response.text();
        assert!(
            text.contains("dependency graph has 2 nodes; max allowed is 1"),
            "{text}"
        );
        assert!(!text.contains(missing_url), "{text}");
    }
    #[tokio::test]
    async fn union_dependencies_return_the_full_view_contents() {
        let (server, backend) = create_test_server().await;
        let resources = [
            json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "One"}]}),
            json!({"resourceType": "Patient", "id": "p2", "name": [{"family": "Two"}]}),
        ];
        for resource in &resources {
            backend
                .create(&tenant(), "Patient", resource.clone(), FhirVersion::R4)
                .await
                .unwrap();
        }
        let columns = json!([
            {"name": "patient_id", "path": "id", "type": "string"},
            {"name": "family", "path": "name.family", "type": "string"},
        ]);
        let union = json!({"resourceType": "ViewDefinition", "id": "union-leaf",
            "url": "http://example.org/union-leaf", "status": "active", "resource": "Patient",
            "select": [{"unionAll": [{"column": columns.clone()}, {"column": columns}]}]});
        backend
            .create(&tenant(), "ViewDefinition", union.clone(), FhirVersion::R4)
            .await
            .unwrap();
        let direct: Vec<Value> = backend
            .sof_runner()
            .unwrap()
            .run_view(&tenant(), union.clone(), Default::default())
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let bundle = helios_sof::SofBundle::R4(serde_json::from_value(json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": resources.iter().map(|resource| json!({"resource": resource})).collect::<Vec<_>>()
        })).unwrap());
        let oracle = helios_sof::run_view_definition(
            helios_sof::parse_view_definition_for_version(union.clone(), FhirVersion::R4).unwrap(),
            bundle,
            helios_sof::ContentType::Json,
        )
        .unwrap();
        let oracle: Vec<Value> = serde_json::from_slice(&oracle).unwrap();
        let bag = |rows: &[Value]| {
            let mut result = rows
                .iter()
                .map(|row| {
                    (
                        row["patient_id"].as_str().unwrap().to_string(),
                        row["family"].as_str().unwrap().to_string(),
                    )
                })
                .collect::<Vec<_>>();
            result.sort();
            result
        };
        let expected = vec![
            ("p1".to_string(), "One".to_string()),
            ("p1".to_string(), "One".to_string()),
            ("p2".to_string(), "Two".to_string()),
            ("p2".to_string(), "Two".to_string()),
        ];
        assert_eq!(bag(&oracle), expected);
        assert_eq!(bag(&direct), expected);
        let mut direct_body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "_format", "valueCode": "json"}, {"name": "subjectResource", "resource": union.clone()}
        ]});
        let response = post_sql_run(&server, &direct_body).await;
        response.assert_status(StatusCode::OK);
        assert_eq!(bag(&response.json::<Vec<Value>>()), expected);
        for kind in ["sql-query", "sql-view"] {
            let subject = sql_lib(
                "union-query",
                None,
                kind,
                "SELECT patient_id, family FROM u",
                &[("u", "http://example.org/union-leaf")],
                vec![],
            );
            let response = post_sql_run(&server, &run_body_inline(subject, "json")).await;
            response.assert_status(StatusCode::OK);
            let rows: Vec<Value> = response.json();
            assert_eq!(rows.len(), 4, "{kind}: retain the union multiset");
            assert_eq!(
                bag(&rows),
                expected,
                "{kind}: materialization must retain every row and its keys"
            );
        }
        // A different branch column order remains outside SQL emitter coverage.
        // Deduplicating the dependency table does not widen that coverage.
        direct_body["parameter"][1]["resource"]["select"][0]["unionAll"][1]["column"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let response = post_sql_run(&server, &direct_body).await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.text().contains("different column schemas"));
    }

    #[tokio::test]
    async fn reference_parameters_library_subjects_reject_unusable_entries() {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "One").await;
        seed_patient(&backend, "p2", "Two").await;
        let leaf_url =
            seed_view_definition(&backend, "refs-leaf", "http://example.org/refs-leaf").await;
        for kind in ["sql-query", "sql-view"] {
            let subject = sql_lib(
                "refs-library",
                None,
                kind,
                "SELECT patient_id FROM t",
                &[("t", &leaf_url)],
                vec![],
            );
            for name in ["patient", "group"] {
                for value in [
                    json!({"valueReference": {"identifier": {"system": "urn:s", "value": "v"}}}),
                    json!({"valueReference": {"display": "target"}}),
                    json!({"valueString": " "}),
                ] {
                    let mut entry = value;
                    entry["name"] = json!(name);
                    let mut body = run_body_inline(subject.clone(), "json");
                    body["parameter"]
                        .as_array_mut()
                        .unwrap()
                        .push(entry.clone());
                    let response = post_sql_run(&server, &body).await;
                    assert_eq!(
                        response.status_code(),
                        StatusCode::BAD_REQUEST,
                        "{kind}/{entry}: {}",
                        response.text()
                    );
                    let outcome: Value = response.json();
                    assert_eq!(outcome["resourceType"], "OperationOutcome");
                    assert_eq!(outcome["issue"][0]["code"], "invalid");
                    assert!(
                        outcome["issue"][0]["details"]["text"]
                            .as_str()
                            .unwrap()
                            .contains(name),
                        "{outcome}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn reference_parameters_library_subjects_normalize_repeated_filters() {
        let (server, backend) = create_test_server().await;
        for (id, family) in [("p1", "One"), ("p2", "Two"), ("p3", "Three")] {
            seed_patient(&backend, id, family).await;
        }
        for (id, patient) in [("g1", "p1"), ("g2", "p2")] {
            backend
                .create(
                    &tenant(),
                    "Group",
                    json!({
                        "resourceType": "Group", "id": id, "type": "person", "actual": true,
                        "member": [{"entity": {"reference": format!("Patient/{patient}")}}]
                    }),
                    FhirVersion::R4,
                )
                .await
                .unwrap();
        }
        let leaf_url =
            seed_view_definition(&backend, "refs-leaf", "http://example.org/refs-leaf").await;
        for kind in ["sql-query", "sql-view"] {
            let subject = sql_lib(
                "refs-library",
                None,
                kind,
                "SELECT patient_id FROM t ORDER BY patient_id",
                &[("t", &leaf_url)],
                vec![],
            );
            for name in ["patient", "group"] {
                let ty = if name == "patient" {
                    "Patient"
                } else {
                    "Group"
                };
                let ids = if name == "patient" {
                    ["p1", "p2"]
                } else {
                    ["g1", "g2"]
                };
                let mut body = run_body_inline(subject.clone(), "json");
                body["parameter"].as_array_mut().unwrap().extend([
                    json!({"name": name, "valueReference": {"reference": format!(" {ty}/{} ", ids[0])}}),
                    json!({"name": name, "valueString": format!(" {ty}/{} ", ids[1])}),
                ]);
                let response = post_sql_run(&server, &body).await;
                response.assert_status(StatusCode::OK);
                let rows: Value = response.json();
                assert_eq!(
                    rows,
                    json!([{"patient_id": "p1"}, {"patient_id": "p2"}]),
                    "{kind}/{name}"
                );
            }
        }
    }
}
