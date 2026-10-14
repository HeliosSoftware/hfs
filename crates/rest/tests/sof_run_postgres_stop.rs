//! `$sql-run` on PostgreSQL when the run cannot finish (#1864): a saturated
//! pool, a statement timeout, a client that goes away.
//!
//! - pool exhausted: `503` with `Retry-After` (a transient condition), not a
//!   `500`;
//! - statement timeout: `504` with OperationOutcome code `timeout`, not a
//!   `500`;
//! - a client that disconnects, from a buffered (JSON) or a streamed (NDJSON)
//!   response: the SoF statement stops within the stop bound (1 s); checked
//!   over a real loopback server (`axum::serve` on `127.0.0.1:0`) with a raw
//!   socket client, so the disconnect is the one a production client causes;
//! - a `408` from the HTTP request timeout (`TimeoutLayer`, as `build_app`
//!   installs it): a buffered run stops within the bound, and a streamed run
//!   that is still setting up (its response head is sent only once `run_view`
//!   returns) never starts its statement.
//!
//! Each statement is found in `pg_stat_activity` by a column-alias marker.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::OnceCell;

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// The tenant every test reads; seeded once.
const TENANT: &str = "sofstop";

/// The stop bound (issue #1864, target 1 s).
const STOP_BOUND: Duration = Duration::from_secs(1);

/// Entries per array of the cartesian Patient: `N * N` rows that PostgreSQL
/// sorts before the first one, so the statement is busy for seconds.
const N: usize = 2000;

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
        max_connections: 3,
        data_dir: Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("data"))
                .unwrap_or_else(|| PathBuf::from("data")),
        ),
        ..Default::default()
    }
}

/// A container with the schema and one cartesian Patient in [`TENANT`].
async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
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
            .expect("failed to start PostgreSQL container");
            let host = container.get_host().await.unwrap().to_string();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let backend = PostgresBackend::new(config(&host, port))
                .await
                .expect("create backend");
            backend.init_schema().await.expect("initialize schema");
            backend
                .create(
                    &TenantContext::new(TenantId::new(TENANT), TenantPermissions::full_access()),
                    "Patient",
                    cartesian_patient(N),
                    FhirVersion::R4,
                )
                .await
                .expect("seed cartesian patient");
            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
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

/// `name` × `address`, with `marker` as the first column's alias so the
/// statement can be found in `pg_stat_activity`.
fn cartesian_view(marker: &str) -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [
            {"forEach": "name", "column": [{"name": marker, "path": "family"}]},
            {"forEach": "address", "column": [{"name": "city", "path": "city"}]}
        ]
    })
}

fn marker(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// A backend over the shared container with `adjust` applied to its config.
async fn backend_with(adjust: impl FnOnce(&mut PostgresConfig)) -> Arc<PostgresBackend> {
    let pg = shared_pg().await;
    let mut cfg = config(&pg.host, pg.port);
    adjust(&mut cfg);
    Arc::new(PostgresBackend::new(cfg).await.expect("create backend"))
}

fn router(backend: Arc<PostgresBackend>) -> axum::Router {
    let runner = backend.sof_runner().expect("in-DB runner");
    let state =
        helios_rest::AppState::new(backend, ServerConfig::for_testing()).with_sof_runner(runner);
    helios_rest::routing::fhir_routes::create_routes(state)
}

/// SoF statements of one run (found by the alias marker) that are executing.
async fn active_statements(observer: &PostgresBackend, alias: &str) -> i64 {
    let client = observer.get_client().await.expect("observer connection");
    client
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
             AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE $1",
            &[&format!("%\"{alias}\"%")],
        )
        .await
        .expect("sample pg_stat_activity")
        .get(0)
}

/// The OperationOutcome issue code of a response body, if any.
fn issue_code(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("issue")?
        .get(0)?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

/// B7a (A4/A11): every pooled connection is busy for longer than the pool's
/// wait timeout. That is a transient condition: `503` with `Retry-After`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_pool_answers_503_with_retry_after() {
    let backend = backend_with(|c| {
        c.max_connections = 1;
        c.pool_wait_timeout_secs = 1;
    })
    .await;
    // Hold the pool's only connection for the whole request.
    let held = backend
        .get_client()
        .await
        .expect("hold the only connection");
    let server = TestServer::new(router(Arc::clone(&backend))).expect("test server");

    let response = server
        .post("/$sql-run?_format=json")
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        }))
        .await;
    drop(held);
    let status = response.status_code();
    let body = response.text();
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "an exhausted pool must answer 503; got {status}: {body}"
    );
    assert!(
        response.headers().get("retry-after").is_some(),
        "a 503 for an exhausted pool must carry Retry-After: {:?}",
        response.headers()
    );
}

/// B7b (A4): the PostgreSQL statement timeout fires during a `$sql-run`. That
/// is a timeout: `504` with OperationOutcome code `timeout`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn statement_timeout_answers_504_timeout() {
    let backend = backend_with(|c| c.statement_timeout_ms = 300).await;
    let server = TestServer::new(router(backend)).expect("test server");

    let response = server
        .post("/$sql-run?_format=json")
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&cartesian_view(&marker("sofstop_timeout")))
        .await;
    let status = response.status_code();
    let body = response.text();
    assert_eq!(
        status,
        StatusCode::GATEWAY_TIMEOUT,
        "a statement timeout must answer 504; got {status}: {body}"
    );
    assert_eq!(
        issue_code(&body).as_deref(),
        Some("timeout"),
        "the OperationOutcome code must be `timeout`: {body}"
    );
}

/// The production request timeout layer (`build_app` installs the same one)
/// with a 1 s timeout, around [`router`].
fn router_with_request_timeout(backend: Arc<PostgresBackend>) -> axum::Router {
    router(backend).layer(tower_http::timeout::TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(1),
    ))
}

/// A real loopback HTTP server (`axum::serve`, hyper HTTP/1.1) for `app`.
async fn serve(app: axum::Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, server)
}

/// Sends a complete `$sql-run` request for `view` in `format` over a raw
/// socket, and returns the socket without reading the response.
async fn send_raw_run(addr: SocketAddr, format: &str, view: &Value) -> tokio::net::TcpStream {
    let body = serde_json::to_vec(view).unwrap();
    let mut socket = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let head = format!(
        "POST /$sql-run?_format={format} HTTP/1.1\r\nHost: {addr}\r\n\
         Content-Type: application/fhir+json\r\nx-tenant-id: {TENANT}\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    socket
}

/// Waits until the run's statement (found by `alias`) is executing.
async fn wait_until_active(observer: &PostgresBackend, alias: &str) {
    let started = Instant::now();
    while active_statements(observer, alias).await == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the run's statement never became active"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// From now (the stop), samples the run's statement until it is gone: whether
/// it was still active once [`STOP_BOUND`] had passed, and how long it stayed.
async fn statement_after_stop(observer: &PostgresBackend, alias: &str) -> (bool, Duration) {
    let stopped_at = Instant::now();
    let mut active_at_bound = None;
    let ran_for = loop {
        let active = active_statements(observer, alias).await > 0;
        let elapsed = stopped_at.elapsed();
        if active_at_bound.is_none() && elapsed >= STOP_BOUND {
            active_at_bound = Some(active);
        }
        if !active || elapsed >= Duration::from_secs(60) {
            break elapsed;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    (active_at_bound.unwrap_or(false), ran_for)
}

/// A15: a client waiting for a buffered (JSON) `$sql-run` closes its socket
/// while the statement runs, before any response byte. The server drops the
/// handler future on the client's EOF (hyper's HTTP/1 connection notices it on
/// a busy connection), which fires the request's stop: the statement stops
/// within the bound. (If the stack did not drop the handler, the buffered run
/// would only be bounded by its deadline; #1864 plan §2.4 escalates then.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffered_disconnect_leaves_no_statement() {
    let backend = backend_with(|_| {}).await;
    let observer = backend_with(|_| {}).await;
    let (addr, server) = serve(router(backend)).await;

    let alias = marker("sofstop_buffered");
    let socket = send_raw_run(addr, "json", &cartesian_view(&alias)).await;
    wait_until_active(&observer, &alias).await;

    drop(socket);
    let (active_at_bound, ran_for) = statement_after_stop(&observer, &alias).await;
    eprintln!(
        "SOF-CANCEL rest buffered disconnect: the statement stayed active {ran_for:?} after the \
         client closed its socket (active at the bound: {active_at_bound})"
    );
    server.abort();
    assert!(
        !active_at_bound,
        "the SoF statement was still active {STOP_BOUND:?} after the client disconnected from \
         a buffered run; it ran {ran_for:?} after the disconnect"
    );
}

/// B12 (A15): a client reading a streamed NDJSON `$sql-run` closes its
/// socket. The SoF statement behind the response must stop within the bound.
/// Uses a real loopback server and a raw TCP client, so the disconnect is the
/// one a production client causes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_disconnect_leaves_no_statement() {
    let backend = backend_with(|_| {}).await;
    let observer = backend_with(|_| {}).await;
    let (addr, server) = serve(router(backend)).await;

    let alias = marker("sofstop_stream");
    let mut socket = send_raw_run(addr, "ndjson", &cartesian_view(&alias)).await;

    // The statement starts; the response head may or may not have arrived.
    wait_until_active(&observer, &alias).await;
    let mut buf = [0u8; 4096];
    let read = tokio::time::timeout(Duration::from_millis(500), socket.read(&mut buf)).await;
    let head_seen = matches!(read, Ok(Ok(n)) if n > 0 && buf[..n].starts_with(b"HTTP/1.1 200"));

    drop(socket);
    let (active_at_bound, ran_for) = statement_after_stop(&observer, &alias).await;
    eprintln!(
        "SOF-CANCEL rest streamed disconnect: response head seen before the close: \
         {head_seen}; the statement stayed active {ran_for:?} after the client closed its \
         socket (active at the bound: {active_at_bound})"
    );
    server.abort();
    assert!(
        !active_at_bound,
        "the SoF statement was still active {STOP_BOUND:?} after the client disconnected; it \
         ran {ran_for:?} after the disconnect"
    );
}

/// A15: a buffered `$sql-run` runs past the HTTP request timeout. The
/// `TimeoutLayer` answers `408` and drops the handler future, which fires the
/// request's stop: the statement stops within the bound of the `408`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_timeout_408_buffered_leaves_no_statement() {
    let backend = backend_with(|_| {}).await;
    let observer = backend_with(|_| {}).await;
    let server = TestServer::new(router_with_request_timeout(backend)).expect("test server");

    let alias = marker("sofstop_408_buffered");
    let request = server
        .post("/$sql-run?_format=json")
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&cartesian_view(&alias));
    let (response, ()) = tokio::join!(
        std::future::IntoFuture::into_future(request),
        wait_until_active(&observer, &alias)
    );
    let status = response.status_code();
    assert_eq!(
        status,
        StatusCode::REQUEST_TIMEOUT,
        "a run past the request timeout answers 408; got {status}: {}",
        response.text()
    );

    let (active_at_bound, ran_for) = statement_after_stop(&observer, &alias).await;
    eprintln!(
        "SOF-CANCEL rest 408 buffered: the statement stayed active {ran_for:?} after the 408 \
         (active at the bound: {active_at_bound})"
    );
    assert!(
        !active_at_bound,
        "the SoF statement was still active {STOP_BOUND:?} after the 408; it ran {ran_for:?} \
         after it"
    );
}

/// A15: a streamed (NDJSON) `$sql-run` sends its response head once
/// `run_view` returns, so the HTTP request timeout can only fire while the run
/// is still setting up. Here the setup waits for a pooled connection (a
/// `group=` reference is loaded first, and the pool's only connection is held
/// by the test), the `TimeoutLayer` answers `408` and drops the setup: once
/// the connection is released, no statement of the run ever starts, and the
/// pool's connection is free at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_timeout_408_streamed_during_setup_leaves_no_statement() {
    let backend = backend_with(|c| {
        c.max_connections = 1;
        c.pool_wait_timeout_secs = 30;
    })
    .await;
    let observer = backend_with(|_| {}).await;
    let held = backend
        .get_client()
        .await
        .expect("hold the only connection");
    let server =
        TestServer::new(router_with_request_timeout(Arc::clone(&backend))).expect("test server");

    let alias = marker("sofstop_408_setup");
    let started = Instant::now();
    let response = server
        .post("/$sql-run?_format=ndjson&group=Group/sofstop-group")
        .add_header(X_TENANT_ID, HeaderValue::from_static(TENANT))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&cartesian_view(&alias))
        .await;
    let status = response.status_code();
    assert_eq!(
        status,
        StatusCode::REQUEST_TIMEOUT,
        "a run still waiting in its setup at the request timeout answers 408; got {status} \
         after {:?}: {}",
        started.elapsed(),
        response.text()
    );

    drop(held);
    let released_at = Instant::now();
    let fresh = tokio::time::timeout(STOP_BOUND, backend.get_client())
        .await
        .expect("the pool's connection is free once the test releases it")
        .expect("a fresh checkout succeeds");
    let freed_after = released_at.elapsed();
    drop(fresh);
    let mut seen_active = false;
    while released_at.elapsed() < Duration::from_millis(1500) {
        if active_statements(&observer, &alias).await > 0 {
            seen_active = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    eprintln!(
        "SOF-CANCEL rest 408 streamed setup: 408 after {:?}; connection free {freed_after:?} \
         after its release; the run's statement seen active afterwards: {seen_active}",
        released_at.duration_since(started)
    );
    assert!(
        !seen_active,
        "a run whose setup was dropped by the 408 must never start its statement"
    );
}
