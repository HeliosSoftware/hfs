//! `patient`/`group` references across every SQL-on-FHIR path, on the stored
//! backends other than SQLite (#1864, acceptance A19).
//!
//! One matrix, run against PostgreSQL and MongoDB (testcontainers) and, as
//! local evidence only, S3 over MinIO (the in-process runner). SQLite is
//! covered by `sof_run.rs`, `sof_sqlquery.rs` and `sof_export.rs`, and S3's
//! CI-safe cells by `persistence/src/backends/s3/tests.rs`
//! (`exists_is_false_for_a_deleted_resource`, MockS3Client).
//!
//! Paths: `$sql-run` with a stored ViewDefinition (`subjectReference`), with an
//! inline ViewDefinition (`subjectResource`) and with a SQLQuery Library
//! subject, and the `$sql-export` kick-off. Answers:
//!
//! | Value | `$sql-run` | `$sql-export` kick-off |
//! |---|---|---|
//! | unusable shape | 400 `invalid` naming the parameter | 400 `invalid` naming the parameter |
//! | missing target | 400 `not-found` naming it | 404 `not-found` naming it |
//! | deleted target | 400 `not-found` naming it | 404 `not-found` naming it |
//! | Group without Patient members | 200, no rows | (accepted) |
//!
//! Run with:
//!   cargo test -p helios-rest --features postgres,mongodb --test sof_references_backends
//!   RUN_MINIO_S3_TESTS=1 cargo test -p helios-rest --features s3 --test sof_references_backends
//!
//! The PostgreSQL and MongoDB modules need Docker. The S3 module also needs
//! `RUN_MINIO_S3_TESTS=1` (MinIO SoF cells are local evidence, not CI).

#![cfg(any(feature = "postgres", feature = "mongodb", feature = "s3"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// The matrix shared by every backend module.
mod matrix {
    #![allow(dead_code)]

    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
    const VIEW_ID: &str = "refs-vd";

    fn patient_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "name": "refs_patients",
            "status": "active",
            "resource": "Patient",
            "select": [{"column": [{"name": "id", "path": "id", "type": "id"}]}]
        })
    }

    fn library() -> Value {
        json!({
            "resourceType": "Library",
            "id": "refs-lib",
            "status": "active",
            "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
            "content": [{
                "contentType": "application/sql",
                "data": B64.encode("SELECT id FROM t ORDER BY id".as_bytes())
            }],
            "relatedArtifact": [{
                "type": "depends-on",
                "label": "t",
                "resource": format!("ViewDefinition/{VIEW_ID}")
            }]
        })
    }

    /// Seeds the matrix's data in `tenant_id`: Patient `p1` (live) and
    /// `p-gone` (deleted), Group `g1` (member `p1`), `g-empty` (no members)
    /// and `g-gone` (deleted), and the stored ViewDefinition.
    pub async fn seed<S: ResourceStorage + ?Sized>(storage: &S, tenant_id: &str) {
        let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
        let resources = [
            ("Patient", json!({"resourceType": "Patient", "id": "p1"})),
            (
                "Patient",
                json!({"resourceType": "Patient", "id": "p-gone"}),
            ),
            (
                "Group",
                json!({"resourceType": "Group", "id": "g1", "type": "person", "actual": true,
                    "member": [{"entity": {"reference": "Patient/p1"}}]}),
            ),
            (
                "Group",
                json!({"resourceType": "Group", "id": "g-empty", "type": "person",
                    "actual": true}),
            ),
            (
                "Group",
                json!({"resourceType": "Group", "id": "g-gone", "type": "person", "actual": true,
                    "member": [{"entity": {"reference": "Patient/p1"}}]}),
            ),
            ("ViewDefinition", {
                let mut view = patient_view();
                view["id"] = json!(VIEW_ID);
                view
            }),
        ];
        for (resource_type, resource) in resources {
            storage
                .create(&tenant, resource_type, resource, FhirVersion::R4)
                .await
                .unwrap_or_else(|e| panic!("seed {resource_type}: {e}"));
        }
        for (resource_type, id) in [("Patient", "p-gone"), ("Group", "g-gone")] {
            storage
                .delete(&tenant, resource_type, id)
                .await
                .unwrap_or_else(|e| panic!("delete {resource_type}/{id}: {e}"));
        }
    }

    /// One `$sql-run` path.
    #[derive(Clone, Copy, Debug)]
    enum RunPath {
        StoredView,
        InlineView,
        Library,
    }

    async fn run(
        server: &TestServer,
        tenant_id: &str,
        path: RunPath,
        param: &str,
        value: &str,
    ) -> axum_test::TestResponse {
        let subject = match path {
            RunPath::StoredView => json!({
                "name": "subjectReference",
                "valueReference": {"reference": format!("ViewDefinition/{VIEW_ID}")}
            }),
            RunPath::InlineView => json!({"name": "subjectResource", "resource": patient_view()}),
            RunPath::Library => json!({"name": "subjectResource", "resource": library()}),
        };
        server
            .post("/$sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_str(tenant_id).unwrap())
            .json(&json!({"resourceType": "Parameters", "parameter": [
                {"name": "_format", "valueCode": "json"},
                subject,
                {"name": param, "valueReference": {"reference": value}}
            ]}))
            .await
    }

    async fn kick_off(
        server: &TestServer,
        tenant_id: &str,
        param: &str,
        value: &str,
    ) -> axum_test::TestResponse {
        server
            .post(&format!(
                "/$sql-export?{param}={}",
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>()
            ))
            .add_header(
                HeaderName::from_static("prefer"),
                HeaderValue::from_static("respond-async"),
            )
            .add_header(X_TENANT_ID, HeaderValue::from_str(tenant_id).unwrap())
            .json(&patient_view())
            .await
    }

    fn issue_text(outcome: &Value) -> String {
        outcome["issue"][0]["diagnostics"]
            .as_str()
            .or_else(|| outcome["issue"][0]["details"]["text"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Asserts `(status, code)` and that the OperationOutcome names `named`;
    /// collects a line per mismatch instead of failing at the first.
    fn expect(
        wrong: &mut Vec<String>,
        cell: &str,
        response: &axum_test::TestResponse,
        status: StatusCode,
        code: &str,
        named: &str,
    ) {
        let got = response.status_code();
        let outcome: Value = serde_json::from_str(&response.text()).unwrap_or(Value::Null);
        let ok = got == status
            && outcome["resourceType"] == "OperationOutcome"
            && outcome["issue"][0]["code"] == code
            && issue_text(&outcome).contains(named);
        eprintln!("SOF-REFS {cell}: {got} {}", outcome["issue"][0]["code"]);
        if !ok {
            wrong.push(format!(
                "{cell}: expected {status} `{code}` naming {named}; got {got}: {}",
                response.text()
            ));
        }
    }

    /// The whole matrix for one backend, whose data [`seed`] wrote under
    /// `tenant_id`. Fails once, listing every cell that disagrees.
    pub async fn check(backend: &str, server: &TestServer, tenant_id: &str) {
        let mut wrong = Vec::new();
        let paths = [RunPath::StoredView, RunPath::InlineView, RunPath::Library];
        let absent_cases = [
            ("patient", "Patient/absent", "missing"),
            ("group", "Group/absent", "missing"),
            ("patient", "Patient/p-gone", "deleted"),
            ("group", "Group/g-gone", "deleted"),
            ("group", "g-gone", "deleted, bare id"),
        ];
        let unusable_cases = [
            ("patient", "http://example.org/fhir/Patient/p1"),
            ("patient", "urn:uuid:0b0e8c86-5a8f-4d2e-9e4a-1d2c3b4a5f60"),
            ("group", "Patient/p1"),
            ("patient", "Patient/p1/_history/1"),
            ("group", "#g1"),
        ];

        for path in paths {
            for (param, value, what) in absent_cases {
                let cell = format!("{backend} {path:?} {param}={value} ({what})");
                let response = run(server, tenant_id, path, param, value).await;
                expect(
                    &mut wrong,
                    &cell,
                    &response,
                    StatusCode::BAD_REQUEST,
                    "not-found",
                    value,
                );
            }
            for (param, value) in unusable_cases {
                let cell = format!("{backend} {path:?} {param}={value} (unusable)");
                let response = run(server, tenant_id, path, param, value).await;
                expect(
                    &mut wrong,
                    &cell,
                    &response,
                    StatusCode::BAD_REQUEST,
                    "invalid",
                    param,
                );
            }
            // Live targets run; a Group without Patient members selects nothing.
            for (param, value, rows) in [
                ("patient", "Patient/p1", json!([{"id": "p1"}])),
                ("group", "Group/g1", json!([{"id": "p1"}])),
                ("group", "Group/g-empty", json!([])),
            ] {
                let cell = format!("{backend} {path:?} {param}={value} (live)");
                let response = run(server, tenant_id, path, param, value).await;
                let got: Value = serde_json::from_str(&response.text()).unwrap_or(Value::Null);
                eprintln!("SOF-REFS {cell}: {} {got}", response.status_code());
                if response.status_code() != StatusCode::OK || got != rows {
                    wrong.push(format!(
                        "{cell}: expected 200 {rows}; got {}: {}",
                        response.status_code(),
                        response.text()
                    ));
                }
            }
        }

        for (param, value, what) in absent_cases {
            let cell = format!("{backend} kick-off {param}={value} ({what})");
            let response = kick_off(server, tenant_id, param, value).await;
            expect(
                &mut wrong,
                &cell,
                &response,
                StatusCode::NOT_FOUND,
                "not-found",
                value,
            );
        }
        for (param, value) in unusable_cases {
            let cell = format!("{backend} kick-off {param}={value} (unusable)");
            let response = kick_off(server, tenant_id, param, value).await;
            expect(
                &mut wrong,
                &cell,
                &response,
                StatusCode::BAD_REQUEST,
                "invalid",
                param,
            );
        }

        assert!(
            wrong.is_empty(),
            "{backend} reference matrix, {} cell(s) wrong:\n  {}",
            wrong.len(),
            wrong.join("\n  ")
        );
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_rest::ServerConfig;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn references_answer_per_operation_on_postgres() {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default()
                .with_tag("16-alpine")
                .with_label("github.run_id", &run_id),
        )
        .start()
        .await
        .expect("failed to start PostgreSQL container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("PostgreSQL port");
        let host = container.get_host().await.expect("host").to_string();
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let backend = PostgresBackend::new(PostgresConfig {
            host,
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        })
        .await
        .expect("PostgresBackend");
        backend.init_schema().await.expect("schema");
        let backend = Arc::new(backend);

        let tenant_id = "sof-refs-pg";
        super::matrix::seed(backend.as_ref(), tenant_id).await;
        let runner = backend.sof_runner().expect("PostgreSQL SoF runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("test server");
        super::matrix::check("postgres", &server, tenant_id).await;
    }
}

#[cfg(feature = "mongodb")]
mod mongodb {
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::{Backend, ResourceStorage};
    use helios_rest::ServerConfig;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn references_answer_per_operation_on_mongodb() {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = super::container_cleanup::with_cleanup_label(
            Mongo::default().with_label("github.run_id", &run_id),
        )
        .start()
        .await
        .expect("failed to start MongoDB container");
        let port = container
            .get_host_port_ipv4(27017)
            .await
            .expect("MongoDB port");
        let host = container.get_host().await.expect("host").to_string();
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: format!("mongodb://{host}:{port}"),
            database_name: "sof_refs".to_string(),
            ..Default::default()
        })
        .expect("MongoBackend");
        backend.initialize().await.expect("initialize");
        let backend = Arc::new(backend);

        let tenant_id = "sof-refs-mongo";
        super::matrix::seed(backend.as_ref(), tenant_id).await;
        let runner = backend.sof_runner().expect("MongoDB SoF runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("test server");
        super::matrix::check("mongodb", &server, tenant_id).await;
    }
}

/// S3 (the in-process runner) over MinIO: local evidence only, behind
/// `RUN_MINIO_S3_TESTS=1`. CI covers S3 through MockS3Client
/// (`persistence/src/backends/s3/tests.rs`).
#[cfg(feature = "s3")]
mod s3 {
    use std::sync::Arc;

    use aws_config::{BehaviorVersion, Region};
    use aws_sdk_s3::config::Credentials;
    use axum_test::TestServer;
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
    use helios_persistence::core::ResourceStorage;
    use helios_rest::ServerConfig;
    use testcontainers::core::{IntoContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{GenericImage, ImageExt};
    use uuid::Uuid;

    const DEFAULT_MINIO_IMAGE: &str = "ghcr.io/coollabsio/minio";
    const DEFAULT_MINIO_TAG: &str = "RELEASE.2025-10-15T17-29-55Z";
    const MINIO_ROOT_USER: &str = "minioadmin";
    const MINIO_ROOT_PASSWORD: &str = "minioadmin";

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn references_answer_per_operation_on_s3_minio() {
        if std::env::var("RUN_MINIO_S3_TESTS").ok().as_deref() != Some("1") {
            eprintln!(
                "skipping references_answer_per_operation_on_s3_minio \
                 (set RUN_MINIO_S3_TESTS=1 to enable)"
            );
            return;
        }
        let image =
            std::env::var("MINIO_IMAGE").unwrap_or_else(|_| DEFAULT_MINIO_IMAGE.to_string());
        let tag = std::env::var("MINIO_TAG").unwrap_or_else(|_| DEFAULT_MINIO_TAG.to_string());
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = super::container_cleanup::with_cleanup_label(
            GenericImage::new(image, tag)
                .with_wait_for(WaitFor::message_on_stderr("API:"))
                .with_exposed_port(9000.tcp())
                .with_env_var("MINIO_ROOT_USER", MINIO_ROOT_USER)
                .with_env_var("MINIO_ROOT_PASSWORD", MINIO_ROOT_PASSWORD)
                .with_cmd(["server", "/data"])
                .with_label("github.run_id", &run_id),
        )
        .start()
        .await
        .expect("failed to start MinIO container");
        let host = container.get_host().await.expect("host").to_string();
        let port = container
            .get_host_port_ipv4(9000)
            .await
            .expect("MinIO port");
        let endpoint_url = format!("http://{host}:{port}");
        // SAFETY: the only test in this module, run before any backend reads
        // the SDK provider chain; the values never change afterwards.
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", MINIO_ROOT_USER);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", MINIO_ROOT_PASSWORD);
            std::env::set_var("AWS_REGION", "us-east-1");
            std::env::set_var("AWS_EC2_METADATA_DISABLED", "true");
        }

        let creds = Credentials::new(MINIO_ROOT_USER, MINIO_ROOT_PASSWORD, None, None, "refs");
        let cfg = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint_url.clone())
            .credentials_provider(creds)
            .load()
            .await;
        let client = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::config::Builder::from(&cfg)
                .force_path_style(true)
                .build(),
        );
        let bucket = format!("hfs-sof-refs-{}", Uuid::new_v4().simple());
        client
            .create_bucket()
            .bucket(&bucket)
            .send()
            .await
            .expect("create MinIO bucket");

        let backend = Arc::new(
            S3Backend::from_env_async(S3BackendConfig {
                tenancy_mode: S3TenancyMode::PrefixPerTenant { bucket },
                prefix: Some(format!("refs/{}", Uuid::new_v4())),
                region: Some("us-east-1".to_string()),
                endpoint_url: Some(endpoint_url),
                force_path_style: true,
                allow_http: true,
                validate_buckets_on_startup: true,
                ..Default::default()
            })
            .await
            .expect("S3 backend for MinIO"),
        );

        let tenant_id = "sof-refs-s3";
        super::matrix::seed(backend.as_ref(), tenant_id).await;
        let runner = backend.sof_runner().expect("S3 in-process SoF runner");
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(runner);
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("test server");
        super::matrix::check("s3", &server, tenant_id).await;
    }
}
