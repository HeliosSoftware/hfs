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
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = Postgres::default()
                    .with_label("github.run_id", &run_id)
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
}
