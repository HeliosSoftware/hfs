//! #1911: PostgreSQL's cross-tenant `count_by_tenant` runs under its own
//! statement budget (`HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS`), below the
//! HTTP request timeout, so a slow count on `GET /admin/tenants` and
//! `GET /console/metrics/tenants` answers `504` with an OperationOutcome
//! instead of the request timeout's generic `408`.
//!
//! The full application (`create_app_with_config`, which carries the
//! `TimeoutLayer`) is served against a PostgreSQL container. The count is held
//! with a lock, not a large store: a second pool opens a transaction holding
//! `LOCK TABLE resources IN ACCESS EXCLUSIVE MODE`, so the count's `SELECT`
//! waits until its budget or the request timeout fires. Each test has its own
//! database, because the lock would stall any other test sharing one.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

mod common;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod admin_tenants_postgres_tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use axum::http::StatusCode;
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
    use tokio::sync::OnceCell;
    use tokio::time::timeout;

    /// The count's text as PostgreSQL reports it in `pg_stat_activity.query`.
    const COUNT_SQL_PREFIX: &str = "SELECT tenant_id, COUNT(*)::bigint FROM resources";

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
                // Pinned to 16: the backend sends `plan_cache_mode` as a startup
                // option, which the module's default postgres:11 rejects.
                let container = super::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("failed to start PostgreSQL container");
                SharedPg {
                    host: container.get_host().await.expect("host").to_string(),
                    port: container.get_host_port_ipv4(5432).await.expect("port"),
                    _container: container,
                }
            })
            .await
    }

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    fn pg_config(dbname: &str) -> PostgresConfig {
        let pg = SHARED_PG.get().expect("container started");
        PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: dbname.to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 3,
            data_dir: Some(data_dir()),
            ..Default::default()
        }
    }

    /// A fresh, schema-initialized database seeded with three live Patients
    /// in tenant `alpha`; returns its name.
    async fn isolated_database() -> String {
        shared_pg().await;
        let dbname = format!("admin_count_{}", uuid::Uuid::new_v4().simple());
        PostgresBackend::new(pg_config("postgres"))
            .await
            .expect("connect to postgres")
            .get_client()
            .await
            .expect("checkout")
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create isolated database");
        let backend = PostgresBackend::new(pg_config(&dbname))
            .await
            .expect("connect for schema");
        backend.init_schema().await.expect("init schema");
        let alpha = TenantContext::new(TenantId::new("alpha"), TenantPermissions::full_access());
        for _ in 0..3 {
            backend
                .create(
                    &alpha,
                    "Patient",
                    json!({"resourceType": "Patient"}),
                    FhirVersion::default(),
                )
                .await
                .expect("seed alpha");
        }
        dbname
    }

    /// The application on `backend`, with `request_timeout` seconds of HTTP
    /// request timeout and auth disabled.
    fn server(backend: PostgresBackend, request_timeout: u64) -> TestServer {
        let config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            seed_conformance: false,
            request_timeout,
            ..ServerConfig::for_testing()
        };
        TestServer::new(helios_rest::create_app_with_config(backend, config)).expect("test server")
    }

    /// Run on a session outside the application's pool, opens a transaction
    /// holding an exclusive lock on `resources`; every count queues behind it
    /// until `ROLLBACK`.
    const LOCK_RESOURCES: &str = "BEGIN; LOCK TABLE resources IN ACCESS EXCLUSIVE MODE";

    fn assert_timeout_outcome(response: &axum_test::TestResponse, what: &str) {
        assert_eq!(
            response.status_code(),
            StatusCode::GATEWAY_TIMEOUT,
            "{what}: {}",
            response.text()
        );
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome", "{what}: {body}");
        assert_eq!(body["issue"][0]["code"], "timeout", "{what}: {body}");
    }

    /// A count slower than its budget answers `504` with an OperationOutcome
    /// on both REST consumers, not the request timeout's `408`, and the next
    /// request after the lock is released counts normally.
    #[tokio::test]
    async fn slow_count_answers_504_on_admin_tenants_and_console_metrics() {
        let dbname = isolated_database().await;
        let backend = PostgresBackend::new(PostgresConfig {
            count_by_tenant_statement_timeout_ms: 1_000,
            ..pg_config(&dbname)
        })
        .await
        .expect("backend");
        let server = server(backend, 10);
        let side = PostgresBackend::new(pg_config(&dbname))
            .await
            .expect("side");
        let blocker = side.get_client().await.expect("blocker session");
        blocker
            .batch_execute(LOCK_RESOURCES)
            .await
            .expect("lock resources");

        for path in ["/admin/tenants", "/console/metrics/tenants"] {
            let started = Instant::now();
            let response = server.get(path).await;
            let elapsed = started.elapsed();
            println!("{path} with a 1000 ms count budget answered after {elapsed:?}");
            // A `504`, not the request timeout's `408`: the budget answered.
            assert_timeout_outcome(&response, path);
        }

        blocker.batch_execute("ROLLBACK").await.expect("release");
        let response = server.get("/admin/tenants").await;
        response.assert_status_ok();
        let body: Value = response.json();
        let alpha = body["tenants"]
            .as_array()
            .expect("tenants")
            .iter()
            .find(|t| t["id"] == "alpha")
            .expect("alpha listed");
        assert_eq!(alpha["resources"], 3, "{body}");
    }

    /// At default settings (#1911 acceptance): the default count budget is
    /// below the default request timeout, so a count that never finishes
    /// answers `504` from the budget instead of `408` from the request
    /// timeout. The status is the proof; elapsed time is only printed and
    /// checked against the budget's floor, never against a ceiling. Takes
    /// about 25 s.
    #[tokio::test]
    async fn default_budget_answers_504_before_the_default_request_timeout() {
        let request_timeout = ServerConfig::default().request_timeout;
        let budget_ms = PostgresConfig::default().effective_count_by_tenant_statement_timeout_ms();
        assert!(
            budget_ms < request_timeout * 1_000,
            "default count budget {budget_ms} ms must be below the default request timeout \
             {request_timeout} s"
        );

        let dbname = isolated_database().await;
        let backend = PostgresBackend::new(pg_config(&dbname))
            .await
            .expect("backend");
        let server = server(backend, request_timeout);
        let side = PostgresBackend::new(pg_config(&dbname))
            .await
            .expect("side");
        let blocker = side.get_client().await.expect("blocker session");
        blocker
            .batch_execute(LOCK_RESOURCES)
            .await
            .expect("lock resources");

        let started = Instant::now();
        let response = server.get("/admin/tenants").await;
        let elapsed = started.elapsed();
        println!(
            "/admin/tenants at defaults (budget {budget_ms} ms, request timeout \
             {request_timeout} s) answered {} after {elapsed:?}",
            response.status_code()
        );
        // `504` rather than `408` shows the budget fired before the request
        // timeout; the floor shows it was the budget, not an earlier failure.
        assert_timeout_outcome(&response, "/admin/tenants at defaults");
        assert!(
            elapsed >= Duration::from_millis(budget_ms - 100),
            "{elapsed:?}"
        );
        blocker.batch_execute("ROLLBACK").await.expect("release");
    }

    /// The race this issue fixes, kept as a control: with the budget at or
    /// above the request timeout, the request timeout answers `408` first and
    /// drops the handler. The count's session is still owned (#1826): it is
    /// not handed out while the count waits, and the budget then ends the
    /// count in PostgreSQL and returns the same session idle, with its
    /// general timeout, while the lock is still held.
    #[tokio::test]
    async fn budget_above_request_timeout_answers_408_and_the_session_stays_owned() {
        let dbname = isolated_database().await;
        let backend = PostgresBackend::new(PostgresConfig {
            max_connections: 1,
            count_by_tenant_statement_timeout_ms: 3_000,
            ..pg_config(&dbname)
        })
        .await
        .expect("backend");
        let server = server(backend.clone(), 1);
        let side = PostgresBackend::new(pg_config(&dbname))
            .await
            .expect("side");
        let blocker = side.get_client().await.expect("blocker session");
        blocker
            .batch_execute(LOCK_RESOURCES)
            .await
            .expect("lock resources");
        let blocker_pid: i32 = blocker
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("blocker pid")
            .get(0);
        let observer = side.get_client().await.expect("observer session");

        let started = Instant::now();
        let response = server.get("/admin/tenants").await;
        println!(
            "/admin/tenants with budget 3000 ms over a 1 s request timeout answered {} after {:?}",
            response.status_code(),
            started.elapsed()
        );
        response.assert_status(StatusCode::REQUEST_TIMEOUT);

        let count_pid: i32 = observer
            .query_one(
                "SELECT pid FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)) AND starts_with(query, $2)",
                &[&blocker_pid, &COUNT_SQL_PREFIX],
            )
            .await
            .expect("the abandoned count is still waiting in PostgreSQL")
            .get(0);
        assert!(
            timeout(Duration::from_millis(500), backend.get_client())
                .await
                .is_err(),
            "busy count session handed out before its budget expired"
        );
        let client = timeout(Duration::from_secs(10), backend.get_client())
            .await
            .expect("the budget settles the abandoned count")
            .expect("checkout");
        let pid: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get(0);
        assert_eq!(pid, count_pid);
        let statement_timeout: String = client
            .query_one("SHOW statement_timeout", &[])
            .await
            .expect("SHOW")
            .get(0);
        assert_eq!(statement_timeout, "30s", "the count budget did not leak");
        let blocker_state: String = observer
            .query_one(
                "SELECT state FROM pg_stat_activity WHERE pid = $1",
                &[&blocker_pid],
            )
            .await
            .expect("blocker state")
            .get(0);
        assert_eq!(blocker_state, "idle in transaction", "lock still held");
        drop(client);
        blocker.batch_execute("ROLLBACK").await.expect("release");
    }
}
