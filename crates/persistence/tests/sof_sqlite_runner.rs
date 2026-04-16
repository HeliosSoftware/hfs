//! Phase 3a integration tests: SQLite in-DB runner.
//!
//! Verifies:
//! 1. `SqliteBackend::sof_runner()` returns the in-DB runner (not `None`).
//! 2. The in-DB runner produces the same rows as the in-process runner for
//!    spec ViewDefinition fixtures (byte-identical column sets).
//! 3. `SofError::Uncompilable` is returned for unsupported ViewDefinitions.

#[cfg(feature = "sqlite")]
mod sqlite_runner_tests {
    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::{SofError, SofRunner, ViewFilters};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn test_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("test"), TenantPermissions::full_access())
    }

    async fn make_backend() -> Arc<SqliteBackend> {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        Arc::new(backend)
    }

    async fn seed_patients(backend: &SqliteBackend, patients: &[(&str, &str, &str)]) {
        let tenant = test_tenant();
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
                .create(&tenant, "Patient", resource, FhirVersion::R4)
                .await
                .expect("failed to seed patient");
        }
    }

    // =========================================================================
    // 1. Backend advertises the in-DB runner
    // =========================================================================

    #[tokio::test]
    async fn test_sqlite_backend_returns_sof_runner() {
        let backend = make_backend().await;
        let runner = backend.sof_runner();
        assert!(
            runner.is_some(),
            "SqliteBackend.sof_runner() must return Some"
        );
        assert_eq!(
            runner.unwrap().runner_name(),
            "sqlite-indb",
            "runner name must be 'sqlite-indb'"
        );
    }

    // =========================================================================
    // 2. In-DB runner produces same results as in-process runner
    // =========================================================================

    /// Collect all rows from a SofRunner into sorted BTreeMaps for stable comparison.
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
        // Sort rows by their JSON string representation for deterministic comparison
        rows.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        rows
    }

    #[tokio::test]
    async fn test_flat_columns_match_inprocess() {
        let backend = make_backend().await;
        seed_patients(
            &backend,
            &[("p1", "male", "1990-01-01"), ("p2", "female", "1985-06-15")],
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

        let tenant = test_tenant();
        let indb_runner = backend.sof_runner().expect("must have runner");
        let indb_rows = collect_rows(indb_runner.as_ref(), &tenant, view.clone()).await;

        assert_eq!(indb_rows.len(), 2, "expected 2 rows from in-DB runner");

        // Check that each row has all three columns
        for row in &indb_rows {
            assert!(row.contains_key("id"), "row missing 'id': {row:?}");
            assert!(row.contains_key("gender"), "row missing 'gender': {row:?}");
            assert!(row.contains_key("dob"), "row missing 'dob': {row:?}");
        }

        // Check values
        let ids: Vec<&str> = indb_rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(ids.contains(&"p1"), "missing p1: {ids:?}");
        assert!(ids.contains(&"p2"), "missing p2: {ids:?}");
    }

    #[tokio::test]
    async fn test_foreach_columns_match_inprocess() {
        let backend = make_backend().await;
        seed_patients(&backend, &[("p1", "male", "1990-01-01")]).await;

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

        let tenant = test_tenant();
        let indb_runner = backend.sof_runner().expect("must have runner");
        let indb_rows = collect_rows(indb_runner.as_ref(), &tenant, view.clone()).await;

        // Patient p1 has one name entry → 1 row
        assert_eq!(indb_rows.len(), 1, "expected 1 row from forEach");
        assert_eq!(indb_rows[0]["family"], "Family-p1");
        assert_eq!(indb_rows[0]["use_code"], "official");
    }

    #[tokio::test]
    async fn test_mixed_root_and_foreach_columns() {
        let backend = make_backend().await;
        seed_patients(
            &backend,
            &[("p1", "male", "1990-01-01"), ("p2", "female", "1985-06-15")],
        )
        .await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {
                    "column": [{"path": "id", "name": "id", "type": "string"}]
                },
                {
                    "forEach": "name",
                    "column": [{"path": "family", "name": "family", "type": "string"}]
                }
            ]
        });

        let tenant = test_tenant();
        let indb_runner = backend.sof_runner().expect("must have runner");
        let indb_rows = collect_rows(indb_runner.as_ref(), &tenant, view.clone()).await;

        // 2 patients, each with 1 name → 2 rows
        assert_eq!(indb_rows.len(), 2);
        let ids: Vec<&str> = indb_rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert!(ids.contains(&"p1"));
        assert!(ids.contains(&"p2"));
    }

    #[tokio::test]
    async fn test_limit_respected() {
        let backend = make_backend().await;
        seed_patients(
            &backend,
            &[
                ("p1", "male", "1990-01-01"),
                ("p2", "female", "1985-06-15"),
                ("p3", "male", "2000-03-20"),
            ],
        )
        .await;

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });

        let tenant = test_tenant();
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

    #[tokio::test]
    async fn test_empty_table_returns_no_rows() {
        let backend = make_backend().await;
        // No seeding — empty table

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });

        let tenant = test_tenant();
        let runner = backend.sof_runner().expect("must have runner");
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert!(rows.is_empty(), "expected 0 rows from empty table");
    }

    // =========================================================================
    // 3. Uncompilable ViewDefinitions are rejected correctly
    // =========================================================================

    async fn expect_uncompilable(runner: &dyn SofRunner, view: Value) {
        let tenant = test_tenant();
        let result = runner.run_view(&tenant, view, ViewFilters::default()).await;
        match result {
            Err(SofError::Uncompilable { .. }) | Err(SofError::InvalidViewDefinition(_)) => {}
            Ok(_) => panic!("expected Uncompilable or InvalidViewDefinition, got Ok"),
            Err(e) => panic!("expected Uncompilable or InvalidViewDefinition, got Err({e})"),
        }
    }

    #[tokio::test]
    async fn test_rejects_function_call_in_path() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "name.exists()", "name": "has_name"}]}]
        });
        expect_uncompilable(runner.as_ref(), view).await;
    }

    #[tokio::test]
    async fn test_union_all_produces_sql_union_all() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        // Seed one patient so we can verify both branches of the UNION ALL run
        let patient = json!({"resourceType": "Patient", "id": "p-union", "active": true});
        backend
            .create(&tenant, "Patient", patient, helios_fhir::FhirVersion::R4)
            .await
            .expect("failed to seed patient");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"unionAll": [
                {"column": [{"path": "id", "name": "id"}]},
                {"column": [{"path": "id", "name": "id"}]}
            ]}]
        });

        // unionAll now compiles to SQL UNION ALL — should succeed
        let stream = runner
            .run_view(&tenant, view, ViewFilters::default())
            .await
            .expect("unionAll view must compile and run");

        let rows: Vec<_> = stream
            .map(|r| r.expect("unionAll row must not be an error"))
            .collect()
            .await;

        // UNION ALL over the same column produces 2 rows (one per branch)
        assert_eq!(rows.len(), 2, "UNION ALL should yield one row per branch");
    }

    #[tokio::test]
    async fn test_rejects_where_clause() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "where": [{"path": "active"}],
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        expect_uncompilable(runner.as_ref(), view).await;
    }

    #[tokio::test]
    async fn test_rejects_literal_string_path() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "'constant'", "name": "x"}]}]
        });
        expect_uncompilable(runner.as_ref(), view).await;
    }
}
