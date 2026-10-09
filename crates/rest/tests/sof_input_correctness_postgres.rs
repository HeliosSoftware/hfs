//! #1862 input correctness on the PostgreSQL in-DB runner.
//! Requires Docker; one PostgreSQL container serves the test binary and each
//! fixture uses a distinct tenant, matching the SOF conformance suites.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_input_correctness_postgres_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
    use futures::TryStreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::{path::PathBuf, sync::Arc};
    use testcontainers::{ImageExt, runners::AsyncRunner};
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    fn config(host: &str, port: u16) -> PostgresConfig {
        PostgresConfig {
            host: host.to_string(),
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        }
    }

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = super::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("start PostgreSQL");
                let host = container.get_host().await.expect("host").to_string();
                let port = container.get_host_port_ipv4(5432).await.expect("port");
                let backend = PostgresBackend::new(config(&host, port))
                    .await
                    .expect("backend");
                backend.init_schema().await.expect("schema");
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    async fn server_with(resources: &[Value]) -> (TestServer, Arc<PostgresBackend>, String) {
        let pg = shared_pg().await;
        let backend = Arc::new(
            PostgresBackend::new(config(&pg.host, pg.port))
                .await
                .expect("backend"),
        );
        let tenant_id = format!("sof_pg_input_{}", uuid::Uuid::new_v4().simple());
        let tenant =
            TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        for resource in resources {
            backend
                .create(
                    &tenant,
                    resource["resourceType"].as_str().unwrap(),
                    resource.clone(),
                    FhirVersion::R4,
                )
                .await
                .expect("seed resource");
        }
        let runner = backend.sof_runner().expect("in-DB runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("server");
        (server, backend, tenant_id)
    }

    #[tokio::test]
    async fn union_dependencies_return_the_full_view_contents() {
        let resources = [
            json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "One"}]}),
            json!({"resourceType": "Patient", "id": "p2", "name": [{"family": "Two"}]}),
        ];
        let (server, backend, tenant_id) = server_with(&resources).await;
        let tenant =
            TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        let columns = json!([
            {"name": "patient_id", "path": "id", "type": "string"},
            {"name": "family", "path": "name.family", "type": "string"},
        ]);
        let union = json!({"resourceType": "ViewDefinition", "id": "union-leaf",
            "url": "http://example.org/union-leaf", "status": "active", "resource": "Patient",
            "select": [{"unionAll": [{"column": columns.clone()}, {"column": columns}]}]});
        backend
            .create(&tenant, "ViewDefinition", union.clone(), FhirVersion::R4)
            .await
            .unwrap();
        let direct: Vec<Value> = backend
            .sof_runner()
            .unwrap()
            .run_view(&tenant, union.clone(), Default::default())
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
        for kind in ["sql-query", "sql-view"] {
            let subject = json!({"resourceType": "Library", "status": "active",
                "type": {"coding": [{"system": "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes", "code": kind}]},
                "content": [{"contentType": "application/sql", "data": B64.encode("SELECT patient_id, family FROM u")}],
                "relatedArtifact": [{"type": "depends-on", "label": "u", "resource": "http://example.org/union-leaf"}]});
            let body = json!({"resourceType": "Parameters", "parameter": [
                {"name": "_format", "valueCode": "json"}, {"name": "subjectResource", "resource": subject}
            ]});
            let response = server
                .post("/$sql-run")
                .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant_id).unwrap())
                .add_header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/fhir+json"),
                )
                .json(&body)
                .await;
            response.assert_status(StatusCode::OK);
            let rows: Vec<Value> = response.json();
            assert_eq!(rows.len(), 4, "{kind}: retain the union multiset");
            assert_eq!(
                bag(&rows),
                expected,
                "{kind}: materialization must retain every row and its keys"
            );
        }
        let mut reversed = union;
        reversed["select"][0]["unionAll"][1]["column"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let response = server
            .post("/$sql-run?_format=json")
            .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant_id).unwrap())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&reversed)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.text().contains("different column schemas"));
    }

    #[tokio::test]
    async fn computed_unnest_nul_returns_422() {
        // JSONB cannot store a NUL string. The invalid request must still be
        // refused before executing SQL, independently of matching data.
        let resources = [json!({"resourceType": "Patient", "id": "computed-focus",
            "name": [{"family": "Smith"}], "extension": [{"url": "ab", "valueString": "yes"}]})];
        let (server, _, tenant_id) = server_with(&resources).await;
        for iteration in ["forEach", "forEachOrNull"] {
            for (path, status) in [
                ("extension('ab').where(true).exists()", StatusCode::OK),
                (
                    "extension('a\\u0000b').where(true).exists()",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
                (
                    "extension('a\0b').where(true).exists()",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
            ] {
                let view = json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
                    "where": [{"path": path}],
                    "select": [{iteration: "name", "column": [{"path": "family", "name": "family"}]}]});
                let response = server
                    .post("/$sql-run?_format=json")
                    .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant_id).unwrap())
                    .add_header(
                        CONTENT_TYPE,
                        HeaderValue::from_static("application/fhir+json"),
                    )
                    .json(&view)
                    .await;
                assert_eq!(
                    response.status_code(),
                    status,
                    "{iteration} {path:?}: {}",
                    response.text()
                );
                let body: Value = response.json();
                if status == StatusCode::OK {
                    assert_eq!(body, json!([{"family": "Smith"}]));
                } else {
                    assert_eq!(body["resourceType"], "OperationOutcome");
                    assert!(
                        body["issue"][0]["details"]["text"]
                            .as_str()
                            .unwrap()
                            .contains("NUL"),
                        "{body}"
                    );
                }
            }
        }
        let view = json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"forEach": "name.where(use = 'a\\u0000b').family", "column": [{"path": "$this", "name": "family"}]}]});
        let response = server
            .post("/$sql-run?_format=json")
            .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant_id).unwrap())
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&view)
            .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(response.text().contains("must be a simple JSON path"));
    }
}
