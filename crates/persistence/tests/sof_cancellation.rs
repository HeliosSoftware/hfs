//! Cancellation of SQL-on-FHIR runs (#1864): when a run is abandoned, its
//! database work stops within [`STOP_BOUND`](sof_cancellation_cases::STOP_BOUND).
//!
//! One module per runner column of the shared table in
//! `common/sof_cancellation_cases.rs`. Each cell starts a run whose backend
//! work is busy before the first row, stops it, and observes the backend:
//!
//! - `sqlite`: the producer holds the only pooled connection while it steps;
//! - `postgres`: `pg_stat_activity`, found by a column-alias marker;
//! - `mongodb`: `$currentOp` (commands and open cursors), found by the same
//!   marker; the initial-command cells also run as a read-only user of an
//!   `--auth` server (plan D-8);
//! - `in_process`: the resources pulled from a scan double, and the run's end,
//!   which comes only after the evaluation returned.
//!
//! Each cell first records how long the abandoned work really ran (printed as
//! `SOF-CANCEL` lines), then asserts the bound, so a failure shows the actual
//! duration.
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres,mongodb --test sof_cancellation
//!
//! The PostgreSQL and MongoDB modules need Docker (testcontainers). Set
//! `HFS_TEST_REQUIRE_MONGODB=1` to make a missing MongoDB container a failure.

#[path = "common/sof_cancellation_cases.rs"]
mod sof_cancellation_cases;

#[cfg(any(feature = "postgres", feature = "mongodb"))]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// The run-capabilities truth table (A2): each runner module's
/// `run_capabilities_truth_table` runs the four view shapes and checks the
/// capabilities the runner declares, and that the declared schema is the
/// document-order walker's. #1867 flips the SQL `unionAll`/`repeat` cells and
/// #1868 the MongoDB cells, in those modules.
#[allow(dead_code)]
mod truth_table {
    use futures::StreamExt;
    use helios_persistence::core::sof_runner::{
        RunCapabilities, RunContext, RunKind, RunRequest, SofRunner, ViewFilters,
    };
    use helios_persistence::core::{ViewColumn, ViewSchema};
    use helios_persistence::tenant::TenantContext;
    use serde_json::{Value, json};

    /// The view shapes, in table column order.
    pub const SHAPES: [&str; 4] = ["flat", "forEach", "unionAll", "repeat"];

    pub fn view(shape: &str) -> Value {
        let (resource, select) = match shape {
            "flat" => (
                "Patient",
                json!([{"column": [
                    {"name": "id", "path": "getResourceKey()"},
                    {"name": "gender", "path": "gender", "type": "code"}
                ]}]),
            ),
            "forEach" => (
                "Patient",
                json!([
                    {"column": [{"name": "id", "path": "getResourceKey()"}]},
                    {"forEach": "name", "column": [{"name": "family", "path": "family"}]}
                ]),
            ),
            "unionAll" => (
                "Patient",
                json!([{"unionAll": [
                    {"column": [{"name": "value", "path": "id"}]},
                    {"column": [{"name": "value", "path": "gender"}]}
                ]}]),
            ),
            "repeat" => (
                "QuestionnaireResponse",
                json!([{"repeat": ["item"], "column": [
                    {"name": "link", "path": "linkId", "type": "string"}
                ]}]),
            ),
            other => panic!("unknown shape {other}"),
        };
        json!({
            "resourceType": "ViewDefinition",
            "resource": resource,
            "status": "active",
            "select": select,
        })
    }

    /// A runner's declared capabilities for a run with `total_order`.
    pub fn declared(total_order: bool) -> RunCapabilities {
        RunCapabilities {
            total_order,
            declared_schema: true,
            cancels_server_side: false,
        }
    }

    /// Names, declared types and collection flags; the decode may be refined
    /// by a SQL compiler.
    fn shape_of(schema: &ViewSchema) -> Vec<(String, Option<String>, bool)> {
        schema
            .columns
            .iter()
            .map(
                |ViewColumn {
                     name,
                     declared_type,
                     collection,
                     ..
                 }| { (name.clone(), declared_type.clone(), *collection) },
            )
            .collect()
    }

    /// Runs every shape on `runner` and checks one table row:
    /// `expected[i]` is the capability cell for `SHAPES[i]`.
    pub async fn assert_row(
        label: &str,
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        expected: [RunCapabilities; 4],
    ) {
        for (shape, expected) in SHAPES.into_iter().zip(expected) {
            let view = view(shape);
            let output = runner
                .run_view(
                    tenant,
                    RunRequest::new(view.clone(), ViewFilters::default()),
                    RunContext::detached(RunKind::Preview),
                )
                .await
                .unwrap_or_else(|e| panic!("{label} {shape}: {e}"));
            assert_eq!(output.capabilities, expected, "{label} {shape}");
            assert_eq!(
                shape_of(&output.schema),
                shape_of(&ViewSchema::from_view_definition(&view)),
                "{label} {shape}: the declared schema is the walker's"
            );
            let mut rows = output.rows;
            while let Some(row) = rows.next().await {
                row.unwrap_or_else(|e| panic!("{label} {shape}: {e}"));
            }
        }
    }
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::{
        RowStream, RunCapabilities, RunContext, RunKind, RunRequest, SofRunner, ViewFilters,
    };
    use helios_persistence::sof::sqlite::SqliteInDbRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use r2d2_sqlite::SqliteConnectionManager;

    use super::sof_cancellation_cases::{
        CancelCase, Runner, STOP_BOUND, assert_cancelled, assert_timed_out, batches_as_items,
        cartesian_patient, cartesian_view, drain, marker, not_applicable, observe_after_stop,
    };

    /// Entries per array of the cartesian Patient: `N * N` rows to sort
    /// before the first row (several seconds in a debug build).
    const N: usize = 1400;

    /// A file database (the producer and the test need the same database
    /// through different pools), removed on drop.
    struct TempDb(std::path::PathBuf);

    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.clone().into_os_string();
                p.push(suffix);
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// One cartesian Patient of `n` × `n` rows, and a runner over a pool of
    /// one connection: while the producer steps, it holds that connection.
    struct Fixture {
        _db: TempDb,
        tenant: TenantContext,
        pool: r2d2::Pool<SqliteConnectionManager>,
        runner: Arc<SqliteInDbRunner>,
    }

    async fn fixture(n: usize) -> Fixture {
        let db = TempDb(std::env::temp_dir().join(format!(
            "sof_cancellation_{}.db",
            uuid::Uuid::new_v4().simple()
        )));
        let backend =
            SqliteBackend::with_config(&db.0, Default::default()).expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        backend
            .create(
                &tenant,
                "Patient",
                cartesian_patient("big", n),
                FhirVersion::R4,
            )
            .await
            .expect("seed cartesian patient");
        let manager = SqliteConnectionManager::file(&db.0)
            .with_init(|c| helios_persistence::sof::sqlite_udfs::register(c));
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .min_idle(Some(1))
            .connection_timeout(Duration::from_secs(120))
            .build(manager)
            .expect("pool of one");
        let runner = Arc::new(SqliteInDbRunner::new(pool.clone()));
        Fixture {
            _db: db,
            tenant,
            pool,
            runner,
        }
    }

    impl Fixture {
        async fn run(
            &self,
            view: serde_json::Value,
            filters: ViewFilters,
            ctx: RunContext,
        ) -> RowStream {
            self.runner
                .run_view(&self.tenant, RunRequest::new(view, filters), ctx)
                .await
                .map(|output| output.rows)
                .expect("run_view")
        }

        /// The columnar run of `view` (Parquet / Arrow output), its batches
        /// as [`batches_as_items`] items.
        async fn run_columnar(&self, view: serde_json::Value, ctx: RunContext) -> RowStream {
            let batches = self
                .runner
                .run_view_columnar(
                    &self.tenant,
                    RunRequest::new(view, ViewFilters::default()),
                    ctx,
                )
                .await
                .expect("run_view_columnar")
                .expect("the view has a columnar path");
            batches_as_items(batches)
        }

        /// Waits until the producer took the pool's only connection.
        async fn wait_until_busy(&self) {
            let deadline = Instant::now() + Duration::from_secs(30);
            while self.pool.state().idle_connections != 0 {
                assert!(
                    Instant::now() < deadline,
                    "the producer never took the connection"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        /// How long the producer keeps the connection after `stopped_at`;
        /// asserts it gave it back within the bound.
        async fn assert_released(&self, cell: &str, started: Instant, stopped_at: Instant) {
            let pool = self.pool.clone();
            let (running_at_bound, ran_for) = observe_after_stop(stopped_at, || {
                let busy = pool.state().idle_connections == 0;
                async move { busy }
            })
            .await;
            eprintln!(
                "SOF-CANCEL sqlite {cell}: stopped {:?} after the run started; producer kept the \
                 connection {ran_for:?} after the stop (still stepping at the bound: \
                 {running_at_bound})",
                stopped_at - started
            );
            assert!(
                !running_at_bound,
                "{cell}: the SQLite producer was still stepping {STOP_BOUND:?} after the stop; it \
                 ran {ran_for:?} after it"
            );
        }
    }

    /// B5: the consumer drops the stream while SQLite is still sorting, before
    /// the first row. The producer must stop stepping and give its pooled
    /// connection back within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_stream_before_the_first_row_stops_the_producer() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::CancelBeforeFirstRow).is_none());
        let fx = fixture(N).await;
        let alias = marker("sofcancel_sqlite");
        let started = Instant::now();
        let stream = fx
            .run(
                cartesian_view(&alias),
                ViewFilters::default(),
                RunContext::detached(RunKind::Preview),
            )
            .await;
        fx.wait_until_busy().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(stream);
        let stopped_at = Instant::now();
        fx.assert_released("cancel_before_first_row (drop)", started, stopped_at)
            .await;
    }

    /// `cancel_before_first_row`, by the cancel token: the run ends with a
    /// `Cancelled` item and the producer stops stepping within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_before_the_first_row_ends_cancelled() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::CancelBeforeFirstRow).is_none());
        let fx = fixture(N).await;
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let started = Instant::now();
        let stream = fx
            .run(
                cartesian_view(&marker("sofcancel_sqlite")),
                ViewFilters::default(),
                ctx,
            )
            .await;
        fx.wait_until_busy().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0);
        assert_cancelled("sqlite cancel_before_first_row", &drained);
        fx.assert_released("cancel_before_first_row (token)", started, stopped_at)
            .await;
    }

    /// `cancel_between_chunks`: the consumer read one batch and stopped
    /// reading, so the producer waits to deliver the next; a cancel ends it
    /// there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_between_batches_ends_cancelled() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::CancelBetweenChunks).is_none());
        let fx = fixture(300).await;
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let started = Instant::now();
        let mut stream = fx
            .run(
                cartesian_view(&marker("sofcancel_sqlite")),
                ViewFilters::default(),
                ctx,
            )
            .await;
        for _ in 0..256 {
            stream.next().await.expect("a row").expect("a row");
        }
        // The producer fills the channel and waits.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            fx.pool.state().idle_connections,
            0,
            "the producer is waiting"
        );

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert!(drained.rows < 300 * 300, "the run stopped early");
        assert_cancelled("sqlite cancel_between_chunks", &drained);
        fx.assert_released("cancel_between_chunks", started, stopped_at)
            .await;
    }

    /// `cancel_mid_stream`: the consumer is reading rows as they come when
    /// the cancel arrives.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_mid_stream_ends_cancelled() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::CancelMidStream).is_none());
        let fx = fixture(600).await;
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let started = Instant::now();
        let mut stream = fx
            .run(
                cartesian_view(&marker("sofcancel_sqlite")),
                ViewFilters::default(),
                ctx,
            )
            .await;
        let mut read = 0usize;
        while read < 10_000 {
            stream.next().await.expect("a row").expect("a row");
            read += 1;
        }
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert!(read + drained.rows < 600 * 600, "the run stopped early");
        assert_cancelled("sqlite cancel_mid_stream", &drained);
        fx.assert_released("cancel_mid_stream", started, stopped_at)
            .await;
    }

    /// `stop_at_cap`: a capped run ends cleanly at its cap and frees the
    /// connection at once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_capped_run_stops_at_its_cap() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::StopAtCap).is_none());
        let fx = fixture(300).await;
        let started = Instant::now();
        let stream = fx
            .run(
                cartesian_view(&marker("sofcancel_sqlite")),
                ViewFilters {
                    limit: Some(10),
                    ..Default::default()
                },
                RunContext::detached(RunKind::Preview),
            )
            .await;
        let drained = drain(stream, Duration::from_secs(60)).await;
        let stopped_at = Instant::now();
        assert!(drained.ended);
        assert_eq!(drained.rows, 10);
        assert!(drained.error.is_none(), "{:?}", drained.error);
        fx.assert_released("stop_at_cap", started, stopped_at).await;
    }

    /// `deadline_ends_as_timeout`: the progress handler sees the deadline
    /// while SQLite sorts; the run ends with a `Timeout` carrying its budget.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_deadline_ends_the_run_as_a_timeout() {
        assert!(not_applicable(Runner::Sqlite, CancelCase::DeadlineEndsAsTimeout).is_none());
        let fx = fixture(N).await;
        let budget = Duration::from_millis(700);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = fx
            .run(
                cartesian_view(&marker("sofcancel_sqlite")),
                ViewFilters::default(),
                ctx,
            )
            .await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        let ended_at = Instant::now();
        assert_timed_out("sqlite deadline_ends_as_timeout", &drained, budget);
        assert!(
            ended_at - started <= budget + STOP_BOUND,
            "the timeout came {:?} after the start",
            ended_at - started
        );
        fx.assert_released("deadline_ends_as_timeout", started, started + budget)
            .await;
    }

    /// The columnar path (Parquet / Arrow `$sql-run`) is the same run with
    /// another sink: a cancel before the first batch ends it `Cancelled` and
    /// frees the connection within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_columnar_cancel_before_the_first_batch_ends_cancelled() {
        let fx = fixture(N).await;
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let started = Instant::now();
        let stream = fx
            .run_columnar(cartesian_view(&marker("sofcancel_sqlite_col")), ctx)
            .await;
        fx.wait_until_busy().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0, "no batch before the cancel");
        assert_cancelled("sqlite columnar cancel_before_first_row", &drained);
        fx.assert_released("columnar cancel_before_first_row", started, stopped_at)
            .await;
    }

    /// The columnar path's deadline ends the run as a `Timeout` with its
    /// budget, as on the row path.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_columnar_run_ends_as_a_timeout_at_its_deadline() {
        let fx = fixture(N).await;
        let budget = Duration::from_millis(700);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = fx
            .run_columnar(cartesian_view(&marker("sofcancel_sqlite_col")), ctx)
            .await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        assert_timed_out("sqlite columnar deadline_ends_as_timeout", &drained, budget);
        fx.assert_released(
            "columnar deadline_ends_as_timeout",
            started,
            started + budget,
        )
        .await;
    }

    /// SQLite row of the truth table: flat and `forEach` statements have a
    /// total order, `unionAll` and `repeat` do not; a stop interrupts the
    /// statement itself.
    #[tokio::test]
    async fn run_capabilities_truth_table() {
        use super::truth_table::{assert_row, declared};

        let backend = SqliteBackend::in_memory().expect("in-memory SQLite backend");
        backend.init_schema().expect("init schema");
        let runner = backend.sof_runner().expect("SQLite SoF runner");
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let stops = |total_order| RunCapabilities {
            cancels_server_side: true,
            ..declared(total_order)
        };
        let row = [stops(true), stops(true), stops(false), stops(false)];
        assert_row("sqlite", runner.as_ref(), &tenant, row).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use deadpool_postgres::Pool;
    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::{
        RowStream, RunCapabilities, RunContext, RunKind, RunRequest, SofError, SofRunner,
        ViewFilters,
    };
    use helios_persistence::sof::postgres::PgInDbRunner;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    use super::sof_cancellation_cases::{
        CancelCase, Runner, STOP_BOUND, assert_cancelled, assert_timed_out, batches_as_items,
        cartesian_patient, cartesian_view, drain, marker, not_applicable, observe_after_stop,
    };

    /// Entries per array of the cartesian Patient: `N * N` rows to sort.
    const N: usize = 2000;

    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let container = super::container_cleanup::with_cleanup_label(
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
                let backend = PostgresBackend::new(config(&host, port))
                    .await
                    .expect("create backend");
                backend.init_schema().await.expect("initialize schema");
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    fn config(host: &str, port: u16) -> PostgresConfig {
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

    /// A pool of one connection to the shared server, with `wait` as its
    /// checkout wait timeout (deadpool waits forever without one) and `app`
    /// as its `application_name`, so the observer can find its session.
    fn pool_of_one(host: &str, port: u16, wait: Option<Duration>, app: &str) -> Pool {
        let mut pool_config = deadpool_postgres::Config::new();
        pool_config.application_name = Some(app.to_string());
        pool_config.host = Some(host.to_string());
        pool_config.port = Some(port);
        pool_config.user = Some("postgres".into());
        pool_config.password = Some("postgres".into());
        pool_config.dbname = Some("postgres".into());
        let mut pool = deadpool_postgres::PoolConfig::new(1);
        pool.timeouts.wait = wait;
        pool_config.pool = Some(pool);
        pool_config
            .create_pool(
                Some(deadpool_postgres::Runtime::Tokio1),
                tokio_postgres::NoTls,
            )
            .expect("create runner pool")
    }

    /// A tenant holding the given Patients, a runner pool of one
    /// connection, and an observer connection.
    struct Fixture {
        tenant: TenantContext,
        pool: Pool,
        /// The runner pool's `application_name`.
        app: String,
        observer: tokio_postgres::Client,
        _connection: tokio::task::JoinHandle<()>,
    }

    async fn fixture_with(patients: Vec<Value>) -> Fixture {
        let pg = shared_pg().await;
        let backend = PostgresBackend::new(config(&pg.host, pg.port))
            .await
            .expect("create backend");
        let tenant = TenantContext::new(
            TenantId::new(format!("sofcancel_{}", uuid::Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        );
        for patient in patients {
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed patient");
        }
        let app = marker("sofcancel_app");
        let pool = pool_of_one(&pg.host, pg.port, None, &app);
        let mut config = tokio_postgres::Config::new();
        config
            .host(&pg.host)
            .port(pg.port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (observer, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let handle = tokio::spawn(async move {
            let _ = connection.await;
        });
        Fixture {
            tenant,
            pool,
            app,
            observer,
            _connection: handle,
        }
    }

    async fn fixture() -> Fixture {
        fixture_with(vec![cartesian_patient("big", N)]).await
    }

    impl Fixture {
        async fn run(
            &self,
            runner: &dyn SofRunner,
            view: Value,
            filters: ViewFilters,
            ctx: RunContext,
        ) -> RowStream {
            runner
                .run_view(&self.tenant, RunRequest::new(view, filters), ctx)
                .await
                .map(|output| output.rows)
                .expect("run_view")
        }

        /// The columnar run of `view` (Parquet / Arrow output), its batches
        /// as [`batches_as_items`] items.
        async fn run_columnar(
            &self,
            runner: &dyn SofRunner,
            view: Value,
            ctx: RunContext,
        ) -> RowStream {
            let batches = runner
                .run_view_columnar(
                    &self.tenant,
                    RunRequest::new(view, ViewFilters::default()),
                    ctx,
                )
                .await
                .expect("run_view_columnar")
                .expect("the view has a columnar path");
            batches_as_items(batches)
        }

        /// The backend pid that runs this run's statement (found by the
        /// alias marker), once it is executing.
        async fn active_pid(&self, alias: &str) -> i32 {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let row = self
                    .observer
                    .query_opt(
                        "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
                         AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE $1 \
                         LIMIT 1",
                        &[&format!("%\"{alias}\"%")],
                    )
                    .await
                    .expect("sample pg_stat_activity");
                if let Some(row) = row {
                    return row.get(0);
                }
                assert!(
                    Instant::now() < deadline,
                    "the run's statement never became active"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        /// The backend pid of the runner pool's session.
        async fn session_pid(&self) -> i32 {
            self.observer
                .query_one(
                    "SELECT pid FROM pg_stat_activity WHERE application_name = $1",
                    &[&self.app],
                )
                .await
                .expect("the runner pool's session")
                .get(0)
        }

        /// Asserts that no statement of this run is active within the bound
        /// after `stopped_at`, and that a fresh borrower on the pool of one
        /// runs `SELECT 1` within the bound. Returns the borrower's backend
        /// pid (equal to the run's when its connection was reused).
        async fn assert_stopped(&self, cell: &str, alias: &str, stopped_at: Instant) -> i32 {
            let borrower = time_next_borrower(self.pool.clone(), stopped_at);
            let (running_at_bound, ran_for) = observe_after_stop(stopped_at, || async {
                active_statements(&self.observer, alias).await > 0
            })
            .await;
            let (borrower_waited, pid) = borrower.await.expect("borrower task");
            eprintln!(
                "SOF-CANCEL postgres {cell}: the statement stayed active {ran_for:?} after the \
                 stop (active at the bound: {running_at_bound}); a fresh borrower on the pool of \
                 one waited {borrower_waited:?}"
            );
            assert!(
                !running_at_bound,
                "{cell}: the stopped statement was still active {STOP_BOUND:?} after the stop; \
                 it ran {ran_for:?} after it"
            );
            assert!(
                borrower_waited <= STOP_BOUND,
                "{cell}: the next borrower of the pool of one waited {borrower_waited:?} for its \
                 SELECT 1"
            );
            pid
        }
    }

    /// Statements of this run (found by the alias marker) that are executing.
    async fn active_statements(observer: &tokio_postgres::Client, alias: &str) -> i64 {
        observer
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE $1",
                &[&format!("%\"{alias}\"%")],
            )
            .await
            .expect("sample pg_stat_activity")
            .get(0)
    }

    /// Times a fresh borrower on the runner's pool of one: checkout plus
    /// `SELECT 1`, from `since` (the stop). Returns the wait and the
    /// borrower's backend pid.
    fn time_next_borrower(pool: Pool, since: Instant) -> tokio::task::JoinHandle<(Duration, i32)> {
        tokio::spawn(async move {
            let probe = async {
                let client = pool.get().await.expect("check out");
                client
                    .query_one("SELECT pg_backend_pid()", &[])
                    .await
                    .expect("SELECT 1")
                    .get::<_, i32>(0)
            };
            let pid = tokio::time::timeout(Duration::from_secs(120), probe)
                .await
                .unwrap_or(-1);
            (since.elapsed(), pid)
        })
    }

    async fn drop_while_active_and_observe(
        fx: &Fixture,
        runner: Arc<PgInDbRunner>,
        alias: &str,
        cell: &str,
    ) {
        let started = Instant::now();
        let stream = fx
            .run(
                runner.as_ref(),
                cartesian_view(alias),
                ViewFilters::default(),
                RunContext::detached(RunKind::Preview),
            )
            .await;
        let pid = fx.active_pid(alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(stream);
        let stopped_at = Instant::now();
        eprintln!(
            "SOF-CANCEL postgres {cell}: dropped {:?} after the run started",
            stopped_at - started
        );
        let borrower_pid = fx.assert_stopped(cell, alias, stopped_at).await;
        assert_eq!(
            borrower_pid, pid,
            "{cell}: a cancel confirmed by the statement lets the connection be reused"
        );
    }

    /// B3: an unpaged run (preview, `$sql-run`) dropped while PostgreSQL still
    /// sorts, before the first row. The statement must end within the bound
    /// and the pool's only connection must serve the next borrower.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_an_unpaged_run_leaves_no_active_statement() {
        assert!(not_applicable(Runner::Postgres, CancelCase::CancelBeforeFirstRow).is_none());
        let fx = fixture().await;
        let runner = Arc::new(PgInDbRunner::new(fx.pool.clone()));
        let alias = marker("sofcancel_pg_unpaged");
        drop_while_active_and_observe(&fx, runner, &alias, "unpaged cancel_before_first_row").await;
    }

    /// B4: a paged export run (`for_export_job` page size) stops below the
    /// page, not at its end. Two sub-cases here: dropped while its page query
    /// runs (the statement is cancelled and drained, the connection reused),
    /// and dropped while its page is delivered to a consumer that stopped
    /// reading (nothing runs; the connection is reused at once). The third,
    /// during the bound lookup, needs a bound statement slow enough to stop
    /// it in, which the public API cannot issue: it is
    /// `sof::postgres::docker_tests::paged_export_stops_during_the_bound_lookup`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paged_export_stops_below_the_page() {
        assert!(not_applicable(Runner::Postgres, CancelCase::CancelBetweenChunks).is_none());
        // During the page query.
        let fx = fixture().await;
        let runner = Arc::new(PgInDbRunner::new(fx.pool.clone()).with_export_page_size(10_000));
        let alias = marker("sofcancel_pg_paged");
        drop_while_active_and_observe(&fx, runner, &alias, "paged export, during the page query")
            .await;

        // During delivery: one page of 120 × 120 rows, far more than the
        // channel holds, so the producer waits to deliver it.
        let fx = fixture_with(vec![cartesian_patient("small", 120)]).await;
        let runner = Arc::new(PgInDbRunner::new(fx.pool.clone()).with_export_page_size(10_000));
        let alias = marker("sofcancel_pg_delivery");
        let mut stream = fx
            .run(
                runner.as_ref(),
                cartesian_view(&alias),
                ViewFilters::default(),
                RunContext::detached(RunKind::ExportJob),
            )
            .await;
        stream.next().await.expect("a row").expect("a row");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let run_pid = fx.session_pid().await;
        assert_eq!(
            active_statements(&fx.observer, &alias).await,
            0,
            "the page statement ended; its rows are being delivered"
        );
        drop(stream);
        let stopped_at = Instant::now();
        let borrower_pid = fx
            .assert_stopped("paged export, during delivery", &alias, stopped_at)
            .await;
        assert_eq!(
            borrower_pid, run_pid,
            "a stop during delivery has nothing to cancel: the connection is reused"
        );
    }

    /// `cancel_before_first_row`, by the cancel token: the run ends with a
    /// `Cancelled` item, the statement within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_before_the_first_row_ends_cancelled() {
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_token");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let stream = fx
            .run(&runner, cartesian_view(&alias), ViewFilters::default(), ctx)
            .await;
        let pid = fx.active_pid(&alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0);
        assert_cancelled("postgres cancel_before_first_row", &drained);
        let borrower_pid = fx
            .assert_stopped("cancel_before_first_row (token)", &alias, stopped_at)
            .await;
        assert_eq!(borrower_pid, pid);
    }

    /// `cancel_between_chunks`: an export reads one Patient per page; the
    /// consumer stops reading inside the first page and cancels. The run
    /// ends `Cancelled` before the next page's SQL, and the connection is
    /// reused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_between_pages_ends_cancelled() {
        assert!(not_applicable(Runner::Postgres, CancelCase::CancelBetweenChunks).is_none());
        let fx = fixture_with(
            (0..3)
                .map(|i| cartesian_patient(&format!("p{i}"), 60))
                .collect(),
        )
        .await;
        let runner = PgInDbRunner::new(fx.pool.clone()).with_export_page_size(1);
        let alias = marker("sofcancel_pg_pages");
        let ctx = RunContext::detached(RunKind::ExportJob);
        let cancel = ctx.cancel.clone();
        let mut stream = fx
            .run(&runner, cartesian_view(&alias), ViewFilters::default(), ctx)
            .await;
        for _ in 0..100 {
            stream.next().await.expect("a row").expect("a row");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert!(100 + drained.rows < 3 * 60 * 60, "the export stopped early");
        assert_cancelled("postgres cancel_between_chunks", &drained);
        fx.assert_stopped("cancel_between_chunks", &alias, stopped_at)
            .await;
    }

    /// `cancel_mid_stream`: an unpaged run whose rows are streaming (the
    /// statement still sends them) is cancelled; the statement is cancelled
    /// and drained, the run ends `Cancelled`, the connection is reused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_mid_stream_ends_cancelled() {
        assert!(not_applicable(Runner::Postgres, CancelCase::CancelMidStream).is_none());
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_mid");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = fx
            .run(&runner, cartesian_view(&alias), ViewFilters::default(), ctx)
            .await;
        let pid = fx.active_pid(&alias).await;
        for _ in 0..1_000 {
            stream.next().await.expect("a row").expect("a row");
        }
        assert!(
            active_statements(&fx.observer, &alias).await > 0,
            "still streaming"
        );
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_cancelled("postgres cancel_mid_stream", &drained);
        let borrower_pid = fx
            .assert_stopped("cancel_mid_stream", &alias, stopped_at)
            .await;
        assert_eq!(borrower_pid, pid);
    }

    /// `stop_at_cap`: a capped flat view carries its cap in the SQL
    /// (`LIMIT`): the server sorts the top rows, ends the statement at the
    /// cap, and the run ends cleanly with exactly the cap; nothing stays
    /// active and the connection is reused. (A cap the client enforces, for
    /// `unionAll`/`repeat` SQL, stops a statement that is still sending rows:
    /// `sof::postgres::docker_tests::a_client_cap_stops_a_statement_that_is_still_sending_rows`.)
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_capped_run_stops_at_its_cap() {
        assert!(not_applicable(Runner::Postgres, CancelCase::StopAtCap).is_none());
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_cap");
        let ctx = RunContext::detached(RunKind::Preview);
        let filters = ViewFilters {
            limit: Some(10),
            ..Default::default()
        };
        let stream = fx.run(&runner, cartesian_view(&alias), filters, ctx).await;
        let pid = fx.active_pid(&alias).await;
        let drained = drain(stream, Duration::from_secs(120)).await;
        let ended_at = Instant::now();
        assert!(drained.ended);
        assert_eq!(drained.rows, 10);
        assert!(drained.error.is_none(), "{:?}", drained.error);
        let borrower_pid = fx.assert_stopped("stop_at_cap", &alias, ended_at).await;
        assert_eq!(borrower_pid, pid);
    }

    /// `deadline_ends_as_timeout`: the run's statement runs under `SET LOCAL
    /// statement_timeout` from its deadline; the server stops it at the
    /// deadline and the run ends `Timeout` with its budget. No cancel request
    /// is in flight, so the connection is reused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_deadline_ends_the_run_as_a_timeout() {
        assert!(not_applicable(Runner::Postgres, CancelCase::DeadlineEndsAsTimeout).is_none());
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_deadline");
        let budget = Duration::from_secs(1);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = fx
            .run(&runner, cartesian_view(&alias), ViewFilters::default(), ctx)
            .await;
        let pid = fx.active_pid(&alias).await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        assert_timed_out("postgres deadline_ends_as_timeout", &drained, budget);
        let borrower_pid = fx
            .assert_stopped("deadline_ends_as_timeout", &alias, started + budget)
            .await;
        assert_eq!(
            borrower_pid, pid,
            "a server-side timeout leaves no cancel in flight"
        );
    }

    /// The columnar path (Parquet / Arrow `$sql-run`) is the same run with
    /// another sink: a cancel before the first batch ends it `Cancelled`, its
    /// statement within the bound, and the connection is reused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_columnar_cancel_before_the_first_batch_ends_cancelled() {
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_col_token");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let stream = fx.run_columnar(&runner, cartesian_view(&alias), ctx).await;
        let pid = fx.active_pid(&alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0, "no batch before the cancel");
        assert_cancelled("postgres columnar cancel_before_first_row", &drained);
        let borrower_pid = fx
            .assert_stopped("columnar cancel_before_first_row", &alias, stopped_at)
            .await;
        assert_eq!(borrower_pid, pid);
    }

    /// The columnar path's statement runs under the same `statement_timeout`
    /// from its deadline: the run ends `Timeout` with its budget.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_columnar_run_ends_as_a_timeout_at_its_deadline() {
        let fx = fixture().await;
        let runner = PgInDbRunner::new(fx.pool.clone());
        let alias = marker("sofcancel_pg_col_deadline");
        let budget = Duration::from_secs(1);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = fx.run_columnar(&runner, cartesian_view(&alias), ctx).await;
        let pid = fx.active_pid(&alias).await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        assert_timed_out(
            "postgres columnar deadline_ends_as_timeout",
            &drained,
            budget,
        );
        let borrower_pid = fx
            .assert_stopped(
                "columnar deadline_ends_as_timeout",
                &alias,
                started + budget,
            )
            .await;
        assert_eq!(
            borrower_pid, pid,
            "a server-side timeout leaves no cancel in flight"
        );
    }

    /// A11: the run's checkout is the backend's: an exhausted pool answers
    /// `Unavailable` (503) once its wait timeout passes, and a connect that
    /// fails before reaching the server is retried.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn checkout_retries_transient_failures_and_answers_unavailable_when_exhausted() {
        let pg = shared_pg().await;
        let fx = fixture_with(vec![json!({"resourceType": "Patient", "id": "p1"})]).await;
        let view = json!({"resourceType": "ViewDefinition", "resource": "Patient",
            "status": "active", "select": [{"column": [{"name": "id", "path": "id"}]}]});

        // Exhausted: the pool's only connection is held.
        let pool = pool_of_one(
            &pg.host,
            pg.port,
            Some(Duration::from_millis(500)),
            "sofcancel",
        );
        let held = pool.get().await.expect("hold the only connection");
        let runner = PgInDbRunner::new(pool.clone());
        let drained = drain(
            fx.run(
                &runner,
                view.clone(),
                ViewFilters::default(),
                RunContext::detached(RunKind::Preview),
            )
            .await,
            Duration::from_secs(10),
        )
        .await;
        assert!(
            matches!(drained.error, Some(SofError::Unavailable { .. })),
            "an exhausted pool is unavailable, got {:?}",
            drained.error
        );
        drop(held);

        // Transient: the first two connects are reset by a proxy before the
        // server sees them; the third goes through.
        let proxy = ResettingProxy::start(&pg.host, pg.port, 2).await;
        let pool = pool_of_one(
            "127.0.0.1",
            proxy.port,
            Some(Duration::from_secs(5)),
            "sofcancel",
        );
        let runner = PgInDbRunner::new(pool);
        let drained = drain(
            fx.run(
                &runner,
                view,
                ViewFilters::default(),
                RunContext::detached(RunKind::Preview),
            )
            .await,
            Duration::from_secs(10),
        )
        .await;
        assert!(drained.error.is_none(), "{:?}", drained.error);
        assert_eq!(drained.rows, 1);
        assert_eq!(proxy.resets(), 2, "two failed connects were retried");
    }

    /// A TCP proxy that resets its first `resets` connections and forwards
    /// the rest to `host:port`.
    struct ResettingProxy {
        port: u16,
        reset: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl ResettingProxy {
        async fn start(host: &str, port: u16, resets: usize) -> Self {
            use std::sync::atomic::Ordering;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local = listener.local_addr().unwrap().port();
            let reset = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let target = format!("{host}:{port}");
            let counter = reset.clone();
            tokio::spawn(async move {
                while let Ok((mut client, _)) = listener.accept().await {
                    if counter.load(Ordering::SeqCst) < resets {
                        counter.fetch_add(1, Ordering::SeqCst);
                        // A reset, not a clean close: linger 0 never blocks.
                        #[allow(deprecated)]
                        let _ = client.set_linger(Some(Duration::ZERO));
                        drop(client);
                        continue;
                    }
                    let target = target.clone();
                    tokio::spawn(async move {
                        if let Ok(mut server) = tokio::net::TcpStream::connect(target).await {
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                        }
                    });
                }
            });
            Self { port: local, reset }
        }

        fn resets(&self) -> usize {
            self.reset.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// PostgreSQL row of the truth table: as SQLite, one statement or export
    /// pages alike; a stop cancels the statement on the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_capabilities_truth_table() {
        use super::truth_table::{assert_row, declared};

        let fx = fixture().await;
        let stops = |total_order| RunCapabilities {
            cancels_server_side: true,
            ..declared(total_order)
        };
        let row = [stops(true), stops(true), stops(false), stops(false)];
        let runner = PgInDbRunner::new(fx.pool.clone());
        assert_row("postgres", &runner, &fx.tenant, row).await;
        let paged = runner
            .for_export_job()
            .expect("PostgreSQL provides a paged export runner");
        assert_row("postgres export pages", paged.as_ref(), &fx.tenant, row).await;
    }
}

#[cfg(feature = "mongodb")]
mod mongodb {
    use std::time::{Duration, Instant};

    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::sof_runner::{
        RowStream, RunCapabilities, RunContext, RunKind, RunRequest, ViewFilters,
    };
    use helios_persistence::core::{Backend, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use mongodb::bson::{Bson, Document, doc};
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    use super::sof_cancellation_cases::{
        CancelCase, Runner, STOP_BOUND, assert_cancelled, assert_timed_out, drain, marker,
        not_applicable, observe_after_stop,
    };

    /// The backend's resource collection (`MongoBackend::RESOURCES_COLLECTION`
    /// is crate-private).
    const RESOURCES: &str = "resources";

    /// Patients in the seeded collection. The `$facet` of a `unionAll` view
    /// consumes all of them before it returns its first batch, so the initial
    /// `aggregate` command is busy for seconds.
    const DOCS: usize = 400_000;

    /// The read-only user of the `--auth` server (D-8).
    const READER: &str = "sof_reader";
    const READER_PASSWORD: &str = "sof-reader-pw";
    const ROOT: &str = "root";
    const ROOT_PASSWORD: &str = "root-pw";

    /// A server and its seeded database.
    struct SharedMongo {
        /// Connection string with full rights (seeding, `$currentOp` of all
        /// users).
        admin_uri: String,
        /// Connection string the runner uses: the admin one, or the read-only
        /// user on the `--auth` server.
        runner_uri: String,
        /// The seeded database ([`DOCS`] Patients of tenant `t1`).
        db: String,
        _container: testcontainers::ContainerAsync<Mongo>,
    }

    static PLAIN: OnceCell<Option<SharedMongo>> = OnceCell::const_new();
    static AUTH: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

    fn required() -> bool {
        std::env::var("HFS_TEST_REQUIRE_MONGODB").is_ok_and(|v| !v.is_empty())
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// Starts a server (with `--auth` and a root user when `auth`), or `None`
    /// when Docker has no MongoDB to give and the test does not require it.
    async fn start(auth: bool) -> Option<(testcontainers::ContainerAsync<Mongo>, String, u16)> {
        let image = Mongo::default().with_label(
            "github.run_id",
            std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
        );
        let request = if auth {
            super::container_cleanup::with_cleanup_label(
                image
                    .with_env_var("MONGO_INITDB_ROOT_USERNAME", ROOT)
                    .with_env_var("MONGO_INITDB_ROOT_PASSWORD", ROOT_PASSWORD),
            )
        } else {
            super::container_cleanup::with_cleanup_label(image)
        };
        let container = match request.start().await {
            Ok(c) => c,
            Err(e) if required() => {
                panic!("MongoDB container did not start ({e}); HFS_TEST_REQUIRE_MONGODB")
            }
            Err(e) => {
                eprintln!("skipping MongoDB cancellation cells: no container ({e})");
                return None;
            }
        };
        let host = container.get_host().await.unwrap().to_string();
        let port = container.get_host_port_ipv4(27017).await.unwrap();
        Some((container, host, port))
    }

    /// Waits until `uri` answers a `ping` (the `--auth` image restarts its
    /// server after creating the root user).
    async fn wait_until_up(uri: &str) -> mongodb::Client {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let client = mongodb::Client::with_uri_str(format!(
                "{uri}{}serverSelectionTimeoutMS=2000",
                if uri.contains('?') { "&" } else { "?" }
            ))
            .await
            .expect("client");
            match client.database("admin").run_command(doc! {"ping": 1}).await {
                Ok(_) => return mongodb::Client::with_uri_str(uri).await.expect("client"),
                Err(e) => {
                    assert!(Instant::now() < deadline, "MongoDB never came up: {e}");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }

    async fn shared(auth: bool) -> Option<&'static SharedMongo> {
        let cell = if auth { &AUTH } else { &PLAIN };
        cell.get_or_init(|| async move {
            let (container, host, port) = start(auth).await?;
            let admin_uri = if auth {
                format!("mongodb://{ROOT}:{ROOT_PASSWORD}@{host}:{port}/?authSource=admin")
            } else {
                format!("mongodb://{host}:{port}")
            };
            let admin = wait_until_up(&admin_uri).await;
            let db = format!("sofcancel_{}", uuid::Uuid::new_v4().simple());
            let seeded = Instant::now();
            seed(&admin, &admin_uri, &db).await;
            eprintln!(
                "SOF-CANCEL mongodb: seeded {DOCS} patients in {:?} (auth: {auth})",
                seeded.elapsed()
            );
            let runner_uri = if auth {
                admin
                    .database(&db)
                    .run_command(doc! {
                        "createUser": READER,
                        "pwd": READER_PASSWORD,
                        "roles": [{"role": "read", "db": &db}],
                    })
                    .await
                    .expect("create the read-only user");
                format!("mongodb://{READER}:{READER_PASSWORD}@{host}:{port}/?authSource={db}")
            } else {
                admin_uri.clone()
            };
            Some(SharedMongo {
                admin_uri,
                runner_uri,
                db,
                _container: container,
            })
        })
        .await
        .as_ref()
    }

    impl SharedMongo {
        /// A backend on the seeded database through the runner's connection
        /// string, with a driver pool of `max_connections`.
        fn backend(&self, max_connections: Option<u32>) -> MongoBackend {
            let mut config = MongoBackendConfig {
                connection_string: self.runner_uri.clone(),
                database_name: self.db.clone(),
                ..Default::default()
            };
            if let Some(n) = max_connections {
                config.max_connections = n;
            }
            MongoBackend::new(config).expect("create MongoBackend")
        }

        async fn admin(&self) -> mongodb::Client {
            mongodb::Client::with_uri_str(&self.admin_uri)
                .await
                .expect("admin client")
        }

        async fn run(&self, view: Value, filters: ViewFilters, ctx: RunContext) -> RowStream {
            let runner = self.backend(None).sof_runner().expect("MongoDB SoF runner");
            runner
                .run_view(&tenant(), RunRequest::new(view, filters), ctx)
                .await
                .map(|output| output.rows)
                .expect("run_view")
        }
    }

    /// The aggregate commands of a run that the server is executing, found by
    /// the marker in their pipeline.
    async fn running_aggregates(admin: &mongodb::Client, alias: &str) -> usize {
        current_ops(admin, false)
            .await
            .iter()
            .filter(|op| {
                op.get_document("command")
                    .is_ok_and(|c| c.contains_key("aggregate") && format!("{c}").contains(alias))
            })
            .count()
    }

    /// Everything the server still holds for a run, found by its marker: an
    /// executing command (`aggregate`, `getMore`) or an open cursor.
    async fn server_work(admin: &mongodb::Client, alias: &str) -> usize {
        current_ops(admin, true)
            .await
            .iter()
            .filter(|op| format!("{op}").contains(alias))
            .count()
    }

    /// `$currentOp` of all users, with idle cursors when `idle_cursors`.
    async fn current_ops(admin: &mongodb::Client, idle_cursors: bool) -> Vec<Document> {
        let mut cursor = admin
            .database("admin")
            .aggregate(vec![doc! {"$currentOp": {
                "allUsers": true,
                "idleConnections": false,
                "idleCursors": idle_cursors,
            }}])
            .await
            .expect("$currentOp");
        let mut ops = Vec::new();
        while cursor.advance().await.expect("$currentOp cursor") {
            ops.push(cursor.deserialize_current().expect("op"));
        }
        ops
    }

    /// Waits until the run's initial `aggregate` shows in `$currentOp`.
    async fn wait_until_running(admin: &mongodb::Client, alias: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while running_aggregates(admin, alias).await == 0 {
            assert!(
                Instant::now() < deadline,
                "the aggregate never showed in $currentOp"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Fills tenant `t1` of `db` with [`DOCS`] Patients: one written through
    /// the backend, the rest cloned from its stored document (new ids), so
    /// the stored shape is exactly what the backend writes.
    async fn seed(admin: &mongodb::Client, admin_uri: &str, db: &str) {
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: admin_uri.to_string(),
            database_name: db.to_string(),
            ..Default::default()
        })
        .expect("create MongoBackend");
        backend
            .initialize()
            .await
            .expect("initialize MongoDB schema");
        let tenant = tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p0", "gender": "female"}),
                FhirVersion::R4,
            )
            .await
            .expect("seed template patient");
        let resources = admin.database(db).collection::<Document>(RESOURCES);
        let template = resources
            .find_one(doc! {"tenant_id": tenant.tenant_id().as_str(), "id": "p0"})
            .await
            .expect("read template")
            .expect("template stored");
        let mut batch = Vec::with_capacity(10_000);
        for i in 1..DOCS {
            let mut d = template.clone();
            d.remove("_id");
            let id = format!("p{i}");
            d.insert("id", id.clone());
            if let Ok(data) = d.get_document_mut("data") {
                data.insert("id", Bson::String(id));
            }
            batch.push(d);
            if batch.len() == 10_000 {
                resources
                    .insert_many(std::mem::take(&mut batch))
                    .ordered(false)
                    .await
                    .expect("insert batch");
            }
        }
        if !batch.is_empty() {
            resources
                .insert_many(batch)
                .ordered(false)
                .await
                .expect("insert batch");
        }
    }

    /// Two branches, no row survives the `where`: the `$facet` has to read
    /// every Patient of the tenant and returns nothing, so the initial
    /// `aggregate` command is busy for the whole scan.
    fn union_view(alias: &str) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{"path": "gender = 'no-such-gender'"}],
            "select": [{"unionAll": [
                {"column": [{"name": alias, "path": "id"}]},
                {"column": [{"name": alias, "path": "gender"}]}
            ]}]
        })
    }

    /// One row per Patient, streamed through the cursor.
    fn flat_view(alias: &str) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [
                {"name": alias, "path": "id"},
                {"name": "gender", "path": "gender"}
            ]}]
        })
    }

    /// Asserts that the run's server work (found by `alias`, through `probe`)
    /// ended within the bound after `stopped_at`, and prints how long it ran.
    async fn assert_ended<F, Fut>(cell: &str, stopped_at: Instant, probe: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let (running_at_bound, ran_for) = observe_after_stop(stopped_at, probe).await;
        eprintln!(
            "SOF-CANCEL mongodb {cell}: the server work stayed in $currentOp {ran_for:?} after \
             the stop (running at the bound: {running_at_bound})"
        );
        assert!(
            !running_at_bound,
            "{cell}: the server work was still in $currentOp {STOP_BOUND:?} after the stop; it \
             ran {ran_for:?} after it"
        );
    }

    /// Lets the work a stop leaves behind finish before the test's runtime
    /// shuts down: the runner's cleanup task polls `$currentOp` for up to
    /// 500 ms, and the driver kills a dropped cursor from a task of its own.
    /// A driver task cut off by the runtime's shutdown panics on its closed
    /// pool, which says nothing about the run.
    async fn let_cleanup_finish() {
        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    /// B6 (`cancel_before_first_row`, drop): a `$facet` (`unionAll`) view
    /// dropped while the initial `aggregate` is still running. The aggregate
    /// must leave `$currentOp` within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_stream_ends_the_aggregate() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::CancelBeforeFirstRow).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let started = Instant::now();
        let stream = mongo
            .run(
                union_view(&alias),
                ViewFilters::default(),
                RunContext::detached(RunKind::Preview),
            )
            .await;
        wait_until_running(&admin, &alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        drop(stream);
        let stopped_at = Instant::now();
        eprintln!(
            "SOF-CANCEL mongodb cancel_before_first_row ($facet, drop): dropped {:?} after the \
             run started",
            stopped_at - started
        );
        assert_ended("cancel_before_first_row (drop)", stopped_at, || async {
            running_aggregates(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// `cancel_before_first_row` (token): the run ends `Cancelled` and the
    /// initial `aggregate` ends within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_before_the_first_row_ends_cancelled() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::CancelBeforeFirstRow).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let stream = mongo
            .run(union_view(&alias), ViewFilters::default(), ctx)
            .await;
        wait_until_running(&admin, &alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0);
        assert_cancelled("mongodb cancel_before_first_row", &drained);
        assert_ended("cancel_before_first_row (token)", stopped_at, || async {
            running_aggregates(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// D-8, the two mechanisms one by one, as the read-only user of an
    /// `--auth` server: (1) it finds its own operation with `$currentOp`
    /// (`allUsers: false`) and kills it with `killOp`, without the `inprog`
    /// or `killop` privileges; (2) closing the connection of a running read
    /// kills it on the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_least_privileged_user_can_kill_its_own_read_and_a_disconnect_ends_it() {
        let Some(mongo) = shared(true).await else {
            return;
        };
        let admin = mongo.admin().await;
        let reader = mongodb::Client::with_uri_str(&mongo.runner_uri)
            .await
            .expect("reader client");
        let resources = reader.database(&mongo.db).collection::<Document>(RESOURCES);
        // The shape of a `unionAll` view: a `$facet` that reads every Patient
        // of the tenant before it returns, named by the marker.
        let pipeline = |alias: &str| {
            vec![
                doc! {"$match": {"tenant_id": "t1"}},
                doc! {"$facet": {
                    alias: [{"$match": {"data.gender": "no-such-gender"}}],
                    "other": [{"$match": {"data.gender": "no-such-other-gender"}}],
                }},
            ]
        };

        // (1) Own-operation $currentOp + killOp.
        let alias = marker("sofkill");
        let comment = format!("comment-{alias}");
        let read = {
            let resources = resources.clone();
            let pipeline = pipeline(&alias);
            let comment = comment.clone();
            tokio::spawn(async move {
                let started = Instant::now();
                let result = resources
                    .aggregate(pipeline)
                    .comment(Bson::String(comment))
                    .await
                    .map(|_| ());
                (result, started.elapsed())
            })
        };
        wait_until_running(&admin, &alias).await;
        let mut found = reader
            .database("admin")
            .aggregate(vec![
                doc! {"$currentOp": {"allUsers": false, "idleConnections": false}},
                doc! {"$match": {"command.comment": &comment}},
            ])
            .await
            .expect("a read-only user lists its own operations");
        assert!(
            found.advance().await.expect("$currentOp cursor"),
            "the read-only user sees its own aggregate"
        );
        let opid = found
            .deserialize_current()
            .expect("op")
            .get("opid")
            .cloned()
            .expect("opid");
        let killed_at = Instant::now();
        reader
            .database("admin")
            .run_command(doc! {"killOp": 1, "op": opid})
            .await
            .expect("a read-only user kills its own operation");
        let (result, _) = tokio::time::timeout(STOP_BOUND, read)
            .await
            .expect("the killed aggregate returns within the bound")
            .expect("task");
        eprintln!(
            "SOF-CANCEL mongodb D-8 own killOp (read-only user): the aggregate returned {:?} \
             after killOp: {result:?}",
            killed_at.elapsed()
        );
        assert!(result.is_err(), "the killed aggregate fails");

        // (2) Disconnect kill: abort the task that awaits the aggregate; the
        // driver closes the connection of a command in flight.
        let alias = marker("sofdisconnect");
        let read = {
            let resources = resources.clone();
            let pipeline = pipeline(&alias);
            tokio::spawn(async move {
                let _ = resources.aggregate(pipeline).await;
            })
        };
        wait_until_running(&admin, &alias).await;
        read.abort();
        let stopped_at = Instant::now();
        assert_ended(
            "D-8 disconnect kill (read-only user)",
            stopped_at,
            || async { running_aggregates(&admin, &alias).await > 0 },
        )
        .await;
        let_cleanup_finish().await;
    }

    /// `cancel_while_planning` (D-8, council S3): the initial `aggregate` of a
    /// `$facet` view, run by the read-only user of an `--auth` server, is
    /// cancelled before it returned; it ends within the bound and the run
    /// ends `Cancelled`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_least_privileged_user_kills_its_own_initial_aggregate() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::CancelWhilePlanning).is_none());
        let Some(mongo) = shared(true).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo_ro");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let stream = mongo
            .run(union_view(&alias), ViewFilters::default(), ctx)
            .await;
        wait_until_running(&admin, &alias).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_cancelled("mongodb cancel_while_planning (read-only user)", &drained);
        assert_ended(
            "cancel_while_planning (read-only user)",
            stopped_at,
            || async { running_aggregates(&admin, &alias).await > 0 },
        )
        .await;
        let_cleanup_finish().await;
    }

    /// D-8 registration race: a stop that lands while the initial `aggregate`
    /// is on its way to the server (sent, not yet in `$currentOp`) still ends
    /// it within the bound. Stops at several small offsets after `run_view`
    /// returned, as the read-only user.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_before_the_aggregate_is_registered_still_kills_it() {
        let Some(mongo) = shared(true).await else {
            return;
        };
        let admin = mongo.admin().await;
        let runner = mongo
            .backend(None)
            .sof_runner()
            .expect("MongoDB SoF runner");
        // Warm the pool so the offsets land on the command, not the connect.
        let warm = runner
            .run_view(
                &tenant(),
                RunRequest::new(
                    flat_view("warm"),
                    ViewFilters {
                        limit: Some(1),
                        ..Default::default()
                    },
                ),
                RunContext::detached(RunKind::Preview),
            )
            .await
            .expect("warm-up run")
            .rows;
        assert!(drain(warm, Duration::from_secs(30)).await.ended);

        let mut seen = 0usize;
        for offset_ms in [0u64, 1, 2, 3, 5, 8, 13, 20] {
            let alias = marker("sofrace");
            let ctx = RunContext::detached(RunKind::Preview);
            let cancel = ctx.cancel.clone();
            let stream = runner
                .run_view(
                    &tenant(),
                    RunRequest::new(union_view(&alias), ViewFilters::default()),
                    ctx,
                )
                .await
                .expect("run_view")
                .rows;
            tokio::time::sleep(Duration::from_millis(offset_ms)).await;
            cancel.cancel();
            let stopped_at = Instant::now();
            let ever_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cell = format!("registration race (stop {offset_ms} ms after run_view)");
            assert_ended(&cell, stopped_at, || {
                let admin = admin.clone();
                let alias = alias.clone();
                let ever_seen = ever_seen.clone();
                async move {
                    let running = running_aggregates(&admin, &alias).await > 0;
                    ever_seen.fetch_or(running, std::sync::atomic::Ordering::SeqCst);
                    running
                }
            })
            .await;
            // Nothing comes back later either.
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(
                running_aggregates(&admin, &alias).await,
                0,
                "{cell}: the aggregate showed up again"
            );
            seen += usize::from(ever_seen.load(std::sync::atomic::Ordering::SeqCst));
            let drained = drain(stream, STOP_BOUND).await;
            assert_cancelled(&cell, &drained);
        }
        eprintln!(
            "SOF-CANCEL mongodb registration race: the aggregate showed in $currentOp after the \
             stop in {seen}/8 runs, and ended within the bound in all"
        );
        let_cleanup_finish().await;
    }

    /// D-8 under a saturated pool: with a driver pool of 2, run A holds one
    /// connection, run B (stopped) the other, and run C waits for a
    /// connection. Stopping B frees its connection to C, so B's cleanup
    /// commands cannot get one and give up after their timeout; B's aggregate
    /// still ends within the bound, through its closed connection.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_under_a_saturated_pool_still_ends_the_operation() {
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let runner = mongo
            .backend(Some(2))
            .sof_runner()
            .expect("MongoDB SoF runner");
        let start = |alias: &str| {
            let runner = runner.clone();
            let view = union_view(alias);
            async move {
                let ctx = RunContext::detached(RunKind::Preview);
                let cancel = ctx.cancel.clone();
                let rows = runner
                    .run_view(
                        &tenant(),
                        RunRequest::new(view, ViewFilters::default()),
                        ctx,
                    )
                    .await
                    .expect("run_view")
                    .rows;
                (rows, cancel)
            }
        };
        let (a, b, c) = (marker("sofsat_a"), marker("sofsat_b"), marker("sofsat_c"));
        let (rows_a, cancel_a) = start(&a).await;
        wait_until_running(&admin, &a).await;
        let (rows_b, cancel_b) = start(&b).await;
        wait_until_running(&admin, &b).await;
        let (rows_c, cancel_c) = start(&c).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            running_aggregates(&admin, &c).await,
            0,
            "run C waits for a pooled connection"
        );

        cancel_b.cancel();
        let stopped_at = Instant::now();
        let drained = drain(rows_b, STOP_BOUND).await;
        assert_cancelled("mongodb saturated pool", &drained);
        assert_ended("saturated pool (run B)", stopped_at, || async {
            running_aggregates(&admin, &b).await > 0
        })
        .await;
        // The connection B gave up went to C, which waited first: the pool
        // stayed saturated while B's cleanup ran.
        let deadline = Instant::now() + STOP_BOUND;
        while running_aggregates(&admin, &c).await == 0 {
            assert!(
                Instant::now() < deadline,
                "run C never got the connection run B gave up"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel_a.cancel();
        cancel_c.cancel();
        drop((rows_a, rows_c));
        for alias in [&a, &c] {
            assert_ended(
                "saturated pool (clean-up of A and C)",
                Instant::now(),
                || async { running_aggregates(&admin, alias).await > 0 },
            )
            .await;
        }
        let_cleanup_finish().await;
    }

    /// `cancel_between_chunks`: the consumer read one batch and stopped
    /// reading, so the producer waits to deliver the next while the cursor
    /// stays open on the server; a cancel ends the run and kills the cursor.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_between_batches_ends_cancelled() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::CancelBetweenChunks).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = mongo
            .run(flat_view(&alias), ViewFilters::default(), ctx)
            .await;
        for _ in 0..256 {
            stream.next().await.expect("a row").expect("a row");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            server_work(&admin, &alias).await > 0,
            "the cursor is open on the server"
        );

        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert!(drained.rows < DOCS, "the run stopped early");
        assert_cancelled("mongodb cancel_between_chunks", &drained);
        assert_ended("cancel_between_chunks (cursor)", stopped_at, || async {
            server_work(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// `cancel_mid_stream`: the consumer is reading rows as they come when the
    /// cancel arrives; the cursor is killed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_mid_stream_ends_cancelled() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::CancelMidStream).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = mongo
            .run(flat_view(&alias), ViewFilters::default(), ctx)
            .await;
        let mut read = 0usize;
        while read < 10_000 {
            stream.next().await.expect("a row").expect("a row");
            read += 1;
        }
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert!(read + drained.rows < DOCS, "the run stopped early");
        assert_cancelled("mongodb cancel_mid_stream", &drained);
        assert_ended("cancel_mid_stream (cursor)", stopped_at, || async {
            server_work(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// `stop_at_cap`: a capped run ends cleanly at its cap and leaves nothing
    /// on the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_capped_run_stops_at_its_cap() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::StopAtCap).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let stream = mongo
            .run(
                flat_view(&alias),
                ViewFilters {
                    limit: Some(10),
                    ..Default::default()
                },
                RunContext::detached(RunKind::Preview),
            )
            .await;
        let drained = drain(stream, Duration::from_secs(60)).await;
        let stopped_at = Instant::now();
        assert!(drained.ended);
        assert_eq!(drained.rows, 10);
        assert!(drained.error.is_none(), "{:?}", drained.error);
        assert_ended("stop_at_cap", stopped_at, || async {
            server_work(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// `deadline_ends_as_timeout`: the run's deadline passes while the initial
    /// `aggregate` of a `$facet` view runs. The server ends it at `maxTimeMS`
    /// (the deadline), and the run ends with a `Timeout` carrying its budget.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_deadline_ends_the_run_as_a_timeout() {
        assert!(not_applicable(Runner::MongoDb, CancelCase::DeadlineEndsAsTimeout).is_none());
        let Some(mongo) = shared(false).await else {
            return;
        };
        let admin = mongo.admin().await;
        let alias = marker("sofcancel_mongo");
        let budget = Duration::from_millis(700);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = mongo
            .run(union_view(&alias), ViewFilters::default(), ctx)
            .await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        let ended_at = Instant::now();
        assert_timed_out("mongodb deadline_ends_as_timeout", &drained, budget);
        assert!(
            ended_at - started <= budget + STOP_BOUND,
            "the timeout came {:?} after the start",
            ended_at - started
        );
        assert_ended("deadline_ends_as_timeout", started + budget, || async {
            running_aggregates(&admin, &alias).await > 0
        })
        .await;
        let_cleanup_finish().await;
    }

    /// MongoDB row of the truth table: the pipeline does not sort, so no
    /// shape has a total order; a stop ends the server's work.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_capabilities_truth_table() {
        use super::truth_table::{assert_row, declared};

        let Some(mongo) = shared(false).await else {
            return;
        };
        let db = format!("sofcaps_{}", uuid::Uuid::new_v4().simple());
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: mongo.admin_uri.clone(),
            database_name: db.clone(),
            ..Default::default()
        })
        .expect("create MongoBackend");
        backend
            .initialize()
            .await
            .expect("initialize MongoDB schema");
        let runner = backend.sof_runner().expect("MongoDB SoF runner");
        let stops = RunCapabilities {
            cancels_server_side: true,
            ..declared(false)
        };
        assert_row("mongodb", runner.as_ref(), &tenant(), [stops; 4]).await;
        let _ = mongo.admin().await.database(&db).drop().await;
    }
}

/// In-process runner (S3, MongoDB compartment fallback): one module per
/// runner column, like the database runners. The scan double counts the
/// resources pulled from it, which is the storage work this runner does; the
/// evaluation work is observed through the run's end, which comes only after
/// the evaluation returned.
mod in_process {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::sof_runner::{
        RowStream, RunContext, RunKind, RunRequest, SofError, SofRunner, ViewFilters,
    };
    use helios_persistence::sof::in_process::{InProcessSofRunner, ResourceScan, ResourceStream};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};

    use super::sof_cancellation_cases::{
        CancelCase, Runner, STOP_BOUND, assert_cancelled, assert_timed_out, drain, not_applicable,
        observe_after_stop,
    };

    /// One resource per type, enough for every truth-table shape to yield a
    /// row.
    struct OneOfEach;

    #[async_trait]
    impl ResourceScan for OneOfEach {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            let resource = match resource_type {
                "Patient" => json!({"resourceType": "Patient", "id": "p1", "gender": "female",
                    "name": [{"family": "F"}]}),
                "QuestionnaireResponse" => json!({"resourceType": "QuestionnaireResponse",
                    "id": "q1", "status": "completed", "item": [{"linkId": "1"}]}),
                _ => return Ok(Box::pin(futures::stream::empty())),
            };
            Ok(Box::pin(futures::stream::iter([Ok(resource)])))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// In-process row of the truth table: rows follow the scan, so no shape
    /// has a total order; nothing runs on a database server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_capabilities_truth_table() {
        use super::truth_table::{assert_row, declared};

        let runner = InProcessSofRunner::new(Arc::new(OneOfEach), FhirVersion::R4, "in-process");
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        assert_row("in-process", &runner, &tenant, [declared(false); 4]).await;
        let job = runner.for_export_job().expect("a per-job scan cache");
        assert_row(
            "in-process export job",
            job.as_ref(),
            &tenant,
            [declared(false); 4],
        )
        .await;
    }

    /// A scan of Patients that counts the resources pulled from it.
    #[derive(Clone)]
    struct CountingScan {
        pulled: Arc<AtomicUsize>,
        /// Time to produce each resource (a slow storage read).
        delay: Option<Duration>,
        /// Resources in the type; `None` is endless.
        total: Option<usize>,
        /// `name` entries per Patient.
        names: usize,
        /// `scan_resources` itself never returns (a storage call that hangs
        /// before the first resource).
        hang_at_start: bool,
    }

    impl CountingScan {
        fn new() -> Self {
            Self {
                pulled: Arc::default(),
                delay: None,
                total: None,
                names: 1,
                hang_at_start: false,
            }
        }

        fn pulled(&self) -> usize {
            self.pulled.load(Ordering::SeqCst)
        }
    }

    fn patient(i: usize, names: usize) -> Value {
        let names: Vec<Value> = (0..names)
            .map(|n| json!({"family": format!("f{i}-{n}")}))
            .collect();
        json!({"resourceType": "Patient", "id": format!("p{i}"), "name": names})
    }

    #[async_trait]
    impl ResourceScan for CountingScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            if self.hang_at_start {
                std::future::pending::<()>().await;
            }
            let this = self.clone();
            Ok(Box::pin(futures::stream::unfold(0usize, move |i| {
                let this = this.clone();
                async move {
                    if this.total.is_some_and(|total| i >= total) {
                        return None;
                    }
                    if let Some(delay) = this.delay {
                        tokio::time::sleep(delay).await;
                    }
                    this.pulled.fetch_add(1, Ordering::SeqCst);
                    Some((Ok(patient(i, this.names)), i + 1))
                }
            })))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// One row per Patient name.
    fn names_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"name": "id", "path": "id"}]},
                {"forEach": "name", "column": [{"name": "family", "path": "family"}]}
            ]
        })
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    async fn run(scan: &CountingScan, filters: ViewFilters, ctx: RunContext) -> RowStream {
        InProcessSofRunner::new(Arc::new(scan.clone()), FhirVersion::R4, "in-process")
            .run_view(&tenant(), RunRequest::new(names_view(), filters), ctx)
            .await
            .map(|output| output.rows)
            .expect("run_view")
    }

    /// Asserts that the scan stopped pulling resources within the bound after
    /// `stopped_at`, and reports how many it pulled after the stop.
    async fn assert_scan_stopped(cell: &str, scan: &CountingScan, stopped_at: Instant) {
        let at_stop = scan.pulled();
        let mut last = at_stop;
        let (running_at_bound, ran_for) = observe_after_stop(stopped_at, || {
            let now = scan.pulled();
            let moving = now != last;
            last = now;
            async move { moving }
        })
        .await;
        // Give a scan that only paused a moment the chance to show it did not
        // stop.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let after = scan.pulled() - at_stop;
        eprintln!(
            "SOF-CANCEL in-process {cell}: the scan pulled {after} resources after the stop and \
             kept pulling for {ran_for:?} (still pulling at the bound: {running_at_bound})"
        );
        assert!(
            !running_at_bound,
            "{cell}: the scan was still pulling resources {STOP_BOUND:?} after the stop"
        );
        assert_eq!(
            scan.pulled(),
            last,
            "{cell}: the scan pulled more resources after it seemed to stop"
        );
    }

    /// `cancel_before_first_row` (drop): a slow scan, dropped before the
    /// first batch is full.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_the_stream_before_the_first_row_stops_the_scan() {
        assert!(not_applicable(Runner::InProcess, CancelCase::CancelBeforeFirstRow).is_none());
        let scan = CountingScan {
            delay: Some(Duration::from_millis(2)),
            ..CountingScan::new()
        };
        let stream = run(
            &scan,
            ViewFilters::default(),
            RunContext::detached(RunKind::Preview),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(scan.pulled() > 0, "the scan started");
        drop(stream);
        let stopped_at = Instant::now();
        assert_scan_stopped("cancel_before_first_row (drop)", &scan, stopped_at).await;
    }

    /// `cancel_before_first_row` (token): the run ends `Cancelled` and the
    /// scan stops.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_before_the_first_row_ends_cancelled() {
        assert!(not_applicable(Runner::InProcess, CancelCase::CancelBeforeFirstRow).is_none());
        let scan = CountingScan {
            delay: Some(Duration::from_millis(2)),
            ..CountingScan::new()
        };
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let stream = run(&scan, ViewFilters::default(), ctx).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_eq!(drained.rows, 0);
        assert_cancelled("in-process cancel_before_first_row", &drained);
        assert_scan_stopped("cancel_before_first_row (token)", &scan, stopped_at).await;
    }

    /// `cancel_before_first_row` while `scan_resources` itself is pending
    /// (the in-process form of "while planning"): the setup await is raced,
    /// so `run_view` returns `Cancelled` within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_while_the_scan_is_starting_ends_cancelled() {
        assert!(not_applicable(Runner::InProcess, CancelCase::CancelWhilePlanning).is_some());
        let scan = CountingScan {
            hang_at_start: true,
            ..CountingScan::new()
        };
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            cancel.cancel();
        });
        let started = Instant::now();
        let runner = InProcessSofRunner::new(Arc::new(scan.clone()), FhirVersion::R4, "in-process");
        let result = tokio::time::timeout(
            Duration::from_millis(200) + STOP_BOUND,
            runner.run_view(
                &tenant(),
                RunRequest::new(names_view(), ViewFilters::default()),
                ctx,
            ),
        )
        .await
        .expect("run_view returns once cancelled");
        eprintln!(
            "SOF-CANCEL in-process cancel_while_planning (pending scan_resources): run_view \
             returned {:?} after the start",
            started.elapsed()
        );
        assert!(
            matches!(result, Err(SofError::Cancelled)),
            "expected Cancelled, got {:?}",
            result.map(|_| "a row stream")
        );
    }

    /// `cancel_between_chunks`: the consumer read one batch and stopped
    /// reading, so the producer waits to deliver the next; a cancel ends it
    /// there and the scan stops.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_between_batches_ends_cancelled() {
        assert!(not_applicable(Runner::InProcess, CancelCase::CancelBetweenChunks).is_none());
        let scan = CountingScan::new();
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = run(&scan, ViewFilters::default(), ctx).await;
        for _ in 0..256 {
            stream.next().await.expect("a row").expect("a row");
        }
        // The producer fills the channel and waits; so does the scan.
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_cancelled("in-process cancel_between_chunks", &drained);
        assert_scan_stopped("cancel_between_chunks", &scan, stopped_at).await;
    }

    /// `cancel_mid_stream`: the consumer is reading rows as they come when the
    /// cancel arrives.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_mid_stream_ends_cancelled() {
        assert!(not_applicable(Runner::InProcess, CancelCase::CancelMidStream).is_none());
        let scan = CountingScan::new();
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let mut stream = run(&scan, ViewFilters::default(), ctx).await;
        for _ in 0..3000 {
            stream.next().await.expect("a row").expect("a row");
        }
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, STOP_BOUND).await;
        assert_cancelled("in-process cancel_mid_stream", &drained);
        assert_scan_stopped("cancel_mid_stream", &scan, stopped_at).await;
    }

    /// `stop_at_cap`: a capped run ends cleanly at its cap, and the endless
    /// scan stops.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_capped_run_stops_at_its_cap() {
        assert!(not_applicable(Runner::InProcess, CancelCase::StopAtCap).is_none());
        let scan = CountingScan::new();
        let stream = run(
            &scan,
            ViewFilters {
                limit: Some(10),
                ..Default::default()
            },
            RunContext::detached(RunKind::Preview),
        )
        .await;
        let drained = drain(stream, Duration::from_secs(30)).await;
        let stopped_at = Instant::now();
        assert!(drained.ended);
        assert_eq!(drained.rows, 10);
        assert!(drained.error.is_none(), "{:?}", drained.error);
        assert_scan_stopped("stop_at_cap", &scan, stopped_at).await;
    }

    /// `deadline_ends_as_timeout`: a slow scan past the run's deadline ends
    /// the run with a `Timeout` carrying its budget, and the scan stops.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_deadline_ends_the_run_as_a_timeout() {
        assert!(not_applicable(Runner::InProcess, CancelCase::DeadlineEndsAsTimeout).is_none());
        let scan = CountingScan {
            delay: Some(Duration::from_millis(2)),
            ..CountingScan::new()
        };
        let budget = Duration::from_millis(700);
        let mut ctx = RunContext::detached(RunKind::Preview);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let stream = run(&scan, ViewFilters::default(), ctx).await;
        let drained = drain(stream, budget + STOP_BOUND).await;
        let ended_at = Instant::now();
        assert_timed_out("in-process deadline_ends_as_timeout", &drained, budget);
        assert!(
            ended_at - started <= budget + STOP_BOUND,
            "the timeout came {:?} after the start",
            ended_at - started
        );
        assert_scan_stopped("deadline_ends_as_timeout", &scan, started + budget).await;
    }

    /// A7 (council S4): the evaluation stop. Chunks of 1024 Patients with large
    /// `name` arrays under a `forEach`; the consumer stops after the first row,
    /// while the next chunk is being evaluated. The run's `Cancelled` item comes
    /// only after the blocking evaluation (and its Rayon work) returned, so its
    /// arrival within the bound is the evaluation stopping within the bound.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_expensive_chunk_stops_within_the_bound() {
        let scan = CountingScan {
            total: Some(3 * 1024),
            names: 400,
            ..CountingScan::new()
        };
        let ctx = RunContext::detached(RunKind::Preview);
        let cancel = ctx.cancel.clone();
        let started = Instant::now();
        let mut stream = run(&scan, ViewFilters::default(), ctx).await;
        stream.next().await.expect("a row").expect("a row");
        let first_row = started.elapsed();
        cancel.cancel();
        let stopped_at = Instant::now();
        let drained = drain(stream, Duration::from_secs(60)).await;
        let ended_after = stopped_at.elapsed();
        eprintln!(
            "SOF-CANCEL in-process expensive chunk: the first chunk took {first_row:?} to its \
             first row; the run ended {ended_after:?} after the cancel ({} more rows)",
            drained.rows
        );
        assert_cancelled("in-process expensive chunk", &drained);
        assert!(
            ended_after < STOP_BOUND,
            "the evaluation ran {ended_after:?} after the cancel"
        );
        assert!(
            drained.rows < 2 * 1024 * 400,
            "the stopped chunk delivered no partial rows beyond the first chunk"
        );
    }
}
