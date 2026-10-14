//! `$sql-export` on PostgreSQL reads through its own export pool, under the
//! export job budget (#1864).
//!
//! Builds the app the way the server does (`create_app_with_config`, so the
//! export runner and the job budget are wired by `build_app`) against a real
//! PostgreSQL 16 (testcontainers, Docker required), and checks that:
//!
//! - `$sql-run` keeps the main pool (no `hfs-export` connection exists after
//!   it), while `$sql-export` reads through an `hfs-export` connection and
//!   returns the `$sql-run` rows as a multiset (the export order is the
//!   resource id, bytewise; `$sql-run`'s is `(last_updated, id)`);
//! - a job whose budget (`HFS_EXPORT_JOB_TIMEOUT_SECS`) is smaller than its
//!   page fails: its result answers `504` with code `timeout`, and the shards
//!   it already wrote are deleted;
//! - cancelling a SQL Query export while its dependency's statement runs
//!   leaves no statement running (the PostgreSQL cell of A14).
//!
//! Each statement is found in `pg_stat_activity` by a column-alias marker.
//! The settings the export connections carry are covered by the persistence
//! crate's own tests.
#![cfg(all(feature = "R4", feature = "postgres"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use base64::Engine as _;
use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const TENANT: &str = "export-pool";

/// The stop bound (issue #1864, target 1 s).
const STOP_BOUND: Duration = Duration::from_secs(1);

/// Entries per array of the cartesian Patient: `N * N` rows in one page, so
/// its statement is busy for seconds.
const N: usize = 2000;

/// A container of its own: the assertions read `pg_stat_activity`, which
/// another test's connections would pollute.
async fn start_pg() -> (ContainerAsync<Postgres>, PostgresConfig) {
    let container = container_cleanup::with_cleanup_label(
        Postgres::default()
            .with_tag("16-alpine")
            .with_label(
                "github.run_id",
                std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            )
            .with_cmd(["postgres", "-c", "fsync=off", "-c", "jit=off"]),
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
    (container, config)
}

/// A backend with the schema and `resources` seeded in [`TENANT`].
async fn seeded_backend(config: &PostgresConfig, resources: &[Value]) -> PostgresBackend {
    let backend = PostgresBackend::new(config.clone())
        .await
        .expect("PostgresBackend");
    backend.init_schema().await.expect("schema");
    let tenant = TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access());
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
    backend
}

/// The server as `build_app` wires it, exporting into `export_dir`.
fn server(
    backend: PostgresBackend,
    export_dir: &Path,
    adjust: impl FnOnce(&mut ServerConfig),
) -> TestServer {
    let mut config = ServerConfig {
        base_url: "http://localhost".to_string(),
        export_dir: export_dir.to_string_lossy().into_owned(),
        ..ServerConfig::for_testing()
    };
    adjust(&mut config);
    TestServer::new(helios_rest::create_app_with_config(backend, config)).expect("test server")
}

/// A pool-independent connection for reading `pg_stat_activity`.
async fn observer(config: &PostgresConfig) -> PostgresBackend {
    PostgresBackend::new(PostgresConfig {
        max_connections: 1,
        ..config.clone()
    })
    .await
    .expect("observer backend")
}

async fn connections_named(observer: &PostgresBackend, name: &str) -> i64 {
    let client = observer.get_client().await.expect("observer connection");
    client
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
            &[&name],
        )
        .await
        .expect("read pg_stat_activity")
        .get(0)
}

/// SoF statements (found by their alias marker) that are executing, and
/// whether one of them runs on an `hfs-export` connection.
async fn active_statements(observer: &PostgresBackend, alias: &str) -> (i64, bool) {
    let client = observer.get_client().await.expect("observer connection");
    let row = client
        .query_one(
            "SELECT count(*), coalesce(bool_or(application_name = 'hfs-export'), false) \
             FROM pg_stat_activity WHERE datname = current_database() \
             AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE $1",
            &[&format!("%\"{alias}\"%")],
        )
        .await
        .expect("sample pg_stat_activity");
    (row.get(0), row.get(1))
}

fn marker(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

fn cartesian_patient(n: usize) -> Value {
    let names: Vec<Value> = (0..n)
        .map(|i| json!({"family": format!("f{i:05}")}))
        .collect();
    let addresses: Vec<Value> = (0..n)
        .map(|i| json!({"city": format!("c{i:05}")}))
        .collect();
    json!({"resourceType": "Patient", "id": "big", "name": names, "address": addresses})
}

/// `name` × `address`, with `alias` as the first column's name so the
/// statement can be found in `pg_stat_activity`.
fn cartesian_view(alias: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "name": "cartesian",
        "resource": "Patient",
        "status": "active",
        "select": [
            {"forEach": "name", "column": [{"name": alias, "path": "family"}]},
            {"forEach": "address", "column": [{"name": "city", "path": "city"}]}
        ]
    })
}

fn ndjson_rows(body: &[u8]) -> Vec<Value> {
    std::str::from_utf8(body)
        .expect("utf-8 ndjson")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("ndjson row"))
        .collect()
}

fn path_of(url: &str) -> String {
    url.strip_prefix("http://localhost")
        .unwrap_or(url)
        .to_string()
}

/// Kicks off `$sql-export` with `body` and returns its status path.
async fn kick_off(server: &TestServer, body: &Value) -> String {
    let submit = server
        .post("/$sql-export?_format=ndjson")
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .json(body)
        .await;
    assert_eq!(
        submit.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        submit.text()
    );
    path_of(
        submit
            .headers()
            .get("content-location")
            .and_then(|v| v.to_str().ok())
            .expect("content-location"),
    )
}

/// Polls `status_path` until the job ends and returns its result.
async fn result_of(server: &TestServer, status_path: &str) -> axum_test::TestResponse {
    let started = Instant::now();
    loop {
        let poll = server
            .get(status_path)
            .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
            .await;
        match poll.status_code() {
            StatusCode::ACCEPTED => {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "the export job never finished"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            StatusCode::SEE_OTHER => {
                let result_path = path_of(
                    poll.headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .expect("result location"),
                );
                return server
                    .get(&result_path)
                    .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
                    .await;
            }
            other => panic!("unexpected export poll status {other}: {}", poll.text()),
        }
    }
}

/// Regular files under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// `$sql-run` stays on the main pool; `$sql-export` reads through the
/// `hfs-export` pool and returns the same rows (as a multiset: the export is
/// in resource-id order).
#[tokio::test]
async fn sql_export_reads_through_the_export_pool_and_sql_run_does_not() {
    let (_container, config) = start_pg().await;
    // Inserted out of id order, so the two orders differ.
    let patients: Vec<Value> = [
        ("p3", vec![]),
        ("p1", vec!["Ann", "Amy"]),
        ("p2", vec!["Bob"]),
    ]
    .into_iter()
    .map(|(id, given)| {
        json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": format!("F-{id}"), "given": given}]
        })
    })
    .collect();
    let backend = seeded_backend(&config, &patients).await;
    let observer = observer(&config).await;
    let export_dir = tempfile::tempdir().expect("export dir");
    let server = server(backend, export_dir.path(), |_| {});

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
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .json(&view)
        .await;
    assert_eq!(run.status_code(), StatusCode::OK, "{}", run.text());
    let run_rows = ndjson_rows(run.as_bytes());
    assert_eq!(run_rows.len(), 4, "{run_rows:?}");
    assert_eq!(
        connections_named(&observer, "hfs-export").await,
        0,
        "$sql-run must stay on the main pool"
    );

    let status_path = kick_off(&server, &view).await;
    let result = result_of(&server, &status_path).await;
    assert_eq!(result.status_code(), StatusCode::OK, "{}", result.text());
    let manifest = result.json::<Value>();

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
            let download = server
                .get(&path_of(url))
                .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
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

    // The export pages by resource id; `$sql-run` orders by last update.
    let ids = |rows: &[Value]| -> Vec<String> {
        rows.iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&exported), ["p1", "p1", "p2", "p3"], "{exported:?}");
    let bag = |rows: &[Value]| {
        let mut v: Vec<String> = rows.iter().map(Value::to_string).collect();
        v.sort();
        v
    };
    assert_eq!(
        bag(&exported),
        bag(&run_rows),
        "same rows, same multiplicity"
    );
    // The export pool's connection is still pooled after the job.
    assert_eq!(
        connections_named(&observer, "hfs-export").await,
        1,
        "$sql-export must read through the export pool"
    );
}

/// D-a: a job budget smaller than its page ends the job as a timeout. The
/// first subject finishes and writes its shard; the second (one page of
/// `N * N` rows) is stopped by the job's deadline (`SET LOCAL
/// statement_timeout` from what is left of a 1 s budget). The job fails, its
/// result answers `504` with code `timeout` naming the subject, the first
/// subject's shard is deleted, and no export statement is left running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_export_whose_budget_is_smaller_than_its_pages_fails_with_timeout() {
    const BUDGET_SECS: u64 = 1;
    let (_container, config) = start_pg().await;
    let backend = seeded_backend(&config, &[cartesian_patient(N)]).await;
    let observer = observer(&config).await;
    let export_dir = tempfile::tempdir().expect("export dir");
    let server = server(backend, export_dir.path(), |c| {
        c.export_job_timeout_secs = BUDGET_SECS;
    });
    let alias = marker("sofbudget");
    let ids = json!({
        "resourceType": "ViewDefinition",
        "name": "ids",
        "resource": "Patient",
        "status": "active",
        "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]
    });
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "subject", "part": [
                {"name": "name", "valueString": "ids"},
                {"name": "subjectResource", "resource": ids}
            ]},
            {"name": "subject", "part": [
                {"name": "name", "valueString": "cartesian"},
                {"name": "subjectResource", "resource": cartesian_view(&alias)}
            ]}
        ]
    });

    let kicked_off = Instant::now();
    let status_path = kick_off(&server, &body).await;
    let result = result_of(&server, &status_path).await;
    let ended = kicked_off.elapsed();
    assert_eq!(
        result.status_code(),
        StatusCode::GATEWAY_TIMEOUT,
        "{}",
        result.text()
    );
    let outcome = result.json::<Value>();
    assert_eq!(outcome["issue"][0]["code"], "timeout", "{outcome}");
    assert_eq!(outcome["issue"][0]["expression"], json!(["cartesian"]));
    eprintln!(
        "SOF-BUDGET export job: result 504 {ended:?} after kick-off (budget {BUDGET_SECS} s)"
    );
    assert!(
        ended < Duration::from_secs(BUDGET_SECS) + STOP_BOUND + Duration::from_secs(3),
        "the job ends at its budget: {ended:?}"
    );

    // The worker deletes the job's shards once it has set the status.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !files_under(export_dir.path()).is_empty() {
        assert!(
            Instant::now() < deadline,
            "the failed job's shards are deleted: {:?}",
            files_under(export_dir.path())
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(active_statements(&observer, &alias).await.0, 0);
}

/// A14 (PostgreSQL cell): cancelling a SQL Query export while its
/// dependency's page statement runs on the export pool stops that statement
/// within the stop bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_sql_query_export_leaves_no_statement() {
    let (_container, config) = start_pg().await;
    let alias = marker("sofsqcancel");
    let mut leaf = cartesian_view(&alias);
    leaf["url"] = json!("http://example.org/cartesian-leaf");
    let backend = seeded_backend(&config, &[cartesian_patient(N), leaf]).await;
    let observer = observer(&config).await;
    let export_dir = tempfile::tempdir().expect("export dir");
    // The dependency is far larger than the default row cap; the cancel, not
    // the cap, must be what ends it.
    let server = server(backend, export_dir.path(), |c| {
        c.sof_sqlquery_max_source_rows_per_vd = 100_000_000;
    });

    let library = json!({
        "resourceType": "Library",
        "status": "active",
        "type": {"coding": [{
            "system": "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes",
            "code": "sql-query"
        }]},
        "content": [{
            "contentType": "application/sql",
            "data": base64::engine::general_purpose::STANDARD.encode("SELECT count(*) AS n FROM c")
        }],
        "relatedArtifact": [{
            "type": "depends-on",
            "label": "c",
            "resource": "http://example.org/cartesian-leaf"
        }]
    });
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "subject", "part": [
            {"name": "name", "valueString": "count"},
            {"name": "subjectResource", "resource": library}
        ]}]
    });
    let status_path = kick_off(&server, &body).await;

    // The dependency's statement runs, on the export pool.
    let started = Instant::now();
    let on_export_pool = loop {
        let (active, on_export_pool) = active_statements(&observer, &alias).await;
        if active > 0 {
            break on_export_pool;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the dependency's statement never became active"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        on_export_pool,
        "a SQL Query export's dependency reads through the export pool"
    );

    let cancel = server
        .delete(&status_path)
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .await;
    assert_eq!(
        cancel.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        cancel.text()
    );
    let cancelled_at = Instant::now();
    while active_statements(&observer, &alias).await.0 != 0 {
        assert!(
            cancelled_at.elapsed() < STOP_BOUND,
            "the dependency's statement still runs {:?} after the cancel",
            cancelled_at.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!(
        "SOF-CANCEL export sql-query dependency (PostgreSQL): statement gone {:?} after the cancel",
        cancelled_at.elapsed()
    );
}
