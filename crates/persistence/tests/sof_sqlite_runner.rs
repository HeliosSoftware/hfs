//! Phase 3a integration tests: SQLite in-DB runner.
//!
//! Verifies:
//! 1. `SqliteBackend::sof_runner()` returns the in-DB runner (not `None`).
//! 2. The in-DB runner produces the same rows as the in-process runner for
//!    spec ViewDefinition fixtures (byte-identical column sets).
//! 3. `SofError::Uncompilable` is returned for unsupported ViewDefinitions.

#[cfg(feature = "sqlite")]
#[path = "common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

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

    /// #1569: the first row by `last_updated, id` is a Patient without
    /// `gender`, `birthDate` or `address`. Every row still carries every
    /// column — a missing value is `null`, never an absent key — because the
    /// output formatters take the column list from the first row.
    #[tokio::test]
    async fn a_bare_first_row_keeps_every_column_as_null() {
        let backend = make_backend().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "bare", "name": [{"family": "Bare"}]}),
                FhirVersion::R4,
            )
            .await
            .expect("seed bare patient");
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient", "id": "full", "active": true, "gender": "female",
                    "birthDate": "2015-12-29", "name": [{"family": "Parker433"}],
                    "address": [{"city": "Everett"}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed full patient");

        let view = json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"column": [
                {"name": "id", "path": "getResourceKey()", "type": "id"},
                {"name": "gender", "path": "gender"},
                {"name": "birth_date", "path": "birthDate", "type": "date"},
                {"name": "family", "path": "name.first().family"},
                {"name": "city", "path": "address.first().city"}
            ]}],
            "where": [{"path": "active.exists().not() or active = true"}]
        });
        let runner = backend.sof_runner().expect("runner");
        let mut stream = runner
            .run_view(&tenant, view, ViewFilters::default())
            .await
            .expect("run_view");
        let mut rows: Vec<Value> = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("row"));
        }
        assert_eq!(rows.len(), 2, "{rows:?}");
        let columns = ["id", "gender", "birth_date", "family", "city"];
        for row in &rows {
            let object = row.as_object().expect("object row");
            for column in columns {
                assert!(object.contains_key(column), "{column} missing from {row}");
            }
        }
        let bare = rows
            .iter()
            .find(|r| r["family"] == "Bare")
            .expect("bare row");
        assert_eq!(bare["gender"], Value::Null, "{bare}");
        assert_eq!(bare["birth_date"], Value::Null, "{bare}");
        assert_eq!(bare["city"], Value::Null, "{bare}");
        let full = rows
            .iter()
            .find(|r| r["family"] == "Parker433")
            .expect("full row");
        assert_eq!(full["gender"], "female", "{full}");
        assert_eq!(full["birth_date"], "2015-12-29", "{full}");
        assert_eq!(full["city"], "Everett", "{full}");
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
        // Mismatched type yields NULL, kept in the row as JSON null (#1569).
        assert_eq!(by_id["o2"].get("patient_key"), Some(&Value::Null));
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
                    {"path": "linkId", "name": "linkId", "type": "string"},
                    {"path": "text", "name": "text"}
                ]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 5, "rows: {:?}", rows);
        // `linkId` is declared `string`, so numeric-looking values stay strings.
        let link_ids: std::collections::HashSet<String> = rows
            .iter()
            .map(|r| {
                r.get("linkId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("linkId must be a string: {r:?}"))
                    .to_string()
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

    /// #1769: a string/code column keeps JSON-looking values as strings.
    #[tokio::test]
    async fn test_scalar_string_columns_stay_strings() {
        let backend = make_backend().await;
        let tenant = test_tenant();
        let codes = ["44054006", "0123", "4548-4", "true", "null", "1e3"];
        for (i, code) in codes.iter().enumerate() {
            backend
                .create(
                    &tenant,
                    "Condition",
                    json!({
                        "resourceType": "Condition",
                        "id": format!("c{i}"),
                        "subject": {"reference": "Patient/p1"},
                        "code": {"coding": [{"system": "http://example.org/cs", "code": code}]}
                    }),
                    FhirVersion::R4,
                )
                .await
                .expect("seed condition");
        }
        let runner = backend.sof_runner().expect("in-DB runner");
        for ty in [Some("code"), Some("string"), None] {
            let mut col = json!({"name": "code", "path": "code.coding.first().code"});
            if let Some(t) = ty {
                col["type"] = json!(t);
            }
            let view = json!({
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Condition",
                "select": [{"column": [
                    {"name": "id", "path": "getResourceKey()"},
                    col
                ]}]
            });
            let rows = collect_rows(runner.as_ref(), &tenant, view).await;
            assert_eq!(rows.len(), codes.len(), "type {ty:?}");
            let mut got: Vec<String> = rows
                .iter()
                .map(|r| {
                    r.get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| panic!("type {ty:?}: code must be a string: {r:?}"))
                        .to_string()
                })
                .collect();
            got.sort();
            let mut want: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
            want.sort();
            assert_eq!(got, want, "type {ty:?}");
        }
    }

    #[tokio::test]
    async fn test_untyped_columns_keep_json_shape() {
        let backend = make_backend().await;
        let tenant = test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "active": true,
                    "name": [{"family": "123", "given": ["Peter"]}]
                }),
                FhirVersion::R4,
            )
            .await
            .expect("seed patient");
        let runner = backend.sof_runner().expect("in-DB runner");
        let view = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {"column": [
                    {"name": "given", "path": "name.given"},
                    {"name": "active", "path": "active"}
                ]},
                {"forEach": "name", "column": [{"name": "family", "path": "family"}]}
            ]
        });
        let rows = collect_rows(runner.as_ref(), &tenant, view).await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        // A repeating last field comes back as the JSON array, not as text.
        assert_eq!(rows[0]["given"], json!(["Peter"]), "{:?}", rows[0]);
        // A boolean is a boolean, not SQLite's INTEGER 1.
        assert_eq!(rows[0]["active"], json!(true), "{:?}", rows[0]);
        // A string under forEach keeps its type even when it reads as a number.
        assert_eq!(rows[0]["family"], json!("123"), "{:?}", rows[0]);
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

    static PREVIEW_SQL_TRACE: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

    fn capture_preview_sql(event: rusqlite::trace::TraceEvent<'_>) {
        if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event {
            PREVIEW_SQL_TRACE
                .lock()
                .unwrap()
                .push(statement.sql().into_owned());
        }
    }

    #[tokio::test]
    async fn test_sqlite_preview_limit_is_in_executed_sql_and_none_is_unlimited() {
        // A private single-connection pool makes :memory: and the trace
        // callback belong to this test, without exposing backend internals.
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::memory())
            .unwrap();
        let tenant = test_tenant();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch(
                "CREATE TABLE resources (
                tenant_id TEXT NOT NULL, resource_type TEXT NOT NULL, id TEXT NOT NULL,
                data TEXT NOT NULL, last_updated TEXT NOT NULL, is_deleted INTEGER NOT NULL
            ); CREATE INDEX test_sof_order ON resources(tenant_id,resource_type,last_updated,id);",
            )
            .unwrap();
            for index in 0..80 {
                let id = format!("p-{index:03}");
                let data = json!({"resourceType":"Patient", "id":id}).to_string();
                conn.execute(
                    "INSERT INTO resources VALUES (?1,'Patient',?2,?3,?4,0)",
                    rusqlite::params![
                        tenant.tenant_id().to_string(),
                        id,
                        data,
                        format!("2024-01-01T00:{:02}:{:02}Z", index / 60, index % 60)
                    ],
                )
                .unwrap();
            }
            conn.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(capture_preview_sql),
            );
        }
        let runner = helios_persistence::sof::sqlite::SqliteInDbRunner::new(pool);
        let alias = format!("sof_sqlite_sql_limit_{}", uuid::Uuid::new_v4().simple());
        let view = preview_flat_view("Patient", &alias);
        let unlimited =
            collect_rows_in_order(&runner, &tenant, view.clone(), ViewFilters::default()).await;
        let limited = collect_rows_in_order(
            &runner,
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
        let statements = PREVIEW_SQL_TRACE
            .lock()
            .unwrap()
            .iter()
            .filter(|sql| sql.contains(&format!("\"{alias}\"")))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(statements.len(), 2, "{statements:?}");
        assert!(!statements[0].contains("LIMIT"), "{statements:?}");
        assert!(
            statements[1].ends_with("\nLIMIT 50"),
            "executed preview SQL: {statements:?}"
        );
        assert!(
            statements
                .iter()
                .all(|sql| sql.contains("?1") && sql.contains("?2")),
            "tenant and resource type must remain bound: {statements:?}"
        );
    }

    static COMPLEX_LIMIT_SQL_TRACE: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

    fn capture_complex_limit_sql(event: rusqlite::trace::TraceEvent<'_>) {
        if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event {
            COMPLEX_LIMIT_SQL_TRACE
                .lock()
                .unwrap()
                .push(statement.sql().into_owned());
        }
    }

    /// A file-backed backend that indexes compartment references (spec
    /// SearchParameters from `data/`), and an in-DB runner over a private
    /// single-connection pool on the same file whose statements are traced
    /// into [`COMPLEX_LIMIT_SQL_TRACE`]. Only this test registers that trace.
    fn make_traced_indexed_runner() -> (
        Arc<SqliteBackend>,
        helios_persistence::sof::sqlite::SqliteInDbRunner,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("fixture directory");
        let path = dir.path().join("sof.db");
        let config = helios_persistence::backends::sqlite::SqliteBackendConfig {
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        let backend =
            SqliteBackend::with_config(&path, config).expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let manager = r2d2_sqlite::SqliteConnectionManager::file(&path).with_init(|conn| {
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            helios_persistence::sof::sqlite_udfs::register(conn)?;
            conn.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(capture_complex_limit_sql),
            );
            Ok(())
        });
        let pool = r2d2::Pool::builder().max_size(1).build(manager).unwrap();
        let runner = helios_persistence::sof::sqlite::SqliteInDbRunner::new(pool);
        (Arc::new(backend), runner, dir)
    }

    /// Runs one view and returns its rows with the view statements it
    /// executed (the traced text, before parameter expansion).
    async fn observe_traced_run(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: Value,
        filters: ViewFilters,
    ) -> (Vec<Value>, Vec<String>) {
        COMPLEX_LIMIT_SQL_TRACE.lock().unwrap().clear();
        let rows = collect_rows_in_order(runner, tenant, view, filters).await;
        let statements = COMPLEX_LIMIT_SQL_TRACE
            .lock()
            .unwrap()
            .iter()
            .filter(|sql| sql.contains("FROM resources r"))
            .cloned()
            .collect();
        (rows, statements)
    }

    /// The run executed exactly one view statement, ending in exactly one
    /// final `LIMIT {limit}` iff a representable limit applies.
    fn assert_traced_execution(statements: &[String], final_limit: Option<usize>, case: &str) {
        let [sql] = statements else {
            panic!("{case}: expected one executed statement, got {statements:?}");
        };
        match final_limit {
            Some(limit) => {
                assert!(sql.ends_with(&format!("\nLIMIT {limit}")), "{case}: {sql}");
                assert_eq!(sql.matches("\nLIMIT ").count(), 1, "{case}: {sql}");
                let body = &sql[..sql.len() - format!("\nLIMIT {limit}").len()];
                let tail = &body[body.rfind("ORDER BY ").expect("final ORDER BY")..];
                assert_eq!(
                    tail.matches('(').count(),
                    tail.matches(')').count(),
                    "{case}: the LIMIT follows the top-level ORDER BY: {sql}"
                );
            }
            None => assert!(!sql.contains("\nLIMIT "), "{case}: {sql}"),
        }
        assert!(
            sql.contains("?1") && sql.contains("?2"),
            "{case}: tenant and resource type stay bound: {sql}"
        );
    }

    /// Limits 0, 1, 50, one above every fixture's row count, and (64-bit)
    /// an oversized `usize`, which keeps the client-side cap only.
    fn complex_limit_matrix() -> Vec<(usize, Option<usize>)> {
        let mut limits = vec![
            (0, Some(0)),
            (1, Some(1)),
            (50, Some(50)),
            (10_000, Some(10_000)),
        ];
        #[cfg(target_pointer_width = "64")]
        limits.push((usize::MAX, None));
        limits
    }

    /// Runs `view` unlimited and under every [`complex_limit_matrix`] limit:
    /// each limited result is exactly the unlimited prefix, and the executed
    /// statement carries one final LIMIT only for representable limits.
    /// Returns the unlimited rows.
    async fn assert_complex_final_limits(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        view: &Value,
        filters: &ViewFilters,
        total: usize,
        case: &str,
    ) -> Vec<Value> {
        let (unlimited, statements) =
            observe_traced_run(runner, tenant, view.clone(), filters.clone()).await;
        assert_eq!(unlimited.len(), total, "{case}: unlimited count");
        assert_traced_execution(&statements, None, &format!("{case}: unlimited"));
        for (limit, final_limit) in complex_limit_matrix() {
            let (limited, statements) = observe_traced_run(
                runner,
                tenant,
                view.clone(),
                ViewFilters {
                    limit: Some(limit),
                    ..filters.clone()
                },
            )
            .await;
            assert_eq!(
                limited,
                unlimited[..limit.min(total)],
                "{case}: limit {limit} must yield the unlimited prefix"
            );
            assert_traced_execution(&statements, final_limit, &format!("{case}: limit {limit}"));
        }
        unlimited
    }

    #[tokio::test]
    async fn test_sqlite_complex_final_limits_0_1_50_large() {
        let (backend, runner, _dir) = make_traced_indexed_runner();
        let context =
            |name: &str| TenantContext::new(TenantId::new(name), TenantPermissions::full_access());

        // Shape matrix: one large resource per type plus small resources, so
        // every shape yields more than 50 rows and each cut falls inside it.
        let tenant = context("shapes");
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
                &runner,
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
        let filtered = context("filtered");
        let other = context("other");
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
            let unlimited =
                assert_complex_final_limits(&runner, &filtered, view, &filters, total, case).await;
            // Resolving the Group yields exactly the same rows under every limit.
            let via_group = assert_complex_final_limits(
                &runner,
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
            &runner,
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
                &runner,
                &other,
                &qr_repeat,
                &group_filters(group),
                total,
                &format!("other-tenant-repeat-{group}"),
            )
            .await;
        }
        assert_complex_final_limits(
            &runner,
            &other,
            &qr_repeat_union,
            &group_filters("g-other"),
            150 + 1,
            "other-tenant-repeat-union-g-other",
        )
        .await;
    }

    async fn seed_preview_patient(
        backend: &SqliteBackend,
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
    async fn test_sqlite_preview_limit_preserves_flat_observation_and_patient_prefix() {
        let backend = make_backend().await;
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
    async fn test_sqlite_preview_limit_applies_after_where() {
        let backend = make_backend().await;
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
    async fn test_sqlite_preview_limit_preserves_foreach_prefix_with_interior_cut() {
        let backend = make_backend().await;
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
    async fn test_sqlite_preview_limit_is_global_across_union_all() {
        let backend = make_backend().await;
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
    async fn test_sqlite_preview_limit_preserves_constants_runtime_filters_and_tenant() {
        let backend = make_backend().await;
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
    async fn test_sqlite_large_nested_chained_cartesian_and_nullable_preview_prefixes() {
        let backend = make_backend().await;
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
    async fn test_sqlite_flat_union_ties_keep_second_column_prefix() {
        let backend = make_backend().await;
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
    async fn test_sqlite_large_repeat_nested_multipath_and_union_preview_prefixes() {
        let backend = make_backend().await;
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
    async fn test_sqlite_large_expansion_preserves_runtime_filters_constants_and_isolation() {
        let backend = make_backend().await;
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
        backend: &SqliteBackend,
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

    /// Compartment filters read `search_index`, which needs the spec
    /// SearchParameters from the workspace `data/` directory.
    fn make_indexed_backend() -> Arc<SqliteBackend> {
        let config = helios_persistence::backends::sqlite::SqliteBackendConfig {
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", config)
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        Arc::new(backend)
    }

    #[tokio::test]
    async fn test_sqlite_union_and_repeat_runtime_filters_lower_into_every_scan() {
        let backend = make_indexed_backend();
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
    // `forEachOrNull` miss is -1). Union: first visible column (NULLS FIRST on
    // SQLite), last_updated, id, common enclosing ordinals, flattened
    // branch number, branch-local identity. Fixture timestamps are set
    // explicitly so no oracle depends on insertion timing.
    // =========================================================================

    /// One expected output row. The SQLite row mapper keeps SQL NULLs as
    /// JSON `null`.
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
        let unlimited =
            collect_rows_in_order(runner, tenant, view.clone(), ViewFilters::default()).await;
        assert_eq!(unlimited.len(), expected.len(), "{case}: row count");
        for (index, (actual, wanted)) in unlimited.iter().zip(expected).enumerate() {
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

    /// A file-backed backend, so the test can rewrite fixture timestamps
    /// through its own connection.
    fn make_file_backend() -> (Arc<SqliteBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("fixture directory");
        let backend = SqliteBackend::with_config(dir.path().join("sof.db"), Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        (Arc::new(backend), dir)
    }

    /// Fixture-only rewrite of `resources.last_updated` (stored as RFC 3339
    /// text, like the backend writes it), scoped by tenant, type and id.
    fn set_last_updated(
        dir: &tempfile::TempDir,
        tenant: &TenantContext,
        resource_type: &str,
        updates: &[(&str, &str)],
    ) {
        let conn = rusqlite::Connection::open(dir.path().join("sof.db")).expect("open fixture db");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        for (id, at) in updates {
            let at: chrono::DateTime<chrono::Utc> = at.parse().expect("fixture timestamp");
            let changed = conn
                .execute(
                    "UPDATE resources SET last_updated = ?1 \
                     WHERE tenant_id = ?2 AND resource_type = ?3 AND id = ?4",
                    rusqlite::params![
                        at.to_rfc3339(),
                        tenant.tenant_id().as_str(),
                        resource_type,
                        id
                    ],
                )
                .expect("rewrite fixture timestamp");
            assert_eq!(changed, 1, "{resource_type}/{id}");
        }
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
    async fn test_sqlite_ordering_contract_large_fixture_oracles() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "Patient",
            &[
                ("p-large", "2024-01-01T00:00:01Z"),
                ("p-empty", "2024-01-01T00:00:02Z"),
                ("p-filtered", "2024-01-01T00:00:03Z"),
            ],
        );
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
    async fn test_sqlite_ordering_contract_tied_timestamps_and_iteration_sources() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "Patient",
            &[("t-c", tied), ("t-b", tied), ("t-a", tied)],
        );
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
        // SQLite's default ASC places the NULL first-column rows first.
        let nullable_first_column = vec![
            row(&[("g", Value::Null), ("v", json!("t-a"))]),
            row(&[("g", Value::Null), ("v", json!("t-c"))]),
            row(&[("g", json!("male")), ("v", json!("t-b"))]),
            row(&[("g", json!("zz")), ("v", json!("t-a"))]),
            row(&[("g", json!("zz")), ("v", json!("t-b"))]),
            row(&[("g", json!("zz")), ("v", json!("t-c"))]),
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
        // contents and occurrence order (`json_each.rowid` is the ordinal for
        // the two-argument primitive form and the type-guarded wrappers).
        let sources = TenantContext::new(
            TenantId::new(format!("sources_{}", uuid::Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        );
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
            &dir,
            &sources,
            "Patient",
            &[
                ("s-2", "2024-03-03T00:00:02Z"),
                ("s-1", "2024-03-03T00:00:01Z"),
            ],
        );
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

    /// The emitter orders SQLite expansions by `json_each.rowid`. Pin the
    /// bundled library's behaviour it relies on: a qualified `alias.rowid` is
    /// the zero-based INTEGER element position for array, object and
    /// primitive sources, it restarts for every invocation (outer row), and a
    /// LEFT JOIN miss yields NULL (lowered to -1).
    #[test]
    fn test_sqlite_json_each_rowid_is_a_per_invocation_occurrence_ordinal() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE t(id TEXT, data TEXT);
            INSERT INTO t VALUES
              ('a', '{"arr":[10,20,30],"obj":{"x":1,"y":2},"prim":"p"}'),
              ('b', '{"arr":[40,50],"obj":{"q":9},"prim":7}'),
              ('c', '{}');"#,
        )
        .unwrap();
        let rows = |sql: &str| -> Vec<String> {
            let mut statement = conn.prepare(sql).unwrap();
            statement
                .query_map([], |row| {
                    Ok(format!(
                        "{}:{}:{}",
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        assert_eq!(
            rows(
                "SELECT t.id, fe.rowid, CAST(fe.value AS TEXT) FROM t, json_each(t.data, '$.arr') fe \
                  ORDER BY t.id, fe.rowid DESC"
            ),
            ["a:2:30", "a:1:20", "a:0:10", "b:1:50", "b:0:40"]
        );
        assert_eq!(
            rows(
                "SELECT t.id, fe.rowid, fe.key FROM t, json_each(t.data, '$.obj') fe ORDER BY t.id, fe.rowid"
            ),
            ["a:0:x", "a:1:y", "b:0:q"]
        );
        assert_eq!(
            rows(
                "SELECT t.id, fe.rowid, CAST(fe.value AS TEXT) FROM t, json_each(t.data, '$.prim') fe ORDER BY t.id, fe.rowid"
            ),
            ["a:0:p", "b:0:7"]
        );
        // Type-guarded single-argument form (nested / object sources).
        assert_eq!(
            rows(
                "SELECT t.id, fe.rowid, typeof(fe.value) FROM t, json_each(CASE \
                  WHEN json_type(t.data, '$.obj') IN ('object', 'array') \
                  THEN json_array(json(json_extract(t.data, '$.obj'))) ELSE '[]' END) fe ORDER BY t.id"
            ),
            ["a:0:text", "b:0:text"]
        );
        assert_eq!(
            rows(
                "SELECT t.id, COALESCE(fe.rowid, -1), typeof(fe.rowid) FROM t \
                  LEFT JOIN json_each(t.data, '$.arr') fe ON 1=1 AND fe.value > 25 \
                  ORDER BY t.id, COALESCE(fe.rowid, -1)"
            ),
            ["a:2:integer", "b:0:integer", "b:1:integer", "c:-1:null"]
        );
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        assert!(!version.is_empty());
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
    async fn test_sqlite_repeat_indices_and_order_match_evaluator() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        );
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
        // ASC NULL placement (SQLite: NULLS FIRST).
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
            family(Value::Null, 2),
            family(json!("F1"), 0),
            family(json!("F2"), 1),
            family(json!("K1"), 3),
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
    async fn test_sqlite_indexed_iteration_rows_match_evaluator() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "Patient",
            &[("ix-c", tied), ("ix-b", tied), ("ix-a", tied)],
        );
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
    async fn test_sqlite_indexed_iteration_after_and_under_repeat_honors_membership() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        );
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
    async fn test_sqlite_indexed_selection_of_json_null_is_present() {
        let backend = make_backend().await;
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
        // indexed selection `[N]` must be exactly occurrence N, plus
        // `%rowIndex` 0.
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
            let expected: Vec<Value> = ordinary
                .get(index)
                .map(|occurrence| {
                    let mut selected = occurrence.clone();
                    selected["i"] = json!(0);
                    selected
                })
                .into_iter()
                .collect();
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
    async fn test_sqlite_indexed_trailing_where_filters_the_selection_like_the_evaluator() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "Patient",
            &[("p-large", tied), ("p-filtered", tied), ("p-empty", tied)],
        );
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
    async fn test_sqlite_where_row_index_reads_the_enclosing_scope_like_the_evaluator() {
        let (backend, dir) = make_file_backend();
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
            &dir,
            &tenant,
            "Patient",
            &[("p-large", tied), ("p-filtered", tied), ("p-empty", tied)],
        );
        set_last_updated(
            &dir,
            &tenant,
            "QuestionnaireResponse",
            &[("qr-c", tied), ("qr-b", tied), ("qr-a", tied)],
        );
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
    async fn test_sqlite_direct_ir_flat_index_executes_with_indexed_semantics() {
        let (backend, dir) = make_file_backend();
        let tenant = test_tenant();
        for patient in flat_index_fixture() {
            backend
                .create(&tenant, "Patient", patient, FhirVersion::R4)
                .await
                .expect("seed flat_index fixture");
        }
        let tied = "2024-07-07T00:00:00Z";
        set_last_updated(&dir, &tenant, "Patient", &[("fx-a", tied), ("fx-b", tied)]);
        let conn = rusqlite::Connection::open(dir.path().join("sof.db")).expect("open fixture db");
        for (case, plan, expected) in flat_index_cases() {
            let emitted = helios_persistence::sof::emit::emit_plan(
                &plan,
                &helios_persistence::sof::dialect::SqliteDialect,
            )
            .unwrap_or_else(|e| panic!("{case}: emit: {e}"));
            let mut statement = conn
                .prepare(&emitted.sql)
                .unwrap_or_else(|e| panic!("{case}: prepare: {e}\n{}", emitted.sql));
            let width = emitted.columns.len();
            let rows: Vec<Vec<Option<String>>> = statement
                .query_map(
                    rusqlite::params![tenant.tenant_id().as_str(), "Patient"],
                    |row| {
                        (0..width)
                            .map(|i| row.get::<_, Option<String>>(i))
                            .collect::<Result<Vec<_>, _>>()
                    },
                )
                .unwrap_or_else(|e| panic!("{case}: query: {e}"))
                .collect::<Result<_, _>>()
                .unwrap_or_else(|e| panic!("{case}: rows: {e}"));
            let expected: Vec<Vec<Option<String>>> = expected
                .iter()
                .map(|r| r.iter().map(|v| v.map(str::to_string)).collect())
                .collect();
            assert_eq!(rows, expected, "{case}\n{}", emitted.sql);
        }
    }

    // =========================================================================
    // #1623: SQLite's documented order is total too, so neither statistics,
    // indexes nor connection PRAGMAs may change the unlimited rows, their
    // order, or the limited prefix. One file-backed database, seeded once;
    // every condition opens a fresh runner pool whose connections apply that
    // condition's PRAGMAs. Plans are logged, not asserted.
    // =========================================================================

    struct StatisticsCondition {
        name: &'static str,
        /// Drop every secondary index on `resources` first (persists).
        drop_indexes: bool,
        /// Run `ANALYZE` first; statistics persist for later conditions.
        analyze_first: bool,
        /// Applied to every runner-pool connection.
        pragmas: &'static str,
    }

    fn statistics_conditions() -> Vec<StatisticsCondition> {
        let condition = |name, drop_indexes, analyze_first, pragmas| StatisticsCondition {
            name,
            drop_indexes,
            analyze_first,
            pragmas,
        };
        vec![
            condition("no-statistics", false, false, ""),
            condition(
                "no-statistics-reverse-unordered",
                false,
                false,
                "PRAGMA reverse_unordered_selects = ON;",
            ),
            condition("after-analyze", false, true, ""),
            condition(
                "reverse-unordered-selects",
                false,
                false,
                "PRAGMA reverse_unordered_selects = ON;",
            ),
            condition(
                "automatic-index-off",
                false,
                false,
                "PRAGMA automatic_index = OFF;",
            ),
            condition(
                "small-cache-file-temp",
                false,
                false,
                "PRAGMA cache_size = -64; PRAGMA temp_store = FILE;",
            ),
            condition(
                "large-cache-memory-temp",
                false,
                false,
                "PRAGMA cache_size = -65536; PRAGMA temp_store = MEMORY;",
            ),
            condition("secondary-indexes-dropped", true, true, ""),
            condition(
                "secondary-indexes-dropped-reverse-unordered",
                false,
                false,
                "PRAGMA reverse_unordered_selects = ON;",
            ),
        ]
    }

    /// Writes the shared prefix-matrix fixture straight into `resources`
    /// (JSON bytes and RFC 3339 text, like the backend), in its reverse id
    /// order, plus noise: other tenants holding copies of every row and a
    /// large unrelated resource type in the tested tenant.
    fn seed_statistics_fixture(path: &std::path::Path, tenant: &str) {
        let mut conn = rusqlite::Connection::open(path).expect("open fixture db");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO resources \
                     (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at) \
                     VALUES (?1, ?2, ?3, '1', ?4, ?5, ?6, ?7)",
                )
                .unwrap();
            for resource in super::sof_prefix_matrix::fixture() {
                let at = resource.last_updated.to_rfc3339();
                insert
                    .execute(rusqlite::params![
                        tenant,
                        resource.resource_type,
                        resource.id,
                        serde_json::to_vec(&resource.data).unwrap(),
                        at,
                        resource.deleted,
                        resource.deleted.then(|| at.clone()),
                    ])
                    .expect("seed fixture resource");
            }
            for other in ["statistics_noise_a", "statistics_noise_b"] {
                tx.execute(
                    "INSERT INTO resources \
                     (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at) \
                     SELECT ?2, resource_type, id, version_id, data, last_updated, is_deleted, deleted_at \
                     FROM resources WHERE tenant_id = ?1",
                    rusqlite::params![tenant, other],
                )
                .expect("seed noise tenant");
            }
            let base = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").unwrap();
            for index in 1..=6000 {
                let id = format!("obs-{index}");
                insert
                    .execute(rusqlite::params![
                        tenant,
                        "Observation",
                        id,
                        serde_json::to_vec(
                            &json!({"resourceType":"Observation","id":id,"status":"final"})
                        )
                        .unwrap(),
                        (base + chrono::Duration::seconds(index)).to_rfc3339(),
                        false,
                        None::<String>,
                    ])
                    .expect("seed noise resource type");
            }
        }
        tx.commit().unwrap();
    }

    /// A fresh single-purpose runner pool on the fixture file whose
    /// connections apply `pragmas`.
    fn statistics_runner(
        path: &std::path::Path,
        pragmas: &'static str,
    ) -> (
        r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
        helios_persistence::sof::sqlite::SqliteInDbRunner,
    ) {
        let manager = r2d2_sqlite::SqliteConnectionManager::file(path).with_init(move |conn| {
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            helios_persistence::sof::sqlite_udfs::register(conn)?;
            conn.execute_batch(pragmas)?;
            Ok(())
        });
        let pool = r2d2::Pool::builder().max_size(2).build(manager).unwrap();
        let runner = helios_persistence::sof::sqlite::SqliteInDbRunner::new(pool.clone());
        (pool, runner)
    }

    /// The limited preview plan's `EXPLAIN QUERY PLAN` details for `view`,
    /// from a runner-pool connection (logged, never asserted).
    fn explain_sqlite_preview(conn: &rusqlite::Connection, tenant: &str, view: &Value) -> String {
        let compiled = helios_persistence::sof::compiler::compile_view_definition_dialect(
            view,
            helios_persistence::sof::compiler::SqlDialect::Sqlite,
            FhirVersion::R4,
        )
        .expect("compile explained view");
        let resource_type = view["resource"].as_str().unwrap();
        let statement = format!("EXPLAIN QUERY PLAN {}\nLIMIT 50", compiled.sql);
        let result = conn.prepare(&statement).and_then(|mut statement| {
            statement
                .query_map(rusqlite::params![tenant, resource_type], |row| {
                    row.get::<_, String>(3)
                })?
                .collect::<Result<Vec<_>, _>>()
        });
        match result {
            Ok(details) => details.join(" | "),
            Err(error) => format!("EXPLAIN QUERY PLAN failed: {error}"),
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
    async fn test_sqlite_complex_prefix_statistics_matrix() {
        let started = std::time::Instant::now();
        let dir = tempfile::tempdir().expect("fixture directory");
        let path = dir.path().join("sof.db");
        let backend =
            SqliteBackend::with_config(&path, Default::default()).expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let tenant_id = "statistics_matrix";
        let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
        seed_statistics_fixture(&path, tenant_id);
        let admin = rusqlite::Connection::open(&path).expect("open admin connection");
        admin
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();

        let shapes = super::sof_prefix_matrix::shapes();
        let mut oracle: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
        let mut baseline_plans: BTreeMap<&str, String> = BTreeMap::new();
        let mut failures: Vec<String> = Vec::new();
        for condition in statistics_conditions() {
            let condition_started = std::time::Instant::now();
            if condition.drop_indexes {
                let indexes: Vec<String> = admin
                    .prepare(
                        "SELECT name FROM sqlite_master \
                         WHERE type = 'index' AND tbl_name = 'resources' AND sql IS NOT NULL",
                    )
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                assert!(!indexes.is_empty(), "resources has secondary indexes");
                for index in indexes {
                    admin
                        .execute_batch(&format!("DROP INDEX \"{index}\""))
                        .expect("drop index");
                }
            }
            if condition.analyze_first {
                admin.execute_batch("ANALYZE").expect("ANALYZE");
            }
            let (pool, runner) = statistics_runner(&path, condition.pragmas);
            let conn = pool.get().expect("runner-pool connection");
            let pragma = |name: &str| -> i64 {
                conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
                    .unwrap()
            };
            let statistics: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = 'sqlite_stat1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let secondary_indexes: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master \
                     WHERE type = 'index' AND tbl_name = 'resources' AND sql IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            println!(
                "[statistics-matrix] condition={} sqlite_stat1={} resources_secondary_indexes={} \
                 reverse_unordered_selects={} automatic_index={} cache_size={} temp_store={}",
                condition.name,
                statistics,
                secondary_indexes,
                pragma("reverse_unordered_selects"),
                pragma("automatic_index"),
                pragma("cache_size"),
                pragma("temp_store")
            );
            for (case, view) in shapes
                .iter()
                .map(|(case, view, _)| (*case, view))
                .filter(|(case, _)| super::sof_prefix_matrix::EXPLAINED_SHAPES.contains(case))
            {
                let signature = explain_sqlite_preview(&conn, tenant_id, view);
                let changed = baseline_plans
                    .entry(case)
                    .or_insert_with(|| signature.clone())
                    != &signature;
                println!(
                    "[statistics-matrix]   plan condition={} shape={case} changed_vs_first={changed}: {signature}",
                    condition.name
                );
            }
            drop(conn);

            for (case, view, minimum) in &shapes {
                let mut runs = Vec::new();
                for _ in 0..3 {
                    runs.push(
                        collect_rows_in_order(
                            &runner,
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
                    &runner,
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
                "[statistics-matrix] condition={} rows ok, {:?}",
                condition.name,
                condition_started.elapsed()
            );
        }
        println!(
            "[statistics-matrix] shapes={} rows={:?} total {:?}",
            shapes.len(),
            oracle
                .iter()
                .map(|(case, rows)| (*case, rows.len()))
                .collect::<Vec<_>>(),
            started.elapsed()
        );
        drop(backend);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
