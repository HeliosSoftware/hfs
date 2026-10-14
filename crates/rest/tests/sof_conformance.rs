//! SQL-on-FHIR v2 official conformance test suite (in-DB runner).
//!
//! Runs every test case in `crates/sof/tests/sql-on-fhir/tests/*.json`
//! against the SQLite in-DB SOF runner via the public HTTP endpoint, and
//! reports all failures at the end. A test case is considered passing if:
//!
//! - `expectError: true`  → the endpoint returns a non-2xx status code.
//! - `expect: [...]`       → the returned NDJSON rows match the expected rows
//!                           (order-insensitive, only checking expected keys).
//!
//! Tests outside the in-DB runner's compilation coverage are skipped via the
//! `KNOWN_SKIPS` table with a reason. Skipped tests are counted but do not
//! cause the suite to fail.
//!
//! ## Skips
//!
//! Some test cases exercise FHIRPath functions that are not yet implemented in
//! `helios-fhirpath`.  Those are listed in `SKIP_TEST_TITLES` together with the
//! reason for the skip.  Skipped tests are counted but do NOT cause the suite to
//! fail.
//!
//! ## CI
//!
//! The test does not require Docker.  It uses an in-memory SQLite backend, so
//! it runs on every PR automatically.

#[allow(dead_code)]
#[path = "common/sof_ordering.rs"]
mod sof_ordering;

/// The shared column-order fixtures and runner checkers (#1864 A2/A3). Included
/// directly because `sof_ordering` keeps its own copy of the module private;
/// the second load of the file is intended.
#[allow(clippy::duplicate_mod)]
#[path = "../../persistence/tests/common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

mod sof_conformance_tests {
    use super::sof_ordering::{self, Comparison, compare_rows, parse_ndjson};
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    // =========================================================================
    // Known-skip list
    //
    // Format: ("fixture_title::test_title", "reason")
    // =========================================================================

    /// Test cases that are intentionally skipped because they require FHIRPath
    /// functions or features not yet implemented in `helios-fhirpath`.
    ///
    /// Each entry is a `(fixture_title::test_title, reason)` pair.  The test
    /// will be counted as "skipped" rather than "failed".
    const KNOWN_SKIPS: &[(&str, &str)] = &[];

    // =========================================================================
    // Fixture loading
    // =========================================================================

    #[derive(Debug)]
    struct TestCase {
        title: String,
        view: Value,
        expect: Option<Vec<Value>>,
        expect_error: bool,
        /// The fixture's `expectColumns`: every row's keys, in this order.
        expect_columns: Option<Vec<String>>,
    }

    #[derive(Debug)]
    struct Fixture {
        title: String,
        resources: Vec<Value>,
        tests: Vec<TestCase>,
    }

    fn load_fixtures() -> Vec<Fixture> {
        // Single canonical fixture set — `crates/sof/tests/sql-on-fhir/tests/`.
        let dir = std::path::Path::new("../sof/tests/sql-on-fhir/tests");
        assert!(
            dir.exists(),
            "conformance fixture directory not found: {}",
            dir.display()
        );

        let mut fixtures = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .expect("failed to read conformance dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();

        for path in paths {
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            let json: Value = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));

            let title = json["title"].as_str().unwrap_or("unknown").to_string();
            let resources: Vec<Value> = json["resources"].as_array().cloned().unwrap_or_default();

            let tests = json["tests"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|t| {
                    let test_title = t["title"].as_str().unwrap_or("unnamed").to_string();
                    let expect_error = t["expectError"].as_bool().unwrap_or(false);
                    let expect = t.get("expect").and_then(|e| e.as_array()).cloned();
                    let view = t["view"].clone();
                    let expect_columns =
                        t.get("expectColumns").and_then(|c| c.as_array()).map(|c| {
                            c.iter()
                                .filter_map(|n| n.as_str().map(str::to_string))
                                .collect()
                        });
                    TestCase {
                        title: test_title,
                        view,
                        expect,
                        expect_error,
                        expect_columns,
                    }
                })
                .collect();

            fixtures.push(Fixture {
                title,
                resources,
                tests,
            });
        }

        fixtures
    }

    /// Lower bounds on the vendored SQL-on-FHIR v2 corpus (22 files / 144 cases
    /// at time of writing). The `PASS_FLOOR` ratchet below counts *passes*, so it
    /// is trivially satisfied by a corpus that shrank — 132 passes out of 132
    /// remaining cases looks identical to 132 out of 144. These floors make a
    /// silently emptied or half-checked-out fixture tree loud instead (issue #307).
    const MIN_FIXTURE_FILES: usize = 22;
    const MIN_TEST_CASES: usize = 144;

    /// Asserts the corpus actually loaded before any pass-count ratchet is trusted.
    fn assert_corpus_intact(fixtures: &[Fixture], backend: &str) {
        let cases: usize = fixtures.iter().map(|f| f.tests.len()).sum();
        assert!(
            fixtures.len() >= MIN_FIXTURE_FILES,
            "{backend}: only {} fixture file(s) loaded (expected at least {MIN_FIXTURE_FILES}). \
             The conformance corpus has shrunk or failed to check out; a pass-count floor \
             cannot detect that on its own.",
            fixtures.len(),
        );
        assert!(
            cases >= MIN_TEST_CASES,
            "{backend}: only {cases} conformance case(s) loaded (expected at least \
             {MIN_TEST_CASES}). Fixture files are present but nearly empty.",
        );
        eprintln!(
            "SOF-CONFORMANCE-{backend}: files={} cases={cases}",
            fixtures.len()
        );
    }

    // =========================================================================
    // Test infrastructure
    // =========================================================================

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let runner = backend
            .sof_runner()
            .expect("SqliteBackend must provide an in-DB SOF runner");

        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        let server = TestServer::new(app).expect("failed to create test server");

        (server, backend)
    }

    async fn seed_resources(backend: &SqliteBackend, resources: &[Value]) {
        let tenant = test_tenant();
        for resource in resources {
            let rt = match resource["resourceType"].as_str() {
                Some(t) => t,
                None => continue, // skip resources without a type
            };
            let id = resource["id"].as_str().unwrap_or("unknown");
            // Some fixtures have no `id` — skip those safely
            let _ = id;
            backend
                .create(&tenant, rt, resource.clone(), FhirVersion::R4)
                .await
                .ok(); // ignore duplicate-key errors if any
        }
    }

    /// Normalises a view from the fixture into a proper ViewDefinition body.
    /// Adds `resourceType: "ViewDefinition"` if absent.
    fn normalise_view(view: &Value) -> Value {
        let mut v = view.clone();
        if let Value::Object(ref mut map) = v {
            map.entry("resourceType")
                .or_insert_with(|| json!("ViewDefinition"));
        }
        v
    }

    // =========================================================================
    // Main conformance test
    // =========================================================================

    #[tokio::test]
    async fn test_sof_v2_conformance_in_db_sqlite() {
        let fixtures = load_fixtures();
        assert_corpus_intact(&fixtures, "sqlite");

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut failure_msgs: Vec<String> = Vec::new();
        let mut column_order_failures: Vec<String> = Vec::new();

        for fixture in &fixtures {
            let (server, backend) = create_test_server().await;
            seed_resources(&backend, &fixture.resources).await;

            for test in &fixture.tests {
                let key = format!("{}::{}", fixture.title, test.title);

                // Check skip list
                if let Some((_, reason)) = KNOWN_SKIPS.iter().find(|(k, _)| *k == key.as_str()) {
                    skipped += 1;
                    eprintln!("  SKIP  {key} — {reason}");
                    continue;
                }

                let view_body = normalise_view(&test.view);

                let resp = server
                    .post("/$sql-run?_format=ndjson")
                    .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
                    .add_header(
                        axum::http::HeaderName::from_static("content-type"),
                        HeaderValue::from_static("application/fhir+json"),
                    )
                    .json(&view_body)
                    .await;

                let status = resp.status_code();

                if test.expect_error {
                    if status.is_success() {
                        let msg = format!("FAIL  {key}: expected error but got {status}");
                        eprintln!("  {msg}");
                        failure_msgs.push(msg);
                        failed += 1;
                    } else {
                        eprintln!("  PASS  {key} (expected error, got {status})");
                        passed += 1;
                    }
                    continue;
                }

                if !status.is_success() {
                    let msg = format!("FAIL  {key}: unexpected HTTP {status}: {}", resp.text());
                    eprintln!("  {msg}");
                    failure_msgs.push(msg);
                    failed += 1;
                    continue;
                }

                // Compare rows
                let body = resp.text();
                let actual = parse_ndjson(&body);
                if let Some(columns) = &test.expect_columns
                    && let Some(mismatch) = column_order_mismatch(&actual, columns)
                {
                    eprintln!("  COLUMN-ORDER  {key}: {mismatch}");
                    column_order_failures.push(format!("{key}: {mismatch}"));
                }

                if let Some(expected) = &test.expect {
                    match compare_rows(Comparison::Official, &actual, expected) {
                        None => {
                            eprintln!("  PASS  {key}");
                            passed += 1;
                        }
                        Some(mismatch) => {
                            let msg = format!(
                                "FAIL  {key}: {mismatch}\n       actual:   {actual:?}\n       expected: {expected:?}"
                            );
                            eprintln!("  {msg}");
                            failure_msgs.push(msg);
                            failed += 1;
                        }
                    }
                } else {
                    // No `expect` assertion — just verify it didn't error
                    eprintln!("  PASS  {key} (no assertion)");
                    passed += 1;
                }
            }
        }

        eprintln!("\nSoF v2 conformance: {passed} passed, {failed} failed, {skipped} skipped");

        // Regression floor — the in-DB compiler currently passes this many
        // upstream conformance fixtures. Lowering this number means the
        // compiler regressed; raising it (after expanding compiler coverage)
        // means more of the spec is now in-DB-compilable. This is intentionally
        // a one-way ratchet so unrelated changes that lose coverage get
        // caught in CI.
        //
        // 126 -> 124: SoF v2 PR #349 removed two `join()` fixtures from the
        // upstream `fhirpath.json` corpus, shrinking the total fixture count
        // (not a compiler regression).
        //
        // 124 -> 132: the upstream sync added `row_index.json` (9 fixtures for
        // the `%rowIndex` environment variable) and the in-DB compiler now
        // supports `%rowIndex` for SQLite.
        //
        // 132 -> 144: nested/sibling `repeat` and `unionAll` nested inside
        // another `select` now compile, so the whole corpus passes.
        const PASS_FLOOR: usize = 144;
        assert!(
            passed >= PASS_FLOOR,
            "regression: only {passed} fixtures pass (floor: {PASS_FLOOR}). \
             Failures:\n  {}",
            failure_msgs.join("\n  "),
        );

        // `expectColumns` (#1864): every row's keys follow ViewDefinition
        // document order, which puts a `select` after a `unionAll` after the
        // `unionAll` columns. Checked apart from the pass floor so a key-order
        // regression fails even while the rows themselves still match.
        assert!(
            column_order_failures.is_empty(),
            "expectColumns key order differs from ViewDefinition document order:\n  {}",
            column_order_failures.join("\n  "),
        );
    }

    /// The first row whose keys are not exactly `columns`, in that order.
    fn column_order_mismatch(rows: &[Value], columns: &[String]) -> Option<String> {
        rows.iter().find_map(|row| {
            let keys: Vec<&str> = row
                .as_object()
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            (keys != columns.iter().map(String::as_str).collect::<Vec<_>>())
                .then(|| format!("row keys {keys:?}, expected {columns:?}"))
        })
    }

    #[tokio::test]
    async fn total_order_exact_fixture_rest_prefixes() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::exact_fixture())
                .await;
        assert_eq!(resources.len(), 3);
        sof_ordering::preview_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            "test-tenant",
            &resources,
        )
        .await;
    }

    #[tokio::test]
    async fn total_order_filtered_matrix_rest_prefixes() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::fixture()).await;
        sof_ordering::preview_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            "test-tenant",
            &resources,
        )
        .await;
    }

    #[tokio::test]
    async fn total_order_collection_cells_rest() {
        let (server, backend) = create_test_server().await;
        let tenant = test_tenant();
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::cell_fixture())
                .await;
        sof_ordering::cell_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            "test-tenant",
            &resources,
        )
        .await;
    }

    // =========================================================================
    // Column order (#1864 A2/A3), SQLite cells
    // =========================================================================

    /// A2: the declared schema is ViewDefinition document order and every
    /// row's keys follow it (`sof_prefix_matrix::schema_order_views`).
    #[tokio::test]
    async fn view_schema_matches_row_keys_in_document_order() {
        use super::sof_prefix_matrix as matrix;
        let (_server, backend) = create_test_server().await;
        seed_resources(&backend, &matrix::union_first_sibling_after_resources()).await;
        matrix::assert_view_schema_matches_row_keys(
            backend.sof_runner().unwrap().as_ref(),
            &test_tenant(),
            "sqlite",
        )
        .await;
    }

    /// A3: a `unionAll` before a sibling `select` keeps the base row sequence
    /// (the statement's `ORDER BY 1` key, the first branch column) while every
    /// row's keys are in document order: runner preview, unlimited and export runner
    /// (SQLite exports through the same unpaged runner), and `$sql-run`.
    #[tokio::test]
    async fn union_first_sibling_after_keeps_row_order() {
        use super::sof_prefix_matrix as matrix;
        let (server, backend) = create_test_server().await;
        seed_resources(&backend, &matrix::union_first_sibling_after_resources()).await;
        matrix::assert_union_first_sibling_after_keeps_row_order(
            backend.sof_runner().unwrap().as_ref(),
            &test_tenant(),
            "sqlite",
        )
        .await;
        // The same sequence and key order through `$sql-run` (NDJSON).
        let response = server
            .post("/$sql-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                axum::http::HeaderName::from_static("content-type"),
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&matrix::union_first_sibling_after_view())
            .await;
        assert!(response.status_code().is_success(), "{}", response.text());
        let rows = parse_ndjson(&response.text());
        matrix::assert_ordered(
            &rows,
            &matrix::union_first_sibling_after_base_sequence(),
            "sqlite/$sql-run: base row sequence",
        );
        matrix::assert_row_keys(
            &rows,
            &matrix::UNION_FIRST_DOCUMENT_ORDER,
            "sqlite/$sql-run",
        );
    }
}
