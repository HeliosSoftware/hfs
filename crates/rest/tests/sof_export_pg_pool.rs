//! `$sql-export` on PostgreSQL reads through the dedicated export pool.
//!
//! Builds the app the way the server does (`create_app_with_config`, so the
//! export runner is chosen by `build_app`) against a real PostgreSQL 16
//! (testcontainers, Docker required), and checks that:
//!
//! - `$sql-run` keeps the main pool: no `hfs-export` connection exists after it;
//! - `$sql-export` completes, its rows are exactly the `$sql-run` rows in the
//!   same order, and it read through an `hfs-export` connection.
//!
//! The settings those connections carry are covered by the persistence
//! crate's own tests.
#![cfg(all(feature = "R4", feature = "postgres"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};
use std::path::PathBuf;
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const TENANT: &str = "export-pool";

/// Counts the server's connections with `application_name`, read with
/// `psql` inside the container so the test needs no client of its own.
async fn connections_named(container: &ContainerAsync<Postgres>, name: &str) -> u32 {
    let mut result = container
        .exec(ExecCommand::new([
            "psql",
            "-U",
            "postgres",
            "-tAc",
            &format!("SELECT count(*) FROM pg_stat_activity WHERE application_name = '{name}'"),
        ]))
        .await
        .expect("psql in the container");
    let stdout = result.stdout_to_vec().await.expect("psql output");
    String::from_utf8(stdout)
        .expect("utf-8")
        .trim()
        .parse()
        .expect("a count")
}

fn ndjson_rows(body: &[u8]) -> Vec<Value> {
    std::str::from_utf8(body)
        .expect("utf-8 ndjson")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("ndjson row"))
        .collect()
}

#[tokio::test]
async fn sql_export_reads_through_the_export_pool_and_sql_run_does_not() {
    let container = container_cleanup::with_cleanup_label(
        Postgres::default().with_tag("16-alpine").with_label(
            "github.run_id",
            std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
        ),
    )
    .start()
    .await
    .expect("start PostgreSQL 16 testcontainer");
    let config = PostgresConfig {
        host: container.get_host().await.expect("host").to_string(),
        port: container.get_host_port_ipv4(5432).await.expect("port"),
        dbname: "postgres".into(),
        user: "postgres".into(),
        password: Some("postgres".into()),
        max_connections: 4,
        data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
        ..Default::default()
    };
    let backend = PostgresBackend::new(config).await.expect("PostgresBackend");
    backend.init_schema().await.expect("schema");

    let tenant = TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access());
    for (id, given) in [
        ("p1", vec!["Ann", "Amy"]),
        ("p2", vec!["Bob"]),
        ("p3", vec![]),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{"family": format!("F-{id}"), "given": given}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed patient");
    }

    let export_dir = tempfile::tempdir().expect("export dir");
    let server_config = ServerConfig {
        base_url: "http://localhost".to_string(),
        export_dir: export_dir.path().to_string_lossy().into_owned(),
        export_pg_work_mem: Some("8MB".to_string()),
        ..ServerConfig::for_testing()
    };
    let server = TestServer::new(helios_rest::create_app_with_config(backend, server_config))
        .expect("test server");
    let tenant_header = HeaderValue::from_static(TENANT);

    let view = json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {"column": [{"name": "id", "path": "getResourceKey()"}]},
            {"forEachOrNull": "name.given", "column": [{"name": "given", "path": "$this"}]}
        ]
    });

    let run = server
        .post("/$sql-run?_format=ndjson")
        .add_header(X_TENANT_ID, tenant_header.clone())
        .json(&view)
        .await;
    assert_eq!(run.status_code(), StatusCode::OK, "{}", run.text());
    let run_rows = ndjson_rows(run.as_bytes());
    assert_eq!(run_rows.len(), 4, "{run_rows:?}");
    assert_eq!(
        connections_named(&container, "hfs-export").await,
        0,
        "$sql-run must stay on the main pool"
    );

    let submit = server
        .post("/$sql-export?_format=ndjson")
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .add_header(X_TENANT_ID, tenant_header.clone())
        .json(&view)
        .await;
    assert_eq!(
        submit.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        submit.text()
    );
    let status_url = submit
        .headers()
        .get("content-location")
        .and_then(|v| v.to_str().ok())
        .expect("content-location")
        .to_string();

    let mut manifest = None;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let poll = server
            .get(&status_url)
            .add_header(X_TENANT_ID, tenant_header.clone())
            .await;
        match poll.status_code() {
            StatusCode::ACCEPTED => continue,
            StatusCode::SEE_OTHER => {
                let result_url = poll
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .expect("result location")
                    .to_string();
                let result = server
                    .get(&result_url)
                    .add_header(X_TENANT_ID, tenant_header.clone())
                    .await;
                assert_eq!(result.status_code(), StatusCode::OK, "{}", result.text());
                manifest = Some(result.json::<Value>());
                break;
            }
            other => panic!("unexpected export poll status {other}: {}", poll.text()),
        }
    }
    let manifest = manifest.expect("export completed");

    let mut exported = Vec::new();
    for output in manifest["parameter"]
        .as_array()
        .expect("manifest parameters")
        .iter()
        .filter(|p| p["name"] == "output")
    {
        for part in output["part"].as_array().expect("output parts") {
            if part["name"] != "location" {
                continue;
            }
            let url = part["valueUri"].as_str().expect("location valueUri");
            let path = &url[url.find("/export/").expect("export download path")..];
            let download = server
                .get(path)
                .add_header(X_TENANT_ID, tenant_header.clone())
                .await;
            assert_eq!(
                download.status_code(),
                StatusCode::OK,
                "{}",
                download.text()
            );
            exported.extend(ndjson_rows(download.as_bytes()));
        }
    }

    // Same rows, multiplicity and order as the unlimited `$sql-run`.
    assert_eq!(exported, run_rows);
    // The export pool's connection is still pooled after the job.
    assert_eq!(
        connections_named(&container, "hfs-export").await,
        1,
        "$sql-export must read through the export pool"
    );
}
