//! The PostgreSQL export pool `$sql-export` jobs read through (#1864).
//!
//! Against a real PostgreSQL 16 (testcontainers, Docker required):
//!
//! - `export_sof_runner(..)` → `for_export_job()` is the paged reader, and it
//!   reads through its own `hfs-export` connections: the same rows as
//!   `sof_runner`, in resource-id order;
//! - a stream dropped mid-result leaves no statement running on its
//!   connection, and the one-connection pool serves the next run in full;
//! - with every export connection busy, a queued run (the export pool has no
//!   wait timeout) still ends at once on a cancel and at its deadline;
//! - the export runner keeps the backend's configured FHIR version (`R5`
//!   feature only: the only observable difference is a version whose
//!   cardinalities differ from R4's).
//!
//! The settings the connections carry are checked by the backend's in-source
//! tests (`export_pool_connections_carry_the_export_settings`), which can
//! reach the main pool to compare.
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres --test sof_pg_export_pool

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod sof_pg_export_pool_tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::sof_runner::{
        RowStream, RunContext, RunKind, RunRequest, SofError, SofRunner, ViewFilters,
    };
    use helios_persistence::core::{ExportRunnerOptions, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio_postgres::NoTls;

    /// The stop bound (issue #1864, target 1 s).
    const STOP_BOUND: Duration = Duration::from_secs(1);

    /// A container of its own: the assertions read `pg_stat_activity`, which
    /// another test's connections would pollute.
    async fn start_pg() -> (testcontainers::ContainerAsync<Postgres>, PostgresConfig) {
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
        .expect("start PostgreSQL 16 testcontainer");
        let config = PostgresConfig {
            host: container.get_host().await.expect("host").to_string(),
            port: container.get_host_port_ipv4(5432).await.expect("port"),
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            max_connections: 3,
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        (container, config)
    }

    async fn backend(config: &PostgresConfig) -> PostgresBackend {
        let backend = PostgresBackend::new(config.clone())
            .await
            .expect("PostgresBackend");
        backend.init_schema().await.expect("schema");
        backend
    }

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    /// A connection outside both pools, for reading `pg_stat_activity`.
    async fn observer(config: &PostgresConfig) -> tokio_postgres::Client {
        let (client, connection) = tokio_postgres::Config::new()
            .host(&config.host)
            .port(config.port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres")
            .application_name("export-pool-test")
            .connect(NoTls)
            .await
            .expect("observer connection");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }

    async fn connections_named(observer: &tokio_postgres::Client, name: &str) -> i64 {
        observer
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
                &[&name],
            )
            .await
            .expect("read pg_stat_activity")
            .get(0)
    }

    /// Export statements (`hfs-export`) that are executing.
    async fn active_export_statements(observer: &tokio_postgres::Client) -> i64 {
        observer
            .query_one(
                "SELECT count(*) FROM pg_stat_activity \
                 WHERE application_name = 'hfs-export' AND state = 'active'",
                &[],
            )
            .await
            .expect("read pg_stat_activity")
            .get(0)
    }

    async fn start(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: &Value,
        ctx: RunContext,
    ) -> RowStream {
        runner
            .run_view(
                tenant,
                RunRequest::new(view.clone(), ViewFilters::default()),
                ctx,
            )
            .await
            .map(|output| output.rows)
            .expect("run_view")
    }

    async fn rows(runner: &dyn SofRunner, tenant: &TenantContext, view: &Value) -> Vec<Value> {
        let mut stream = start(
            runner,
            tenant,
            view,
            RunContext::detached(RunKind::ExportJob),
        )
        .await;
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row"));
        }
        rows
    }

    /// `name` × `address` of one Patient with `n` of each: `n * n` rows.
    fn cartesian_patient(id: &str, n: usize) -> Value {
        let names: Vec<Value> = (0..n)
            .map(|i| json!({"family": format!("f{i:05}")}))
            .collect();
        let addresses: Vec<Value> = (0..n)
            .map(|i| json!({"city": format!("c{i:05}")}))
            .collect();
        json!({"resourceType": "Patient", "id": id, "name": names, "address": addresses})
    }

    fn cartesian_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"forEach": "name", "column": [{"name": "family", "path": "family"}]},
                {"forEach": "address", "column": [{"name": "city", "path": "city"}]}
            ]
        })
    }

    /// D-a wiring chain: `export_sof_runner(..)` → `for_export_job()` is the
    /// paged reader (rows in bytewise resource-id order, where `sof_runner`
    /// gives `(last_updated, id)`) and reads on `hfs-export` connections; the
    /// request runner never opens one.
    #[tokio::test]
    async fn export_runner_reads_through_the_export_pool() {
        let (_container, config) = start_pg().await;
        let backend = backend(&config).await;
        let tenant = tenant("export_pool");
        // Inserted out of id order, so the two orders differ.
        for (id, given) in [("c", vec![]), ("a", vec!["Ann", "Amy"]), ("b", vec!["Bob"])] {
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
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"name": "id", "path": "getResourceKey()"}]},
                {"forEachOrNull": "name.given", "column": [{"name": "given", "path": "$this"}]}
            ]
        });

        let observer = observer(&config).await;
        let main_runner = backend.sof_runner().expect("main runner");
        let main_rows = rows(main_runner.as_ref(), &tenant, &view).await;
        assert_eq!(connections_named(&observer, "hfs-export").await, 0);

        let export_runner = backend
            .export_sof_runner(&ExportRunnerOptions::default())
            .expect("export runner");
        assert_eq!(export_runner.runner_name(), "postgres-indb");
        // Lazy: nothing is opened until a run needs a connection.
        assert_eq!(connections_named(&observer, "hfs-export").await, 0);
        let job_runner = export_runner
            .for_export_job()
            .expect("the PostgreSQL export runner has a per-job reader");
        let export_rows = rows(job_runner.as_ref(), &tenant, &view).await;

        // The same multiset; the paged reader's order is the resource id.
        let ids = |rows: &[Value]| -> Vec<String> {
            rows.iter()
                .map(|r| r["id"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(ids(&export_rows), ["a", "a", "b", "c"], "{export_rows:?}");
        assert_eq!(ids(&main_rows), ["c", "a", "a", "b"], "{main_rows:?}");
        let sorted = |rows: &[Value]| {
            let mut v: Vec<String> = rows.iter().map(Value::to_string).collect();
            v.sort();
            v
        };
        assert_eq!(sorted(&export_rows), sorted(&main_rows));
        // The export connection stays pooled after the run.
        assert_eq!(connections_named(&observer, "hfs-export").await, 1);
    }

    /// A consumer that stops reading mid-result does not leave the rest of
    /// the statement running on an export connection: within the stop bound
    /// no `hfs-export` session is active, and the one-connection pool serves
    /// the next run in full.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn abandoned_export_stream_leaves_no_statement_running() {
        let (_container, config) = start_pg().await;
        let backend = backend(&config).await;
        let tenant = tenant("export_abandon");
        backend
            .create(
                &tenant,
                "Patient",
                cartesian_patient("big", 300),
                FhirVersion::R4,
            )
            .await
            .expect("seed patient");
        let view = cartesian_view();

        let observer = observer(&config).await;
        let job_runner = backend
            .export_sof_runner(&ExportRunnerOptions {
                max_connections: 1,
                ..ExportRunnerOptions::default()
            })
            .expect("export runner")
            .for_export_job()
            .expect("per-job reader");

        let mut stream = start(
            job_runner.as_ref(),
            &tenant,
            &view,
            RunContext::detached(RunKind::ExportJob),
        )
        .await;
        stream.next().await.expect("first row").expect("row");
        assert_eq!(connections_named(&observer, "hfs-export").await, 1);
        drop(stream);
        let dropped_at = Instant::now();

        while active_export_statements(&observer).await != 0 {
            assert!(
                dropped_at.elapsed() < STOP_BOUND,
                "abandoned export statement still running after {:?}",
                dropped_at.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        eprintln!(
            "SOF-EXPORT-POOL abandoned stream: no active hfs-export statement {:?} after the drop",
            dropped_at.elapsed()
        );

        // The one-connection pool still serves the next run, in full.
        let all = rows(job_runner.as_ref(), &tenant, &view).await;
        assert_eq!(all.len(), 300 * 300);
    }

    /// With every export connection busy, a run waits for one (the export
    /// pool has no wait timeout), and that wait still ends at once on a
    /// cancel and at the run's deadline, so a queued export job stays
    /// cancellable and bounded (§4.2 case 7, R-j). Once the connection is
    /// back, the pool serves the next run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_queued_export_stays_cancellable_when_the_pool_is_smaller() {
        let (_container, config) = start_pg().await;
        let backend = backend(&config).await;
        let tenant = tenant("export_queue");
        backend
            .create(
                &tenant,
                "Patient",
                cartesian_patient("big", 300),
                FhirVersion::R4,
            )
            .await
            .expect("seed patient");
        let small = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "getResourceKey()"}]}]
        });
        let job_runner = backend
            .export_sof_runner(&ExportRunnerOptions {
                max_connections: 1,
                ..ExportRunnerOptions::default()
            })
            .expect("export runner")
            .for_export_job()
            .expect("per-job reader");

        // The only export connection, held by a run nobody reads.
        let mut holder = start(
            job_runner.as_ref(),
            &tenant,
            &cartesian_view(),
            RunContext::detached(RunKind::ExportJob),
        )
        .await;
        holder.next().await.expect("first row").expect("row");

        // Cancelled while queued.
        let ctx = RunContext::detached(RunKind::ExportJob);
        let cancel = ctx.cancel.clone();
        let mut queued = start(job_runner.as_ref(), &tenant, &small, ctx).await;
        let waited = tokio::time::timeout(Duration::from_millis(500), queued.next()).await;
        assert!(waited.is_err(), "the run waits for the busy connection");
        cancel.cancel();
        let cancelled_at = Instant::now();
        let item = tokio::time::timeout(STOP_BOUND, queued.next())
            .await
            .expect("a queued run ends within the bound of its cancel");
        assert!(
            matches!(item, Some(Err(SofError::Cancelled))),
            "a cancelled queued run ends Cancelled: {item:?}"
        );
        eprintln!(
            "SOF-EXPORT-POOL queued run: ended Cancelled {:?} after the cancel",
            cancelled_at.elapsed()
        );

        // Past its deadline while queued.
        let mut ctx = RunContext::detached(RunKind::ExportJob);
        let budget = Duration::from_millis(500);
        ctx.deadline = Some(tokio::time::Instant::now() + budget);
        let started = Instant::now();
        let mut queued = start(job_runner.as_ref(), &tenant, &small, ctx).await;
        let item = tokio::time::timeout(budget + STOP_BOUND, queued.next())
            .await
            .expect("a queued run ends within the bound of its deadline");
        assert!(
            matches!(item, Some(Err(SofError::Timeout { .. }))),
            "a queued run past its deadline ends Timeout: {item:?}"
        );
        assert!(started.elapsed() >= budget);
        eprintln!(
            "SOF-EXPORT-POOL queued run: ended Timeout {:?} after start (budget {budget:?})",
            started.elapsed()
        );

        // The holder gives the connection back; the pool serves the next run.
        drop(holder);
        let all = tokio::time::timeout(
            Duration::from_secs(10),
            rows(job_runner.as_ref(), &tenant, &small),
        )
        .await
        .expect("the returned connection serves the next run");
        assert_eq!(all, vec![json!({"id": "big"})]);
    }

    /// R-d: the export runner compiles with the backend's configured FHIR
    /// version, as `sof_runner` does. `Encounter.class` is one `Coding` in R4
    /// and a list of `CodeableConcept` in R5, so `class.code` with
    /// `collection: false` is refused on an R5 backend and accepted on an R4
    /// one. An export runner without the version would compile as R4 and
    /// accept it on R5 too.
    #[cfg(feature = "R5")]
    #[tokio::test]
    async fn export_runner_keeps_the_configured_fhir_version() {
        let (_container, config) = start_pg().await;
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Encounter",
            "status": "active",
            "select": [{"column": [
                {"name": "class_code", "path": "class.code", "collection": false}
            ]}]
        });
        let tenant = tenant("export_version");
        for (version, refused) in [(FhirVersion::R5, true), (FhirVersion::R4, false)] {
            let backend = backend(&PostgresConfig {
                fhir_version: version,
                ..config.clone()
            })
            .await;
            let export_runner = backend
                .export_sof_runner(&ExportRunnerOptions::default())
                .expect("export runner");
            for (name, runner) in [
                ("export runner", std::sync::Arc::clone(&export_runner)),
                (
                    "per-job reader",
                    export_runner.for_export_job().expect("per-job reader"),
                ),
                ("request runner", backend.sof_runner().expect("main runner")),
            ] {
                let started = runner
                    .run_view(
                        &tenant,
                        RunRequest::new(view.clone(), ViewFilters::default()),
                        RunContext::detached(RunKind::ExportJob),
                    )
                    .await;
                match started {
                    Err(SofError::InvalidViewDefinition(reason)) if refused => {
                        assert!(
                            reason.contains("class.code"),
                            "{version:?} {name}: {reason}"
                        );
                    }
                    Ok(_) if !refused => {}
                    Err(e) => panic!("{version:?} {name}: unexpected error {e}"),
                    Ok(_) => panic!("{version:?} {name}: must refuse class.code as a collection"),
                }
            }
        }
    }
}
