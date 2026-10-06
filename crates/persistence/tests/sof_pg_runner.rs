//! Phase 3b integration tests: PostgreSQL in-DB runner.
//!
//! Verifies:
//! 1. `PostgresBackend::sof_runner()` returns the in-DB runner (not `None`).
//! 2. The in-DB runner produces correct rows for spec ViewDefinition fixtures.
//! 3. `SofError::Uncompilable` is returned for unsupported ViewDefinitions.
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres -- sof_pg
//!
//! Requires Docker for testcontainers.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[path = "common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

mod sof_pg_runner_tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    // =========================================================================
    // Shared container setup (identical to postgres_tests.rs pattern)
    // =========================================================================

    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                // Pin the major version. testcontainers-modules defaults to
                // postgres:11, which is EOL and predates `plan_cache_mode` — a GUC
                // the backend sends as a startup option, so PG 11 rejects every
                // connection FATAL. The rest of the repo runs 16.
                // `SHARED_PG` is a static and never dropped; the cleanup label
                // lets the exit hook remove the container.
                let container = super::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("Failed to start PostgreSQL container");

                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("Failed to get host port");

                let host = container
                    .get_host()
                    .await
                    .expect("Failed to get host")
                    .to_string();

                let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("data"))
                    .unwrap_or_else(|| PathBuf::from("data"));

                let config = PostgresConfig {
                    host: host.clone(),
                    port,
                    dbname: "postgres".to_string(),
                    user: "postgres".to_string(),
                    password: Some("postgres".to_string()),
                    max_connections: 5,
                    data_dir: Some(data_dir),
                    ..Default::default()
                };

                let backend = PostgresBackend::new(config)
                    .await
                    .expect("Failed to create PostgresBackend");

                backend
                    .init_schema()
                    .await
                    .expect("Failed to initialize schema");

                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    async fn create_backend() -> Arc<PostgresBackend> {
        let pg = shared_pg().await;

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        };

        Arc::new(
            PostgresBackend::new(config)
                .await
                .expect("Failed to create PostgresBackend"),
        )
    }

    /// Keep plan-sensitive fixtures on fixed tables isolated from concurrent
    /// tests, with normal schema and planner settings.
    async fn create_dedicated_backend() -> (
        Arc<PostgresBackend>,
        testcontainers::ContainerAsync<Postgres>,
    ) {
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default().with_tag("16-alpine").with_label(
                "github.run_id",
                std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            ),
        )
        .start()
        .await
        .expect("start isolated fixture PostgreSQL");
        let config = PostgresConfig {
            host: container.get_host().await.unwrap().to_string(),
            port: container.get_host_port_ipv4(5432).await.unwrap(),
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            max_connections: 5,
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        let backend = Arc::new(
            PostgresBackend::new(config)
                .await
                .expect("create isolated backend"),
        );
        backend
            .init_schema()
            .await
            .expect("initialize isolated fixture schema");
        (backend, container)
    }

    fn test_tenant() -> TenantContext {
        let unique_id = format!("sof_pg_{}", uuid::Uuid::new_v4().simple());
        TenantContext::new(TenantId::new(&unique_id), TenantPermissions::full_access())
    }

    async fn seed_patients(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        patients: &[(&str, &str, &str)],
    ) {
        for (id, gender, dob) in patients {
            let resource = json!({
                "resourceType": "Patient",
                "id": id,
                "gender": gender,
                "birthDate": dob,
                "active": true,
                "name": [{"family": format!("Family-{id}"), "use": "official"}]
            });
            backend
                .create(tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }
    }

    async fn collect_rows(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
    ) -> Vec<BTreeMap<String, Value>> {
        let mut stream = runner
            .run_view(tenant, view, ViewFilters::default())
            .await
            .expect("run_view must succeed");

        let mut rows: Vec<BTreeMap<String, Value>> = Vec::new();
        while let Some(result) = stream.next().await {
            let row = result.expect("row must not be an error");
            let sorted: BTreeMap<String, Value> = row
                .as_object()
                .expect("row must be an object")
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            rows.push(sorted);
        }
        rows.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        rows
    }

    // =========================================================================
    // 1. Backend advertises the in-DB runner
    // =========================================================================

    #[tokio::test]
    async fn test_pg_backend_returns_sof_runner() {
        let backend = create_backend().await;
        let runner = backend.sof_runner();
        assert!(
            runner.is_some(),
            "PostgresBackend.sof_runner() must return Some"
        );
        assert_eq!(
            runner.unwrap().runner_name(),
            "postgres-indb",
            "runner name must be 'postgres-indb'"
        );
    }

    // =========================================================================
    // 2. Flat column queries
    // =========================================================================

    #[tokio::test]
    async fn test_pg_flat_columns() {
        let backend = create_backend().await;
        let tenant = test_tenant();

        seed_patients(
            &backend,
            &tenant,
            &[
                ("pg1", "male", "1990-01-01"),
                ("pg2", "female", "1985-06-15"),
            ],
        )
        .await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "column": [
                    {"path": "id", "name": "id", "type": "string"},
                    {"path": "gender", "name": "gender", "type": "string"},
                    {"path": "birthDate", "name": "dob", "type": "string"}
                ]
            }]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;

        assert_eq!(rows.len(), 2, "expected 2 rows");
        for row in &rows {
            assert!(row.contains_key("id"), "row missing 'id': {row:?}");
            assert!(row.contains_key("gender"), "row missing 'gender': {row:?}");
            assert!(row.contains_key("dob"), "row missing 'dob': {row:?}");
        }
        let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(ids.contains(&"pg1"), "missing pg1: {ids:?}");
        assert!(ids.contains(&"pg2"), "missing pg2: {ids:?}");
    }

    // =========================================================================
    // 3. forEach (LATERAL JOIN) queries
    // =========================================================================

    #[tokio::test]
    async fn test_pg_foreach_columns() {
        let backend = create_backend().await;
        let tenant = test_tenant();

        seed_patients(&backend, &tenant, &[("pg3", "male", "1990-01-01")]).await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{
                "forEach": "name",
                "column": [
                    {"path": "family", "name": "family", "type": "string"},
                    {"path": "use", "name": "use_code", "type": "string"}
                ]
            }]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;

        assert_eq!(rows.len(), 1, "expected 1 row (one name entry)");
        assert_eq!(rows[0]["family"], "Family-pg3");
        assert_eq!(rows[0]["use_code"], "official");
    }

    #[tokio::test]
    async fn test_pg_mixed_root_and_foreach() {
        let backend = create_backend().await;
        let tenant = test_tenant();

        seed_patients(
            &backend,
            &tenant,
            &[
                ("pg4", "male", "1990-01-01"),
                ("pg5", "female", "1985-06-15"),
            ],
        )
        .await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]}
            ]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;

        assert_eq!(rows.len(), 2, "expected 2 rows (2 patients × 1 name each)");
        let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(ids.contains(&"pg4"));
        assert!(ids.contains(&"pg5"));
    }

    // =========================================================================
    // 4. Limit and empty table
    // =========================================================================

    #[tokio::test]
    async fn test_pg_limit_respected() {
        let backend = create_backend().await;
        let tenant = test_tenant();

        seed_patients(
            &backend,
            &tenant,
            &[
                ("pg6", "male", "1990-01-01"),
                ("pg7", "female", "1985-06-15"),
                ("pg8", "male", "2000-03-20"),
            ],
        )
        .await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let mut stream = runner
            .run_view(
                &tenant,
                view,
                ViewFilters {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .expect("run_view must succeed");

        let mut count = 0;
        while stream.next().await.is_some() {
            count += 1;
        }
        assert_eq!(count, 2, "limit=2 must return exactly 2 rows");
    }

    /// Runner-path compartment fidelity (audit item #3 closeout for the
    /// Postgres in-DB runner): an Appointment whose patient link is
    /// `Appointment.participant.actor` (nested, not top-level
    /// subject/patient) is correctly included via the search-index
    /// EXISTS clause. The old hardcoded `subject.reference` /
    /// `patient.reference` JSONB filter could not see this case.
    #[tokio::test]
    async fn test_pg_appointment_compartment_runner() {
        let backend = create_backend().await;
        let tenant = test_tenant();

        let appt_in = json!({
            "resourceType": "Appointment",
            "id": "appt-alice",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/alice"}, "status": "accepted"}
            ]
        });
        let appt_out = json!({
            "resourceType": "Appointment",
            "id": "appt-bob",
            "status": "booked",
            "participant": [
                {"actor": {"reference": "Patient/bob"}, "status": "accepted"}
            ]
        });
        for res in [appt_in, appt_out] {
            backend
                .create(&tenant, "Appointment", res, FhirVersion::R4)
                .await
                .expect("failed to seed appointment");
        }

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Appointment",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "appt_id"}]}]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let mut stream = runner
            .run_view(
                &tenant,
                view,
                ViewFilters {
                    patient: vec!["Patient/alice".to_string()],
                    ..Default::default()
                },
            )
            .await
            .expect("run_view must succeed");

        let mut ids = Vec::new();
        while let Some(result) = stream.next().await {
            let row = result.expect("row must not be an error");
            if let Some(id) = row.get("appt_id").and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            }
        }
        assert_eq!(
            ids,
            vec!["appt-alice".to_string()],
            "patient compartment must include alice's Appointment via participant.actor"
        );
    }

    #[tokio::test]
    async fn test_pg_empty_table_returns_no_rows() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        // No seeding

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });

        let runner = backend.sof_runner().expect("must have runner");
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert!(rows.is_empty(), "expected 0 rows from empty tenant");
    }

    // =========================================================================
    // 5. FHIRPath expressions previously rejected by the in-DB runner that
    //    the new IR-based pipeline now compiles to SQL.
    // =========================================================================

    #[tokio::test]
    async fn test_pg_compiles_bare_boolean_where() {
        let backend = create_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p-active", "active": true}),
                FhirVersion::R4,
            )
            .await
            .expect("seed active");
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p-inactive", "active": false}),
                FhirVersion::R4,
            )
            .await
            .expect("seed inactive");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{"path": "active"}],
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1, "only active=true patient should match");
    }

    #[tokio::test]
    async fn test_pg_compiles_exists_function_in_path() {
        let backend = create_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "X"}]}),
                FhirVersion::R4,
            )
            .await
            .expect("seed p1");
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p2"}),
                FhirVersion::R4,
            )
            .await
            .expect("seed p2");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "has_name"}]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 2);
    }
    /// Preserve the database's order and fail on every row error.
    async fn collect_rows_in_order(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        filters: ViewFilters,
    ) -> Vec<Value> {
        let mut stream = runner
            .run_view(tenant, view, filters)
            .await
            .expect("run view");
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row must succeed"));
        }
        rows
    }

    fn preview_flat_view(resource: &str, alias: &str) -> Value {
        json!({"resourceType":"ViewDefinition", "resource":resource,
            "select":[{"column":[{"path":"id","name":alias}]}]})
    }

    /// A dedicated PostgreSQL with `pg_stat_statements`, its backend, and a
    /// separate observer connection. Query aliases do not distinguish
    /// `pg_stat_statements` query IDs, so no other test may share it.
    struct ObservablePg {
        backend: Arc<PostgresBackend>,
        observer: tokio_postgres::Client,
        connection_task: tokio::task::JoinHandle<()>,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    impl Drop for ObservablePg {
        fn drop(&mut self) {
            self.connection_task.abort();
        }
    }

    async fn create_observable_backend() -> ObservablePg {
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default()
                .with_tag("16-alpine")
                .with_label(
                    "github.run_id",
                    std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
                )
                .with_cmd([
                    "postgres",
                    "-c",
                    "fsync=off",
                    "-c",
                    "shared_preload_libraries=pg_stat_statements",
                    "-c",
                    "compute_query_id=on",
                ]),
        )
        .start()
        .await
        .expect("start observable PostgreSQL");
        let host = container.get_host().await.unwrap().to_string();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let backend = PostgresBackend::new(PostgresConfig {
            host: host.clone(),
            port,
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            data_dir: Some(data_dir),
            ..Default::default()
        })
        .await
        .expect("create observable backend");
        backend
            .init_schema()
            .await
            .expect("initialize observable schema");
        let mut config = tokio_postgres::Config::new();
        config
            .host(&host)
            .port(port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (observer, connection) = config.connect(tokio_postgres::NoTls).await.unwrap();
        let connection_task = tokio::spawn(async move {
            connection.await.expect("observer connection");
        });
        observer
            .batch_execute("CREATE EXTENSION pg_stat_statements")
            .await
            .unwrap();
        ObservablePg {
            backend: Arc::new(backend),
            observer,
            connection_task,
            _container: container,
        }
    }

    /// One `pg_stat_statements` entry: normalized query text, calls, and the
    /// rows the server returned across those calls.
    type ObservedStatement = (String, i64, i64);

    /// Resets `pg_stat_statements`, runs one view, and returns its rows with
    /// the top-level statements matching `pattern` that it executed. The
    /// statistics are cumulative and normalize LIMIT literals, so only a
    /// reset before each run isolates that run's calls and server rows.
    async fn observe_isolated_run(
        observer: &tokio_postgres::Client,
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        filters: ViewFilters,
        pattern: &str,
    ) -> (Vec<Value>, Vec<ObservedStatement>) {
        observer
            .batch_execute("SELECT pg_stat_statements_reset()")
            .await
            .expect("reset statement statistics");
        let rows = collect_rows_in_order(runner, tenant, view, filters).await;
        let mut statements = Vec::new();
        // The row producer has finished before the channel closes. A bounded
        // poll also accommodates statistics publication at query completion.
        for _ in 0..50 {
            statements = observer
                .query(
                    "SELECT query, calls, rows FROM pg_stat_statements \
                     WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                     AND toplevel AND query LIKE $1 AND query NOT LIKE '%pg_stat_statements%'",
                    &[&pattern],
                )
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1), row.get(2)))
                .collect::<Vec<ObservedStatement>>();
            if statements.iter().any(|(_, calls, _)| *calls >= 1) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        (rows, statements)
    }

    /// The isolated run executed exactly one statement, once; the server
    /// returned exactly the rows the runner yielded; and that statement ends
    /// in one final output LIMIT iff `final_limit`. Scalar selections'
    /// intrinsic `LIMIT … OFFSET …` is not an output LIMIT.
    fn assert_isolated_execution(
        statements: &[ObservedStatement],
        rows: usize,
        final_limit: bool,
        case: &str,
    ) {
        let [(sql, calls, server_rows)] = statements else {
            panic!("{case}: expected one executed statement, got {statements:?}");
        };
        assert_eq!(*calls, 1, "{case}: calls {statements:?}");
        assert_eq!(
            usize::try_from(*server_rows).unwrap(),
            rows,
            "{case}: server output rows must equal yielded rows: {sql}"
        );
        assert_eq!(
            has_normalized_final_limit(sql),
            final_limit,
            "{case}: final LIMIT: {sql}"
        );
        assert_eq!(
            output_limit_count(sql),
            usize::from(final_limit),
            "{case}: output LIMIT count: {sql}"
        );
    }

    /// `LIMIT $n` occurrences in normalized SQL not followed by `OFFSET`.
    fn output_limit_count(sql: &str) -> usize {
        let words: Vec<&str> = sql.split_whitespace().collect();
        words
            .windows(2)
            .enumerate()
            .filter(|(at, pair)| {
                pair[0] == "LIMIT"
                    && pair[1].starts_with('$')
                    && words.get(at + 2) != Some(&"OFFSET")
            })
            .count()
    }

    #[tokio::test]
    async fn test_pg_preview_limit_is_in_executed_sql_and_none_is_unlimited() {
        let pg = create_observable_backend().await;
        let backend = &pg.backend;
        let observer = &pg.observer;
        let tenant = test_tenant();
        for index in 0..80 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType":"Patient", "id":format!("p-{index:03}")
                    }),
                    FhirVersion::R4,
                )
                .await
                .expect("seed patient");
        }
        let runner = backend.sof_runner().unwrap();
        let alias = format!("sof_pg_sql_limit_{}", uuid::Uuid::new_v4().simple());
        let view = preview_flat_view("Patient", &alias);
        let pattern = format!("%\"{alias}\"%");
        let (unlimited, statements) = observe_isolated_run(
            observer,
            runner.as_ref(),
            &tenant,
            view.clone(),
            ViewFilters::default(),
            &pattern,
        )
        .await;
        assert_eq!(unlimited.len(), 80);
        assert_isolated_execution(&statements, 80, false, "flat unlimited");
        let (limited, statements) = observe_isolated_run(
            observer,
            runner.as_ref(),
            &tenant,
            view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
            &pattern,
        )
        .await;
        assert_eq!(limited, unlimited[..50]);
        assert_isolated_execution(&statements, 50, true, "flat limited");
        assert!(
            !statements[0].0.contains("MATERIALIZED"),
            "executed preview SQL: {statements:?}"
        );
        // #1623: an expansion now executes its own final output LIMIT too.
        // Each isolated call shows the LIMIT (or its absence) and the server
        // returns only the rows the runner yields.
        backend
            .create(&tenant, "Patient", large_patient_fixture(), FhirVersion::R4)
            .await
            .expect("seed observable expansion");
        let complex_alias = format!("{alias}_expanded");
        let complex_view = json!({"resourceType":"ViewDefinition","resource":"Patient",
            "select":[{"column":[{"path":"id","name":complex_alias}]},
                {"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        let pattern = format!("%\"{complex_alias}\"%");
        let (unlimited, statements) = observe_isolated_run(
            observer,
            runner.as_ref(),
            &tenant,
            complex_view.clone(),
            ViewFilters::default(),
            &pattern,
        )
        .await;
        assert_eq!(unlimited.len(), 150);
        assert_isolated_execution(&statements, 150, false, "expansion unlimited");
        let (limited, limited_statements) = observe_isolated_run(
            observer,
            runner.as_ref(),
            &tenant,
            complex_view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
            &pattern,
        )
        .await;
        assert_eq!(limited, unlimited[..50]);
        assert_isolated_execution(&limited_statements, 50, true, "expansion limited");
        let (unlimited_sql, limited_sql) = (&statements[0].0, &limited_statements[0].0);
        for sql in [unlimited_sql, limited_sql] {
            assert!(
                !sql.contains("MATERIALIZED")
                    && sql.starts_with("SELECT")
                    && sql.contains("ORDER BY r.last_updated, r.id"),
                "expansion SQL keeps its deterministic order: {sql}"
            );
        }
        // The limited statement is the unlimited one plus its final LIMIT.
        let (body, limit) = limited_sql
            .rsplit_once("LIMIT")
            .expect("limited statement has a LIMIT");
        assert_eq!(body.trim_end(), unlimited_sql.trim_end());
        assert!(limit.trim().starts_with('$'), "{limited_sql}");
    }

    /// Limits 0, 1, 50, one above every fixture's row count, and (64-bit)
    /// an oversized `usize` with the SQL integer it would need, if any.
    fn complex_limit_matrix() -> Vec<(usize, bool)> {
        let mut limits = vec![(0, true), (1, true), (50, true), (10_000, true)];
        #[cfg(target_pointer_width = "64")]
        limits.push((usize::MAX, false));
        limits
    }

    /// Runs `view` unlimited and under every [`complex_limit_matrix`] limit,
    /// each as an isolated observed call. Every limited result is exactly
    /// the unlimited prefix; every representable limit executes one final
    /// output LIMIT and the server returns only the yielded rows; unlimited
    /// and oversized calls execute none. Returns the unlimited rows.
    async fn assert_complex_final_limits(
        pg: &ObservablePg,
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: &Value,
        filters: &ViewFilters,
        total: usize,
        case: &str,
    ) -> Vec<Value> {
        let pattern = "%FROM resources r%";
        let (unlimited, statements) = observe_isolated_run(
            &pg.observer,
            runner,
            tenant,
            view.clone(),
            filters.clone(),
            pattern,
        )
        .await;
        assert_eq!(unlimited.len(), total, "{case}: unlimited count");
        assert_isolated_execution(&statements, total, false, &format!("{case}: unlimited"));
        for (limit, representable) in complex_limit_matrix() {
            let (limited, statements) = observe_isolated_run(
                &pg.observer,
                runner,
                tenant,
                view.clone(),
                ViewFilters {
                    limit: Some(limit),
                    ..filters.clone()
                },
                pattern,
            )
            .await;
            let expected = limit.min(total);
            assert_eq!(
                limited,
                unlimited[..expected],
                "{case}: limit {limit} must yield the unlimited prefix"
            );
            assert_isolated_execution(
                &statements,
                expected,
                representable,
                &format!("{case}: limit {limit}"),
            );
        }
        unlimited
    }

    #[tokio::test]
    async fn test_pg_complex_final_limits_0_1_50_large() {
        let pg = create_observable_backend().await;
        let backend = pg.backend.clone();
        let runner = backend.sof_runner().unwrap();

        // Shape matrix: one large resource per type plus small resources, so
        // every shape yields more than 50 rows and each cut falls inside it.
        let tenant = test_tenant();
        backend
            .create(&tenant, "Patient", large_patient_fixture(), FhirVersion::R4)
            .await
            .expect("seed large patient");
        for index in 0..60 {
            let mut patient = json!({"resourceType":"Patient","id":format!("px-{index:03}")});
            if index % 2 == 0 {
                patient["name"] = json!([
                    {"family":format!("Px-{index:03}-a"),"given":["g"]},
                    {"family":format!("Px-{index:03}-b")}
                ]);
            }
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed small patient");
        }
        let large_qr = |id: &str, status: &str, subject: &str| {
            json!({"resourceType":"QuestionnaireResponse","id":id,"status":status,
                "subject":{"reference":subject},
                "item":(1..=150).map(|index| json!({
                    "linkId":format!("{id}-Item-{index}"),
                    "answer":[{"valueString":format!("Answer-{index}"),
                        "item":[{"linkId":format!("{id}-Child-{index}")}]}]
                })).collect::<Vec<_>>()})
        };
        backend
            .create(
                &tenant,
                "QuestionnaireResponse",
                large_qr("qr-large", "completed", "Patient/p-large"),
                FhirVersion::R4,
            )
            .await
            .expect("seed large questionnaire response");
        let tie =
            |value: &str| json!([{"path":"'tie'","name":"tie"},{"path":value,"name":"value"}]);
        let patient = |select: Value| json!({"resourceType":"ViewDefinition","resource":"Patient","select":select});
        let qr = |select: Value| {
            json!({"resourceType":"ViewDefinition","resource":"QuestionnaireResponse",
                "select":select})
        };
        let shapes = [
            (
                "nullable-forEachOrNull",
                patient(json!([{"column":[{"path":"id","name":"id"}]},
                    {"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}])),
                150 + 30 * 2 + 30,
            ),
            (
                "cartesian",
                patient(json!([{"column":[{"path":"id","name":"id"}]},
                    {"forEach":"name","column":[{"path":"family","name":"family"}]},
                    {"forEach":"address","column":[{"path":"city","name":"city"}]}])),
                150 * 10,
            ),
            (
                "union-equal-first-column",
                patient(json!([{"unionAll":[
                    {"column":tie("id")},
                    {"forEach":"name","column":tie("family")}
                ]}])),
                61 + 150 + 30 * 2,
            ),
            (
                "union-with-repeat-branch",
                qr(json!([{"unionAll":[
                    {"repeat":["item"],"column":tie("linkId")},
                    {"column":tie("id")}
                ]}])),
                150 + 1,
            ),
            (
                "multi-path-repeat",
                qr(json!([{"repeat":["item","answer.item"],"column":tie("linkId")}])),
                300,
            ),
            (
                "indexed",
                patient(json!([{"column":[{"path":"id","name":"id"}]},
                    {"forEachOrNull":"name[1]","column":[{"path":"family","name":"family"},
                        {"path":"%rowIndex","name":"index","type":"integer"}]}])),
                61,
            ),
        ];
        for (case, view, total) in &shapes {
            assert_complex_final_limits(
                &pg,
                runner.as_ref(),
                &tenant,
                view,
                &ViewFilters::default(),
                *total,
                case,
            )
            .await;
        }

        // Constants, `_since`, Patient compartment, tenant isolation and
        // deleted resources combined with every limit, for union and repeat.
        let filtered = test_tenant();
        let other = test_tenant();
        let seed = |context: TenantContext, resource: Value| {
            let backend = backend.clone();
            async move {
                let resource_type = resource["resourceType"].as_str().unwrap().to_string();
                backend
                    .create(&context, &resource_type, resource, FhirVersion::R4)
                    .await
                    .expect("seed filtered fixture");
            }
        };
        let mut old_patient = large_patient_fixture();
        old_patient["id"] = json!("p-old");
        seed(filtered.clone(), old_patient).await;
        seed(
            filtered.clone(),
            large_qr("qr-old", "completed", "Patient/p-large"),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let since = chrono::Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        seed(filtered.clone(), large_patient_fixture()).await;
        let mut female = large_patient_fixture();
        female["id"] = json!("p-female");
        female["gender"] = json!("female");
        seed(filtered.clone(), female).await;
        let mut unlisted = large_patient_fixture();
        unlisted["id"] = json!("p-unlisted");
        seed(filtered.clone(), unlisted).await;
        let mut deleted = large_patient_fixture();
        deleted["id"] = json!("p-deleted");
        seed(filtered.clone(), deleted).await;
        for (id, status, subject) in [
            ("qr-large", "completed", "Patient/p-large"),
            ("qr-progress", "in-progress", "Patient/p-large"),
            ("qr-unlisted", "completed", "Patient/p-unlisted"),
            ("qr-deleted", "completed", "Patient/p-large"),
        ] {
            seed(filtered.clone(), large_qr(id, status, subject)).await;
        }
        backend
            .delete(&filtered, "Patient", "p-deleted")
            .await
            .expect("delete patient");
        backend
            .delete(&filtered, "QuestionnaireResponse", "qr-deleted")
            .await
            .expect("delete questionnaire response");
        // Another tenant reuses the eligible ids, subjects and time window.
        seed(other.clone(), large_patient_fixture()).await;
        seed(
            other.clone(),
            large_qr("qr-large", "completed", "Patient/p-large"),
        )
        .await;
        // The eligible patients again, as a Group resolved per tenant. The
        // other tenant reuses the Group id for a patient nobody references.
        let group = |id: &str, members: &[&str]| {
            json!({"resourceType":"Group","id":id,"type":"person","actual":true,
                "member":members.iter().map(|member| json!({"entity":{"reference":
                    format!("Patient/{member}")}})).collect::<Vec<_>>()})
        };
        seed(
            filtered.clone(),
            group("g-eligible", &["p-large", "p-old", "p-female", "p-deleted"]),
        )
        .await;
        seed(other.clone(), group("g-eligible", &["p-nobody"])).await;
        seed(other.clone(), group("g-other", &["p-large"])).await;

        let with_constant = |mut view: Value, name: &str, value: &str, path: &str| {
            view["constant"] = json!([{"name":name,"valueString":value}]);
            view["where"] = json!([{"path":path}]);
            view
        };
        let patient_union = with_constant(
            patient(json!([{"unionAll":[
                {"column":tie("id")},
                {"forEach":"name","column":tie("family")}
            ]}])),
            "g",
            "male",
            "gender = %g",
        );
        let qr_repeat = with_constant(
            qr(json!([{"repeat":["item","answer.item"],"column":tie("linkId")}])),
            "s",
            "completed",
            "status = %s",
        );
        let qr_repeat_union = with_constant(
            qr(json!([{"unionAll":[
                {"repeat":["item"],"column":tie("linkId")},
                {"column":tie("id")}
            ]}])),
            "s",
            "completed",
            "status = %s",
        );
        let filters = ViewFilters {
            since: Some(since),
            patient: ["p-large", "p-old", "p-female", "p-deleted"]
                .map(|id| format!("Patient/{id}"))
                .to_vec(),
            ..Default::default()
        };
        let group_filters = |group: &str| ViewFilters {
            since: Some(since),
            group: vec![format!("Group/{group}")],
            ..Default::default()
        };
        let filtered_cases = [
            (
                "filtered-union",
                &patient_union,
                1 + 150,
                "p-large",
                "Family-",
            ),
            ("filtered-repeat", &qr_repeat, 300, "", "qr-large-"),
            (
                "filtered-repeat-union",
                &qr_repeat_union,
                150 + 1,
                "qr-large",
                "qr-large-",
            ),
        ];
        for (case, view, total, resource_row, node_prefix) in filtered_cases {
            let unlimited = assert_complex_final_limits(
                &pg,
                runner.as_ref(),
                &filtered,
                view,
                &filters,
                total,
                case,
            )
            .await;
            // Resolving the Group yields exactly the same rows under every limit.
            let via_group = assert_complex_final_limits(
                &pg,
                runner.as_ref(),
                &filtered,
                view,
                &group_filters("g-eligible"),
                total,
                &format!("{case}-group"),
            )
            .await;
            assert_eq!(via_group, unlimited, "{case}: Group and Patient filters");
            // Only the eligible, live, same-tenant resource contributes:
            // its own row (if any) and its nodes.
            assert!(
                unlimited.iter().all(|row| {
                    let value = row["value"].as_str().unwrap_or_default();
                    value == resource_row || value.starts_with(node_prefix)
                }),
                "{case}: {unlimited:?}"
            );
            assert_eq!(
                unlimited
                    .iter()
                    .filter(|row| !resource_row.is_empty() && row["value"] == resource_row)
                    .count(),
                usize::from(!resource_row.is_empty()),
                "{case}"
            );
        }
        // The other tenant sees only its own copy, also capped in SQL.
        assert_complex_final_limits(
            &pg,
            runner.as_ref(),
            &other,
            &qr_repeat,
            &filters,
            300,
            "other-tenant-repeat",
        )
        .await;
        // Group ids resolve in the caller's tenant only, under every limit.
        for (group, total) in [("g-other", 300), ("g-eligible", 0)] {
            assert_complex_final_limits(
                &pg,
                runner.as_ref(),
                &other,
                &qr_repeat,
                &group_filters(group),
                total,
                &format!("other-tenant-repeat-{group}"),
            )
            .await;
        }
        assert_complex_final_limits(
            &pg,
            runner.as_ref(),
            &other,
            &qr_repeat_union,
            &group_filters("g-other"),
            150 + 1,
            "other-tenant-repeat-union-g-other",
        )
        .await;
    }

    fn has_normalized_final_limit(sql: &str) -> bool {
        let mut words = sql.split_whitespace().rev();
        let Some(value) = words.next().and_then(|word| word.strip_prefix('$')) else {
            return false;
        };
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && words.next() == Some("LIMIT")
    }

    async fn seed_preview_patient(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        index: usize,
        gender: &str,
    ) {
        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType":"Patient", "id":format!("p-{index:03}"), "gender":gender,
                    "name":[{"family":format!("Family-{index:03}-a")},
                        {"family":format!("Family-{index:03}-b")},
                        {"family":format!("Family-{index:03}-c")}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed preview patient");
    }

    async fn assert_preview_prefix(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        expected_total: usize,
    ) {
        let unlimited =
            collect_rows_in_order(runner, tenant, view.clone(), ViewFilters::default()).await;
        assert_eq!(unlimited.len(), expected_total);
        let limited = collect_rows_in_order(
            runner,
            tenant,
            view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(limited.len(), 50);
        assert_eq!(
            limited,
            unlimited[..50],
            "preview must preserve the ordered output prefix"
        );
    }

    #[tokio::test]
    async fn test_pg_preview_limit_preserves_flat_observation_and_patient_prefix() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        for index in 0..80 {
            seed_preview_patient(&backend, &tenant, index, "male").await;
            backend.create(&tenant, "Observation", json!({
                "resourceType":"Observation", "id":format!("o-{index:03}"), "status":"final",
                "code":{"text":"preview fixture"}
            }), FhirVersion::R4).await.expect("seed observation");
        }
        let runner = backend.sof_runner().unwrap();
        for resource in ["Patient", "Observation"] {
            let view = preview_flat_view(resource, "id");
            assert_preview_prefix(runner.as_ref(), &tenant, view.clone(), 80).await;
            for (limit, expected) in [(0, 0), (1, 1), (500, 80)] {
                let rows = collect_rows_in_order(
                    runner.as_ref(),
                    &tenant,
                    view.clone(),
                    ViewFilters {
                        limit: Some(limit),
                        ..Default::default()
                    },
                )
                .await;
                assert_eq!(rows.len(), expected);
            }
        }
    }

    #[tokio::test]
    async fn test_pg_preview_limit_applies_after_where() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        for index in 0..120 {
            seed_preview_patient(
                &backend,
                &tenant,
                index,
                if index < 60 { "female" } else { "male" },
            )
            .await;
        }
        let runner = backend.sof_runner().unwrap();
        let mut view = preview_flat_view("Patient", "id");
        view["where"] = json!([{"path":"gender = 'male'"}]);
        assert_preview_prefix(runner.as_ref(), &tenant, view, 60).await;
    }

    #[tokio::test]
    async fn test_pg_preview_preserves_single_large_foreach_prefix() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient", "id": "p-large",
                    "name": (1..=150).map(|index| json!({"family": format!("Family-{index}")}))
                        .collect::<Vec<_>>()
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed large collection");
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        assert_preview_prefix(runner.as_ref(), &tenant, view.clone(), 150).await;
        let mut limits = vec![(0, 0), (1, 1), (150, 150), (10_000, 150)];
        #[cfg(target_pointer_width = "64")]
        limits.push((usize::MAX, 150));
        for (limit, expected) in limits {
            let rows = collect_rows_in_order(
                runner.as_ref(),
                &tenant,
                view.clone(),
                ViewFilters {
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(rows.len(), expected, "final output limit {limit}");
        }
    }

    #[tokio::test]
    async fn test_pg_preview_limit_preserves_foreach_prefix_with_interior_cut() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        for index in 0..20 {
            seed_preview_patient(&backend, &tenant, index, "male").await;
        }
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        assert_preview_prefix(runner.as_ref(), &tenant, view.clone(), 60).await;
        let limited = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            limited[48]["id"], limited[49]["id"],
            "cap must cut inside a three-name resource"
        );
        assert_ne!(limited[47]["id"], limited[49]["id"]);
    }

    #[tokio::test]
    async fn test_pg_preview_limit_is_global_across_union_all() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        for index in 0..40 {
            seed_preview_patient(&backend, &tenant, index, &format!("branch-b-{index:03}")).await;
        }
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
        "select":[{"unionAll":[
            {"column":[{"path":"id","name":"value"}]},
            {"column":[{"path":"gender","name":"value"}]}
        ]}]});
        assert_preview_prefix(runner.as_ref(), &tenant, view, 80).await;
    }

    #[tokio::test]
    async fn test_pg_preview_limit_preserves_constants_runtime_filters_and_tenant() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let other = TenantContext::new(
            TenantId::new(format!("other_{}", uuid::Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        );
        let since = chrono::Utc::now();
        for index in 0..81 {
            seed_preview_patient(
                &backend,
                &tenant,
                index,
                if index < 20 { "female" } else { "male" },
            )
            .await;
        }
        seed_preview_patient(&backend, &other, 20, "male").await;
        backend
            .delete(&tenant, "Patient", "p-021")
            .await
            .expect("delete patient");
        let runner = backend.sof_runner().unwrap();
        let mut view = preview_flat_view("Patient", "id");
        view["constant"] = json!([{"name":"g","valueString":"male"}]);
        view["where"] = json!([{"path":"gender = %g"}]);
        let mut filters = ViewFilters {
            since: Some(since),
            patient: (10..80)
                .map(|index| format!("Patient/p-{index:03}"))
                .collect(),
            ..Default::default()
        };
        let unlimited =
            collect_rows_in_order(runner.as_ref(), &tenant, view.clone(), filters.clone()).await;
        assert_eq!(unlimited.len(), 59);
        assert!(
            unlimited
                .iter()
                .all(|row| row["id"] != "p-021" && row["id"] != "p-080")
        );
        filters.limit = Some(50);
        let limited =
            collect_rows_in_order(runner.as_ref(), &tenant, view.clone(), filters.clone()).await;
        assert_eq!(limited, unlimited[..50]);
        // A future since filter must still exclude every otherwise eligible row.
        filters.since = Some(chrono::Utc::now() + chrono::Duration::days(1));
        assert!(
            collect_rows_in_order(runner.as_ref(), &tenant, view, filters)
                .await
                .is_empty()
        );
    }

    fn large_patient_fixture() -> Value {
        json!({
            "resourceType":"Patient", "id":"p-large", "gender":"male", "active":true,
            "name": (1..=150).map(|index| json!({
                "family":format!("Family-{index}"),
                "use":if index <= 75 { "official" } else { "temp" },
                "given":[format!("Given-{index}-a"),format!("Given-{index}-b")]
            })).collect::<Vec<_>>(),
            "address":(1..=10).map(|index| json!({"city":format!("City-{index}")})).collect::<Vec<_>>()
        })
    }

    async fn assert_large_preview_prefix(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        total: usize,
        case: &str,
    ) {
        let unlimited =
            collect_rows_in_order(runner, tenant, view.clone(), ViewFilters::default()).await;
        assert_eq!(unlimited.len(), total, "{case}: unlimited count");
        let limited = collect_rows_in_order(
            runner,
            tenant,
            view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(limited.len(), 50, "{case}: output cap");
        assert_eq!(limited, unlimited[..50], "{case}: ordered prefix");
    }

    #[tokio::test]
    async fn test_pg_large_nested_chained_cartesian_and_nullable_preview_prefixes() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        for resource in [
            large_patient_fixture(),
            json!({"resourceType":"Patient","id":"p-empty"}),
            json!({"resourceType":"Patient","id":"p-filtered","name":[{"family":"Rejected","use":"temp"}]}),
        ] {
            backend
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("seed expanded fixture");
        }
        let runner = backend.sof_runner().unwrap();
        let cases = [
            (
                "single-large",
                json!([{"forEach":"name","column":[{"path":"family","name":"family"}]}]),
                150,
            ),
            (
                "nested",
                json!([{"forEach":"name","select":[
                    {"column":[{"path":"family","name":"family"}]},
                    {"forEach":"given","column":[{"path":"$this","name":"given"}]}
                ]}]),
                300,
            ),
            (
                "chained",
                json!([{"forEach":"name.given","column":[{"path":"$this","name":"given"}]}]),
                300,
            ),
            (
                "cartesian",
                json!([
                    {"forEach":"name","column":[{"path":"family","name":"family"}]},
                    {"forEach":"address","column":[{"path":"city","name":"city"}]}
                ]),
                1500,
            ),
            (
                "nullable",
                json!([{"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
                152,
            ),
            (
                "nullable-where-on",
                json!([{"forEachOrNull":"name.where(use = 'official')",
                "column":[{"path":"family","name":"family"}]}]),
                77,
            ),
            (
                "row-index",
                json!([{"forEach":"name","column":[
                    {"path":"family","name":"family"},{"path":"%rowIndex","name":"index","type":"integer"}
                ]}]),
                150,
            ),
            (
                "expanded-union-ties",
                json!([{"unionAll":[
                    {"forEach":"name","column":[{"path":"'tie'","name":"tie"},{"path":"family","name":"value"}]},
                    {"forEach":"name","column":[{"path":"'tie'","name":"tie"},{"path":"given[0]","name":"value"}]}
                ]}]),
                300,
            ),
            (
                "outer-foreach-union",
                json!([{"forEach":"name","unionAll":[
                    {"column":[{"path":"'tie'","name":"tie"},{"path":"family","name":"value"}]},
                    {"forEach":"given","column":[{"path":"'tie'","name":"tie"},{"path":"$this","name":"value"}]}
                ]}]),
                450,
            ),
        ];
        for (case, select, total) in cases {
            let mut view =
                json!({"resourceType":"ViewDefinition","resource":"Patient","select":select});
            // Nullable cases include absent/rejected collections; the others
            // isolate the large resource so every output sort key can tie.
            if !case.starts_with("nullable") {
                view["where"] = json!([{"path":"id = 'p-large'"}]);
            }
            assert_large_preview_prefix(runner.as_ref(), &tenant, view, total, case).await;
        }
    }

    #[tokio::test]
    async fn test_pg_flat_union_ties_keep_second_column_prefix() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        for index in 0..80 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType":"Patient","id":format!("u-{index:03}"),"gender":format!("Second-{index}")}),
                    FhirVersion::R4,
                )
                .await
                .expect("seed union");
        }
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition","resource":"Patient",
        "select":[{"unionAll":[
            {"column":[{"path":"'tie'","name":"tie"},{"path":"id","name":"value"}]},
            {"column":[{"path":"'tie'","name":"tie"},{"path":"gender","name":"value"}]}
        ]}]});
        assert_large_preview_prefix(runner.as_ref(), &tenant, view, 160, "flat-union-ties").await;
    }

    #[tokio::test]
    async fn test_pg_large_repeat_nested_multipath_and_union_preview_prefixes() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        let large_qr = json!({
            "resourceType":"QuestionnaireResponse", "id":"qr-large", "status":"completed",
            "item":(1..=150).map(|index| json!({
                "linkId":format!("Item-{index}"),
                "answer":[{"valueString":format!("Answer-{index}"),"item":[{"linkId":format!("Child-{index}")}]}]
            })).collect::<Vec<_>>()
        });
        backend
            .create(
                &tenant,
                "QuestionnaireResponse",
                large_qr.clone(),
                FhirVersion::R4,
            )
            .await
            .expect("seed repeat");
        let runner = backend.sof_runner().unwrap();
        let descending_row_index = json!([{"repeat":["item","answer.item"],"column":[
            {"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"},
            {"path":"%rowIndex","name":"index","type":"integer"}]}]);
        let cases = [
            (
                "repeat",
                json!([{"repeat":["item"],"column":[
                {"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"}]}]),
                150,
            ),
            (
                "repeat-nested",
                json!([{"repeat":["item"],"select":[
                    {"column":[{"path":"'tie'","name":"tie"},{"path":"linkId","name":"item"}]},
                    {"forEachOrNull":"answer","column":[{"path":"valueString","name":"answer"}]}
                ]}]),
                150,
            ),
            (
                "repeat-multipath",
                json!([{"repeat":["item","answer.item"],"column":[
                {"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"}]}]),
                300,
            ),
            (
                "repeat-union",
                json!([{"unionAll":[
                    {"repeat":["item"],"column":[{"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"}]},
                    {"repeat":["item","answer.item"],"column":[{"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"}]}
                ]}]),
                450,
            ),
            (
                "repeat-row-index",
                json!([{"repeat":["item"],"column":[
                {"path":"'tie'","name":"tie"},{"path":"linkId","name":"value"},
                {"path":"%rowIndex","name":"index","type":"integer"}]}]),
                150,
            ),
            (
                // #1623: the case above never descends (the children live
                // under `answer.item`); this one does, so its indices are
                // not just the item positions.
                "repeat-row-index-descends",
                descending_row_index.clone(),
                300,
            ),
        ];
        for (case, select, total) in cases {
            assert_large_preview_prefix(runner.as_ref(), &tenant, json!({
                "resourceType":"ViewDefinition","resource":"QuestionnaireResponse","select":select
            }), total, case).await;
        }
        // The descending indices equal the evaluator's, in its order (the
        // constant first column leaves traversal order as the tie-break).
        let view = json!({"resourceType":"ViewDefinition","resource":"QuestionnaireResponse",
            "status":"active","select":descending_row_index});
        let rows = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            view.clone(),
            ViewFilters::default(),
        )
        .await;
        assert_eq!(rows, evaluator_rows(&view, &[large_qr]));
    }

    #[tokio::test]
    async fn test_pg_large_expansion_preserves_runtime_filters_constants_and_isolation() {
        let (backend, _container) = create_dedicated_backend().await;
        let tenant = test_tenant();
        let other = TenantContext::new(
            TenantId::new(format!("other-{}", uuid::Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        );
        let since = chrono::Utc::now() - chrono::Duration::seconds(1);
        for context in [&tenant, &other] {
            backend
                .create(context, "Patient", large_patient_fixture(), FhirVersion::R4)
                .await
                .expect("seed eligible expansion");
        }
        let mut deleted = large_patient_fixture();
        deleted["id"] = json!("p-deleted");
        backend
            .create(&tenant, "Patient", deleted, FhirVersion::R4)
            .await
            .expect("seed deleted expansion");
        backend
            .delete(&tenant, "Patient", "p-deleted")
            .await
            .expect("delete expansion");
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition","resource":"Patient",
            "constant":[{"name":"g","valueString":"male"}],"where":[{"path":"gender = %g"}],
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        let mut filters = ViewFilters {
            since: Some(since),
            patient: vec!["Patient/p-large".into(), "Patient/p-deleted".into()],
            ..Default::default()
        };
        let unlimited =
            collect_rows_in_order(runner.as_ref(), &tenant, view.clone(), filters.clone()).await;
        assert_eq!(unlimited.len(), 150);
        assert!(unlimited.iter().all(|row| row["id"] == "p-large"));
        filters.limit = Some(50);
        let limited =
            collect_rows_in_order(runner.as_ref(), &tenant, view.clone(), filters.clone()).await;
        assert_eq!(limited, unlimited[..50]);
        filters.since = Some(chrono::Utc::now() + chrono::Duration::days(1));
        assert!(
            collect_rows_in_order(runner.as_ref(), &tenant, view.clone(), filters.clone())
                .await
                .is_empty()
        );
        filters.since = Some(since);
        filters.patient = vec!["Patient/missing".into()];
        assert!(
            collect_rows_in_order(runner.as_ref(), &tenant, view, filters)
                .await
                .is_empty()
        );
    }

    // =========================================================================
    // #1623: runtime resource predicates are lowered into every resource scan
    // (each unionAll branch, each recursive seed), with slots allocated once.
    // =========================================================================

    fn runtime_filter_qr(id: &str, status: &str, subject: &str, items: Value) -> Value {
        json!({"resourceType":"QuestionnaireResponse", "id":id, "status":status,
            "subject":{"reference":subject}, "item":items})
    }

    /// Seeds the runtime-filter fixture and returns the `_since` instant that
    /// separates the old resource (`qr-a`, `pt-a`) from the newer ones.
    async fn seed_runtime_filter_fixture(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        other: &TenantContext,
    ) -> chrono::DateTime<chrono::Utc> {
        let old = [
            runtime_filter_qr(
                "qr-a",
                "completed",
                "Patient/pa",
                json!([{"linkId":"a1","item":[{"linkId":"a1.1"}]},{"linkId":"a2"}]),
            ),
            json!({"resourceType":"Patient","id":"pa","name":[{"family":"fam-pa"}]}),
        ];
        for resource in old {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            backend
                .create(tenant, &resource_type, resource, FhirVersion::R4)
                .await
                .expect("seed old resource");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let since = chrono::Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let new = [
            runtime_filter_qr(
                "qr-b",
                "completed",
                "Patient/pb",
                json!([{"linkId":"b1","item":[{"linkId":"b1.1"}]}]),
            ),
            runtime_filter_qr("qr-c", "completed", "Patient/pa", json!([{"linkId":"c1"}])),
            runtime_filter_qr("qr-d", "completed", "Patient/pa", json!([{"linkId":"d1"}])),
            runtime_filter_qr(
                "qr-e",
                "in-progress",
                "Patient/pa",
                json!([{"linkId":"e1"}]),
            ),
            json!({"resourceType":"Patient","id":"pb","name":[{"family":"fam-pb"}]}),
            json!({"resourceType":"Group","id":"g1","type":"person","actual":true,
                "member":[{"entity":{"reference":"Patient/pa"}}]}),
        ];
        for resource in new {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            backend
                .create(tenant, &resource_type, resource, FhirVersion::R4)
                .await
                .expect("seed new resource");
        }
        backend
            .delete(tenant, "QuestionnaireResponse", "qr-d")
            .await
            .expect("delete qr-d");
        // Another tenant reuses an eligible id, subject and timestamp window.
        backend
            .create(
                other,
                "QuestionnaireResponse",
                runtime_filter_qr("qr-c", "completed", "Patient/pa", json!([{"linkId":"o1"}])),
                FhirVersion::R4,
            )
            .await
            .expect("seed other tenant");
        since
    }

    fn runtime_filter_rows(values: &[(&str, &str)]) -> Vec<Value> {
        values
            .iter()
            .map(|(v, kind)| json!({"v":v, "kind":kind}))
            .collect()
    }

    #[tokio::test]
    async fn test_pg_union_and_repeat_runtime_filters_lower_into_every_scan() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let other = TenantContext::new(
            TenantId::new(format!("other_{}", uuid::Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        );
        let since = seed_runtime_filter_fixture(&backend, &tenant, &other).await;
        let runner = backend.sof_runner().unwrap();
        let qr_view = |select: Value| {
            json!({"resourceType":"ViewDefinition",
            "resource":"QuestionnaireResponse",
            "constant":[{"name":"s","valueString":"completed"}],
            "where":[{"path":"status = %s"}], "select":select})
        };
        // The flat branch precedes the expanded branch: a predicate spliced
        // before the final ORDER BY would only constrain the last branch.
        let union = qr_view(json!([{"unionAll":[
            {"column":[{"path":"id","name":"v"},{"path":"'resource'","name":"kind"}]},
            {"forEach":"item","column":[{"path":"linkId","name":"v"},{"path":"'item'","name":"kind"}]}
        ]}]));
        let repeat = qr_view(json!([{"repeat":["item"],
            "column":[{"path":"linkId","name":"v"},{"path":"'node'","name":"kind"}]}]));
        let repeat_union = qr_view(json!([{"unionAll":[
            {"repeat":["item"],"column":[{"path":"linkId","name":"v"},{"path":"'node'","name":"kind"}]},
            {"column":[{"path":"id","name":"v"},{"path":"'resource'","name":"kind"}]}
        ]}]));
        let patient_union = json!({"resourceType":"ViewDefinition","resource":"Patient",
        "select":[{"unionAll":[
            {"column":[{"path":"id","name":"v"},{"path":"'resource'","name":"kind"}]},
            {"forEach":"name","column":[{"path":"family","name":"v"},{"path":"'name'","name":"kind"}]}
        ]}]});
        let patient_pa = ViewFilters {
            patient: vec!["Patient/pa".into()],
            ..Default::default()
        };
        let group_g1 = ViewFilters {
            group: vec!["Group/g1".into()],
            ..Default::default()
        };
        let since_only = ViewFilters {
            since: Some(since),
            ..Default::default()
        };
        let since_and_patient = ViewFilters {
            since: Some(since),
            patient: vec!["Patient/pa".into()],
            ..Default::default()
        };
        let cases = [
            (
                "union/patient",
                &union,
                &patient_pa,
                runtime_filter_rows(&[
                    ("a1", "item"),
                    ("a2", "item"),
                    ("c1", "item"),
                    ("qr-a", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "union/group",
                &union,
                &group_g1,
                runtime_filter_rows(&[
                    ("a1", "item"),
                    ("a2", "item"),
                    ("c1", "item"),
                    ("qr-a", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "union/since",
                &union,
                &since_only,
                runtime_filter_rows(&[
                    ("b1", "item"),
                    ("c1", "item"),
                    ("qr-b", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "union/since+patient",
                &union,
                &since_and_patient,
                runtime_filter_rows(&[("c1", "item"), ("qr-c", "resource")]),
            ),
            (
                "repeat/patient",
                &repeat,
                &patient_pa,
                runtime_filter_rows(&[
                    ("a1", "node"),
                    ("a1.1", "node"),
                    ("a2", "node"),
                    ("c1", "node"),
                ]),
            ),
            (
                "repeat/group",
                &repeat,
                &group_g1,
                runtime_filter_rows(&[
                    ("a1", "node"),
                    ("a1.1", "node"),
                    ("a2", "node"),
                    ("c1", "node"),
                ]),
            ),
            (
                "repeat/since",
                &repeat,
                &since_only,
                runtime_filter_rows(&[("b1", "node"), ("b1.1", "node"), ("c1", "node")]),
            ),
            (
                "repeat-union/patient",
                &repeat_union,
                &patient_pa,
                runtime_filter_rows(&[
                    ("a1", "node"),
                    ("a1.1", "node"),
                    ("a2", "node"),
                    ("c1", "node"),
                    ("qr-a", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "repeat-union/group",
                &repeat_union,
                &group_g1,
                runtime_filter_rows(&[
                    ("a1", "node"),
                    ("a1.1", "node"),
                    ("a2", "node"),
                    ("c1", "node"),
                    ("qr-a", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "repeat-union/since",
                &repeat_union,
                &since_only,
                runtime_filter_rows(&[
                    ("b1", "node"),
                    ("b1.1", "node"),
                    ("c1", "node"),
                    ("qr-b", "resource"),
                    ("qr-c", "resource"),
                ]),
            ),
            (
                "repeat-union/since+patient",
                &repeat_union,
                &since_and_patient,
                runtime_filter_rows(&[("c1", "node"), ("qr-c", "resource")]),
            ),
            (
                "patient-union/patient",
                &patient_union,
                &patient_pa,
                runtime_filter_rows(&[("fam-pa", "name"), ("pa", "resource")]),
            ),
            (
                "patient-union/since",
                &patient_union,
                &since_only,
                runtime_filter_rows(&[("fam-pb", "name"), ("pb", "resource")]),
            ),
        ];
        // Report every failing case at once.
        let mut failures = Vec::new();
        for (case, view, filters, expected) in cases {
            let rows = match runner
                .run_view(&tenant, view.clone(), filters.clone())
                .await
            {
                Ok(mut stream) => {
                    let mut rows = Vec::new();
                    while let Some(row) = stream.next().await {
                        rows.push(row);
                    }
                    rows.into_iter().collect::<Result<Vec<_>, _>>()
                }
                Err(error) => Err(error),
            };
            match rows {
                Ok(rows) if rows == expected => {}
                Ok(rows) => failures.push(format!("{case}: got {rows:?}, expected {expected:?}")),
                Err(error) => failures.push(format!("{case}: error {error}")),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        // Without runtime filters every live, same-tenant row is returned.
        let unfiltered = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            repeat.clone(),
            ViewFilters::default(),
        )
        .await;
        assert_eq!(
            unfiltered,
            runtime_filter_rows(&[
                ("a1", "node"),
                ("a1.1", "node"),
                ("a2", "node"),
                ("b1", "node"),
                ("b1.1", "node"),
                ("c1", "node")
            ])
        );
    }

    // =========================================================================
    // #1623: frozen ordering contract — hand-derived full-order oracles.
    //
    // Ordinary/expanded: last_updated, id, every occurrence ordinal (a
    // `forEachOrNull` miss is -1). Union: first visible column (NULLS LAST on
    // PostgreSQL), last_updated, id, common enclosing ordinals, flattened
    // branch number, branch-local identity. Fixture timestamps are set
    // explicitly so no oracle depends on insertion timing.
    // =========================================================================

    /// One expected output row. PostgreSQL retains SQL NULLs as JSON null.
    fn row(pairs: &[(&str, Value)]) -> Value {
        Value::Object(
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
        )
    }

    /// The unlimited run equals `expected` exactly (order and multiplicity);
    /// each capped run equals the corresponding prefix.
    async fn assert_order_oracle(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        expected: &[Value],
        case: &str,
    ) {
        // The evaluator omits absent cells, while PostgreSQL now retains
        // every projected key (#1769). Keep exact row comparisons by filling
        // only missing declared cells in the oracle, never in actual output.
        let columns = helios_sof::TableSchema::sql_output_layout(&view).column_names();
        let expected: Vec<Value> = expected
            .iter()
            .map(|wanted| {
                let mut wanted = wanted.clone();
                let cells = wanted.as_object_mut().expect("oracle row object");
                for column in &columns {
                    cells.entry(column.clone()).or_insert(Value::Null);
                }
                wanted
            })
            .collect();
        let unlimited =
            collect_rows_in_order(runner, tenant, view.clone(), ViewFilters::default()).await;
        assert_eq!(unlimited.len(), expected.len(), "{case}: row count");
        for (index, (actual, wanted)) in unlimited.iter().zip(&expected).enumerate() {
            assert_eq!(actual, wanted, "{case}: row {index}");
        }
        for limit in [0usize, 1, 50] {
            let limited = collect_rows_in_order(
                runner,
                tenant,
                view.clone(),
                ViewFilters {
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(
                limited,
                expected[..limit.min(expected.len())],
                "{case}: limit {limit} prefix"
            );
        }
    }

    /// Fixture-only rewrite of `resources.last_updated`, scoped by tenant,
    /// type and id, through a direct connection to the shared database.
    async fn set_last_updated(
        tenant: &TenantContext,
        resource_type: &str,
        updates: &[(&str, &str)],
    ) {
        let pg = shared_pg().await;
        let mut config = tokio_postgres::Config::new();
        config
            .host(&pg.host)
            .port(pg.port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect fixture client");
        let connection_task = tokio::spawn(async move {
            connection.await.expect("fixture connection");
        });
        for (id, at) in updates {
            let at: chrono::DateTime<chrono::Utc> = at.parse().expect("fixture timestamp");
            let changed = client
                .execute(
                    "UPDATE resources SET last_updated = $1 \
                     WHERE tenant_id = $2 AND resource_type = $3 AND id = $4",
                    &[&at, &tenant.tenant_id().as_str(), &resource_type, id],
                )
                .await
                .expect("rewrite fixture timestamp");
            assert_eq!(changed, 1, "{resource_type}/{id}");
        }
        drop(client);
        connection_task.abort();
    }

    fn patient_view(select: Value, only: Option<&str>) -> Value {
        let mut view =
            json!({"resourceType":"ViewDefinition","resource":"Patient","select":select});
        if let Some(id) = only {
            view["where"] = json!([{"path": format!("id = '{id}'")}]);
        }
        view
    }

    #[tokio::test]
    async fn test_pg_ordering_contract_large_fixture_oracles() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        for resource in [
            large_patient_fixture(),
            json!({"resourceType":"Patient","id":"p-empty"}),
            json!({"resourceType":"Patient","id":"p-filtered","name":[{"family":"Rejected","use":"temp"}]}),
        ] {
            backend
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("seed ordering fixture");
        }
        set_last_updated(
            &tenant,
            "Patient",
            &[
                ("p-large", "2024-01-01T00:00:01Z"),
                ("p-empty", "2024-01-01T00:00:02Z"),
                ("p-filtered", "2024-01-01T00:00:03Z"),
            ],
        )
        .await;
        let runner = backend.sof_runner().unwrap();
        let s = |v: String| json!(v);
        let tie = || json!("tie");

        // The exact nullable fixture: family-only forEachOrNull. p-empty's
        // synthetic miss row follows p-large's 150 occurrences.
        let mut nullable: Vec<Value> = (1..=150)
            .map(|i| row(&[("family", s(format!("Family-{i}")))]))
            .collect();
        nullable.push(row(&[("family", Value::Null)]));
        nullable.push(row(&[("family", json!("Rejected"))]));

        let mut nullable_on: Vec<Value> = (1..=75)
            .map(|i| {
                row(&[
                    ("id", json!("p-large")),
                    ("family", s(format!("Family-{i}"))),
                ])
            })
            .collect();
        nullable_on.push(row(&[("id", json!("p-empty")), ("family", Value::Null)]));
        nullable_on.push(row(&[("id", json!("p-filtered")), ("family", Value::Null)]));

        let cartesian: Vec<Value> = (1..=150)
            .flat_map(|i| {
                (1..=10).map(move |j| {
                    row(&[
                        ("family", json!(format!("Family-{i}"))),
                        ("city", json!(format!("City-{j}"))),
                    ])
                })
            })
            .collect();
        let nested: Vec<Value> = (1..=150)
            .flat_map(|i| {
                ["a", "b"].map(|g| {
                    row(&[
                        ("family", json!(format!("Family-{i}"))),
                        ("given", json!(format!("Given-{i}-{g}"))),
                    ])
                })
            })
            .collect();
        let chained: Vec<Value> = (1..=150)
            .flat_map(|i| ["a", "b"].map(|g| row(&[("given", json!(format!("Given-{i}-{g}")))])))
            .collect();
        // Equal first visible column: branch 0 (all families, by occurrence)
        // precedes branch 1 within the one resource.
        let union_equal_first: Vec<Value> = (1..=150)
            .map(|i| row(&[("tie", tie()), ("value", s(format!("Family-{i}")))]))
            .chain((1..=150).map(|i| row(&[("tie", tie()), ("value", s(format!("Given-{i}-a")))])))
            .collect();
        // Shared `forEach: name` orders before the branch number: per name,
        // its family row, then its given rows.
        let outer_union: Vec<Value> = (1..=150)
            .flat_map(|i| {
                [
                    format!("Family-{i}"),
                    format!("Given-{i}-a"),
                    format!("Given-{i}-b"),
                ]
                .map(|value| row(&[("tie", json!("tie")), ("value", json!(value))]))
            })
            .collect();
        // A flat branch mixed with a two-level expansion branch, across
        // resources: resource key first, then branch, then (name, given).
        let mut mixed_union = vec![row(&[("tie", tie()), ("v", json!("p-large"))])];
        mixed_union.extend((1..=150).flat_map(|i| {
            ["a", "b"].map(|g| {
                row(&[
                    ("tie", json!("tie")),
                    ("v", json!(format!("Given-{i}-{g}"))),
                ])
            })
        }));
        mixed_union.push(row(&[("tie", tie()), ("v", json!("p-empty"))]));
        mixed_union.push(row(&[("tie", tie()), ("v", json!("p-filtered"))]));

        let cases = [
            (
                "nullable",
                patient_view(
                    json!([{"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
                    None,
                ),
                nullable,
            ),
            (
                "nullable-where-on",
                patient_view(
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"forEachOrNull":"name.where(use = 'official')",
                            "column":[{"path":"family","name":"family"}]}]),
                    None,
                ),
                nullable_on,
            ),
            (
                "cartesian",
                patient_view(
                    json!([
                        {"forEach":"name","column":[{"path":"family","name":"family"}]},
                        {"forEach":"address","column":[{"path":"city","name":"city"}]}
                    ]),
                    Some("p-large"),
                ),
                cartesian,
            ),
            (
                "nested",
                patient_view(
                    json!([{"forEach":"name","select":[
                        {"column":[{"path":"family","name":"family"}]},
                        {"forEach":"given","column":[{"path":"$this","name":"given"}]}
                    ]}]),
                    Some("p-large"),
                ),
                nested,
            ),
            (
                "chained",
                patient_view(
                    json!([{"forEach":"name.given","column":[{"path":"$this","name":"given"}]}]),
                    Some("p-large"),
                ),
                chained,
            ),
            (
                "union-equal-first-column",
                patient_view(
                    json!([{"unionAll":[
                        {"forEach":"name","column":[{"path":"'tie'","name":"tie"},{"path":"family","name":"value"}]},
                        {"forEach":"name","column":[{"path":"'tie'","name":"tie"},{"path":"given[0]","name":"value"}]}
                    ]}]),
                    Some("p-large"),
                ),
                union_equal_first,
            ),
            (
                "outer-foreach-union",
                patient_view(
                    json!([{"forEach":"name","unionAll":[
                        {"column":[{"path":"'tie'","name":"tie"},{"path":"family","name":"value"}]},
                        {"forEach":"given","column":[{"path":"'tie'","name":"tie"},{"path":"$this","name":"value"}]}
                    ]}]),
                    Some("p-large"),
                ),
                outer_union,
            ),
            (
                "mixed-flat-and-two-level-union",
                patient_view(
                    json!([{"unionAll":[
                        {"column":[{"path":"'tie'","name":"tie"},{"path":"id","name":"v"}]},
                        {"forEach":"name","select":[{"forEach":"given",
                            "column":[{"path":"'tie'","name":"tie"},{"path":"$this","name":"v"}]}]}
                    ]}]),
                    None,
                ),
                mixed_union,
            ),
        ];
        for (case, view, expected) in cases {
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, case).await;
        }
    }

    #[tokio::test]
    async fn test_pg_ordering_contract_tied_timestamps_and_iteration_sources() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        // Seeded in reverse id order, then given one shared timestamp: only
        // the id tie-break can produce t-a, t-b, t-c.
        for (id, gender, prefix) in [
            ("t-c", None, "C"),
            ("t-b", Some("male"), "B"),
            ("t-a", None, "A"),
        ] {
            let mut patient = json!({"resourceType":"Patient","id":id,
                "name":[{"family":format!("{prefix}-1")},{"family":format!("{prefix}-2")}]});
            if let Some(gender) = gender {
                patient["gender"] = json!(gender);
            }
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed tied patient");
        }
        let tied = "2024-02-02T00:00:00Z";
        set_last_updated(
            &tenant,
            "Patient",
            &[("t-c", tied), ("t-b", tied), ("t-a", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();
        let ids = ["t-a", "t-b", "t-c"];
        let prefixes = ["A", "B", "C"];

        let flat: Vec<Value> = ids.iter().map(|id| row(&[("id", json!(id))])).collect();
        let expanded: Vec<Value> = ids
            .iter()
            .zip(prefixes)
            .flat_map(|(id, p)| {
                [1, 2].map(|n| row(&[("id", json!(id)), ("family", json!(format!("{p}-{n}")))]))
            })
            .collect();
        let flat_union: Vec<Value> = ids
            .iter()
            .flat_map(|id| {
                [json!(id), json!("second")].map(|v| row(&[("tie", json!("tie")), ("v", v)]))
            })
            .collect();
        let expanded_union: Vec<Value> = ids
            .iter()
            .zip(prefixes)
            .flat_map(|(id, p)| {
                [json!(id), json!(format!("{p}-1")), json!(format!("{p}-2"))]
                    .map(|v| row(&[("tie", json!("tie")), ("v", v)]))
            })
            .collect();
        // PostgreSQL's default ASC places the NULL first-column rows last.
        let nullable_first_column = vec![
            row(&[("g", json!("male")), ("v", json!("t-b"))]),
            row(&[("g", json!("zz")), ("v", json!("t-a"))]),
            row(&[("g", json!("zz")), ("v", json!("t-b"))]),
            row(&[("g", json!("zz")), ("v", json!("t-c"))]),
            row(&[("g", Value::Null), ("v", json!("t-a"))]),
            row(&[("g", Value::Null), ("v", json!("t-c"))]),
        ];
        let cases = [
            (
                "tied-flat",
                patient_view(json!([{"column":[{"path":"id","name":"id"}]}]), None),
                flat,
            ),
            (
                "tied-expanded",
                patient_view(
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"forEach":"name","column":[{"path":"family","name":"family"}]}]),
                    None,
                ),
                expanded,
            ),
            (
                "tied-flat-union",
                patient_view(
                    json!([{"unionAll":[
                        {"column":[{"path":"'tie'","name":"tie"},{"path":"id","name":"v"}]},
                        {"column":[{"path":"'tie'","name":"tie"},{"path":"'second'","name":"v"}]}
                    ]}]),
                    None,
                ),
                flat_union,
            ),
            (
                "tied-expanded-union",
                patient_view(
                    json!([{"unionAll":[
                        {"column":[{"path":"'tie'","name":"tie"},{"path":"id","name":"v"}]},
                        {"forEach":"name","column":[{"path":"'tie'","name":"tie"},{"path":"family","name":"v"}]}
                    ]}]),
                    None,
                ),
                expanded_union,
            ),
            (
                // Visibly identical rows from both branches keep their
                // multiplicity: the hidden keys never collapse them.
                "tied-duplicate-visible-union",
                patient_view(
                    json!([{"unionAll":[
                        {"column":[{"path":"'tie'","name":"tie"}]},
                        {"column":[{"path":"'tie'","name":"tie"}]}
                    ]}]),
                    None,
                ),
                vec![row(&[("tie", json!("tie"))]); 6],
            ),
            (
                "tied-nullable-first-column-union",
                patient_view(
                    json!([{"unionAll":[
                        {"column":[{"path":"gender","name":"g"},{"path":"id","name":"v"}]},
                        {"column":[{"path":"'zz'","name":"g"},{"path":"id","name":"v"}]}
                    ]}]),
                    None,
                ),
                nullable_first_column,
            ),
        ];
        for (case, view, expected) in cases {
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, case).await;
        }

        // Object- and primitive-valued iteration sources keep their row
        // contents and occurrence order.
        let sources = test_tenant();
        for patient in [
            json!({"resourceType":"Patient","id":"s-1","gender":"female",
                "name":[{"family":"F1"},{"family":"F2"}],
                "contact":[{"name":{"family":"K1"}},{"name":{"family":"K2"}},{"name":{"family":"K3"}}]}),
            json!({"resourceType":"Patient","id":"s-2","gender":"male",
                "name":[{"family":"F3"}],"contact":[{"name":{"family":"K4"}}]}),
        ] {
            backend
                .create(&sources, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed source patient");
        }
        set_last_updated(
            &sources,
            "Patient",
            &[
                ("s-2", "2024-03-03T00:00:02Z"),
                ("s-1", "2024-03-03T00:00:01Z"),
            ],
        )
        .await;
        let source_cases = [
            (
                "primitive-root-source",
                patient_view(
                    json!([{"column":[{"path":"id","name":"id"}]},
                        {"forEach":"gender","column":[{"path":"$this","name":"g"}]}]),
                    None,
                ),
                vec![
                    row(&[("id", json!("s-1")), ("g", json!("female"))]),
                    row(&[("id", json!("s-2")), ("g", json!("male"))]),
                ],
            ),
            (
                "object-chained-source",
                patient_view(
                    json!([{"forEach":"contact.name","column":[{"path":"family","name":"family"}]}]),
                    None,
                ),
                ["K1", "K2", "K3", "K4"]
                    .map(|f| row(&[("family", json!(f))]))
                    .to_vec(),
            ),
            (
                "primitive-nested-source",
                patient_view(
                    json!([{"forEach":"name","select":[
                        {"forEach":"family","column":[{"path":"$this","name":"f"}]}]}]),
                    None,
                ),
                ["F1", "F2", "F3"].map(|f| row(&[("f", json!(f))])).to_vec(),
            ),
        ];
        for (case, view, expected) in source_cases {
            assert_order_oracle(runner.as_ref(), &sources, view, &expected, case).await;
        }
    }

    // =========================================================================
    // #1623 2C: recursive traversal identity and `%rowIndex`.
    //
    // A standalone `repeat:` orders by its first visible column (explicit
    // NULL placement), last_updated, id, the complete traversal identity, then
    // post-repeat occurrence ordinals. Repeat-scope `%rowIndex` is the node's
    // pre-order position per resource, before post-repeat expansion; every
    // expected index is also checked against the in-process evaluator.
    // =========================================================================

    /// Rows the in-process evaluator (`helios-sof`) produces for `view` over
    /// `resources`, in its order, shaped like this runner's rows.
    fn evaluator_rows(view: &Value, resources: &[Value]) -> Vec<Value> {
        let view = helios_sof::parse_view_definition_for_version(view.clone(), FhirVersion::R4)
            .expect("evaluator view");
        let bundle = helios_sof::create_bundle_from_resources_for_version(
            resources.to_vec(),
            FhirVersion::R4,
        )
        .expect("evaluator bundle");
        let result = helios_sof::process_view_definition(view, bundle).expect("evaluator run");
        result
            .rows
            .into_iter()
            .map(|r| {
                let pairs: Vec<(&str, Value)> = result
                    .columns
                    .iter()
                    .map(String::as_str)
                    .zip(r.values.into_iter().map(|v| v.unwrap_or(Value::Null)))
                    .collect();
                row(&pairs)
            })
            .collect()
    }

    fn qr(id: &str, items: Value) -> Value {
        json!({"resourceType":"QuestionnaireResponse","id":id,"status":"completed","item":items})
    }

    /// Three QuestionnaireResponses, in id order: `qr-a` repeats a child
    /// under both `item` and `answer.item`; `qr-b` has multiple answers with
    /// nested items next to a multi-level `item` tree (the collision
    /// fixture); `qr-c` descends three levels through `item` only.
    fn recursion_fixture() -> Vec<Value> {
        vec![
            qr(
                "qr-a",
                json!([{"linkId":"p","item":[{"linkId":"q"}],
                    "answer":[{"valueString":"pa","item":[{"linkId":"pa.i"}]}]}]),
            ),
            qr(
                "qr-b",
                json!([
                    {"linkId":"a",
                        "item":[{"linkId":"a.x","item":[{"linkId":"a.x.y"}]},{"linkId":"a.z"}],
                        "answer":[
                            {"valueString":"ans-a1","item":[{"linkId":"a.ans1.i1"},{"linkId":"a.ans1.i2"}]},
                            {"valueString":"ans-a2","item":[{"linkId":"a.ans2.i1"}]}]},
                    {"linkId":"b","answer":[{"valueString":"ans-b1"},{"valueString":"ans-b2"}]},
                    {"linkId":"c"}
                ]),
            ),
            qr(
                "qr-c",
                json!([{"linkId":"c1","item":[{"linkId":"c1.1","item":[{"linkId":"c1.1.1"}]}]},
                    {"linkId":"c2"}]),
            ),
        ]
    }

    fn qr_view(select: Value) -> Value {
        json!({"resourceType":"ViewDefinition","resource":"QuestionnaireResponse",
            "status":"active","select":select})
    }

    /// `(linkId, %rowIndex)` pairs of `rows`.
    fn link_index(rows: &[Value]) -> Vec<(String, i64)> {
        rows.iter()
            .map(|r| {
                (
                    r["link"].as_str().expect("link").to_string(),
                    r["i"].as_i64().expect("index"),
                )
            })
            .collect()
    }

    fn pairs(expected: &[(&str, i64)]) -> Vec<(String, i64)> {
        expected
            .iter()
            .map(|(link, index)| (link.to_string(), *index))
            .collect()
    }

    fn tie_columns() -> Value {
        json!([{"path":"'tie'","name":"tie"},{"path":"linkId","name":"link"},
            {"path":"%rowIndex","name":"i","type":"integer"}])
    }

    #[tokio::test]
    async fn test_pg_repeat_indices_and_order_match_evaluator() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let fixture = recursion_fixture();
        // Seeded in reverse id order, then given one shared timestamp: only
        // the id tie-break orders the resources.
        for resource in fixture.iter().rev() {
            backend
                .create(
                    &tenant,
                    "QuestionnaireResponse",
                    resource.clone(),
                    FhirVersion::R4,
                )
                .await
                .expect("seed recursion fixture");
        }
        let tied = "2024-04-04T00:00:00Z";
        set_last_updated(
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();

        let item = qr_view(json!([{"repeat":["item"],"column":tie_columns()}]));
        let mixed = qr_view(json!([{"repeat":["item","answer.item"],"column":tie_columns()}]));
        let repeated = qr_view(json!([{"repeat":["item","item"],"column":tie_columns()}]));
        let answer_columns = json!([{"path":"valueString","name":"ans"},
            {"path":"%rowIndex","name":"ans_i","type":"integer"}]);
        let inner = qr_view(json!([{"repeat":["item","answer.item"],"select":[
            {"column":tie_columns()},
            {"forEach":"answer","column":answer_columns.clone()}]}]));
        let outer = qr_view(json!([{"repeat":["item","answer.item"],"select":[
            {"column":tie_columns()},
            {"forEachOrNull":"answer","column":answer_columns}]}]));
        let union = qr_view(json!([{"unionAll":[
            {"repeat":["item","answer.item"],"column":tie_columns()},
            {"column":[{"path":"'tie'","name":"tie"},{"path":"'root'","name":"link"},
                {"path":"%rowIndex","name":"i","type":"integer"}]}]}]));
        // Resource-dependent siblings of the repeat: a `where()` projection,
        // an indexed forEach and a resource-rooted forEach.
        let siblings = qr_view(json!([
            {"repeat":["item"],"column":tie_columns()},
            {"column":[{"path":"item.where(linkId = 'c').linkId","name":"w"}]},
            {"forEach":"item[0]","column":[{"path":"linkId","name":"first"}]},
            {"forEach":"item","column":[{"path":"linkId","name":"top"}]}]));

        // A constant first column leaves the resource key and traversal
        // order as the tie-breaks: exactly the evaluator's order.
        let mut results = BTreeMap::new();
        for (case, view) in [
            ("item", &item),
            ("item+answer.item", &mixed),
            ("repeated-path", &repeated),
            ("post-repeat-inner", &inner),
            ("post-repeat-outer", &outer),
            ("union-repeat-branch", &union),
        ] {
            let expected = evaluator_rows(view, &fixture);
            assert!(!expected.is_empty(), "{case}");
            assert_order_oracle(runner.as_ref(), &tenant, view.clone(), &expected, case).await;
            results.insert(case, expected);
        }

        // Hand-checked pre-order indices: a node's subtree (all paths) comes
        // before its next sibling; `item` children before `answer.item`
        // children; each resource restarts at 0.
        assert_eq!(
            link_index(&results["item"]),
            pairs(&[
                ("p", 0),
                ("q", 1),
                ("a", 0),
                ("a.x", 1),
                ("a.x.y", 2),
                ("a.z", 3),
                ("b", 4),
                ("c", 5),
                ("c1", 0),
                ("c1.1", 1),
                ("c1.1.1", 2),
                ("c2", 3),
            ])
        );
        assert_eq!(
            link_index(&results["item+answer.item"]),
            pairs(&[
                ("p", 0),
                ("q", 1),
                ("pa.i", 2),
                ("a", 0),
                ("a.x", 1),
                ("a.x.y", 2),
                ("a.z", 3),
                ("a.ans1.i1", 4),
                ("a.ans1.i2", 5),
                ("a.ans2.i1", 6),
                ("b", 7),
                ("c", 8),
                ("c1", 0),
                ("c1.1", 1),
                ("c1.1.1", 2),
                ("c2", 3),
            ])
        );
        assert_eq!(
            link_index(&results["repeated-path"][..6]),
            pairs(&[("p", 0), ("q", 1), ("q", 2), ("p", 3), ("q", 4), ("q", 5)])
        );
        // Resource-dependent siblings: hand-derived order — per resource,
        // each node (traversal order) crossed with the resource's top-level
        // items (the post-repeat ordinal). The evaluator nests the root
        // `forEach` outside the repeat, so only its row multiset is compared.
        let mut expected = Vec::new();
        for resource in &fixture {
            let tops: Vec<&str> = resource["item"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["linkId"].as_str().unwrap())
                .collect();
            let w = if tops.contains(&"c") {
                json!("c")
            } else {
                Value::Null
            };
            for node in evaluator_rows(&item, std::slice::from_ref(resource)) {
                for top in &tops {
                    let mut pairs: Vec<(&str, Value)> = node
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.clone()))
                        .collect();
                    pairs.extend([
                        ("w", w.clone()),
                        ("first", json!(tops[0])),
                        ("top", json!(top)),
                    ]);
                    expected.push(row(&pairs));
                }
            }
        }
        let mut evaluated = evaluator_rows(&siblings, &fixture);
        let mut sorted_expected = expected.clone();
        evaluated.sort_by_key(|r| r.to_string());
        sorted_expected.sort_by_key(|r| r.to_string());
        assert_eq!(evaluated, sorted_expected, "same rows as the evaluator");
        assert_order_oracle(
            runner.as_ref(),
            &tenant,
            siblings,
            &expected,
            "resource-siblings",
        )
        .await;

        // Post-repeat forEach drops answer-less nodes but keeps the
        // pre-expansion node index (`b` stays 7); its own index restarts.
        let answer_row = |link: &str, i: i64, ans: &str, ans_i: i64| {
            row(&[
                ("tie", json!("tie")),
                ("link", json!(link)),
                ("i", json!(i)),
                ("ans", json!(ans)),
                ("ans_i", json!(ans_i)),
            ])
        };
        assert_eq!(
            results["post-repeat-inner"],
            vec![
                answer_row("p", 0, "pa", 0),
                answer_row("a", 0, "ans-a1", 0),
                answer_row("a", 0, "ans-a2", 1),
                answer_row("b", 7, "ans-b1", 0),
                answer_row("b", 7, "ans-b2", 1),
            ]
        );
        assert_eq!(results["post-repeat-outer"].len(), 18);
        assert_eq!(
            results["post-repeat-outer"][1],
            row(&[
                ("tie", json!("tie")),
                ("link", json!("q")),
                ("i", json!(1)),
                ("ans", Value::Null),
                ("ans_i", json!(0))
            ])
        );

        // Primary key is the first visible column; equal values fall back to
        // the traversal identity (both `p` copies, then the four `q` copies).
        let by_link = {
            let mut view = qr_view(json!([{"repeat":["item","item"],"column":[
                {"path":"linkId","name":"link"},{"path":"%rowIndex","name":"i","type":"integer"}]}]));
            view["where"] = json!([{"path":"id = 'qr-a'"}]);
            view
        };
        let expected: Vec<Value> = [("p", 0), ("p", 3), ("q", 1), ("q", 2), ("q", 4), ("q", 5)]
            .iter()
            .map(|(link, i)| row(&[("link", json!(link)), ("i", json!(i))]))
            .collect();
        let mut evaluated = evaluator_rows(&by_link, &fixture);
        let mut oracle = expected.clone();
        evaluated.sort_by_key(|r| r.to_string());
        oracle.sort_by_key(|r| r.to_string());
        assert_eq!(evaluated, oracle, "same rows as the evaluator");
        assert_order_oracle(
            runner.as_ref(),
            &tenant,
            by_link,
            &expected,
            "first-column-primary",
        )
        .await;

        // Two distinct seed paths; a NULL first column takes the engine's
        // ASC NULL placement (PostgreSQL: NULLS LAST).
        let patient = json!({"resourceType":"Patient","id":"pt-seeds",
            "name":[{"family":"F1"},{"family":"F2"}],
            "contact":[{"name":{"family":"K1"}}]});
        backend
            .create(&tenant, "Patient", patient.clone(), FhirVersion::R4)
            .await
            .expect("seed multi-seed patient");
        let seeds = json!({"resourceType":"ViewDefinition","resource":"Patient","status":"active",
            "select":[{"repeat":["name","contact"],"column":[
                {"path":"family","name":"family"},{"path":"%rowIndex","name":"i","type":"integer"}]}]});
        let family = |f: Value, i: i64| row(&[("family", f), ("i", json!(i))]);
        assert_eq!(
            evaluator_rows(&seeds, std::slice::from_ref(&patient)),
            vec![
                family(json!("F1"), 0),
                family(json!("F2"), 1),
                family(Value::Null, 2),
                family(json!("K1"), 3)
            ]
        );
        let expected = vec![
            family(json!("F1"), 0),
            family(json!("F2"), 1),
            family(json!("K1"), 3),
            family(Value::Null, 2),
        ];
        assert_order_oracle(runner.as_ref(), &tenant, seeds, &expected, "multiple-seeds").await;
    }

    // =========================================================================
    // #1623 2D: indexed iteration (`forEach[OrNull]: "<path>[N]"`).
    //
    // Indexed clauses keep their correlated scalar lowering: one row per
    // enclosing occurrence, whose columns read the N-th element of the
    // flattened chain (selected in element order). The selected occurrence's
    // presence is a membership filter honored by ordinary, nested, union and
    // repeat scopes: absent with `forEach` → no row; with `forEachOrNull` →
    // one row evaluated against the empty iteration context. `%rowIndex` in
    // the indexed scope is the evaluator's value — 0 for the singleton
    // iteration and for the empty context. Every oracle below is also the
    // in-process evaluator's output, in its order.
    // =========================================================================

    /// Three Patients, in id order (seeded reversed, timestamps tied):
    /// `ix-a` has three names (flattened givens `a00, a01, a10`), two
    /// telecoms and three contacts (2, 1 and 0 telecoms); `ix-b` has one
    /// given-less name, one telecom and one contact with two telecoms;
    /// `ix-c` has none. (No name has exactly one given: the evaluator indexes
    /// the characters of a singleton string, so `name.given[1]` over
    /// `["b00"]` yields `"0"` there — an evaluator defect, not a semantics to
    /// mirror.)
    fn indexed_fixture() -> Vec<Value> {
        vec![
            json!({"resourceType":"Patient","id":"ix-a",
                "name":[{"family":"A0","given":["a00","a01"]},{"family":"A1","given":["a10"]},
                    {"family":"A2"}],
                "telecom":[{"value":"t-a0"},{"value":"t-a1"}],
                "contact":[{"telecom":[{"value":"c-a0-0"},{"value":"c-a0-1"}]},
                    {"telecom":[{"value":"c-a1-0"}]},{"gender":"other"}]}),
            json!({"resourceType":"Patient","id":"ix-b",
                "name":[{"family":"B0"}],
                "telecom":[{"value":"t-b0"}],
                "contact":[{"telecom":[{"value":"c-b0-0"},{"value":"c-b0-1"}]}]}),
            json!({"resourceType":"Patient","id":"ix-c","gender":"unknown"}),
        ]
    }

    fn id_column() -> Value {
        json!({"column":[{"path":"id","name":"id"}]})
    }

    fn index_columns(value_path: &str, value: &str, index: &str) -> Value {
        json!([{"path":value_path,"name":value},
            {"path":"%rowIndex","name":index,"type":"integer"}])
    }

    #[tokio::test]
    async fn test_pg_indexed_iteration_rows_match_evaluator() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let fixture = indexed_fixture();
        for patient in fixture.iter().rev() {
            backend
                .create(&tenant, "Patient", patient.clone(), FhirVersion::R4)
                .await
                .expect("seed indexed fixture");
        }
        let tied = "2024-05-05T00:00:00Z";
        set_last_updated(
            &tenant,
            "Patient",
            &[("ix-c", tied), ("ix-b", tied), ("ix-a", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();

        let r = |id: &str, pairs: &[(&str, Value)]| {
            let mut all = vec![("id", json!(id))];
            all.extend(pairs.iter().cloned());
            row(&all)
        };
        let k = || ("k", json!("k"));
        let cases: Vec<(&str, Value, Vec<Value>)> = vec![
            (
                "name[1] forEach",
                patient_view(
                    json!([id_column(),
                        {"forEach":"name[1]","column":index_columns("family","f","i")}]),
                    None,
                ),
                vec![r("ix-a", &[("f", json!("A1")), ("i", json!(0))])],
            ),
            (
                "name[1] forEachOrNull",
                patient_view(
                    json!([id_column(),
                        {"forEachOrNull":"name[1]","column":[
                            {"path":"family","name":"f"},
                            {"path":"%rowIndex","name":"i","type":"integer"},
                            {"path":"'k'","name":"k"}]}]),
                    None,
                ),
                vec![
                    r("ix-a", &[("f", json!("A1")), ("i", json!(0)), k()]),
                    r("ix-b", &[("f", Value::Null), ("i", json!(0)), k()]),
                    r("ix-c", &[("f", Value::Null), ("i", json!(0)), k()]),
                ],
            ),
            (
                "flattened name.given[1]",
                patient_view(
                    json!([id_column(),
                        {"forEach":"name.given[1]","column":index_columns("$this","g","i")}]),
                    None,
                ),
                vec![r("ix-a", &[("g", json!("a01")), ("i", json!(0))])],
            ),
            (
                "flattened name.given[2] crosses names",
                patient_view(
                    json!([id_column(),
                        {"forEach":"name.given[2]","column":index_columns("$this","g","i")}]),
                    None,
                ),
                vec![r("ix-a", &[("g", json!("a10")), ("i", json!(0))])],
            ),
            (
                "out-of-range forEach",
                patient_view(
                    json!([id_column(),
                        {"forEach":"name[5]","column":index_columns("family","f","i")}]),
                    None,
                ),
                vec![],
            ),
            (
                "out-of-range forEachOrNull",
                patient_view(
                    json!([id_column(),
                        {"forEachOrNull":"name.given[5]","column":[
                            {"path":"%rowIndex","name":"i","type":"integer"},
                            {"path":"%rowIndex + 1","name":"i1","type":"integer"},
                            {"path":"'k'","name":"k"}]}]),
                    None,
                ),
                ["ix-a", "ix-b", "ix-c"]
                    .iter()
                    .map(|id| r(id, &[("i", json!(0)), ("i1", json!(1)), k()]))
                    .collect(),
            ),
            (
                "nested under ordinary forEach",
                patient_view(
                    json!([id_column(),
                        {"forEach":"contact","column":[
                            {"path":"%rowIndex","name":"ci","type":"integer"}],
                         "select":[{"forEach":"telecom[1]",
                            "column":index_columns("value","tv","ti")}]}]),
                    None,
                ),
                vec![
                    r(
                        "ix-a",
                        &[("ci", json!(0)), ("tv", json!("c-a0-1")), ("ti", json!(0))],
                    ),
                    r(
                        "ix-b",
                        &[("ci", json!(0)), ("tv", json!("c-b0-1")), ("ti", json!(0))],
                    ),
                ],
            ),
            (
                "forEachOrNull nested under ordinary forEach",
                patient_view(
                    json!([id_column(),
                        {"forEach":"contact","column":[
                            {"path":"%rowIndex","name":"ci","type":"integer"}],
                         "select":[{"forEachOrNull":"telecom[1]",
                            "column":index_columns("value","tv","ti")}]}]),
                    None,
                ),
                vec![
                    r(
                        "ix-a",
                        &[("ci", json!(0)), ("tv", json!("c-a0-1")), ("ti", json!(0))],
                    ),
                    r(
                        "ix-a",
                        &[("ci", json!(1)), ("tv", Value::Null), ("ti", json!(0))],
                    ),
                    r(
                        "ix-a",
                        &[("ci", json!(2)), ("tv", Value::Null), ("ti", json!(0))],
                    ),
                    r(
                        "ix-b",
                        &[("ci", json!(0)), ("tv", json!("c-b0-1")), ("ti", json!(0))],
                    ),
                ],
            ),
            (
                "sibling alongside another expansion",
                patient_view(
                    json!([id_column(),
                        {"forEach":"name","column":index_columns("family","f","ni")},
                        {"forEach":"telecom[1]","column":index_columns("value","tv","ti")}]),
                    None,
                ),
                ["A0", "A1", "A2"]
                    .iter()
                    .enumerate()
                    .map(|(ni, family)| {
                        r(
                            "ix-a",
                            &[
                                ("f", json!(family)),
                                ("ni", json!(ni)),
                                ("tv", json!("t-a1")),
                                ("ti", json!(0)),
                            ],
                        )
                    })
                    .collect(),
            ),
            (
                "union branches",
                patient_view(
                    json!([id_column(), {"unionAll":[
                        {"forEach":"name[1]","column":index_columns("family","v","i")},
                        {"forEachOrNull":"telecom[1]","column":index_columns("value","v","i")}]}]),
                    None,
                ),
                vec![
                    r("ix-a", &[("v", json!("A1")), ("i", json!(0))]),
                    r("ix-a", &[("v", json!("t-a1")), ("i", json!(0))]),
                    r("ix-b", &[("v", Value::Null), ("i", json!(0))]),
                    r("ix-c", &[("v", Value::Null), ("i", json!(0))]),
                ],
            ),
        ];
        for (case, view, expected) in cases {
            assert_eq!(
                evaluator_rows(&view, &fixture),
                expected,
                "{case}: evaluator"
            );
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, case).await;
        }
    }

    #[tokio::test]
    async fn test_pg_indexed_iteration_after_and_under_repeat_honors_membership() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let fixture = recursion_fixture();
        for resource in fixture.iter().rev() {
            backend
                .create(
                    &tenant,
                    "QuestionnaireResponse",
                    resource.clone(),
                    FhirVersion::R4,
                )
                .await
                .expect("seed recursion fixture");
        }
        let tied = "2024-06-06T00:00:00Z";
        set_last_updated(
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();

        // Sibling of the repeat: `qr-a` has a single top-level item, so its
        // `item[1]` is absent and none of its nodes survive.
        let after = qr_view(json!([{"repeat":["item"],"column":tie_columns()},
            {"forEach":"item[1]","column":[{"path":"linkId","name":"second"}]}]));
        let node = |link: &str, i: i64, extra: &[(&str, Value)]| {
            let mut all = vec![
                ("tie", json!("tie")),
                ("link", json!(link)),
                ("i", json!(i)),
            ];
            all.extend(extra.iter().cloned());
            row(&all)
        };
        let mut expected_after: Vec<Value> = ["a", "a.x", "a.x.y", "a.z", "b", "c"]
            .iter()
            .enumerate()
            .map(|(i, link)| node(link, i as i64, &[("second", json!("b"))]))
            .collect();
        expected_after.extend(
            ["c1", "c1.1", "c1.1.1", "c2"]
                .iter()
                .enumerate()
                .map(|(i, link)| node(link, i as i64, &[("second", json!("c2"))])),
        );

        // Nested under the repeat: only nodes with a second answer survive
        // `forEach`; `forEachOrNull` keeps every node with the empty context.
        let under = |kind: &str| {
            qr_view(json!([{"repeat":["item","answer.item"],"select":[
                {"column":tie_columns()},
                {kind:"answer[1]","column":[{"path":"valueString","name":"ans"},
                    {"path":"%rowIndex","name":"ans_i","type":"integer"}]}]}]))
        };
        let expected_under = vec![
            node("a", 0, &[("ans", json!("ans-a2")), ("ans_i", json!(0))]),
            node("b", 7, &[("ans", json!("ans-b2")), ("ans_i", json!(0))]),
        ];
        let under_or_null_expected = evaluator_rows(&under("forEachOrNull"), &fixture);
        assert_eq!(under_or_null_expected.len(), 16);
        assert_eq!(
            under_or_null_expected
                .iter()
                .filter(|r| !r["ans"].is_null())
                .cloned()
                .collect::<Vec<_>>(),
            expected_under
        );
        assert!(
            under_or_null_expected
                .iter()
                .all(|r| r["ans_i"] == json!(0))
        );

        for (case, view, expected) in [
            ("indexed sibling after repeat", after, expected_after),
            (
                "indexed forEach under repeat",
                under("forEach"),
                expected_under,
            ),
            (
                "indexed forEachOrNull under repeat",
                under("forEachOrNull"),
                under_or_null_expected.clone(),
            ),
        ] {
            assert_eq!(
                evaluator_rows(&view, &fixture),
                expected,
                "{case}: evaluator"
            );
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, case).await;
        }
    }

    #[tokio::test]
    async fn test_pg_indexed_selection_of_json_null_is_present() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        // The SQL navigation emits the JSON `null` element as an occurrence:
        // selecting it yields a row whose value is null, unlike an absent
        // selection, which drops the row. (The in-process evaluator skips
        // null elements while navigating — `name.given` is `n0, n2` there —
        // for ordinary and indexed iteration alike; SQL navigation is
        // unchanged here, so only SQL-internal consistency is asserted.)
        let patient = json!({"resourceType":"Patient","id":"jn",
            "name":[{"given":["n0",null,"n2"]}]});
        backend
            .create(&tenant, "Patient", patient, FhirVersion::R4)
            .await
            .expect("seed json-null patient");
        let runner = backend.sof_runner().unwrap();
        let columns = json!([{"path":"$this","name":"g"},{"path":"'present'","name":"p"}]);
        // The ordinary iteration's occurrences, in element order: the
        // indexed selection `[N]` must be occurrence N, plus `%rowIndex` 0.
        let ordinary = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            patient_view(json!([{"forEach":"name.given","column":columns}]), None),
            ViewFilters::default(),
        )
        .await;
        assert_eq!(ordinary.len(), 3, "{ordinary:?}");
        assert!(ordinary[1]["g"].is_null(), "{ordinary:?}");
        assert_eq!(ordinary[1]["p"], json!("present"));
        let mut indexed_columns = columns.as_array().unwrap().clone();
        indexed_columns.push(json!({"path":"%rowIndex","name":"i","type":"integer"}));
        for index in 0..4 {
            let view = patient_view(
                json!([{"forEach":format!("name.given[{index}]"),"column":indexed_columns}]),
                None,
            );
            let mut expected: Vec<Value> = ordinary
                .get(index)
                .map(|occurrence| {
                    let mut selected = occurrence.clone();
                    selected["i"] = json!(0);
                    selected
                })
                .into_iter()
                .collect();
            if index == 1 {
                // The scalar lowering keeps the selected element's JSON
                // representation: the JSON `null` occurrence projects as JSON
                // `null` (key present), while the lateral lowering's SQL NULL
                // is omitted by the row mapper — the row itself is present.
                expected = vec![json!({"g": null, "p": "present", "i": 0})];
            }
            assert_order_oracle(
                runner.as_ref(),
                &tenant,
                view,
                &expected,
                &format!("name.given[{index}]"),
            )
            .await;
        }
    }

    // =========================================================================
    // #1623 review A1: a trailing `where(crit)` on an indexed iteration
    // (`name[N].where(crit)`) filters the SELECTED occurrence — FHIRPath
    // indexes first, then filters — exactly like the evaluator: `forEach`
    // drops a rejected or absent selection, `forEachOrNull` evaluates the
    // empty context for it. Over the exact nullable fixture
    // (`p-large`'s names 1..=75 are `official`, 76..=150 `temp`).
    // =========================================================================

    /// The nullable fixture in output order (tied timestamps, then id).
    fn trailing_where_fixture() -> Vec<Value> {
        vec![
            json!({"resourceType":"Patient","id":"p-empty"}),
            json!({"resourceType":"Patient","id":"p-filtered","name":[{"family":"Rejected","use":"temp"}]}),
            large_patient_fixture(),
        ]
    }

    /// `(case, view, expected rows)` for the trailing-`where` cases.
    fn trailing_where_cases() -> Vec<(&'static str, Value, Vec<Value>)> {
        let r = |id: &str, pairs: &[(&str, Value)]| {
            let mut all = vec![("id", json!(id))];
            all.extend(pairs.iter().cloned());
            row(&all)
        };
        let view = |kind: &str, path: &str, value_path: &str| {
            patient_view(
                json!([{"column":[{"path":"id","name":"id"}]},
                    {kind:path,"column":[{"path":value_path,"name":"v"},
                        {"path":"%rowIndex","name":"i","type":"integer"}]}]),
                None,
            )
        };
        let ids = ["p-empty", "p-filtered", "p-large"];
        // `forEachOrNull` rows: every patient, the selection's value only
        // where it was selected and accepted, `%rowIndex` always 0.
        let or_null = |selected: &[(&str, &str)]| -> Vec<Value> {
            ids.iter()
                .map(|id| {
                    let value = selected
                        .iter()
                        .find(|(sid, _)| sid == id)
                        .map_or(Value::Null, |(_, v)| json!(v));
                    r(id, &[("v", value), ("i", json!(0))])
                })
                .collect()
        };
        let selected = |id: &str, value: &str| r(id, &[("v", json!(value)), ("i", json!(0))]);
        vec![
            (
                "forEach name[100] rejected by where",
                view("forEach", "name[100].where(use = 'official')", "family"),
                vec![],
            ),
            (
                "forEach name[10] accepted by where",
                view("forEach", "name[10].where(use = 'official')", "family"),
                vec![selected("p-large", "Family-11")],
            ),
            (
                "forEach name[0] where use = temp",
                view("forEach", "name[0].where(use = 'temp')", "family"),
                vec![selected("p-filtered", "Rejected")],
            ),
            (
                "forEach name[100] where use = temp",
                view("forEach", "name[100].where(use = 'temp')", "family"),
                vec![selected("p-large", "Family-101")],
            ),
            (
                "forEach out-of-range name[150] with where",
                view("forEach", "name[150].where(use = 'temp')", "family"),
                vec![],
            ),
            (
                "forEachOrNull name[100] rejected by where",
                view(
                    "forEachOrNull",
                    "name[100].where(use = 'official')",
                    "family",
                ),
                or_null(&[]),
            ),
            (
                "forEachOrNull name[10] accepted by where",
                view(
                    "forEachOrNull",
                    "name[10].where(use = 'official')",
                    "family",
                ),
                or_null(&[("p-large", "Family-11")]),
            ),
            (
                "forEachOrNull name[0] where use = temp",
                view("forEachOrNull", "name[0].where(use = 'temp')", "family"),
                or_null(&[("p-filtered", "Rejected")]),
            ),
            (
                "forEach flattened name.given[201] accepted by where",
                view("forEach", "name.given[201].where($this.exists())", "$this"),
                vec![selected("p-large", "Given-101-b")],
            ),
            (
                "forEach flattened name.given[201] rejected by where",
                view("forEach", "name.given[201].where($this.empty())", "$this"),
                vec![],
            ),
            (
                "forEachOrNull flattened name.given[201] rejected by where",
                view(
                    "forEachOrNull",
                    "name.given[201].where($this.empty())",
                    "'k'",
                ),
                // (`$this` over the empty context is `{}` in the evaluator, so a
                // constant shows the empty-context row instead. The criteria
                // avoid `$this = '<text>'`, which PostgreSQL cannot compile
                // in any `forEach` filter, indexed or not.)
                ids.iter()
                    .map(|id| r(id, &[("v", json!("k")), ("i", json!(0))]))
                    .collect(),
            ),
        ]
    }

    #[tokio::test]
    async fn test_pg_indexed_trailing_where_filters_the_selection_like_the_evaluator() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let fixture = trailing_where_fixture();
        for patient in fixture.iter().rev() {
            backend
                .create(&tenant, "Patient", patient.clone(), FhirVersion::R4)
                .await
                .expect("seed trailing-where fixture");
        }
        let tied = "2024-07-07T00:00:00Z";
        set_last_updated(
            &tenant,
            "Patient",
            &[("p-large", tied), ("p-filtered", tied), ("p-empty", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();
        for (case, view, expected) in trailing_where_cases() {
            assert_eq!(
                evaluator_rows(&view, &fixture),
                expected,
                "{case}: evaluator"
            );
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, case).await;
        }
    }

    // =========================================================================
    // #1623 review N1: `%rowIndex` inside an iteration path's `where(crit)`
    // reads the ENCLOSING scope. The evaluator evaluates the `forEach`
    // expression — criterion included — with the enclosing iteration's
    // variables and binds the new index only for the produced columns:
    // indexed (`given[0].where(..)`) and ordinary (`given.where(..)`)
    // iterations alike, and a `where()` inside a column expression keeps its
    // column's scope.
    // =========================================================================

    /// `(case, view, expected rows)` over the nullable fixture
    /// ([`trailing_where_fixture`], output order `p-empty`, `p-filtered`,
    /// `p-large`).
    fn row_index_where_patient_cases() -> Vec<(String, Value, Vec<Value>)> {
        // The outer `forEach: "name"` occurrences: `p-filtered`'s given-less
        // name, then `p-large`'s 150 names (`Family-<n>`, givens
        // `Given-<n>-a|b`), as `(family, outer %rowIndex, p-large name n)`.
        let outer: Vec<(Value, usize, Option<usize>)> =
            std::iter::once((json!("Rejected"), 0, None))
                .chain((0..150).map(|k| (json!(format!("Family-{}", k + 1)), k, Some(k + 1))))
                .collect();
        let given = |n: usize, s: &str| json!(format!("Given-{n}-{s}"));
        let nested_view = |kind: &str, path: &str| {
            patient_view(
                json!([{"forEach":"name","column":[{"path":"family","name":"family"},
                        {"path":"%rowIndex","name":"outer_i","type":"integer"}],
                    "select":[{kind:path,"column":[{"path":"$this","name":"given"},
                        {"path":"%rowIndex","name":"inner_i","type":"integer"}]}]}]),
                None,
            )
        };
        let nested_row = |family: &Value, outer_i: usize, given: Value, inner_i: usize| {
            row(&[
                ("family", family.clone()),
                ("outer_i", json!(outer_i)),
                ("given", given),
                ("inner_i", json!(inner_i)),
            ])
        };
        let top_view = |kind: &str, path: &str| {
            patient_view(
                json!([{"column":[{"path":"id","name":"id"}]},
                    {kind:path,"column":[{"path":"family","name":"v"},
                        {"path":"%rowIndex","name":"i","type":"integer"}]}]),
                None,
            )
        };
        let top_row =
            |id: &str, v: Value, i: usize| row(&[("id", json!(id)), ("v", v), ("i", json!(i))]);
        let mut cases: Vec<(String, Value, Vec<Value>)> = Vec::new();

        // Indexed iteration under an ordinary `forEach`: the criterion reads
        // the OUTER `%rowIndex`; the selection's own columns read 0.
        for k in [0usize, 1, 149, 150] {
            let expected = outer
                .iter()
                .filter(|(_, outer_i, n)| *outer_i == k && n.is_some())
                .map(|(family, outer_i, n)| nested_row(family, *outer_i, given(n.unwrap(), "a"), 0))
                .collect();
            cases.push((
                format!("indexed forEach given[0].where(%rowIndex = {k}) under forEach name"),
                nested_view("forEach", &format!("given[0].where(%rowIndex = {k})")),
                expected,
            ));
        }
        cases.push((
            "ordinary forEach given.where(%rowIndex = 1) under forEach name".into(),
            nested_view("forEach", "given.where(%rowIndex = 1)"),
            vec![
                nested_row(&json!("Family-2"), 1, given(2, "a"), 0),
                nested_row(&json!("Family-2"), 1, given(2, "b"), 1),
            ],
        ));
        // A `where()` inside a column expression keeps the column's scope.
        cases.push((
            "column where() reads the column's forEach scope".into(),
            patient_view(
                json!([{"forEach":"name","column":[{"path":"family","name":"family"},
                    {"path":"%rowIndex","name":"outer_i","type":"integer"},
                    {"path":"given.where(%rowIndex = 1).exists()","name":"picked",
                        "type":"boolean"}]}]),
                None,
            ),
            outer
                .iter()
                .map(|(family, outer_i, n)| {
                    row(&[
                        ("family", family.clone()),
                        ("outer_i", json!(outer_i)),
                        ("picked", json!(*outer_i == 1 && n.is_some())),
                    ])
                })
                .collect(),
        ));

        // Top level: the enclosing scope is the resource (`%rowIndex` 0).
        cases.push((
            "top-level indexed forEach name[1].where(%rowIndex = 0)".into(),
            top_view("forEach", "name[1].where(%rowIndex = 0)"),
            vec![top_row("p-large", json!("Family-2"), 0)],
        ));
        cases.push((
            "top-level indexed forEach name[1].where(%rowIndex = 1)".into(),
            top_view("forEach", "name[1].where(%rowIndex = 1)"),
            vec![],
        ));
        cases.push((
            "top-level indexed forEachOrNull name[0].where(%rowIndex = 0)".into(),
            top_view("forEachOrNull", "name[0].where(%rowIndex = 0)"),
            vec![
                top_row("p-empty", Value::Null, 0),
                top_row("p-filtered", json!("Rejected"), 0),
                top_row("p-large", json!("Family-1"), 0),
            ],
        ));
        cases.push((
            "top-level indexed forEachOrNull name[0].where(%rowIndex = 1)".into(),
            top_view("forEachOrNull", "name[0].where(%rowIndex = 1)"),
            ["p-empty", "p-filtered", "p-large"]
                .iter()
                .map(|id| top_row(id, Value::Null, 0))
                .collect(),
        ));
        cases.push((
            "top-level ordinary forEach name.where(%rowIndex = 0)".into(),
            top_view("forEach", "name.where(%rowIndex = 0)"),
            std::iter::once(top_row("p-filtered", json!("Rejected"), 0))
                .chain((0..150).map(|k| top_row("p-large", json!(format!("Family-{}", k + 1)), k)))
                .collect(),
        ));
        cases.push((
            "top-level ordinary forEach name.where(%rowIndex = 1)".into(),
            top_view("forEach", "name.where(%rowIndex = 1)"),
            vec![],
        ));
        cases
    }

    /// `(case, view, expected rows)` over [`recursion_fixture`]: under an
    /// ordinary `forEach: "item"` the criterion reads the item's position
    /// (`qr-a`: `p`; `qr-b`: `a`, `b`, `c`; `qr-c`: `c1`, `c2`); under
    /// `repeat: [item, answer.item]` it reads the node's pre-order
    /// `%rowIndex` (`qr-b`'s `b` is node 7, `qr-a`'s `p` and `qr-b`'s `a`
    /// are node 0).
    fn row_index_where_questionnaire_cases(fixture: &[Value]) -> Vec<(String, Value, Vec<Value>)> {
        let view = |kind: &str, path: &str| {
            qr_view(json!([{"repeat":["item","answer.item"],"select":[
                {"column":tie_columns()},
                {kind:path,"column":[{"path":"valueString","name":"ans"},
                    {"path":"%rowIndex","name":"ans_i","type":"integer"}]}]}]))
        };
        let node = |link: &str, i: i64, ans: Value, ans_i: i64| {
            row(&[
                ("tie", json!("tie")),
                ("link", json!(link)),
                ("i", json!(i)),
                ("ans", ans),
                ("ans_i", json!(ans_i)),
            ])
        };
        let or_null_view = view("forEachOrNull", "answer[0].where(%rowIndex = 7)");
        let or_null = evaluator_rows(&or_null_view, fixture);
        assert_eq!(or_null.len(), 16, "{or_null:?}");
        assert_eq!(
            or_null
                .iter()
                .filter(|r| !r["ans"].is_null())
                .cloned()
                .collect::<Vec<_>>(),
            vec![node("b", 7, json!("ans-b1"), 0)]
        );
        assert!(or_null.iter().all(|r| r["ans_i"] == json!(0)));
        let item_view = |kind: &str, path: &str| {
            qr_view(
                json!([{"forEach":"item","column":[{"path":"linkId","name":"link"},
                    {"path":"%rowIndex","name":"outer_i","type":"integer"}],
                "select":[{kind:path,"column":[{"path":"valueString","name":"ans"},
                    {"path":"%rowIndex","name":"ans_i","type":"integer"}]}]}]),
            )
        };
        let item = |link: &str, outer_i: i64, ans: Value, ans_i: i64| {
            row(&[
                ("link", json!(link)),
                ("outer_i", json!(outer_i)),
                ("ans", ans),
                ("ans_i", json!(ans_i)),
            ])
        };
        let items = [("p", 0), ("a", 0), ("b", 1), ("c", 2), ("c1", 0), ("c2", 1)];
        // Every item; the selected answer only where `pick` names it.
        let or_null_items = |pick: &[(&str, &str, i64)]| -> Vec<Value> {
            items
                .iter()
                .flat_map(|(link, outer_i)| {
                    let picked: Vec<Value> = pick
                        .iter()
                        .filter(|(l, _, _)| l == link)
                        .map(|(_, ans, ans_i)| item(link, *outer_i, json!(ans), *ans_i))
                        .collect();
                    if picked.is_empty() {
                        vec![item(link, *outer_i, Value::Null, 0)]
                    } else {
                        picked
                    }
                })
                .collect()
        };
        vec![
            (
                "indexed forEachOrNull answer[1].where(%rowIndex = 1) under forEach item".into(),
                item_view("forEachOrNull", "answer[1].where(%rowIndex = 1)"),
                or_null_items(&[("b", "ans-b2", 0)]),
            ),
            (
                "indexed forEachOrNull answer[1].where(%rowIndex = 0) under forEach item".into(),
                item_view("forEachOrNull", "answer[1].where(%rowIndex = 0)"),
                or_null_items(&[("a", "ans-a2", 0)]),
            ),
            (
                "indexed forEach answer[1].where(%rowIndex = 1) under forEach item".into(),
                item_view("forEach", "answer[1].where(%rowIndex = 1)"),
                vec![item("b", 1, json!("ans-b2"), 0)],
            ),
            (
                "ordinary forEachOrNull answer.where(%rowIndex = 1) under forEach item".into(),
                item_view("forEachOrNull", "answer.where(%rowIndex = 1)"),
                or_null_items(&[("b", "ans-b1", 0), ("b", "ans-b2", 1)]),
            ),
            (
                "indexed forEach answer[0].where(%rowIndex = 7) under repeat".into(),
                view("forEach", "answer[0].where(%rowIndex = 7)"),
                vec![node("b", 7, json!("ans-b1"), 0)],
            ),
            (
                "indexed forEach answer[0].where(%rowIndex = 0) under repeat".into(),
                view("forEach", "answer[0].where(%rowIndex = 0)"),
                vec![
                    node("p", 0, json!("pa"), 0),
                    node("a", 0, json!("ans-a1"), 0),
                ],
            ),
            (
                "indexed forEachOrNull answer[0].where(%rowIndex = 7) under repeat".into(),
                or_null_view,
                or_null,
            ),
            (
                "ordinary forEach answer.where(%rowIndex = 7) under repeat".into(),
                view("forEach", "answer.where(%rowIndex = 7)"),
                vec![
                    node("b", 7, json!("ans-b1"), 0),
                    node("b", 7, json!("ans-b2"), 1),
                ],
            ),
        ]
    }

    #[tokio::test]
    async fn test_pg_where_row_index_reads_the_enclosing_scope_like_the_evaluator() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        let patients = trailing_where_fixture();
        for patient in patients.iter().rev() {
            backend
                .create(&tenant, "Patient", patient.clone(), FhirVersion::R4)
                .await
                .expect("seed trailing-where fixture");
        }
        let questionnaires = recursion_fixture();
        for resource in questionnaires.iter().rev() {
            backend
                .create(
                    &tenant,
                    "QuestionnaireResponse",
                    resource.clone(),
                    FhirVersion::R4,
                )
                .await
                .expect("seed recursion fixture");
        }
        let tied = "2024-08-08T00:00:00Z";
        set_last_updated(
            &tenant,
            "Patient",
            &[("p-large", tied), ("p-filtered", tied), ("p-empty", tied)],
        )
        .await;
        set_last_updated(
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        )
        .await;
        let runner = backend.sof_runner().unwrap();
        let cases = row_index_where_patient_cases()
            .into_iter()
            .map(|case| (case, &patients))
            .chain(
                row_index_where_questionnaire_cases(&questionnaires)
                    .into_iter()
                    .map(|case| (case, &questionnaires)),
            )
            .collect::<Vec<_>>();
        for ((case, view, expected), fixture) in &cases {
            assert_eq!(
                &evaluator_rows(view, fixture),
                expected,
                "{case}: evaluator"
            );
        }
        for ((case, view, expected), _) in cases {
            assert_order_oracle(runner.as_ref(), &tenant, view, &expected, &case).await;
        }
    }

    // =========================================================================
    // #1623 2D: direct-IR `flat_index`. Only the MongoDB lowering produces it
    // (normal SQL compilation uses `ScalarFromChain`), but the SQL emitter
    // supports it for direct IR with the same semantics: the source path is
    // flattened through every field, the element is picked in element order
    // after the ON filter, prior (sibling) iterations keep their rows, and the
    // singleton iteration's `%rowIndex` is 0 — also on a `forEachOrNull` miss.
    // =========================================================================

    fn flat_index_fixture() -> Vec<Value> {
        vec![
            json!({"resourceType":"Patient","id":"fx-a",
                "telecom":[{"value":"t0"},{"value":"t1"}],
                "contact":[{"telecom":[{"system":"email","value":"e0"},
                        {"system":"phone","value":"p0"}]},
                    {"telecom":[{"system":"phone","value":"p1"}]}]}),
            json!({"resourceType":"Patient","id":"fx-b",
                "telecom":[{"value":"u0"}],
                "contact":[{"telecom":[{"system":"phone","value":"q0"}]}]}),
        ]
    }

    /// `Project(id, <value columns>…, i = %rowIndex)` over a `flat_index`
    /// unnest of `contact.telecom` (alias `fe`), optionally above an ordinary
    /// `telecom` unnest (alias `ft`, projected as `t` / `ti`).
    fn flat_index_plan(
        index: i64,
        left_join: bool,
        phone_only: bool,
        prior_sibling: bool,
    ) -> helios_persistence::sof::ir::PlanNode {
        use helios_persistence::sof::ir::{
            BinOp, Column, JsonPath, LitValue, PathStep, PlanNode, RowIndexScope, SqlExpr, SqlType,
        };
        let path = |root: &str, fields: &[&str]| SqlExpr::JsonPath {
            root: root.to_string(),
            path: JsonPath(
                fields
                    .iter()
                    .map(|f| PathStep::Field(f.to_string()))
                    .collect(),
            ),
        };
        let text = |name: &str, expr: SqlExpr| Column {
            name: name.to_string(),
            expr,
            collection: false,
            ty: SqlType::Text,
            decode: helios_persistence::sof::decode::ColumnDecode::Auto,
        };
        let mut plan = PlanNode::Scan {
            alias: "r".into(),
            resource_type: "Patient".into(),
        };
        let mut columns = vec![text("id", path("r.data", &["id"]))];
        if prior_sibling {
            plan = PlanNode::LateralUnnest {
                parent: Box::new(plan),
                source: path("r.data", &["telecom"]),
                out_alias: "ft".into(),
                left_join: false,
                on_filter: None,
                flat_index: None,
            };
            columns.push(text("t", path("ft.value", &["value"])));
            columns.push(text(
                "ti",
                SqlExpr::RowIndex(RowIndexScope::ForEach("ft".into())),
            ));
        }
        let on_filter = phone_only.then(|| SqlExpr::BinOp {
            op: BinOp::Eq,
            lhs: Box::new(path("fe.value", &["system"])),
            rhs: Box::new(SqlExpr::Lit(LitValue::Str("phone".into()))),
        });
        plan = PlanNode::LateralUnnest {
            parent: Box::new(plan),
            source: path("r.data", &["contact", "telecom"]),
            out_alias: "fe".into(),
            left_join,
            on_filter,
            flat_index: Some(index),
        };
        columns.push(text("v", path("fe.value", &["value"])));
        columns.push(text(
            "i",
            SqlExpr::RowIndex(RowIndexScope::ForEach("fe".into())),
        ));
        PlanNode::Project {
            parent: Box::new(plan),
            columns,
        }
    }

    /// `(case, plan, expected rows)`; each row lists its columns in plan order.
    #[allow(clippy::type_complexity)]
    fn flat_index_cases() -> Vec<(
        &'static str,
        helios_persistence::sof::ir::PlanNode,
        Vec<Vec<Option<&'static str>>>,
    )> {
        vec![
            (
                "flattened [1], forEach",
                flat_index_plan(1, false, false, false),
                vec![vec![Some("fx-a"), Some("p0"), Some("0")]],
            ),
            (
                "flattened [1] after the ON filter, forEachOrNull",
                flat_index_plan(1, true, true, false),
                vec![
                    vec![Some("fx-a"), Some("p1"), Some("0")],
                    vec![Some("fx-b"), None, Some("0")],
                ],
            ),
            (
                "out of range, forEach",
                flat_index_plan(3, false, false, false),
                vec![],
            ),
            (
                "prior sibling iteration keeps its rows",
                flat_index_plan(0, true, false, true),
                vec![
                    vec![Some("fx-a"), Some("t0"), Some("0"), Some("e0"), Some("0")],
                    vec![Some("fx-a"), Some("t1"), Some("1"), Some("e0"), Some("0")],
                    vec![Some("fx-b"), Some("u0"), Some("0"), Some("q0"), Some("0")],
                ],
            ),
        ]
    }

    #[tokio::test]
    async fn test_pg_direct_ir_flat_index_executes_with_indexed_semantics() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        for patient in flat_index_fixture() {
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed flat_index fixture");
        }
        let tied = "2024-07-07T00:00:00Z";
        set_last_updated(&tenant, "Patient", &[("fx-a", tied), ("fx-b", tied)]).await;
        let pg = shared_pg().await;
        let mut config = tokio_postgres::Config::new();
        config
            .host(&pg.host)
            .port(pg.port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect fixture client");
        let connection_task = tokio::spawn(async move {
            connection.await.expect("fixture connection");
        });
        for (case, plan, expected) in flat_index_cases() {
            let emitted = helios_persistence::sof::emit::emit_plan(
                &plan,
                &helios_persistence::sof::dialect::PgDialect,
            )
            .unwrap_or_else(|e| panic!("{case}: emit: {e}"));
            let rows = client
                .query(
                    emitted.sql.as_str(),
                    &[&tenant.tenant_id().as_str(), &"Patient"],
                )
                .await
                .unwrap_or_else(|e| panic!("{case}: query: {e:?}\n{}", emitted.sql));
            let rows: Vec<Vec<Option<String>>> = rows
                .iter()
                .map(|row| {
                    (0..emitted.columns.len())
                        .map(|i| row.get::<_, Option<String>>(i))
                        .collect()
                })
                .collect();
            let expected: Vec<Vec<Option<String>>> = expected
                .iter()
                .map(|r| r.iter().map(|v| v.map(str::to_string)).collect())
                .collect();
            assert_eq!(rows, expected, "{case}\n{}", emitted.sql);
        }
        drop(client);
        connection_task.abort();
    }

    // =========================================================================
    // #1623: the documented order is total, so neither statistics nor planner
    // settings may change the unlimited rows, their order, or the limited
    // prefix. One isolated database, seeded once; every condition recreates
    // the backend so each runner-pool session starts with that condition's
    // settings. Plans are logged, not asserted: the assertions are on rows.
    // =========================================================================

    /// One planner condition: database-level settings every new runner-pool
    /// session inherits, and the pool's own `plan_cache_mode` (a startup
    /// option, which overrides database-level settings).
    struct PlannerCondition {
        name: &'static str,
        /// Run `ANALYZE` before this condition; statistics persist for the
        /// conditions after it.
        analyze_first: bool,
        settings: &'static [(&'static str, &'static str)],
        plan_cache_mode: &'static str,
    }

    const PARALLEL_SETTINGS: [(&str, &str); 5] = [
        ("max_parallel_workers_per_gather", "4"),
        ("parallel_setup_cost", "0"),
        ("parallel_tuple_cost", "0"),
        ("min_parallel_table_scan_size", "0"),
        ("min_parallel_index_scan_size", "0"),
    ];

    fn planner_conditions() -> Vec<PlannerCondition> {
        let condition = |name, analyze_first, settings, plan_cache_mode| PlannerCondition {
            name,
            analyze_first,
            settings,
            plan_cache_mode,
        };
        vec![
            condition("before-analyze", false, &[], "force_custom_plan"),
            condition("after-analyze", true, &[], "force_custom_plan"),
            condition(
                "hashjoin-off",
                false,
                &[("enable_hashjoin", "off")],
                "force_custom_plan",
            ),
            condition(
                "mergejoin-off",
                false,
                &[("enable_mergejoin", "off")],
                "force_custom_plan",
            ),
            condition(
                "nestloop-off",
                false,
                &[("enable_nestloop", "off")],
                "force_custom_plan",
            ),
            condition(
                "sort-off",
                false,
                &[("enable_sort", "off")],
                "force_custom_plan",
            ),
            condition(
                "incremental-sort-off",
                false,
                &[("enable_incremental_sort", "off")],
                "force_custom_plan",
            ),
            condition(
                "work-mem-64kB",
                false,
                &[("work_mem", "64kB")],
                "force_custom_plan",
            ),
            condition(
                "work-mem-64MB",
                false,
                &[("work_mem", "64MB")],
                "force_custom_plan",
            ),
            condition(
                "parallel-0",
                false,
                &[("max_parallel_workers_per_gather", "0")],
                "force_custom_plan",
            ),
            condition("parallel-4", false, &PARALLEL_SETTINGS, "force_custom_plan"),
            condition("plan-cache-custom", false, &[], "force_custom_plan"),
            condition("plan-cache-generic", false, &[], "force_generic_plan"),
            // Each runner call prepares a fresh statement, so `auto` plans
            // every call as one of its first five executions.
            condition("plan-cache-auto-fresh-statement", false, &[], "auto"),
        ]
    }

    /// Settings recorded per condition from a runner-pool session.
    const WATCHED_SETTINGS: [&str; 12] = [
        "enable_hashjoin",
        "enable_mergejoin",
        "enable_nestloop",
        "enable_sort",
        "enable_incremental_sort",
        "work_mem",
        "max_parallel_workers_per_gather",
        "parallel_setup_cost",
        "parallel_tuple_cost",
        "min_parallel_table_scan_size",
        "min_parallel_index_scan_size",
        "plan_cache_mode",
    ];

    async fn connect_admin(
        host: &str,
        port: u16,
    ) -> (tokio_postgres::Client, tokio::task::JoinHandle<()>) {
        let mut config = tokio_postgres::Config::new();
        config
            .host(host)
            .port(port)
            .user("postgres")
            .password("postgres")
            .dbname("postgres");
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("connect admin client");
        let task = tokio::spawn(async move {
            connection.await.expect("admin connection");
        });
        (client, task)
    }

    /// Writes the shared prefix-matrix fixture straight into `resources`, in
    /// its reverse id order, plus noise: other tenants holding copies of
    /// every row and a large unrelated resource type in the tested tenant.
    async fn seed_planner_fixture(client: &tokio_postgres::Client, tenant: &str) {
        let fixture = super::sof_prefix_matrix::fixture();
        let types: Vec<String> = fixture
            .iter()
            .map(|r| r.resource_type.to_string())
            .collect();
        let ids: Vec<String> = fixture.iter().map(|r| r.id.clone()).collect();
        let data: Vec<Value> = fixture.iter().map(|r| r.data.clone()).collect();
        let at: Vec<chrono::DateTime<chrono::Utc>> =
            fixture.iter().map(|r| r.last_updated).collect();
        let deleted: Vec<bool> = fixture.iter().map(|r| r.deleted).collect();
        let inserted = client
            .execute(
                "INSERT INTO resources \
                 (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at) \
                 SELECT $1, t, i, '1', d, ts, del, CASE WHEN del THEN ts END \
                 FROM UNNEST($2::text[], $3::text[], $4::jsonb[], $5::timestamptz[], $6::bool[]) \
                 WITH ORDINALITY AS u(t, i, d, ts, del, ord) ORDER BY ord",
                &[&tenant, &types, &ids, &data, &at, &deleted],
            )
            .await
            .expect("seed planner fixture");
        assert_eq!(usize::try_from(inserted).unwrap(), fixture.len());
        for other in ["planner_noise_a", "planner_noise_b"] {
            client
                .execute(
                    "INSERT INTO resources \
                     (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at) \
                     SELECT $2, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at \
                     FROM resources WHERE tenant_id = $1",
                    &[&tenant, &other],
                )
                .await
                .expect("seed noise tenant");
        }
        client
            .execute(
                "INSERT INTO resources \
                 (tenant_id, resource_type, id, version_id, data, last_updated) \
                 SELECT $1, 'Observation', 'obs-' || g, '1', \
                    jsonb_build_object('resourceType', 'Observation', 'id', 'obs-' || g, 'status', 'final'), \
                    TIMESTAMPTZ '2024-01-01 00:00:00+00' + g * INTERVAL '1 second' \
                 FROM generate_series(1, 6000) g",
                &[&tenant],
            )
            .await
            .expect("seed noise resource type");
    }

    /// Pre-order plan node types (`(Parallel)` when parallel aware).
    fn plan_signature(plan: &Value, out: &mut Vec<String>) {
        let mut node = plan["Node Type"].as_str().unwrap_or("?").to_string();
        if plan["Parallel Aware"] == json!(true) {
            node.push_str(" (Parallel)");
        }
        out.push(node);
        if let Some(children) = plan["Plans"].as_array() {
            for child in children {
                plan_signature(child, out);
            }
        }
    }

    /// The limited preview plan's node types for `view`, from a runner-pool
    /// session, or the EXPLAIN error (logged, never asserted).
    async fn explain_preview(
        client: &deadpool_postgres::Client,
        tenant: &str,
        view: &Value,
    ) -> String {
        let compiled = helios_persistence::sof::compiler::compile_view_definition_dialect(
            view,
            helios_persistence::sof::compiler::SqlDialect::Postgres,
            FhirVersion::R4,
        )
        .expect("compile explained view");
        let resource_type = view["resource"].as_str().unwrap();
        let statement = format!("EXPLAIN (FORMAT JSON) {}\nLIMIT 50", compiled.sql);
        match client
            .query(statement.as_str(), &[&tenant, &resource_type])
            .await
        {
            Ok(rows) => {
                let explained: Value = rows[0].get(0);
                let mut nodes = Vec::new();
                plan_signature(&explained[0]["Plan"], &mut nodes);
                nodes.join(" > ")
            }
            Err(error) => format!("EXPLAIN failed: {error}"),
        }
    }

    /// Where two row arrays first differ, for failure messages.
    fn first_difference(actual: &[Value], expected: &[Value]) -> String {
        match actual.iter().zip(expected).position(|(a, e)| a != e) {
            Some(at) => format!(
                "first difference at row {at}: got {} expected {} (lengths {} / {})",
                actual[at],
                expected[at],
                actual.len(),
                expected.len()
            ),
            None => format!("lengths {} / {}", actual.len(), expected.len()),
        }
    }

    #[tokio::test]
    async fn test_pg_complex_prefix_planner_matrix() {
        let started = std::time::Instant::now();
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default()
                .with_tag("16-alpine")
                .with_label(
                    "github.run_id",
                    std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
                )
                .with_cmd([
                    "postgres",
                    "-c",
                    "fsync=off",
                    "-c",
                    "synchronous_commit=off",
                    "-c",
                    "full_page_writes=off",
                    // No background ANALYZE: "before-analyze" stays unanalyzed.
                    "-c",
                    "autovacuum=off",
                ])
                // Parallel hash joins and sorts use dynamic shared memory.
                .with_shm_size(512 * 1024 * 1024),
        )
        .start()
        .await
        .expect("start planner matrix PostgreSQL");
        let host = container.get_host().await.unwrap().to_string();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let backend_config = |plan_cache_mode: &str| -> PostgresConfig {
            serde_json::from_value(json!({
                "host": host, "port": port, "dbname": "postgres", "user": "postgres",
                "password": "postgres", "max_connections": 5,
                "plan_cache_mode": plan_cache_mode,
                "data_dir": PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"),
            }))
            .expect("planner matrix config")
        };
        PostgresBackend::new(backend_config("force_custom_plan"))
            .await
            .expect("create schema backend")
            .init_schema()
            .await
            .expect("initialize planner matrix schema");
        let (admin, admin_task) = connect_admin(&host, port).await;
        let tenant_id = "planner_matrix";
        let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
        seed_planner_fixture(&admin, tenant_id).await;
        let seeded = started.elapsed();

        let shapes = super::sof_prefix_matrix::shapes();
        let mut oracle: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
        let mut baseline_plans: BTreeMap<&str, String> = BTreeMap::new();
        let mut failures: Vec<String> = Vec::new();
        for condition in planner_conditions() {
            let condition_started = std::time::Instant::now();
            if condition.analyze_first {
                admin.batch_execute("ANALYZE").await.expect("ANALYZE");
            }
            admin
                .batch_execute("ALTER DATABASE postgres RESET ALL")
                .await
                .expect("reset database settings");
            for (name, value) in condition.settings {
                admin
                    .batch_execute(&format!("ALTER DATABASE postgres SET {name} = '{value}'"))
                    .await
                    .unwrap_or_else(|e| panic!("{}: set {name}: {e}", condition.name));
            }
            // A new backend, so every runner-pool session starts after the
            // settings change.
            let backend = Arc::new(
                PostgresBackend::new(backend_config(condition.plan_cache_mode))
                    .await
                    .expect("create condition backend"),
            );
            let runner = backend.sof_runner().unwrap();
            let client = backend.get_client().await.expect("runner-pool session");
            let effective: BTreeMap<String, String> = client
                .query(
                    "SELECT n, current_setting(n) FROM unnest($1::text[]) n",
                    &[&WATCHED_SETTINGS.to_vec()],
                )
                .await
                .expect("read effective settings")
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect();
            for (name, value) in condition.settings {
                assert_eq!(
                    effective[*name], *value,
                    "{}: runner-pool sessions must see {name}",
                    condition.name
                );
            }
            assert_eq!(
                effective["plan_cache_mode"], condition.plan_cache_mode,
                "{}",
                condition.name
            );
            let statistics: (f32, bool) = client
                .query_one(
                    "SELECT c.reltuples, s.last_analyze IS NOT NULL \
                     FROM pg_class c JOIN pg_stat_user_tables s ON s.relid = c.oid \
                     WHERE c.relname = 'resources'",
                    &[],
                )
                .await
                .map(|row| (row.get(0), row.get(1)))
                .expect("read resources statistics");
            println!(
                "[planner-matrix] condition={} analyzed={} reltuples={} settings={effective:?}",
                condition.name, statistics.1, statistics.0
            );
            for (case, view) in shapes
                .iter()
                .map(|(case, view, _)| (*case, view))
                .filter(|(case, _)| super::sof_prefix_matrix::EXPLAINED_SHAPES.contains(case))
            {
                let signature = explain_preview(&client, tenant_id, view).await;
                let changed = baseline_plans
                    .entry(case)
                    .or_insert_with(|| signature.clone())
                    != &signature;
                println!(
                    "[planner-matrix]   plan condition={} shape={case} changed_vs_first={changed}: {signature}",
                    condition.name
                );
            }
            drop(client);

            for (case, view, minimum) in &shapes {
                let mut runs = Vec::new();
                for _ in 0..3 {
                    runs.push(
                        collect_rows_in_order(
                            runner.as_ref(),
                            &tenant,
                            view.clone(),
                            ViewFilters::default(),
                        )
                        .await,
                    );
                }
                let unlimited = &runs[0];
                let label = format!("{} / {case}", condition.name);
                if unlimited.len() < (*minimum).max(51) {
                    failures.push(format!("{label}: only {} rows", unlimited.len()));
                }
                for (run, rows) in runs.iter().enumerate().skip(1) {
                    if rows != unlimited {
                        failures.push(format!(
                            "{label}: repeated run {run} differs: {}",
                            first_difference(rows, unlimited)
                        ));
                    }
                }
                let limited = collect_rows_in_order(
                    runner.as_ref(),
                    &tenant,
                    view.clone(),
                    ViewFilters {
                        limit: Some(50),
                        ..Default::default()
                    },
                )
                .await;
                if limited[..] != unlimited[..50.min(unlimited.len())] {
                    failures.push(format!(
                        "{label}: limit 50 is not the unlimited prefix: {}",
                        first_difference(&limited, unlimited)
                    ));
                }
                let expected = oracle.entry(case).or_insert_with(|| unlimited.clone());
                if unlimited != expected {
                    failures.push(format!(
                        "{label}: order differs from the first condition: {}",
                        first_difference(unlimited, expected)
                    ));
                }
            }
            println!(
                "[planner-matrix] condition={} rows ok, {:?}",
                condition.name,
                condition_started.elapsed()
            );
            drop(runner);
            drop(backend);
        }
        admin
            .batch_execute("ALTER DATABASE postgres RESET ALL")
            .await
            .expect("reset database settings");
        drop(admin);
        admin_task.abort();
        println!(
            "[planner-matrix] shapes={} rows={:?} seeded in {seeded:?}, total {:?}",
            shapes.len(),
            oracle
                .iter()
                .map(|(case, rows)| (*case, rows.len()))
                .collect::<Vec<_>>(),
            started.elapsed()
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    // =========================================================================
    // Scalar string columns keep their type (#1769)
    // =========================================================================

    const ISSUE_CODES: [&str; 6] = ["44054006", "0123", "4548-4", "true", "null", "1e3"];

    async fn seed_conditions(backend: &PostgresBackend, tenant: &TenantContext) {
        for (i, code) in ISSUE_CODES.iter().enumerate() {
            let resource = json!({
                "resourceType": "Condition",
                "id": format!("c{i}"),
                "subject": {"reference": "Patient/p1"},
                "code": {"coding": [{"system": "http://example.org/cs", "code": code}]}
            });
            backend
                .create(tenant, "Condition", resource, FhirVersion::R4)
                .await
                .expect("failed to seed condition");
        }
    }

    async fn assert_codes_are_strings(column: Value) {
        let backend = create_backend().await;
        let tenant = test_tenant();
        seed_conditions(&backend, &tenant).await;
        let runner = backend.sof_runner().unwrap();
        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Condition", "status": "active",
            "select": [{"column": [
                {"name": "id", "path": "getResourceKey()"},
                column
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), ISSUE_CODES.len(), "rows: {rows:?}");
        for (i, code) in ISSUE_CODES.iter().enumerate() {
            let row = rows
                .iter()
                .find(|r| r["id"] == json!(format!("c{i}")))
                .unwrap_or_else(|| panic!("missing row c{i}: {rows:?}"));
            assert_eq!(row["code"], json!(code), "row c{i}");
        }
    }

    #[tokio::test]
    async fn test_pg_code_column_stays_string() {
        assert_codes_are_strings(
            json!({"name": "code", "path": "code.coding.first().code", "type": "code"}),
        )
        .await;
    }

    #[tokio::test]
    async fn test_pg_string_typed_column_stays_string() {
        assert_codes_are_strings(
            json!({"name": "code", "path": "code.coding.first().code", "type": "string"}),
        )
        .await;
    }

    #[tokio::test]
    async fn test_pg_untyped_root_column_stays_string() {
        assert_codes_are_strings(json!({"name": "code", "path": "code.coding.first().code"})).await;
    }

    #[tokio::test]
    async fn test_pg_sql_null_keeps_its_key_as_null() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        for (id, family) in [("n1", None), ("n2", Some("Smith"))] {
            let mut resource = json!({"resourceType": "Patient", "id": id});
            if let Some(f) = family {
                resource["name"] = json!([{"family": f}]);
            }
            backend
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("seed");
        }
        let runner = backend.sof_runner().unwrap();
        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"column": [
                {"name": "id", "path": "id", "type": "id"},
                {"name": "family", "path": "name.first().family", "type": "string"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(row.contains_key("family"), "key must be present: {row:?}");
        }
        let n1 = rows.iter().find(|r| r["id"] == json!("n1")).unwrap();
        assert_eq!(n1["family"], Value::Null);
    }

    #[tokio::test]
    async fn test_pg_declared_boolean_and_decimal_stay_typed() {
        let backend = create_backend().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation", "id": "o1", "status": "final",
                    "code": {"text": "x"},
                    "valueQuantity": {"value": 42.5}
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed");
        let runner = backend.sof_runner().unwrap();
        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Observation", "status": "active",
            "select": [{"column": [
                {"name": "has_code", "path": "code.exists()", "type": "boolean"},
                {"name": "v", "path": "valueQuantity.value", "type": "decimal"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["has_code"], json!(true));
        assert_eq!(rows[0]["v"].as_f64(), Some(42.5));
    }
}
