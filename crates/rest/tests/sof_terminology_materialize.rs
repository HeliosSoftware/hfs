//! Integration tests for write-path terminology materialization (#1824,
//! plan §5 PR 2): a SQLQuery/SQLView Library write expands the ValueSets it
//! depends on into HFS storage (T1), a ValueSet write some Library depends on
//! gains its expansion in the candidate (T2), with the terminology server as
//! the engine when configured and local enumerated expansion otherwise;
//! strict / lenient / off modes; SMART scopes; the write-free preconditions;
//! batch and transaction bundles over the bundle overlay.
//!
//! The terminology server is a mock `axum` app served over a real socket by
//! `axum_test`, so HFS's HTTP client talks to it exactly as it would to HTS.

mod sof_terminology_materialize_tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Query, State};
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum::middleware::Next;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use chrono::Utc;
    use helios_auth::{Principal, ScopeSet};
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
    const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
    const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
    const TAG_SYSTEM: &str = "http://heliossoftware.com/fhir/CodeSystem/sof-terminology";

    const CONDITIONS_URL: &str = "https://example.org/ViewDefinition/conditions";
    const CVD_URL: &str = "http://example.org/ValueSet/cardiovascular-disease";
    const SCT: &str = "http://snomed.info/sct";
    const ICD10: &str = "http://hl7.org/fhir/sid/icd-10";

    // ---- mock terminology server ---------------------------------------

    #[derive(Default)]
    struct Mock {
        value_sets: Vec<Value>,
        concept_maps: Vec<Value>,
        expand_calls: usize,
        fail_expand: bool,
    }

    type Shared = Arc<Mutex<Mock>>;

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
        let m = mock.lock().unwrap();
        let pool = if resource_type == "ValueSet" {
            &m.value_sets
        } else {
            &m.concept_maps
        };
        let entries: Vec<Value> = pool
            .iter()
            .filter(|r| query.get("url").is_none_or(|u| r["url"] == json!(u)))
            .filter(|r| {
                query
                    .get("version")
                    .is_none_or(|v| r["version"] == json!(v))
            })
            .map(|r| json!({"resource": r}))
            .collect();
        Json(json!({"resourceType": "Bundle", "type": "searchset", "entry": entries}))
    }

    /// Starts the mock over a real socket; returns its base URL and state.
    async fn mock_hts() -> (TestServer, String, Shared) {
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

    // ---- HFS test server -----------------------------------------------

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    struct Hfs {
        server: TestServer,
        backend: Arc<SqliteBackend>,
    }

    fn config(mode: &str, hts: Option<&str>) -> ServerConfig {
        ServerConfig {
            base_url: "http://localhost".to_string(),
            sof_terminology_materialize: mode.to_string(),
            terminology_server: hts.map(str::to_string),
            ..ServerConfig::for_testing()
        }
    }

    async fn server_with(config: ServerConfig, scopes: Option<&str>) -> Hfs {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);
        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let mut app = helios_rest::routing::fhir_routes::create_routes(state);
        if let Some(scopes) = scopes {
            let principal = Principal {
                subject: "client-1".to_string(),
                issuer: "https://idp.example.com".to_string(),
                tenant_id: Some("test-tenant".to_string()),
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
        Hfs { server, backend }
    }

    async fn server(mode: &str, hts: Option<&str>) -> Hfs {
        server_with(config(mode, hts), None).await
    }

    impl Hfs {
        async fn put(&self, path: &str, body: &Value) -> axum_test::TestResponse {
            self.server
                .put(path)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(body)
                .await
        }

        async fn post(&self, path: &str, body: &Value) -> axum_test::TestResponse {
            self.server
                .post(path)
                .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(body)
                .await
        }

        async fn stored(&self, resource_type: &str, id: &str) -> Option<(String, Value)> {
            self.backend
                .read(&tenant(), resource_type, id)
                .await
                .expect("read")
                .map(|s| (s.version_id().to_string(), s.content().clone()))
        }

        async fn seed(&self, resource_type: &str, resource: Value) {
            let id = resource["id"].as_str().expect("fixture id").to_string();
            self.backend
                .create_or_update(&tenant(), resource_type, &id, resource, FhirVersion::R4)
                .await
                .unwrap_or_else(|e| panic!("seed {resource_type}/{id}: {e}"));
        }

        async fn seed_conditions(&self) {
            self.seed(
                "ViewDefinition",
                json!({
                    "resourceType": "ViewDefinition", "id": "conditions", "url": CONDITIONS_URL,
                    "status": "active", "resource": "Condition",
                    "select": [{"column": [
                        {"path": "subject.getReferenceKey(Patient)", "name": "patient_id", "type": "string"},
                        {"path": "code.coding.first().system", "name": "system", "type": "uri"},
                        {"path": "code.coding.first().code", "name": "code", "type": "code"}
                    ]}]
                }),
            )
            .await;
            for (patient, system, code) in [
                ("p1", SCT, "22298006"),
                ("p2", ICD10, "I21"),
                ("p3", SCT, "73211009"),
            ] {
                self.seed(
                    "Condition",
                    json!({
                        "resourceType": "Condition", "id": format!("c-{patient}"),
                        "subject": {"reference": format!("Patient/{patient}")},
                        "code": {"coding": [{"system": system, "code": code}]}
                    }),
                )
                .await;
            }
        }

        /// Runs `library` inline and returns the status and body.
        async fn run(&self, library: &Value) -> (StatusCode, Value) {
            let body = json!({"resourceType": "Parameters", "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": library}
            ]});
            let response = self.post("/$sql-run", &body).await;
            let status = response.status_code();
            (status, response.json())
        }
    }

    // ---- fixtures --------------------------------------------------------

    fn compose_only_vs(id: &str, url: &str, version: Option<&str>) -> Value {
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

    fn filter_vs(id: &str, url: &str) -> Value {
        json!({
            "resourceType": "ValueSet", "id": id, "url": url, "status": "active",
            "compose": {"include": [{"system": SCT, "filter": [{"property": "concept", "op": "is-a", "value": "49601007"}]}]}
        })
    }

    fn library(id: &str, type_code: &str, sql: &str, depends_on: &[(&str, &str)]) -> Value {
        json!({
            "resourceType": "Library", "id": id, "name": id, "status": "active",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": type_code}]},
            "content": [{"contentType": "application/sql", "data": B64.encode(sql.as_bytes())}],
            "relatedArtifact": depends_on.iter().map(|(label, target)| json!({
                "type": "depends-on", "label": label, "resource": target
            })).collect::<Vec<_>>()
        })
    }

    const CVD_SQL: &str = "SELECT conditions.patient_id, conditions.code FROM conditions \
        WHERE EXISTS (SELECT 1 FROM cvd_codes WHERE cvd_codes.system = conditions.system \
        AND cvd_codes.code = conditions.code)";

    fn cvd_query(id: &str, dependency: &str) -> Value {
        library(
            id,
            "sql-query",
            CVD_SQL,
            &[("conditions", CONDITIONS_URL), ("cvd_codes", dependency)],
        )
    }

    fn expansion_of(resource: &Value) -> Option<&Value> {
        resource.get("expansion")
    }

    fn has_tag(resource: &Value, code: &str) -> bool {
        resource
            .pointer("/meta/tag")
            .and_then(Value::as_array)
            .is_some_and(|tags| {
                tags.iter()
                    .any(|t| t["system"] == TAG_SYSTEM && t["code"] == code)
            })
    }

    fn issue_code(body: &Value) -> String {
        body["issue"][0]["code"].as_str().unwrap_or("").to_string()
    }

    // =====================================================================
    // T1 and T2 on the direct paths, local expansion (no terminology server)
    // =====================================================================

    #[tokio::test]
    async fn put_library_expands_its_stored_enumerated_value_set_locally_and_tags_it() {
        let hfs = server("strict", None).await;
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

    #[tokio::test]
    async fn post_value_set_after_the_library_is_transformed_in_the_candidate_and_if_match_still_works()
     {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        // The Library first: its dependency is unresolvable, which a write
        // never validated, so it is stored as is.
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
            .server
            .put(&format!("/ValueSet/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_str(&etag).unwrap())
            .json(&again)
            .await;
        assert_eq!(put.status_code(), StatusCode::OK, "{}", put.text());

        let (status, rows) = hfs.run(&cvd_query("inline", CVD_URL)).await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn value_set_nobody_depends_on_is_stored_as_sent() {
        let hfs = server("strict", None).await;
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

    #[tokio::test]
    async fn unexpandable_dependency_is_a_422_in_strict_a_warning_in_lenient_and_ignored_when_off()
    {
        for (mode, expected_status) in [
            ("strict", StatusCode::UNPROCESSABLE_ENTITY),
            ("lenient", StatusCode::CREATED),
            ("off", StatusCode::CREATED),
        ] {
            let hfs = server(mode, None).await;
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

    #[tokio::test]
    async fn member_cap_rejects_the_write_with_nothing_written() {
        let hfs = server_with(
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

    #[tokio::test]
    async fn transitive_dependencies_through_a_sqlview_are_materialized_and_cycles_are_rejected() {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let view_url = "http://example.org/Library/cvd-view";
        hfs.seed(
            "Library",
            json!({"url": view_url})
                .as_object()
                .map(|_| {
                    let mut v = library(
                        "cvd-view",
                        "sql-view",
                        "SELECT system, code FROM cvd_codes",
                        &[("cvd_codes", CVD_URL)],
                    );
                    v["url"] = json!(view_url);
                    v
                })
                .unwrap(),
        )
        .await;
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

    #[tokio::test]
    async fn patch_adding_a_dependency_materializes_it() {
        let hfs = server("strict", None).await;
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
        let response = hfs
            .server
            .patch("/Library/q")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .bytes(serde_json::to_vec(&patch).unwrap().into())
            .content_type("application/json-patch+json")
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
    }

    // =====================================================================
    // The terminology server as the engine and as the source of definitions
    // =====================================================================

    #[tokio::test]
    async fn with_a_terminology_server_every_expansion_comes_from_it() {
        let (_mock, url, shared) = mock_hts().await;
        let hfs = server("strict", Some(&url)).await;
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

    #[tokio::test]
    async fn a_definition_only_the_terminology_server_holds_is_fetched_stored_and_expanded() {
        let (_mock, url, shared) = mock_hts().await;
        shared
            .lock()
            .unwrap()
            .value_sets
            .push(compose_only_vs("remote", CVD_URL, Some("2026")));
        let hfs = server("strict", Some(&url)).await;
        hfs.seed_conditions().await;
        let pinned = format!("{CVD_URL}|2026");
        let lib = hfs
            .put("/Library/cvd-q", &cvd_query("cvd-q", &pinned))
            .await;
        assert_eq!(lib.status_code(), StatusCode::CREATED, "{}", lib.text());

        let stored = hfs
            .server
            .get("/ValueSet?url=http://example.org/ValueSet/cardiovascular-disease")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let bundle: Value = stored.json();
        let entries: Vec<&Value> = bundle["entry"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["resource"]["resourceType"] == "ValueSet")
            .collect();
        assert_eq!(entries.len(), 1, "{bundle}");
        let vs = &entries[0]["resource"];
        assert_eq!(vs["version"], "2026");
        assert_ne!(vs["id"], "remote", "the create assigned its own id");
        assert!(expansion_of(vs).is_some());
        assert!(has_tag(vs, "fetched-from-terminology-server"), "{vs}");

        let (status, rows) = hfs.run(&cvd_query("inline", &pinned)).await;
        assert_eq!(status, StatusCode::OK, "{rows}");
        assert_eq!(rows.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_failing_terminology_server_fails_the_write_with_exception_and_writes_nothing() {
        let (_mock, url, shared) = mock_hts().await;
        shared.lock().unwrap().fail_expand = true;
        let hfs = server("strict", Some(&url)).await;
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

    #[tokio::test]
    async fn an_unreachable_terminology_server_is_an_exception_in_strict_and_a_stored_library_in_lenient()
     {
        for (mode, status) in [
            ("strict", StatusCode::UNPROCESSABLE_ENTITY),
            ("lenient", StatusCode::CREATED),
        ] {
            let hfs = server(mode, Some("http://127.0.0.1:9")).await;
            hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
                .await;
            let lib = hfs
                .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
                .await;
            assert_eq!(lib.status_code(), status, "{mode}: {}", lib.text());
            assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_none());
        }
    }

    // =====================================================================
    // Write-free preconditions come first (D13, R3-2, R4-3)
    // =====================================================================

    #[tokio::test]
    async fn stale_if_match_is_a_412_before_anything_is_touched() {
        let hfs = server("strict", Some("http://127.0.0.1:9")).await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        hfs.seed(
            "Library",
            library("cvd-q", "sql-query", "SELECT 1 AS one", &[]),
        )
        .await;
        let before = hfs.stored("ValueSet", "cvd").await.unwrap();
        let put = hfs
            .server
            .put("/Library/cvd-q")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(IF_MATCH, HeaderValue::from_static("W/\"999\""))
            .json(&cvd_query("cvd-q", CVD_URL))
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

    #[tokio::test]
    async fn conditional_create_that_matches_returns_the_existing_library_without_touching_anything()
     {
        let hfs = server("strict", Some("http://127.0.0.1:9")).await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let mut existing = cvd_query("cvd-q", CVD_URL);
        existing["url"] = json!("http://example.org/Library/cvd-q");
        hfs.seed("Library", existing.clone()).await;
        let before = hfs.stored("ValueSet", "cvd").await.unwrap();
        let post = hfs
            .server
            .post("/Library")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )
            .json(&existing)
            .await;
        assert_eq!(post.status_code(), StatusCode::OK, "{}", post.text());
        assert_eq!(hfs.stored("ValueSet", "cvd").await.unwrap(), before);
    }

    #[tokio::test]
    async fn conditional_create_without_a_match_and_an_unexpandable_dependency_writes_nothing() {
        let hfs = server("strict", None).await;
        hfs.seed("ValueSet", filter_vs("cvd", CVD_URL)).await;
        let mut lib = cvd_query("cvd-q", CVD_URL);
        lib["url"] = json!("http://example.org/Library/cvd-q");
        let post = hfs
            .server
            .post("/Library")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )
            .json(&lib)
            .await;
        assert_eq!(
            post.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            post.text()
        );
        let libraries = hfs
            .server
            .get("/Library")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await
            .json::<Value>();
        assert!(
            libraries["entry"]
                .as_array()
                .is_none_or(|e| e.iter().all(|x| x["resource"]["resourceType"] != "Library")),
            "{libraries}"
        );
    }

    #[tokio::test]
    async fn conditional_create_without_a_match_materializes_after_the_committed_create() {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let mut lib = cvd_query("cvd-q", CVD_URL);
        lib["url"] = json!("http://example.org/Library/cvd-q");
        let post = hfs
            .server
            .post("/Library")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=http://example.org/Library/cvd-q"),
            )
            .json(&lib)
            .await;
        assert_eq!(post.status_code(), StatusCode::CREATED, "{}", post.text());
        assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
    }

    // =====================================================================
    // Authorization (D12)
    // =====================================================================

    #[tokio::test]
    async fn materialization_needs_the_callers_value_set_update_scope() {
        // Library-only scopes: strict refuses with 403 naming ValueSet.
        let hfs = server_with(
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
        let hfs = server_with(
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
        let hfs = server_with(config("strict", None), Some("user/*.cruds")).await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let put = hfs
            .put("/Library/cvd-q", &cvd_query("cvd-q", CVD_URL))
            .await;
        assert_eq!(put.status_code(), StatusCode::CREATED, "{}", put.text());
        assert!(expansion_of(&hfs.stored("ValueSet", "cvd").await.unwrap().1).is_some());
    }

    // =====================================================================
    // Bundles
    // =====================================================================

    fn bundle(kind: &str, entries: Vec<Value>) -> Value {
        json!({"resourceType": "Bundle", "type": kind, "entry": entries})
    }

    fn entry(method: &str, url: &str, resource: Option<Value>) -> Value {
        let mut e = json!({"request": {"method": method, "url": url}});
        if let Some(resource) = resource {
            e["resource"] = resource;
        }
        e
    }

    #[tokio::test]
    async fn batch_entries_materialize_independently() {
        let hfs = server("strict", None).await;
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

    #[tokio::test]
    async fn transaction_with_value_set_and_library_commits_once_in_either_order() {
        for reversed in [false, true] {
            let hfs = server("strict", None).await;
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
            let response = hfs.post("/", &bundle("transaction", entries)).await;
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

    #[tokio::test]
    async fn transaction_needing_a_side_write_is_refused_before_persistence() {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        let response = hfs
            .post(
                "/",
                &bundle(
                    "transaction",
                    vec![entry(
                        "PUT",
                        "Library/cvd-q",
                        Some(cvd_query("cvd-q", CVD_URL)),
                    )],
                ),
            )
            .await;
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

    #[tokio::test]
    async fn transaction_with_an_unresolvable_dependency_and_no_terminology_server_commits() {
        // Nothing could supply the dependency, and a Library write never
        // validated its dependencies (D4): the Library is stored as sent.
        let hfs = server("strict", None).await;
        let response = hfs
            .post(
                "/",
                &bundle(
                    "transaction",
                    vec![entry(
                        "PUT",
                        "Library/cvd-q",
                        Some(cvd_query("cvd-q", CVD_URL)),
                    )],
                ),
            )
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        assert!(hfs.stored("Library", "cvd-q").await.is_some());
    }

    #[tokio::test]
    async fn transaction_deleting_a_value_set_makes_it_unresolvable_for_a_later_library() {
        // The deleted ValueSet is never resurrected: with a terminology server
        // that holds the same canonical, the Library's dependency is found
        // only there, which a transaction cannot store, so the bundle is
        // refused; without one, nothing can supply it and the Library is
        // stored as sent, with the ValueSet gone.
        let (_mock, url, shared) = mock_hts().await;
        shared
            .lock()
            .unwrap()
            .value_sets
            .push(compose_only_vs("remote", CVD_URL, None));
        for hts in [Some(url.as_str()), None] {
            let hfs = server("strict", hts).await;
            hfs.seed_conditions().await;
            let mut expanded = compose_only_vs("cvd", CVD_URL, None);
            expanded["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}]});
            hfs.seed("ValueSet", expanded).await;
            let response = hfs
                .post(
                    "/",
                    &bundle(
                        "transaction",
                        vec![
                            entry("DELETE", "ValueSet/cvd", None),
                            entry("POST", "Library", Some(cvd_query("cvd-q", CVD_URL))),
                        ],
                    ),
                )
                .await;
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
                let gone = hfs.backend.read(&tenant(), "ValueSet", "cvd").await;
                assert!(
                    !matches!(gone, Ok(Some(_))),
                    "the delete committed: {gone:?}"
                );
                let (status, _) = hfs.run(&cvd_query("inline", CVD_URL)).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "no resurrection");
            }
        }
    }

    #[tokio::test]
    async fn conditional_post_of_a_value_set_cannot_be_combined_with_materialization() {
        let hfs = server("strict", None).await;
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
        let response = hfs.post("/", &with_library).await;
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
        let hfs = server("lenient", None).await;
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

    #[tokio::test]
    async fn transaction_patch_adding_a_dependency_on_an_in_bundle_compose_only_value_set_commits()
    {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        hfs.seed("Library", library("q", "sql-query", "SELECT 1 AS one", &[]))
            .await;
        let patch = json!({"resourceType": "Parameters", "parameter": [
            {"name": "operation", "part": [
                {"name": "type", "valueCode": "add"},
                {"name": "path", "valueString": "Library"},
                {"name": "name", "valueString": "relatedArtifact"},
                {"name": "value", "valueRelatedArtifact": {"type": "depends-on", "label": "cvd_codes", "resource": CVD_URL}}
            ]}
        ]});
        let response = hfs
            .post(
                "/",
                &bundle(
                    "transaction",
                    vec![
                        entry(
                            "POST",
                            "ValueSet",
                            Some(compose_only_vs("cvd", CVD_URL, None)),
                        ),
                        entry("PATCH", "Library/q", Some(patch)),
                    ],
                ),
            )
            .await;
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

    #[tokio::test]
    async fn transaction_patch_declaring_an_unprepared_dependency_rolls_back() {
        let hfs = server("strict", None).await;
        hfs.seed("ValueSet", compose_only_vs("cvd", CVD_URL, None))
            .await;
        hfs.seed("Library", library("q", "sql-query", "SELECT 1 AS one", &[]))
            .await;
        let patch = json!({"resourceType": "Parameters", "parameter": [
            {"name": "operation", "part": [
                {"name": "type", "valueCode": "add"},
                {"name": "path", "valueString": "Library"},
                {"name": "name", "valueString": "relatedArtifact"},
                {"name": "value", "valueRelatedArtifact": {"type": "depends-on", "label": "cvd_codes", "resource": CVD_URL}}
            ]}
        ]});
        let response = hfs
            .post(
                "/",
                &bundle(
                    "transaction",
                    vec![entry("PATCH", "Library/q", Some(patch))],
                ),
            )
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.text()
        );
        let (_, lib) = hfs.stored("Library", "q").await.unwrap();
        assert_eq!(lib["relatedArtifact"], json!([]), "rolled back: {lib}");
    }

    #[tokio::test]
    async fn two_in_bundle_versions_of_one_url_are_two_candidates() {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        let mut v1 = compose_only_vs("v1", CVD_URL, Some("1"));
        v1["compose"]["include"][1]["concept"] = json!([]);
        v1["compose"]["include"].as_array_mut().unwrap().pop();
        let v2 = compose_only_vs("v2", CVD_URL, Some("2"));
        let response = hfs
            .post(
                "/",
                &bundle(
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
                ),
            )
            .await;
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

    #[tokio::test]
    async fn deleting_the_newest_stored_version_in_the_bundle_resolves_an_unpinned_reference_to_the_survivor()
     {
        let hfs = server("strict", None).await;
        hfs.seed_conditions().await;
        let mut v1 = compose_only_vs("v1", CVD_URL, Some("1"));
        v1["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}]});
        hfs.seed("ValueSet", v1).await;
        let mut v2 = compose_only_vs("v2", CVD_URL, Some("2"));
        v2["expansion"] = json!({"contains": [{"system": SCT, "code": "22298006"}, {"system": ICD10, "code": "I21"}]});
        hfs.seed("ValueSet", v2).await;
        let response = hfs
            .post(
                "/",
                &bundle(
                    "transaction",
                    vec![
                        entry("DELETE", "ValueSet/v2", None),
                        entry("PUT", "Library/q", Some(cvd_query("q", CVD_URL))),
                    ],
                ),
            )
            .await;
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
}
