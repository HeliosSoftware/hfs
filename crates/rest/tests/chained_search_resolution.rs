//! Chained and `_has` searches end to end over HTTP (#1389).
//!
//! Backend `search()` does not read chains, so every caller has to resolve
//! them first. This file pins three things on each SQL backend:
//!
//! - a chained or `_has` token terminal of the form `system|` matches every
//!   code in that system, through the REST search path;
//! - a transaction conditional reference with a chain or `_has` resolves to
//!   the resource the chain selects;
//! - a bulk export `_typeFilter` with a chain or `_has` exports only what the
//!   filter selects.
//!
//! The SQLite side of the last two lives in `batch_conformance.rs` and
//! `bulk_export.rs`; the PostgreSQL module below needs Docker.

use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::{Value, json};

const MRN: &str = "http://example.org/mrn";
const LOINC: &str = "http://loinc.org";

/// Seeds, through the REST API:
///
/// - `pt1` (MRN `MRN12345`) and `pt2` (`http://other.example|X1`);
/// - `ob1` (LOINC `1234-5`, subject `pt1`) and `ob2` (SNOMED `271649006`,
///   subject `pt2`);
/// - `org-acme` (name Acme), managing `pt2`.
async fn seed(server: &TestServer) {
    for resource in [
        json!({"resourceType": "Organization", "id": "org-acme", "name": "Acme"}),
        json!({"resourceType": "Patient", "id": "pt1",
               "identifier": [{"system": MRN, "value": "MRN12345"}]}),
        json!({"resourceType": "Patient", "id": "pt2",
               "identifier": [{"system": "http://other.example", "value": "X1"}],
               "managingOrganization": {"reference": "Organization/org-acme"}}),
        json!({"resourceType": "Observation", "id": "ob1", "status": "final",
               "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
               "subject": {"reference": "Patient/pt1"}}),
        json!({"resourceType": "Observation", "id": "ob2", "status": "final",
               "code": {"coding": [{"system": "http://snomed.info/sct", "code": "271649006"}]},
               "subject": {"reference": "Patient/pt2"}}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server.put(&path).json(&resource).await;
        assert!(response.status_code().is_success(), "seeding {path}");
    }
}

/// The sorted ids of a searchset.
async fn search_ids(
    server: &TestServer,
    resource_type: &str,
    key: &str,
    value: &str,
) -> Vec<String> {
    let response = server
        .get(&format!("/{resource_type}"))
        .add_query_param(key, value)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{resource_type}?{key}={value}: {}",
        response.text()
    );
    let body: Value = response.json();
    let mut ids: Vec<String> = body["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

/// `system|` on a chained or reverse-chained token terminal selects every code
/// in that system, and nothing from another system.
async fn assert_system_only_chains(server: &TestServer) {
    for (resource_type, key, value, expected) in [
        (
            "Observation",
            "subject:Patient.identifier",
            "http://example.org/mrn|",
            &["ob1"][..],
        ),
        (
            "Patient",
            "_has:Observation:subject:code",
            "http://loinc.org|",
            &["pt1"][..],
        ),
        // Same selections through `system|code`, as a cross-check.
        (
            "Observation",
            "subject:Patient.identifier",
            "http://example.org/mrn|MRN12345",
            &["ob1"][..],
        ),
        (
            "Patient",
            "_has:Observation:subject:code",
            "http://loinc.org|1234-5",
            &["pt1"][..],
        ),
        // A system nobody uses selects nothing.
        (
            "Patient",
            "_has:Observation:subject:code",
            "http://unused.example|",
            &[][..],
        ),
    ] {
        assert_eq!(
            search_ids(server, resource_type, key, value).await,
            expected,
            "{resource_type}?{key}={value}"
        );
    }
}

#[tokio::test]
async fn sqlite_system_only_chained_token_over_http() {
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_rest::{ServerConfig, create_app_with_config};

    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");
    let server =
        TestServer::new(create_app_with_config(backend, ServerConfig::for_testing())).unwrap();

    seed(&server).await;
    assert_system_only_chains(&server).await;
}

#[cfg(feature = "postgres")]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[cfg(feature = "postgres")]
mod postgres {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum_test::TestServer;
    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::{
        BulkExportJobStore, DefaultExportWorker, ExportClaimStrategy, ExportOutputStore, WorkerId,
    };
    use helios_rest::ServerConfig;
    use helios_rest::bulk_export_auth::BearerScopeAuth;
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    const BASE_URL: &str = "http://localhost:8080";

    // Shared container setup, as in `patient_everything_postgres.rs`: one PG
    // container for the binary, one tenant per test.
    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

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
                .expect("failed to start PostgreSQL container");
                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("failed to get host port");
                let host = container
                    .get_host()
                    .await
                    .expect("failed to get host")
                    .to_string();
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    struct PgServer {
        server: TestServer,
        backend: Arc<PostgresBackend>,
        output: Arc<LocalFsOutputStore>,
        _tmp: tempfile::TempDir,
    }

    /// A PostgreSQL-backed server with bulk export wired in, on a fresh
    /// tenant that requests without an `X-Tenant-ID` header fall back to.
    async fn pg_server(name: &str) -> PgServer {
        let pg = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let backend = PostgresBackend::new(PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        })
        .await
        .expect("failed to create PostgresBackend");
        backend
            .init_schema()
            .await
            .expect("failed to initialize schema");
        let backend = Arc::new(backend);

        let tmp = tempfile::tempdir().expect("tempdir");
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path(),
            "https://wrong-internal.example",
        ));
        let config = ServerConfig {
            base_url: BASE_URL.to_string(),
            default_tenant: format!("chains_pg_{name}_{}", uuid::Uuid::new_v4().simple()),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::clone(&backend), config).with_bulk_export(
            backend.clone() as Arc<dyn BulkExportJobStore>,
            output.clone() as Arc<dyn ExportOutputStore>,
            Arc::new(BearerScopeAuth),
        );
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        PgServer {
            server: TestServer::new(app).expect("failed to create test server"),
            backend,
            output,
            _tmp: tmp,
        }
    }

    #[tokio::test]
    async fn postgres_system_only_chained_token_over_http() {
        let pg = pg_server("system_only").await;
        super::seed(&pg.server).await;
        super::assert_system_only_chains(&pg.server).await;
    }

    /// Posts a transaction whose Immunization names its patient with
    /// `reference`, and returns the stored patient reference, if any.
    async fn post_with_patient_reference(
        server: &TestServer,
        reference: &str,
    ) -> (StatusCode, Option<String>) {
        let bundle = json!({
            "resourceType": "Bundle",
            "type": "transaction",
            "entry": [{
                "fullUrl": "urn:uuid:11111111-1111-1111-1111-111111111111",
                "resource": {
                    "resourceType": "Immunization",
                    "status": "completed",
                    "vaccineCode": {"coding": [{"system": "http://hl7.org/fhir/sid/cvx", "code": "140"}]},
                    "patient": {"reference": reference},
                    "occurrenceDateTime": "2020-01-01"
                },
                "request": {"method": "POST", "url": "Immunization"}
            }]
        });
        let response = server.post("/").json(&bundle).await;
        let status = response.status_code();
        let stored: Value = server.get("/Immunization").await.json();
        let patient = stored["entry"][0]["resource"]["patient"]["reference"]
            .as_str()
            .map(str::to_string);
        (status, patient)
    }

    /// A chained or `_has` conditional reference resolves to the resource the
    /// chain selects, on PostgreSQL as on SQLite (`batch_conformance.rs`).
    #[tokio::test]
    async fn postgres_chained_conditional_references_resolve() {
        for (name, reference, expected) in [
            // Only `pt1` has a LOINC Observation.
            (
                "has",
                "Patient?_has:Observation:subject:code=1234-5",
                "Patient/pt1",
            ),
            // Only `pt2` is managed by Acme.
            ("dotted", "Patient?organization.name=Acme", "Patient/pt2"),
        ] {
            let pg = pg_server(name).await;
            super::seed(&pg.server).await;
            let (status, patient) = post_with_patient_reference(&pg.server, reference).await;
            assert_eq!(status, StatusCode::OK, "{reference}");
            assert_eq!(patient.as_deref(), Some(expected), "{reference}");
        }

        // No Observation carries this code: the reference matches nothing
        // and the bundle is rejected, writing nothing.
        let pg = pg_server("has_none").await;
        super::seed(&pg.server).await;
        let (status, patient) =
            post_with_patient_reference(&pg.server, "Patient?_has:Observation:subject:code=0000-0")
                .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(patient, None);
    }

    /// Runs every claimable export job with a worker bound to this server's
    /// output store. Only this test creates export jobs in the shared
    /// container, so no other test's worker can claim them.
    async fn drain_workers(pg: &PgServer) {
        let worker_id = WorkerId::new("chains-pg-worker");
        let worker = DefaultExportWorker::new(
            pg.backend.clone(),
            pg.backend.clone(),
            pg.output.clone(),
            worker_id.clone(),
        );
        while let Some(lease) = pg
            .backend
            .claim_next(&worker_id, Duration::from_secs(60), 3)
            .await
            .expect("claim_next")
        {
            worker.run_job(lease).await.expect("run_job");
        }
    }

    /// A `_typeFilter` with a chain or `_has` exports only what it selects,
    /// on PostgreSQL as on SQLite (`bulk_export.rs`).
    #[tokio::test]
    async fn postgres_type_filter_with_a_chain_is_applied() {
        for (name, filter, expected) in [
            ("has", "Patient?_has:Observation:subject:code=1234-5", "pt1"),
            ("dotted", "Patient?organization.name=Acme", "pt2"),
        ] {
            let pg = pg_server(&format!("export_{name}")).await;
            super::seed(&pg.server).await;

            let kickoff = pg
                .server
                .get("/$export")
                .add_header("prefer", "respond-async")
                .add_query_param("_type", "Patient")
                .add_query_param("_typeFilter", filter)
                .await;
            assert_eq!(kickoff.status_code(), StatusCode::ACCEPTED, "{filter}");
            let status_url = kickoff
                .headers()
                .get("content-location")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let status_path = status_url.strip_prefix(BASE_URL).unwrap().to_string();

            drain_workers(&pg).await;

            let done = pg.server.get(&status_path).await;
            assert_eq!(
                done.status_code(),
                StatusCode::OK,
                "{filter}: {}",
                done.text()
            );
            let manifest: Value = done.json();
            let files = manifest["output"].as_array().expect("output array");
            assert_eq!(files.len(), 1, "{filter}: one Patient file");
            let file_path = files[0]["url"]
                .as_str()
                .unwrap()
                .strip_prefix(BASE_URL)
                .unwrap()
                .to_string();
            let download = pg.server.get(&file_path).await;
            assert_eq!(download.status_code(), StatusCode::OK, "{filter}");
            let ids: Vec<String> = download
                .text()
                .lines()
                .map(|line| {
                    serde_json::from_str::<Value>(line).unwrap()["id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect();
            assert_eq!(ids, vec![expected.to_string()], "{filter}");
        }
    }
}
