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
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
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

    #[cfg(all(feature = "R4", feature = "R5"))]
    #[tokio::test]
    async fn configured_fhir_version_controls_cardinality_and_compartment() {
        use helios_persistence::core::sof_runner::SofError;

        for version in [FhirVersion::R4, FhirVersion::R5] {
            let backend = SqliteBackend::with_config(
                ":memory:",
                SqliteBackendConfig {
                    fhir_version: version,
                    data_dir: Some(
                        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"),
                    ),
                    ..Default::default()
                },
            )
            .expect("create configured backend");
            backend.init_schema().expect("initialize schema");
            let tenant = test_tenant();
            let runner = backend.sof_runner().expect("runner");

            // Composition.subject is a singleton Reference in R4 and a
            // collection of References in R5. The same path is valid in both.
            let cardinality = runner
                .run_view(
                    &tenant,
                    json!({"resourceType":"ViewDefinition", "resource":"Composition",
                        "select":[{"column":[{"name":"subject", "path":"subject.reference",
                            "collection":false}]}]}),
                    ViewFilters::default(),
                )
                .await;
            if version == FhirVersion::R5 {
                assert!(matches!(
                    cardinality,
                    Err(SofError::InvalidViewDefinition(_))
                ));
            } else {
                let rows = cardinality
                    .expect("R4 singleton path must compile")
                    .collect::<Vec<_>>()
                    .await;
                assert!(rows.is_empty());
            }

            for patient in ["p1", "p2"] {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        json!({"resourceType":"Patient", "id":patient}),
                        version,
                    )
                    .await
                    .expect("seed patient");
                backend
                    .create(&tenant, "Task", json!({"resourceType":"Task", "id":format!("task-{patient}"),
                        "status":"requested", "intent":"order", "for":{"reference":format!("Patient/{patient}")}}), version)
                    .await
                    .expect("seed task");
            }
            let view = json!({"resourceType":"ViewDefinition", "resource":"Task",
                "select":[{"column":[{"path":"id", "name":"task_id"}]}]});
            let unfiltered = collect_rows_in_order(
                runner.as_ref(),
                &tenant,
                view.clone(),
                ViewFilters::default(),
            )
            .await;
            assert_eq!(
                unfiltered.len(),
                2,
                "seeded tasks must be visible for {version}"
            );
            let rows = collect_rows_in_order(
                runner.as_ref(),
                &tenant,
                view,
                ViewFilters {
                    patient: vec!["Patient/p1".into()],
                    ..Default::default()
                },
            )
            .await;
            // Task is a Patient-compartment member in R5, but not in R4.
            let ids: Vec<&str> = rows
                .iter()
                .map(|row| row["task_id"].as_str().unwrap())
                .collect();
            if version == FhirVersion::R5 {
                assert_eq!(ids, ["task-p1"]);
            } else {
                assert!(ids.is_empty());
            }
        }
    }

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

    /// A quote, a backslash, or both in a `where` string literal or in a
    /// string constant must match exactly the stored value: the literal is
    /// inlined through the dialect's string literal, the constant is bound.
    #[tokio::test]
    async fn test_string_literals_and_constants_with_quotes_and_backslashes() {
        let backend = make_backend().await;
        let runner = backend.sof_runner().expect("must have runner");
        let tenant = test_tenant();
        let families = [
            ("p-plain", "Smith"),
            ("p-quote", "O'Brien"),
            ("p-backslash", "Back\\slash"),
            ("p-both", "it's a\\b"),
            ("p-escape", "a\\'b"),
        ];
        for (id, family) in families {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]}),
                    helios_fhir::FhirVersion::R4,
                )
                .await
                .expect("seed");
        }
        // FHIRPath source for a string: `\` and `'` are backslash-escaped.
        let fhirpath_string =
            |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
        for (id, family) in families {
            let literal_view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "where": [{"path": format!("name.first().family = {}", fhirpath_string(family))}],
                "select": [{"column": [{"path": "id", "name": "id"}]}]
            });
            let constant_view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Patient",
                "constant": [{"name": "f", "valueString": family}],
                "where": [{"path": "name.first().family = %f"}],
                "select": [{"column": [{"path": "id", "name": "id"}]}]
            });
            for (kind, view) in [("literal", literal_view), ("constant", constant_view)] {
                let rows = collect_rows(runner.as_ref(), &tenant, view).await;
                assert_eq!(rows.len(), 1, "{kind} {family:?}: {rows:?}");
                assert_eq!(rows[0]["id"], id, "{kind} {family:?}");
            }
        }
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

    /// Backend that loads the full SearchParameter set, so
    /// `QuestionnaireResponse.subject` is indexed for compartment filters.
    async fn make_backend_with_search_params() -> Arc<SqliteBackend> {
        let config = SqliteBackendConfig {
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", config)
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        Arc::new(backend)
    }

    /// #1701: runtime filters must reach every `unionAll` branch, not just the last.
    #[tokio::test]
    async fn runtime_filters_restrict_every_union_all_branch() {
        let backend = make_backend().await;
        seed_patients(
            &backend,
            &[("p1", "male", "1990-01-01"), ("p2", "female", "1991-02-02")],
        )
        .await;
        let tenant = test_tenant();
        let runner = backend.sof_runner().unwrap();
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"unionAll":[
                {"column":[{"path":"id","name":"value"}]},
                {"column":[{"path":"gender","name":"value"}]}]}]});
        let values = |rows: Vec<Value>| {
            let mut v: Vec<String> = rows
                .iter()
                .map(|row| row["value"].as_str().unwrap().to_string())
                .collect();
            v.sort();
            v
        };
        let rows = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            view.clone(),
            ViewFilters {
                patient: vec!["Patient/p1".into()],
                ..Default::default()
            },
        )
        .await;
        assert_eq!(values(rows), ["male", "p1"]);
        let rows = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            view,
            ViewFilters {
                since: Some(chrono::Utc::now() + chrono::Duration::days(1)),
                ..Default::default()
            },
        )
        .await;
        assert!(rows.is_empty(), "{rows:?}");
    }

    /// #1701: runtime filters must reach `repeat` seeds and the join back to
    /// `resources`; a node-only `repeat` used to fail to prepare.
    #[tokio::test]
    async fn runtime_filters_restrict_repeat_views() {
        let backend = make_backend_with_search_params().await;
        let tenant = test_tenant();
        for qr in [
            json!({"resourceType":"QuestionnaireResponse", "id":"qr-1", "status":"completed",
                "subject":{"reference":"Patient/p1"},
                "item":[{"linkId":"a"}, {"linkId":"b","item":[{"linkId":"b.1"}]}]}),
            json!({"resourceType":"QuestionnaireResponse", "id":"qr-2", "status":"completed",
                "subject":{"reference":"Patient/p2"},
                "item":[{"linkId":"z"}]}),
        ] {
            backend
                .create(&tenant, "QuestionnaireResponse", qr, FhirVersion::R4)
                .await
                .expect("failed to seed questionnaire response");
        }
        let runner = backend.sof_runner().unwrap();
        let node_only = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]}]});
        let with_join_back = json!({"resourceType":"ViewDefinition",
            "resource":"QuestionnaireResponse",
            "select":[{"column":[{"path":"id","name":"qr"}]},
                {"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]}]});
        for view in [node_only, with_join_back] {
            let rows = collect_rows_in_order(
                runner.as_ref(),
                &tenant,
                view.clone(),
                ViewFilters {
                    patient: vec!["Patient/p1".into()],
                    ..Default::default()
                },
            )
            .await;
            let mut ids: Vec<&str> = rows
                .iter()
                .map(|row| row["link_id"].as_str().unwrap())
                .collect();
            ids.sort();
            assert_eq!(ids, ["a", "b", "b.1"], "{view}");
            let rows = collect_rows_in_order(
                runner.as_ref(),
                &tenant,
                view.clone(),
                ViewFilters {
                    since: Some(chrono::Utc::now() + chrono::Duration::days(1)),
                    ..Default::default()
                },
            )
            .await;
            assert!(rows.is_empty(), "{view}: {rows:?}");
        }
    }

    /// #1707: `_since` and `patient` together must reach a `repeat` that has no
    /// join back to `resources`, bare or as a single-branch `unionAll`, and
    /// keep matching rows as well as drop the rest.
    #[tokio::test]
    async fn runtime_filters_combine_on_repeat_without_join_back() {
        let backend = make_backend_with_search_params().await;
        let tenant = test_tenant();
        for qr in [
            json!({"resourceType":"QuestionnaireResponse", "id":"qr-1", "status":"completed",
                "subject":{"reference":"Patient/p1"},
                "item":[{"linkId":"a"}, {"linkId":"b","item":[{"linkId":"b.1"}]}]}),
            json!({"resourceType":"QuestionnaireResponse", "id":"qr-2", "status":"completed",
                "subject":{"reference":"Patient/p2"},
                "item":[{"linkId":"z"}]}),
        ] {
            backend
                .create(&tenant, "QuestionnaireResponse", qr, FhirVersion::R4)
                .await
                .expect("failed to seed questionnaire response");
        }
        let runner = backend.sof_runner().unwrap();
        let node_only = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]}]});
        let single_branch_union = json!({"resourceType":"ViewDefinition",
            "resource":"QuestionnaireResponse",
            "select":[{"unionAll":[
                {"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]}]}]});
        let past = chrono::Utc::now() - chrono::Duration::days(1);
        let future = chrono::Utc::now() + chrono::Duration::days(1);
        for view in [node_only, single_branch_union] {
            for (patient, since, expected) in [
                ("Patient/p1", past, vec!["a", "b", "b.1"]),
                ("Patient/p2", past, vec!["z"]),
                ("Patient/p1", future, vec![]),
            ] {
                let rows = collect_rows_in_order(
                    runner.as_ref(),
                    &tenant,
                    view.clone(),
                    ViewFilters {
                        patient: vec![patient.into()],
                        since: Some(since),
                        ..Default::default()
                    },
                )
                .await;
                let mut ids: Vec<&str> = rows
                    .iter()
                    .map(|row| row["link_id"].as_str().unwrap())
                    .collect();
                ids.sort();
                assert_eq!(ids, expected, "{view}: {patient} since {since}");
            }
        }
    }

    /// #1707: a `patient` list must not hit SQLite's expression-depth limit (1000)
    /// or its bind-variable limit (32766), whichever resource the view reads.
    #[tokio::test]
    async fn patient_filter_takes_thousands_of_values() {
        let backend = make_backend_with_search_params().await;
        seed_patients(
            &backend,
            &[
                ("p1", "female", "1990-01-01"),
                ("p2", "male", "1985-06-15"),
                ("p3", "male", "1970-03-03"),
            ],
        )
        .await;
        let tenant = test_tenant();
        for n in 1..=3 {
            let obs = json!({"resourceType":"Observation","id":format!("obs-{n}"),
                "status":"final","code":{"text":"x"},
                "subject":{"reference":format!("Patient/p{n}")}});
            backend
                .create(&tenant, "Observation", obs, FhirVersion::R4)
                .await
                .expect("seed observation");
        }
        let runner = backend.sof_runner().unwrap();
        for n in [2_000usize, 40_000] {
            let mut patient = vec!["Patient/p1".to_string()];
            patient.extend((0..n - 2).map(|i| format!("Patient/absent-{i}")));
            patient.push("Patient/p2".to_string());
            for (resource, expected) in [
                ("Patient", ["p1", "p2"]),
                ("Observation", ["obs-1", "obs-2"]),
            ] {
                let rows = collect_rows_in_order(
                    runner.as_ref(),
                    &tenant,
                    preview_flat_view(resource, "id"),
                    ViewFilters {
                        patient: patient.clone(),
                        ..Default::default()
                    },
                )
                .await;
                let mut ids: Vec<&str> =
                    rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
                ids.sort();
                assert_eq!(ids, expected, "{resource} with {n} patient values");
            }
        }
    }

    /// #1701: a `group` that resolves to no Patient members (absent, empty,
    /// or device-only) selects nothing instead of running unfiltered.
    #[tokio::test]
    async fn group_resolving_to_no_patients_selects_nothing() {
        let backend = make_backend().await;
        seed_patients(
            &backend,
            &[("p1", "female", "1990-01-01"), ("p2", "male", "1985-06-15")],
        )
        .await;
        let tenant = test_tenant();
        for (rt, res) in [
            (
                "Group",
                json!({"resourceType":"Group","id":"g-empty","type":"person","actual":true}),
            ),
            (
                "Group",
                json!({"resourceType":"Group","id":"g-devices","type":"device","actual":true,
                    "member":[{"entity":{"reference":"Device/d1"}}]}),
            ),
            (
                "Observation",
                json!({"resourceType":"Observation","id":"obs-1","status":"final",
                    "code":{"text":"x"},"subject":{"reference":"Patient/p1"}}),
            ),
        ] {
            backend
                .create(&tenant, rt, res, FhirVersion::R4)
                .await
                .expect("seed");
        }
        let runner = backend.sof_runner().unwrap();

        for group in ["Group/missing", "Group/g-empty", "Group/g-devices"] {
            for resource in ["Patient", "Observation"] {
                let rows = collect_rows_in_order(
                    runner.as_ref(),
                    &tenant,
                    preview_flat_view(resource, "id"),
                    ViewFilters {
                        group: vec![group.into()],
                        ..Default::default()
                    },
                )
                .await;
                assert!(rows.is_empty(), "{group} on {resource}: {rows:?}");
            }
        }

        // An explicit patient still applies alongside an empty group.
        let rows = collect_rows_in_order(
            runner.as_ref(),
            &tenant,
            preview_flat_view("Patient", "id"),
            ViewFilters {
                patient: vec!["Patient/p1".into()],
                group: vec!["Group/g-empty".into()],
                ..Default::default()
            },
        )
        .await;
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["p1"]);
    }
}
