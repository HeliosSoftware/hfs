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

    #[tokio::test]
    async fn test_pg_preview_limit_is_in_executed_sql_and_none_is_unlimited() {
        // Query aliases do not distinguish pg_stat_statements query IDs. Use a
        // dedicated container so another test cannot supply representative SQL.
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
        let unlimited = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            view.clone(),
            ViewFilters::default(),
        )
        .await;
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
        assert_eq!(unlimited.len(), 80);
        assert_eq!(limited, unlimited[..50]);
        let pattern = format!("%\"{alias}\"%");
        let mut statements = Vec::new();
        // The row producer has finished before the channel closes. A bounded
        // poll also accommodates statistics publication at query completion.
        for _ in 0..20 {
            statements = observer.query(
                "SELECT query, calls, rows FROM pg_stat_statements                  WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())                  AND toplevel AND query LIKE $1", &[&pattern]
            ).await.unwrap().iter().map(|row| (
                row.get::<_, String>(0), row.get::<_, i64>(1), row.get::<_, i64>(2)
            )).collect::<Vec<_>>();
            if statements
                .iter()
                .any(|(sql, _, rows)| has_normalized_final_limit(sql) && *rows == 50)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(
            statements
                .iter()
                .any(|(sql, calls, rows)| has_normalized_final_limit(sql)
                    && !sql.contains("MATERIALIZED")
                    && *calls == 1
                    && *rows == 50),
            "executed preview SQL must end in LIMIT and produce 50 rows: {statements:?}"
        );
        assert!(
            statements
                .iter()
                .any(|(sql, calls, rows)| !has_normalized_final_limit(sql)
                    && *calls == 1
                    && *rows == 80),
            "unlimited execution must produce all 80 rows: {statements:?}"
        );
        // Expansion retains identical SQL for both runs, with the cap applied
        // only by the row consumer. pg_stat_statements merges their calls.
        backend
            .create(&tenant, "Patient", large_patient_fixture(), FhirVersion::R4)
            .await
            .expect("seed observable expansion");
        let complex_alias = format!("{alias}_expanded");
        let complex_view = json!({"resourceType":"ViewDefinition","resource":"Patient",
            "select":[{"column":[{"path":"id","name":complex_alias}]},
                {"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        let unlimited = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            complex_view.clone(),
            ViewFilters::default(),
        )
        .await;
        let limited = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            complex_view,
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(unlimited.len(), 150);
        assert_eq!(limited, unlimited[..50]);
        let pattern = format!("%\"{complex_alias}\"%");
        let mut complex_statements = Vec::new();
        for _ in 0..20 {
            complex_statements = observer.query(
                "SELECT query, calls, rows FROM pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) AND toplevel AND query LIKE $1",
                &[&pattern],
            ).await.unwrap().iter().map(|row| (
                row.get::<_, String>(0),row.get::<_, i64>(1),row.get::<_, i64>(2)
            )).collect::<Vec<_>>();
            if complex_statements.iter().any(|(_, calls, _)| *calls == 2) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(
            complex_statements.len(),
            1,
            "both expansion calls must share SQL identity: {complex_statements:?}"
        );
        assert!(
            complex_statements.iter().any(|(sql, calls, rows)| {
                !sql.contains("MATERIALIZED")
                    && !has_normalized_final_limit(sql)
                    && sql.starts_with("SELECT")
                    && sql.contains("ORDER BY r.last_updated, r.id")
                    && *calls == 2
                    && *rows == 300
            }),
            "both expansions must execute the same unlimited SQL despite the client cap: {complex_statements:?}"
        );
        connection_task.abort();
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
            assert_eq!(rows.len(), expected, "runtime-only limit {limit}");
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
        backend.create(&tenant, "QuestionnaireResponse", json!({
            "resourceType":"QuestionnaireResponse", "id":"qr-large", "status":"completed",
            "item":(1..=150).map(|index| json!({
                "linkId":format!("Item-{index}"),
                "answer":[{"valueString":format!("Answer-{index}"),"item":[{"linkId":format!("Child-{index}")}]}]
            })).collect::<Vec<_>>()
        }), FhirVersion::R4).await.expect("seed repeat");
        let runner = backend.sof_runner().unwrap();
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
        ];
        for (case, select, total) in cases {
            assert_large_preview_prefix(runner.as_ref(), &tenant, json!({
                "resourceType":"ViewDefinition","resource":"QuestionnaireResponse","select":select
            }), total, case).await;
        }
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
