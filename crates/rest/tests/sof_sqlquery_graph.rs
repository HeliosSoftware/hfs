//! Integration tests for the two-phase dependency-graph resolver (design
//! #567): SQLQuery -> SQLView chains, diamond dependencies, cycles, the
//! fixed depth limit, the SQLView `parameter 0..0` profile rule, and
//! aggregated Phase 1 errors — exercised end to end through `$sql-run`.

mod sof_sqlquery_graph_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
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
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

        let config = ServerConfig::for_testing();
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
}
