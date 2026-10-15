//! PostgreSQL stop cells that need a server and the runner's internals
//! (#1864): raw SQL the public `run_view` cannot issue (slow to plan, a slow
//! page bound, a cancel converted into a localized error), the `#[cfg(test)]`
//! recorder of what a run sent (`SET LOCAL` values, cancel requests, the
//! connection's fate), and fault injection through TCP proxies (a cancel
//! request that never arrives, a `ROLLBACK` that stalls).
//!
//! Every stop asserts that the producer ended within the bound and that the
//! next borrower of a pool of one gets a working session.
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres --lib -- sof::postgres

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

use super::super::runtime::SqlRunPlan;
use super::*;
use crate::backends::postgres::PostgresConfig;
use crate::core::ResourceStorage;
use crate::core::sof_producer::row_batch_channel;
use crate::core::sof_runner::{RunKind, ViewFilters};
use crate::tenant::{TenantId, TenantPermissions};

/// The stop bound (issue #1864, target 1 s).
const STOP_BOUND: Duration = Duration::from_secs(1);

/// Relations of the slow-to-plan join: an exhaustive join search over a
/// 14-relation clique plans for several seconds.
const PLAN_RELATIONS: usize = 14;

struct SharedPg {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            // Never dropped (a static); the `container_cleanup` exit hook
            // removes it.
            let container = crate::container_cleanup::with_cleanup_label(
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
            .expect("start PostgreSQL");
            let host = container.get_host().await.unwrap().to_string();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let backend = PostgresBackend::new(backend_config(&host, port))
                .await
                .expect("create backend");
            backend.init_schema().await.expect("initialize schema");
            let client = backend.get_client().await.expect("client");
            client
                .batch_execute(
                    "CREATE TABLE IF NOT EXISTS sofplan (a int, b int);
                     CREATE OR REPLACE FUNCTION sof_localized_cancel() RETURNS int AS $$
                     BEGIN
                       PERFORM pg_sleep(30);
                       RETURN 1;
                     EXCEPTION WHEN query_canceled THEN
                       -- what a server with lc_messages=de_DE reports for a cancel
                       RAISE EXCEPTION USING ERRCODE = '57014',
                         MESSAGE = 'Abbruch der Anweisung wegen Benutzeranforderung';
                     END $$ LANGUAGE plpgsql;",
                )
                .await
                .expect("test objects");
            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn backend_config(host: &str, port: u16) -> PostgresConfig {
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".into(),
        user: "postgres".into(),
        password: Some("postgres".into()),
        max_connections: 3,
        data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
        ..Default::default()
    }
}

/// A pool of one connection to `host:port` (the server or a proxy in front
/// of it), with the planner settings the slow-to-plan join needs.
fn pool_of_one(host: &str, port: u16) -> Pool {
    let mut config = deadpool_postgres::Config::new();
    config.host = Some(host.to_string());
    config.port = Some(port);
    config.user = Some("postgres".into());
    config.password = Some("postgres".into());
    config.dbname = Some("postgres".into());
    config.options =
        Some("-c join_collapse_limit=64 -c from_collapse_limit=64 -c geqo=off".to_string());
    config.pool = Some(deadpool_postgres::PoolConfig::new(1));
    config
        .create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls)
        .expect("pool of one")
}

async fn observer() -> tokio_postgres::Client {
    let pg = shared_pg().await;
    let mut config = tokio_postgres::Config::new();
    config
        .host(&pg.host)
        .port(pg.port)
        .user("postgres")
        .password("postgres")
        .dbname("postgres");
    let (client, connection) = config.connect(NoTls).await.expect("observer");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

fn marker(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// A run of raw SQL whose single column is `column`, kept as text.
fn raw_run(pool: Pool, sql: String, column: &str) -> PgRun {
    PgRun {
        pool,
        sql,
        params: Vec::new(),
        columns: vec![column.to_string()],
        decodes: vec![ColumnDecode::Text],
        limit: None,
        page_size: None,
        bound_sql: export_bound_sql().to_string(),
        recorder: Some(Recorder::default()),
    }
}

/// A run of `view` for `tenant`, compiled as the runner does, paged when
/// `page_size` is set.
fn view_run(pool: Pool, tenant: &str, view: &Value, page_size: Option<usize>) -> PgRun {
    let plan = SqlRunPlan::compile(view, SqlDialect::Postgres, FhirVersion::default_enabled())
        .expect("compile view");
    let filters = ViewFilters::default();
    let prepared = match page_size {
        Some(_) => plan.finish_paged(tenant, &filters),
        None => plan.finish(tenant, &filters),
    }
    .expect("finish view");
    PgRun {
        pool,
        sql: prepared.query.sql,
        params: prepared
            .params
            .into_iter()
            .map(PgParam::from_runtime)
            .collect(),
        columns: prepared.query.columns,
        decodes: prepared.query.column_decodes,
        limit: prepared.client_limit,
        page_size,
        bound_sql: export_bound_sql().to_string(),
        recorder: Some(Recorder::default()),
    }
}

/// A Patient whose `name` and `address` arrays have `n` entries each.
fn cartesian_patient(id: &str, n: usize) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "name": (0..n).map(|i| json!({"family": format!("f{i:05}")})).collect::<Vec<_>>(),
        "address": (0..n).map(|i| json!({"city": format!("c{i:05}")})).collect::<Vec<_>>(),
    })
}

/// `name` × `address`, with the marker as the first column's alias.
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

/// A fresh tenant holding `patients`; returns its id.
async fn seed(patients: Vec<Value>) -> String {
    let pg = shared_pg().await;
    let backend = PostgresBackend::new(backend_config(&pg.host, pg.port))
        .await
        .expect("create backend");
    let tenant_id = format!("sofstop_{}", uuid::Uuid::new_v4().simple());
    let tenant = TenantContext::new(
        TenantId::new(tenant_id.clone()),
        TenantPermissions::full_access(),
    );
    for patient in patients {
        backend
            .create(&tenant, "Patient", patient, FhirVersion::R4)
            .await
            .expect("seed patient");
    }
    tenant_id
}

/// Statements of this run (found by the marker) that are executing.
async fn active(observer: &tokio_postgres::Client, marker: &str) -> Vec<i32> {
    observer
        .query(
            "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
             AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE $1",
            &[&format!("%{marker}%")],
        )
        .await
        .expect("sample pg_stat_activity")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

/// Waits until the run's statement executes; returns its backend pid.
async fn wait_active(observer: &tokio_postgres::Client, marker: &str) -> i32 {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(pid) = active(observer, marker).await.first() {
            return *pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the run's statement never became active"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// How long the marked statement stays active after `stopped_at` (polled
/// every 10 ms, up to `max`).
async fn active_for(
    observer: &tokio_postgres::Client,
    marker: &str,
    stopped_at: std::time::Instant,
    max: Duration,
) -> Duration {
    loop {
        let elapsed = stopped_at.elapsed();
        if active(observer, marker).await.is_empty() || elapsed >= max {
            return elapsed;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// `pg_stat_activity` shows a statement's text from its Parse on, so a test
/// that wants to stop the statement while it executes (not while the runner
/// still prepares it) waits a little after it shows up.
async fn settle_into_execution() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Waits (up to `max`) until the run recorded its connection's fate.
async fn fate(recorder: &Recorder, max: Duration) -> Option<ConnectionFate> {
    let deadline = std::time::Instant::now() + max;
    loop {
        let fate = recorder.events().iter().find_map(|event| match event {
            Recorded::Fate(fate) => Some(*fate),
            _ => None,
        });
        if fate.is_some() || std::time::Instant::now() >= deadline {
            return fate;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The next borrower on the pool of one: its backend pid, and whether it is
/// inside a transaction (`transaction_read_only` would be `on` inside the
/// run's `READ ONLY` transaction).
async fn next_borrower(pool: &Pool) -> (i32, String) {
    let client = tokio::time::timeout(STOP_BOUND, pool.get())
        .await
        .expect("the pool's connection is free within the bound")
        .expect("check out");
    let row = client
        .query_one(
            "SELECT pg_backend_pid(), current_setting('transaction_read_only')",
            &[],
        )
        .await
        .expect("SELECT on the next borrower");
    (row.get(0), row.get(1))
}

async fn collect(rows: RowStream, max: Duration) -> (usize, Option<SofError>) {
    let mut rows = rows;
    let mut count = 0;
    let mut error = None;
    tokio::time::timeout(max, async {
        while let Some(item) = rows.next().await {
            match item {
                Ok(_) => count += 1,
                Err(e) => error = Some(e),
            }
        }
    })
    .await
    .expect("the stream ends");
    (count, error)
}

fn ctx_with_deadline(budget: Option<Duration>) -> RunContext {
    let mut ctx = RunContext::detached(RunKind::Preview);
    ctx.deadline = budget.map(|budget| tokio::time::Instant::now() + budget);
    ctx
}

/// A8 (C8): a stop while PostgreSQL is still planning the statement (the
/// Bind of `query_raw`) cancels the planning: the statement leaves
/// `pg_stat_activity` within the bound and the connection is reused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_planning_stops_within_the_bound() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofplan");
    let relations = (0..PLAN_RELATIONS)
        .map(|i| format!("sofplan t{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let predicates = (0..PLAN_RELATIONS)
        .flat_map(|i| ((i + 1)..PLAN_RELATIONS).map(move |j| format!("t{i}.a = t{j}.b")))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT t0.a::text AS \"{m}\" FROM {relations} WHERE {predicates}");
    let run = raw_run(pool.clone(), sql, &m);
    let recorder = run.recorder.clone().unwrap();
    let ctx = RunContext::detached(RunKind::Preview);
    let cancel = ctx.cancel.clone();
    let rows = spawn_pg_run(run, &ctx);
    let pid = wait_active(&observer, &m).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    cancel.cancel();
    let stopped_at = std::time::Instant::now();
    let (_, error) = collect(rows, STOP_BOUND).await;
    let ran_for = active_for(&observer, &m, stopped_at, Duration::from_secs(30)).await;
    eprintln!(
        "SOF-CANCEL postgres cancel_while_planning: the statement stayed active {ran_for:?} after the cancel"
    );
    assert!(
        ran_for <= STOP_BOUND,
        "planning ran {ran_for:?} after the cancel"
    );
    assert!(matches!(error, Some(SofError::Cancelled)), "{error:?}");
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse)
    );
    assert_eq!(next_borrower(&pool).await, (pid, "off".to_string()));
}

/// B4's third sub-case: a stop during the page-bound lookup. The bound
/// statement here sleeps (its marker is in a comment); the stop cancels it
/// within the bound, no page statement runs, and the connection is reused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paged_export_stops_during_the_bound_lookup() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let tenant = seed(vec![cartesian_patient("p1", 5)]).await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofbound");
    let mut run = view_run(pool.clone(), &tenant, &cartesian_view(&m), Some(10));
    run.bound_sql = format!(
        "SELECT b.id FROM ({}) b CROSS JOIN pg_sleep(30) /* {m} bound */",
        export_bound_sql()
    );
    let recorder = run.recorder.clone().unwrap();
    let rows = spawn_pg_run(run, &RunContext::detached(RunKind::ExportJob));
    let pid = wait_active(&observer, &format!("{m} bound")).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    drop(rows);
    let stopped_at = std::time::Instant::now();
    let ran_for = active_for(&observer, &m, stopped_at, Duration::from_secs(30)).await;
    eprintln!(
        "SOF-CANCEL postgres paged export, during the bound lookup: the bound statement stayed active {ran_for:?} after the drop"
    );
    assert!(
        ran_for <= STOP_BOUND,
        "the bound lookup ran {ran_for:?} after the drop"
    );
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse)
    );
    assert!(
        !recorder.events().contains(&Recorded::Statement("page")),
        "no page statement after a stop in the bound lookup"
    );
    assert_eq!(next_borrower(&pool).await, (pid, "off".to_string()));
}

/// D-a (C4/S13): every page of an export runs in its own transaction under
/// `SET LOCAL statement_timeout` from what is left of the run's deadline:
/// one value per page, non-increasing, each the time left when its
/// transaction started (rounded up), in the order BEGIN, bound, page, COMMIT.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paged_export_pages_run_under_set_local_from_the_remaining_deadline() {
    let pg = shared_pg().await;
    let tenant = seed(
        (0..3)
            .map(|i| cartesian_patient(&format!("p{i}"), 10))
            .collect(),
    )
    .await;
    let pool = pool_of_one(&pg.host, pg.port);
    let run = view_run(pool, &tenant, &cartesian_view(&marker("sofpages")), Some(1));
    let recorder = run.recorder.clone().unwrap();
    let ctx = ctx_with_deadline(Some(Duration::from_secs(30)));
    let deadline = ctx.deadline.unwrap();
    let spawned_at = Instant::now();
    let (rows, error) = collect(spawn_pg_run(run, &ctx), Duration::from_secs(30)).await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(rows, 3 * 10 * 10);

    let events = recorder.events();
    let begins: Vec<(Instant, i32)> = events
        .iter()
        .filter_map(|event| match event {
            Recorded::Begin { at, timeout_ms } => {
                Some((*at, timeout_ms.expect("a SET LOCAL per page")))
            }
            _ => None,
        })
        .collect();
    // Three pages, then the transaction whose bound finds no more ids.
    assert_eq!(begins.len(), 4, "{events:?}");
    for window in begins.windows(2) {
        assert!(
            window[1].1 <= window[0].1,
            "not renewed per page: {begins:?}"
        );
    }
    // The runner computes each value from the time left just before it
    // records the `Begin` instant, so a millisecond boundary can fall between
    // the two: the value lies between the time left at that instant and the
    // time left at the previous `Begin` (the spawn, for the first), both
    // rounded up like the value.
    let mut earliest = spawned_at;
    for (at, timeout_ms) in &begins {
        let at_least = statement_timeout_ms(deadline.saturating_duration_since(*at));
        let at_most = statement_timeout_ms(deadline.saturating_duration_since(earliest));
        assert!(
            (at_least..=at_most).contains(timeout_ms),
            "{timeout_ms} ms outside the time left between the previous and this BEGIN: [{at_least}, {at_most}] ms"
        );
        assert!(
            *timeout_ms > 25_000,
            "the deadline is the run's, not a page's"
        );
        earliest = *at;
    }
    let sequence: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            Recorded::Begin { .. } => Some("begin"),
            Recorded::Statement("bound") => Some("bound"),
            Recorded::Statement("page") => Some("page"),
            Recorded::Commit => Some("commit"),
            _ => None,
        })
        .collect();
    let page = ["begin", "bound", "page", "commit"];
    let expected: Vec<&str> = page
        .iter()
        .chain(&page)
        .chain(&page)
        .chain(&["begin", "bound", "commit"])
        .copied()
        .collect();
    assert_eq!(sequence, expected, "{events:?}");
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse)
    );
}

/// D-a (C4): with a 1 s budget, a consumer that stops reading inside the
/// first page and resumes after the deadline gets a `Timeout` with that
/// budget, and no SQL runs after the deadline: the run's last transaction is
/// the first page's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_consumer_paused_past_the_deadline_fails_the_next_page_before_any_sql() {
    let pg = shared_pg().await;
    let tenant = seed(
        (0..2)
            .map(|i| cartesian_patient(&format!("p{i}"), 120))
            .collect(),
    )
    .await;
    let pool = pool_of_one(&pg.host, pg.port);
    let run = view_run(pool, &tenant, &cartesian_view(&marker("sofpause")), Some(1));
    let recorder = run.recorder.clone().unwrap();
    let budget = Duration::from_secs(1);
    let ctx = ctx_with_deadline(Some(budget));
    let deadline = ctx.deadline.unwrap();
    let mut rows = spawn_pg_run(run, &ctx);
    rows.next().await.expect("a row").expect("a row");
    tokio::time::sleep_until(deadline + Duration::from_millis(300)).await;
    let (more, error) = collect(rows, Duration::from_secs(5)).await;
    assert!(more < 2 * 120 * 120, "the run stopped at its deadline");
    assert!(
        matches!(error, Some(SofError::Timeout { budget: Some(b), .. }) if b <= budget && budget - b < Duration::from_millis(200)),
        "{error:?}"
    );
    let events = recorder.events();
    let begins: Vec<Instant> = events
        .iter()
        .filter_map(|event| match event {
            Recorded::Begin { at, .. } => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(begins.len(), 1, "only the first page ran: {events:?}");
    assert!(begins[0] < deadline);
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == Recorded::Statement("page"))
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse)
    );
}

/// What a proxy does with one direction of a session connection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProxyMode {
    /// Accept a cancel request's connection, then never forward it.
    BlackholeCancel,
    /// Stop forwarding client bytes once a `ROLLBACK` passes.
    StallRollback,
}

/// The cancel request's code in the first 8 bytes of a connection.
const CANCEL_REQUEST_CODE: [u8; 4] = 80877102u32.to_be_bytes();

/// A TCP proxy in front of `host:port`.
async fn start_proxy(host: &str, port: u16, mode: ProxyMode) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap().port();
    let target = format!("{host}:{port}");
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let target = target.clone();
            tokio::spawn(async move {
                let (mut client_read, mut client_write) = client.into_split();
                let mut head = [0u8; 8];
                if client_read.read_exact(&mut head).await.is_err() {
                    return;
                }
                if mode == ProxyMode::BlackholeCancel && head[4..8] == CANCEL_REQUEST_CODE {
                    // Read and keep the request; never forward or close it.
                    let mut sink = Vec::new();
                    let _ = client_read.read_to_end(&mut sink).await;
                    std::future::pending::<()>().await;
                }
                let Ok(server) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let (mut server_read, mut server_write) = server.into_split();
                if server_write.write_all(&head).await.is_err() {
                    return;
                }
                let upstream = tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    let mut stalled = false;
                    loop {
                        let n = match client_read.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        if mode == ProxyMode::StallRollback
                            && buf[..n].windows(8).any(|w| w == b"ROLLBACK")
                        {
                            stalled = true;
                        }
                        if !stalled && server_write.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                });
                let _ = tokio::io::copy(&mut server_read, &mut client_write).await;
                upstream.abort();
            });
        }
    });
    local
}

/// Runs `SELECT ... FROM pg_sleep(secs)` marked by `m`, through `pool`.
fn sleep_run(pool: Pool, m: &str, secs: u32) -> PgRun {
    raw_run(
        pool,
        format!("SELECT 1::text AS \"{m}\" FROM pg_sleep({secs})"),
        m,
    )
}

/// R-n / D-14 c: the cancel request reaches a proxy that swallows it. The
/// statement never confirms the cancel, so the drain times out, the
/// connection is closed and the producer ends within the bound; the next
/// borrower gets a fresh session. The statement itself then ends at its
/// `SET LOCAL statement_timeout` (the run's deadline): the stated exception.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_request_that_cannot_complete_closes_the_connection() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let port = start_proxy(&pg.host, pg.port, ProxyMode::BlackholeCancel).await;
    let pool = pool_of_one("127.0.0.1", port);
    let m = marker("sofblackhole");
    let run = sleep_run(pool.clone(), &m, 30);
    let recorder = run.recorder.clone().unwrap();
    let budget = Duration::from_secs(4);
    let ctx = ctx_with_deadline(Some(budget));
    let deadline = ctx.deadline.unwrap();
    let rows = spawn_pg_run(run, &ctx);
    let pid = wait_active(&observer, &m).await;
    settle_into_execution().await;

    drop(rows);
    let stopped_at = std::time::Instant::now();
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Close),
        "the producer ended within the bound and closed the connection"
    );
    eprintln!(
        "SOF-CANCEL postgres cancel request black-holed: producer ended {:?} after the drop",
        stopped_at.elapsed()
    );
    let (next_pid, in_tx) = next_borrower(&pool).await;
    assert_ne!(next_pid, pid, "a fresh session");
    assert_eq!(in_tx, "off");
    // The stated exception: the statement is bounded by its SET LOCAL.
    let ran_for = active_for(&observer, &m, stopped_at, Duration::from_secs(30)).await;
    assert!(
        tokio::time::Instant::now() <= deadline + STOP_BOUND,
        "the statement outlived its SET LOCAL statement_timeout ({ran_for:?} after the drop)"
    );
}

/// R-n: the cancel works but the `ROLLBACK` after it stalls (a proxy holds
/// it). The cleanup gives up after its timeout, closes the connection, and
/// the producer ends within the bound; the next borrower gets a fresh
/// session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_rollback_closes_the_connection() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let port = start_proxy(&pg.host, pg.port, ProxyMode::StallRollback).await;
    let pool = pool_of_one("127.0.0.1", port);
    let m = marker("sofstall");
    let run = sleep_run(pool.clone(), &m, 30);
    let recorder = run.recorder.clone().unwrap();
    let rows = spawn_pg_run(run, &RunContext::detached(RunKind::Preview));
    let pid = wait_active(&observer, &m).await;
    settle_into_execution().await;

    drop(rows);
    let stopped_at = std::time::Instant::now();
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Close)
    );
    let events = recorder.events();
    assert!(events.contains(&Recorded::CancelRequest), "{events:?}");
    assert!(events.contains(&Recorded::Rollback), "{events:?}");
    let ran_for = active_for(&observer, &m, stopped_at, Duration::from_secs(30)).await;
    assert!(
        ran_for <= STOP_BOUND,
        "the cancelled statement ran {ran_for:?}"
    );
    let (next_pid, in_tx) = next_borrower(&pool).await;
    assert_ne!(next_pid, pid, "a fresh session");
    assert_eq!(in_tx, "off");
}

/// `stop_at_cap` for a cap the client enforces (the SQL of `unionAll` and
/// `repeat` views has no `LIMIT`): when the cap is reached the statement is
/// still sending rows; it is cancelled and drained within the bound, the run
/// ends cleanly with exactly the cap, and the connection is reused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_cap_stops_a_statement_that_is_still_sending_rows() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofcap");
    let mut run = raw_run(
        pool.clone(),
        format!(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 1000000000) \
             SELECT x::text AS \"{m}\" FROM c"
        ),
        &m,
    );
    run.limit = Some(10);
    let recorder = run.recorder.clone().unwrap();
    let rows = spawn_pg_run(run, &RunContext::detached(RunKind::Preview));
    let (count, error) = collect(rows, Duration::from_secs(10)).await;
    let ended_at = std::time::Instant::now();
    assert_eq!(count, 10);
    assert!(error.is_none(), "{error:?}");
    let ran_for = active_for(&observer, &m, ended_at, Duration::from_secs(30)).await;
    eprintln!(
        "SOF-CANCEL postgres stop_at_cap (client cap): the statement stayed active {ran_for:?} after the cap"
    );
    assert!(
        ran_for <= STOP_BOUND,
        "the statement ran {ran_for:?} past the cap"
    );
    assert!(recorder.events().contains(&Recorded::CancelRequest));
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse),
        "{:?}",
        recorder.events()
    );
}

/// A8/R-m: 200 stops at different points never let a cancel reach the next
/// borrower of a pool of one, whose own statement (a 20 ms sleep) must
/// always succeed. Odd iterations stop an unpaged run mid-stream (a cancel
/// is sent); even ones stop an export after a small page was fully received
/// and committed (nothing runs, so no cancel is sent at all).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_cancel_never_reaches_the_next_borrower() {
    let pg = shared_pg().await;
    // One page per Patient: 3600 rows, more than the channel holds, so a
    // paged run that read under 1000 rows is still delivering its first page.
    let tenant = seed(vec![
        cartesian_patient("p1", 60),
        cartesian_patient("p2", 60),
    ])
    .await;
    let pool = pool_of_one(&pg.host, pg.port);
    let view = cartesian_view(&marker("sofstress"));
    let mut cancels = 0;
    for i in 0..200usize {
        let paged = i % 2 == 0;
        let run = view_run(pool.clone(), &tenant, &view, paged.then_some(1));
        let recorder = run.recorder.clone().unwrap();
        let mut rows = spawn_pg_run(run, &RunContext::detached(RunKind::Preview));
        let read = if paged {
            (i * 37) % 1000
        } else {
            (i * 37) % 3000
        };
        for _ in 0..read {
            match rows.next().await {
                Some(row) => {
                    row.expect("a row");
                }
                None => break,
            }
        }
        drop(rows);
        let client = pool.get().await.expect("next borrower");
        client
            .batch_execute("SELECT pg_sleep(0.02)")
            .await
            .unwrap_or_else(|e| panic!("iteration {i}: the next borrower's statement failed: {e}"));
        drop(client);
        let events = recorder.events();
        if events.contains(&Recorded::CancelRequest) {
            cancels += 1;
            assert!(
                !paged,
                "iteration {i}: a stop during delivery sent a cancel: {events:?}"
            );
        }
    }
    eprintln!(
        "SOF-CANCEL postgres next-borrower stress: 200 stops, {cancels} cancel requests, no stray cancel"
    );
}

/// A9 (C6): a statement stopped by the run's own `SET LOCAL
/// statement_timeout` ends as `Timeout` with the run's budget; no cancel
/// request is in flight, so the connection is reused after `ROLLBACK`. A
/// `57014` that the run did not cause (another session cancelled it before
/// the deadline) is a backend fault with its SQLSTATE, and the connection is
/// closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_statement_timeout_57014_is_not_a_user_cancel() {
    let pg = shared_pg().await;
    let observer = observer().await;

    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("softimeout");
    let run = sleep_run(pool.clone(), &m, 30);
    let recorder = run.recorder.clone().unwrap();
    let budget = Duration::from_secs(1);
    let rows = spawn_pg_run(run, &ctx_with_deadline(Some(budget)));
    let pid = wait_active(&observer, &m).await;
    let (_, error) = collect(rows, budget + STOP_BOUND).await;
    assert!(
        matches!(error, Some(SofError::Timeout { budget: Some(b), .. }) if b <= budget),
        "{error:?}"
    );
    assert!(!recorder.events().contains(&Recorded::CancelRequest));
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse)
    );
    assert_eq!(next_borrower(&pool).await, (pid, "off".to_string()));

    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofforeign");
    let run = sleep_run(pool.clone(), &m, 30);
    let recorder = run.recorder.clone().unwrap();
    let rows = spawn_pg_run(run, &ctx_with_deadline(Some(Duration::from_secs(60))));
    let pid = wait_active(&observer, &m).await;
    settle_into_execution().await;
    observer
        .query_one("SELECT pg_cancel_backend($1)", &[&pid])
        .await
        .expect("cancel from another session");
    let (_, error) = collect(rows, STOP_BOUND).await;
    assert!(
        matches!(&error, Some(SofError::Backend { sqlstate: Some(s), .. }) if s == "57014"),
        "{error:?}"
    );
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Close)
    );
    assert_ne!(next_borrower(&pool).await.0, pid);
}

/// A9: a cancel the statement reports in another language (as a server with
/// `lc_messages=de_DE` does; emulated by a function that re-raises the
/// cancel with the German text, since the test image has no locales) cannot
/// be told apart from a cancel still in flight: the connection is closed.
/// The run's error is still `Cancelled`: the kind comes from the stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_localized_cancel_message_closes_the_connection() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("soflocalized");
    let run = raw_run(
        pool.clone(),
        format!("SELECT sof_localized_cancel()::text AS \"{m}\""),
        &m,
    );
    let recorder = run.recorder.clone().unwrap();
    let ctx = RunContext::detached(RunKind::Preview);
    let cancel = ctx.cancel.clone();
    let rows = spawn_pg_run(run, &ctx);
    let pid = wait_active(&observer, &m).await;
    settle_into_execution().await;
    cancel.cancel();
    let (_, error) = collect(rows, STOP_BOUND).await;
    assert!(matches!(error, Some(SofError::Cancelled)), "{error:?}");
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Close)
    );
    assert_ne!(next_borrower(&pool).await.0, pid);
}

/// A9: a cancel that the statement confirms with the cancel request's own
/// error is the one case where a cancelled connection goes back to the pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_only_a_user_cancel_lets_the_connection_be_reused() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofusercancel");
    let run = sleep_run(pool.clone(), &m, 30);
    let recorder = run.recorder.clone().unwrap();
    let ctx = RunContext::detached(RunKind::Preview);
    let cancel = ctx.cancel.clone();
    let rows = spawn_pg_run(run, &ctx);
    let pid = wait_active(&observer, &m).await;
    settle_into_execution().await;
    cancel.cancel();
    let stopped_at = std::time::Instant::now();
    let (_, error) = collect(rows, STOP_BOUND).await;
    assert!(matches!(error, Some(SofError::Cancelled)), "{error:?}");
    assert!(active_for(&observer, &m, stopped_at, Duration::from_secs(30)).await <= STOP_BOUND);
    assert_eq!(
        fate(&recorder, STOP_BOUND).await,
        Some(ConnectionFate::Reuse),
        "{:?}",
        recorder.events()
    );
    let events = recorder.events();
    assert!(events.contains(&Recorded::CancelRequest), "{events:?}");
    assert_eq!(next_borrower(&pool).await, (pid, "off".to_string()));
}

/// R-a: a producer aborted while its statement runs inside the `READ ONLY`
/// transaction never returns that session to the pool: the next borrower
/// gets a fresh session outside any transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_aborted_producer_never_repools_an_open_transaction() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let m = marker("sofabort");
    let run = sleep_run(pool.clone(), &m, 2);
    let (out, stop, _rows) = row_batch_channel(RUNNER, &RunContext::detached(RunKind::Preview));
    let producer = tokio::spawn(produce(run, stop, PgSink::Rows(out)));
    let pid = wait_active(&observer, &m).await;
    producer.abort();
    let _ = producer.await;
    let (next_pid, in_tx) = next_borrower(&pool).await;
    assert_ne!(
        next_pid, pid,
        "the aborted run's session was closed, not re-pooled"
    );
    assert_eq!(in_tx, "off");
}

/// A10: rows cross the batch channel in the server's order, each once
/// (duplicates kept), and an error mid-stream is the last item, after
/// exactly the rows before it, with its SQLSTATE.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pg_row_batches_keep_order_multiplicity_and_error_position() {
    let pg = shared_pg().await;
    let pool = pool_of_one(&pg.host, pg.port);
    let run = raw_run(
        pool,
        "SELECT (CASE WHEN i = 3001 THEN 1 / (i - i) ELSE i / 2 END)::text AS n \
         FROM generate_series(0, 5999) i"
            .to_string(),
        "n",
    );
    let mut rows = spawn_pg_run(run, &RunContext::detached(RunKind::Preview));
    let mut seen = Vec::new();
    let mut error = None;
    while let Some(item) = rows.next().await {
        match item {
            Ok(row) => {
                assert!(error.is_none(), "a row after the error item");
                seen.push(row["n"].as_str().expect("text").parse::<i64>().unwrap());
            }
            Err(e) => error = Some(e),
        }
    }
    let expected: Vec<i64> = (0..=3000).map(|i| i / 2).collect();
    assert_eq!(seen, expected);
    assert!(
        matches!(&error, Some(SofError::Backend { sqlstate: Some(s), .. }) if s == "22012"),
        "{error:?}"
    );
}

/// A pool of one connection to the database `dbname` of the shared server.
fn pool_of_one_in(dbname: &str) -> Pool {
    let pg = SHARED_PG.get().expect("the shared server is started");
    let mut config = deadpool_postgres::Config::new();
    config.host = Some(pg.host.clone());
    config.port = Some(pg.port);
    config.user = Some("postgres".into());
    config.password = Some("postgres".into());
    config.dbname = Some(dbname.to_string());
    config.pool = Some(deadpool_postgres::PoolConfig::new(1));
    config
        .create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls)
        .expect("pool of one")
}

/// Group lookups of `dbname` that are executing while they wait on a lock.
async fn lock_blocked_group_lookups(observer: &tokio_postgres::Client, dbname: &str) -> Vec<i32> {
    observer
        .query(
            "SELECT pid FROM pg_stat_activity WHERE datname = $1 AND state = 'active' \
             AND wait_event_type = 'Lock' AND query LIKE '%resource_type = ''Group''%'",
            &[&dbname],
        )
        .await
        .expect("sample pg_stat_activity")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

/// #1864 N11 (L1-F3): the Group load of a run's setup stops like the run's
/// own statements. With `resources` locked `ACCESS EXCLUSIVE` by another
/// session (in a database of its own, so no other cell waits on the lock),
/// the Group lookup waits on the lock; a cancel, and the run's deadline, each
/// end `run_view` within the bound with their own kind, and the lookup leaves
/// `pg_stat_activity` within the bound while the lock is still held (a cancel
/// request, not the closed socket, stopped it; `SET LOCAL` bounds it at the
/// deadline on the server as well).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_during_a_lock_blocked_group_load_stops_its_statement() {
    let pg = shared_pg().await;
    let observer = observer().await;
    let dbname = marker("sofgroup");
    observer
        .batch_execute(&format!("CREATE DATABASE {dbname}"))
        .await
        .expect("create the cell's database");
    let backend = PostgresBackend::new(PostgresConfig {
        dbname: dbname.clone(),
        ..backend_config(&pg.host, pg.port)
    })
    .await
    .expect("create backend");
    backend.init_schema().await.expect("initialize schema");
    drop(backend);

    let mut locker_config = tokio_postgres::Config::new();
    locker_config
        .host(&pg.host)
        .port(pg.port)
        .user("postgres")
        .password("postgres")
        .dbname(&dbname);
    let (locker, connection) = locker_config.connect(NoTls).await.expect("locker");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    locker
        .batch_execute("BEGIN; LOCK TABLE resources IN ACCESS EXCLUSIVE MODE")
        .await
        .expect("lock resources");

    let runner = PgInDbRunner::new(pool_of_one_in(&dbname));
    let tenant = TenantContext::new(TenantId::new("sofgroup"), TenantPermissions::full_access());
    let view = json!({"resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
        "select": [{"column": [{"name": "id", "path": "id"}]}]});
    for cell in ["cancel", "deadline"] {
        let budget = Duration::from_millis(1500);
        let mut ctx = RunContext::detached(RunKind::Preview);
        let deadline = tokio::time::Instant::now() + budget;
        if cell == "deadline" {
            ctx.deadline = Some(deadline);
        }
        let cancel = ctx.cancel.clone();
        let run = {
            let tenant = tenant.clone();
            let view = view.clone();
            let runner = PgInDbRunner {
                pool: runner.pool.clone(),
                fhir_version: runner.fhir_version,
                export_page_size: None,
            };
            tokio::spawn(async move {
                let filters = ViewFilters {
                    group: vec!["Group/g1".into()],
                    ..Default::default()
                };
                runner
                    .run_view(&tenant, RunRequest::new(view, filters), ctx)
                    .await
                    .map(|output| output.schema)
            })
        };
        let waiting = tokio::time::Instant::now() + Duration::from_secs(30);
        while lock_blocked_group_lookups(&observer, &dbname)
            .await
            .is_empty()
        {
            assert!(
                tokio::time::Instant::now() < waiting,
                "{cell}: the Group lookup never waited on the lock"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let stopped_at = if cell == "cancel" {
            cancel.cancel();
            std::time::Instant::now()
        } else {
            tokio::time::sleep_until(deadline).await;
            std::time::Instant::now()
        };
        let result = tokio::time::timeout(STOP_BOUND, run)
            .await
            .unwrap_or_else(|_| panic!("{cell}: run_view still waits on the Group lookup"))
            .unwrap();
        match (cell, &result) {
            ("cancel", Err(SofError::Cancelled)) | ("deadline", Err(SofError::Timeout { .. })) => {}
            _ => panic!("{cell}: {result:?}"),
        }
        let gone_after = loop {
            let elapsed = stopped_at.elapsed();
            if lock_blocked_group_lookups(&observer, &dbname)
                .await
                .is_empty()
                || elapsed > STOP_BOUND
            {
                break elapsed;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        eprintln!(
            "SOF-CANCEL pg group load {cell}: lookup left pg_stat_activity {gone_after:?} after the stop"
        );
        assert!(
            gone_after <= STOP_BOUND,
            "{cell}: the Group lookup still waited {gone_after:?} after the stop"
        );
    }
    locker
        .batch_execute("ROLLBACK")
        .await
        .expect("release the lock");
}
