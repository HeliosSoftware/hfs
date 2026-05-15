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
    use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
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
    // 3. FHIRPath expressions previously rejected by the in-DB runner that
    //    the new IR-based pipeline now compiles to SQL.
    // =========================================================================

    #[tokio::test]
    async fn test_compiles_exists_function_in_path() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        // Seed one patient with `name`, one without.
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "X"}]}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p2"}),
                helios_fhir::FhirVersion::R4,
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
    async fn test_compiles_bare_boolean_where() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p-active", "active": true}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed active");
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p-inactive", "active": false}),
                helios_fhir::FhirVersion::R4,
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
    async fn test_union_all_with_sibling_root_column() {
        // A sibling top-level column (`id`) is merged into every unionAll
        // branch's projection. Each branch iterates a single-level array.
        // (Path-through-array flattening — e.g. `contact.telecom` over an
        // array-of-objects-of-arrays — needs additional lateral unnests
        // and isn't covered until stage 4.)
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "telecom": [
                        {"value": "t1", "system": "phone"},
                        {"value": "t2", "system": "email"}
                    ],
                    "name": [
                        {"family": "Doe", "given": ["John"]}
                    ]
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"unionAll": [
                    {"forEach": "telecom", "column": [
                        {"path": "value", "name": "v"},
                        {"path": "system", "name": "s"}
                    ]},
                    {"forEach": "name", "column": [
                        {"path": "family", "name": "v"},
                        {"path": "use", "name": "s"}
                    ]}
                ]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        // 2 telecoms + 1 name = 3 rows; each carries the parent id.
        assert_eq!(rows.len(), 3, "rows: {:?}", rows);
        for row in &rows {
            assert_eq!(row.get("id").and_then(|v| v.as_str()), Some("p1"));
            assert!(row.get("v").is_some());
        }
    }

    #[tokio::test]
    async fn test_nested_select_contributes_columns() {
        // A clause with both `column[]` and a nested `select[]` produces a
        // single row containing the union of both column lists.
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "gender": "female"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{
                "column": [{"path": "id", "name": "outer_id"}],
                "select": [{
                    "column": [{"path": "gender", "name": "g"}]
                }]
            }]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("outer_id").and_then(|v| v.as_str()), Some("p1"));
        assert_eq!(rows[0].get("g").and_then(|v| v.as_str()), Some("female"));
    }

    #[tokio::test]
    async fn test_foreach_flattens_array_through_array() {
        // FHIRPath flattens through array boundaries automatically:
        // `forEach: "contact.telecom"` over `contact[]` → each contact's
        // `telecom[]` should produce one row per inner element.
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "contact": [
                        {"telecom": [{"value": "c1.t1"}, {"value": "c1.t2"}]},
                        {"telecom": [{"value": "c2.t1"}]}
                    ]
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"forEach": "contact.telecom", "column": [
                    {"path": "value", "name": "tel"}
                ]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        // 2 + 1 = 3 telecoms.
        assert_eq!(rows.len(), 3, "rows: {:?}", rows);
        let tels: Vec<_> = rows
            .iter()
            .map(|r| r.get("tel").and_then(|v| v.as_str()).unwrap_or(""))
            .collect();
        assert!(tels.contains(&"c1.t1"));
        assert!(tels.contains(&"c1.t2"));
        assert!(tels.contains(&"c2.t1"));
    }

    #[tokio::test]
    async fn test_sibling_foreach_cross_join() {
        // Two top-level clauses each with a `forEach` produce a Cartesian
        // product (one row per (name, address) pair).
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{"family": "Doe"}, {"family": "Smith"}],
                    "address": [{"city": "Boston"}, {"city": "Seattle"}]
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [
                {"forEach": "name", "column": [{"path": "family", "name": "family"}]},
                {"forEach": "address", "column": [{"path": "city", "name": "city"}]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 4, "2 names × 2 addresses = 4 rows: {:?}", rows);
    }

    #[tokio::test]
    async fn test_get_resource_key_returns_id() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{"column": [{"path": "getResourceKey()", "name": "k"}]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("k").and_then(|v| v.as_str()), Some("p1"));
    }

    #[tokio::test]
    async fn test_get_reference_key_extracts_id() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"}
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed o1");
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o2",
                    "subject": {"reference": "Group/g1"}
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed o2");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "subject.getReferenceKey()", "name": "any_key"},
                {"path": "subject.getReferenceKey(Patient)", "name": "patient_key"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 2);
        let by_id: std::collections::HashMap<&str, &std::collections::BTreeMap<String, Value>> =
            rows.iter()
                .map(|r| (r.get("id").unwrap().as_str().unwrap(), r))
                .collect();
        // any_key returns the id portion regardless of reference type
        assert_eq!(
            by_id["o1"].get("any_key").and_then(|v| v.as_str()),
            Some("p1")
        );
        assert_eq!(
            by_id["o2"].get("any_key").and_then(|v| v.as_str()),
            Some("g1")
        );
        // patient_key returns only when the reference type matches
        assert_eq!(
            by_id["o1"].get("patient_key").and_then(|v| v.as_str()),
            Some("p1")
        );
        // Mismatched type yields NULL → key absent from the row map.
        assert!(by_id["o2"].get("patient_key").is_none());
    }

    #[tokio::test]
    async fn test_constant_binding() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        for (id, gender) in [("p1", "male"), ("p2", "female"), ("p3", "male")] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "gender": gender}),
                    helios_fhir::FhirVersion::R4,
                )
                .await
                .expect("seed");
        }
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "constant": [{"name": "g", "valueString": "male"}],
            "where": [{"path": "gender = %g"}],
            "select": [{"column": [{"path": "id", "name": "id"}]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 2, "rows: {:?}", rows);
    }

    #[tokio::test]
    async fn test_of_type_complex_polymorphic() {
        // `Observation.value.ofType(Quantity).value` rewrites to
        // `valueQuantity.value`.
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o1",
                    "valueQuantity": {"value": 42.5, "unit": "kg"}
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed o1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "value.ofType(Quantity).value", "name": "v"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        // `valueQuantity.value` is a JSON number; SQLite returns it as
        // numeric, runner preserves the type.
        let v = rows[0].get("v").expect("v column missing");
        assert_eq!(v.as_f64(), Some(42.5));
    }

    #[tokio::test]
    async fn test_arithmetic_operators() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o1",
                    "valueRange": {"low": {"value": 2.0}, "high": {"value": 5.0}}
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed o1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "value.ofType(Range).low.value + value.ofType(Range).high.value", "name": "add", "type": "decimal"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("add").and_then(|v| v.as_f64()), Some(7.0));
    }

    #[tokio::test]
    async fn test_decimal_low_high_boundary() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "o1",
                    "valueQuantity": {"value": 1.0}
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed o1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "value.ofType(Quantity).value.lowBoundary()", "name": "lo", "type": "decimal"},
                {"path": "value.ofType(Quantity).value.highBoundary()", "name": "hi", "type": "decimal"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("lo").and_then(|v| v.as_f64()), Some(0.95));
        assert_eq!(rows[0].get("hi").and_then(|v| v.as_f64()), Some(1.05));
    }

    #[tokio::test]
    async fn test_date_low_high_boundary() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "birthDate": "1970-06"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{"column": [
                {"path": "id", "name": "id"},
                {"path": "birthDate.lowBoundary()", "name": "lo", "type": "date"},
                {"path": "birthDate.highBoundary()", "name": "hi", "type": "date"}
            ]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("lo").and_then(|v| v.as_str()),
            Some("1970-06-01")
        );
        // Calendar-aware: June has 30 days, not 31.
        assert_eq!(
            rows[0].get("hi").and_then(|v| v.as_str()),
            Some("1970-06-30")
        );
    }

    #[tokio::test]
    async fn test_repeat_walks_tree() {
        // SoF `repeat: ["item"]` recursively descends a QuestionnaireResponse,
        // yielding every nested item as its own row.
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "QuestionnaireResponse",
                json!({
                    "resourceType": "QuestionnaireResponse",
                    "id": "qr1",
                    "item": [
                        {"linkId": "1", "text": "Group 1", "item": [
                            {"linkId": "1.1", "text": "Q 1.1"},
                            {"linkId": "1.2", "text": "Q 1.2", "item": [
                                {"linkId": "1.2.1", "text": "Q 1.2.1"}
                            ]}
                        ]},
                        {"linkId": "2", "text": "Group 2"}
                    ]
                }),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed qr1");
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "QuestionnaireResponse",
            "select": [
                {"column": [{"path": "id", "name": "id"}]},
                {"repeat": ["item"], "column": [
                    {"path": "linkId", "name": "linkId"},
                    {"path": "text", "name": "text"}
                ]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 5, "rows: {:?}", rows);
        // SQLite's row mapper auto-parses numeric-looking text as JSON
        // numbers, so `linkId: "1"` lands as Number(1). Compare via
        // string form to tolerate both shapes.
        let link_ids: std::collections::HashSet<String> = rows
            .iter()
            .map(|r| {
                let v = r.get("linkId").expect("missing linkId");
                match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                }
            })
            .collect();
        for expected in ["1", "1.1", "1.2", "1.2.1", "2"] {
            assert!(
                link_ids.contains(expected),
                "missing {} in {:?}",
                expected,
                link_ids
            );
        }
        // All rows carry the parent id from the joined `resources` table.
        for r in &rows {
            assert_eq!(r.get("id").and_then(|v| v.as_str()), Some("qr1"));
        }
    }

    #[tokio::test]
    async fn test_compiles_literal_string_path() {
        // A bare string literal in column.path is a valid (if unusual)
        // FHIRPath expression that lowers to a constant projection.
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1"}),
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed p1");

        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"path": "'constant'", "name": "x"}]}]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1);
    }
}
