//! SQL-on-FHIR v2 official conformance test suite — PostgreSQL in-DB runner.
//!
//! Mirrors `sof_conformance.rs` (which targets SQLite) but wires the
//! HTTP server's storage backend to a PostgreSQL container via
//! `testcontainers`. Same fixture set (`crates/sof/tests/sql-on-fhir/tests/`),
//! same comparator, same regression-floor pattern.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).
//! Matches the gating used by `crates/persistence/tests/sof_pg_runner.rs`
//! and the other testcontainers-backed integration tests in this repo —
//! bare `#[tokio::test]`, no `#[ignore]`, no env-var opt-in. CI's
//! self-hosted runner has Docker available; the per-run container label
//! (`github.run_id`) lets the workflow's cleanup job reap it.

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

#[allow(dead_code)]
#[path = "common/sof_ordering.rs"]
mod sof_ordering;

/// The shared column-order fixtures and runner checkers (#1864 A2/A3). Included
/// directly because `sof_ordering` keeps its own copy of the module private;
/// the second load of the file is intended.
#[allow(clippy::duplicate_mod)]
#[path = "../../persistence/tests/common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

mod sof_conformance_postgres_tests {
    use super::sof_ordering::{self, Comparison, compare_rows, parse_ndjson};
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    // =========================================================================
    // Shared container setup — single PG container for the whole suite,
    // mirroring `crates/persistence/tests/sof_pg_runner.rs`. Each conformance
    // fixture runs under a unique tenant id inside the same database so the
    // container starts up once.
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
                .expect("failed to start PostgreSQL container");

                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("failed to get host port");

                let host = container
                    .get_host()
                    .await
                    .expect("failed to get host")
                    .to_string();

                // `data_dir` points at the workspace `data/` directory so the
                // backend can load search-parameter definitions for the active
                // FHIR version.
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
                    .expect("failed to create PostgresBackend");

                backend
                    .init_schema()
                    .await
                    .expect("failed to initialize schema");

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
                .expect("failed to create PostgresBackend"),
        )
    }

    // =========================================================================
    // Known-skip list — same set as the SQLite suite (the IR is dialect-
    // independent so anything skipped on SQLite is also skipped on PG).
    // =========================================================================

    const KNOWN_SKIPS: &[(&str, &str)] = &[];

    // =========================================================================
    // Fixture loading (identical to sof_conformance.rs)
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
        let dir = std::path::Path::new("../sof/tests/sql-on-fhir/tests");
        assert!(
            dir.exists(),
            "conformance fixture directory not found: {}",
            dir.display()
        );
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .expect("failed to read conformance dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();

        let mut fixtures = Vec::new();
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
                .map(|t| TestCase {
                    title: t["title"].as_str().unwrap_or("unnamed").to_string(),
                    view: t["view"].clone(),
                    expect: t.get("expect").and_then(|e| e.as_array()).cloned(),
                    expect_error: t["expectError"].as_bool().unwrap_or(false),
                    expect_columns: t.get("expectColumns").and_then(|c| c.as_array()).map(|c| {
                        c.iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    }),
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
    // Per-fixture HTTP server. Each fixture seeds its own resources under a
    // unique tenant so the shared container can host the whole suite without
    // cross-fixture bleed.
    // =========================================================================

    fn unique_tenant() -> (TenantContext, String) {
        let id = format!("sof_pg_conf_{}", uuid::Uuid::new_v4().simple());
        let tenant = TenantContext::new(TenantId::new(&id), TenantPermissions::full_access());
        (tenant, id)
    }

    async fn create_test_server(backend: Arc<PostgresBackend>) -> TestServer {
        let runner = backend
            .sof_runner()
            .expect("PostgresBackend must provide an in-DB SOF runner");
        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    async fn seed_resources(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        resources: &[Value],
    ) {
        for resource in resources {
            let rt = match resource["resourceType"].as_str() {
                Some(t) => t,
                None => continue,
            };
            backend
                .create(tenant, rt, resource.clone(), FhirVersion::R4)
                .await
                .ok();
        }
    }

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
    async fn test_sof_v2_conformance_in_db_postgres() {
        let fixtures = load_fixtures();
        assert_corpus_intact(&fixtures, "postgres");
        let backend = create_backend().await;

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut failure_msgs: Vec<String> = Vec::new();
        let mut column_order_failures: Vec<String> = Vec::new();

        for fixture in &fixtures {
            let (tenant, tenant_id) = unique_tenant();
            seed_resources(&backend, &tenant, &fixture.resources).await;
            let server = create_test_server(Arc::clone(&backend)).await;

            for test in &fixture.tests {
                let key = format!("{}::{}", fixture.title, test.title);

                if let Some((_, reason)) = KNOWN_SKIPS.iter().find(|(k, _)| *k == key.as_str()) {
                    skipped += 1;
                    eprintln!("  SKIP  {key} — {reason}");
                    continue;
                }

                let view_body = normalise_view(&test.view);
                let resp = server
                    .post("/$sql-run?_format=ndjson")
                    .add_header(X_TENANT_ID, HeaderValue::from_str(&tenant_id).unwrap())
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
                    eprintln!("  PASS  {key} (no assertion)");
                    passed += 1;
                }
            }
        }

        eprintln!(
            "\nSoF v2 conformance (PostgreSQL): {passed} passed, {failed} failed, {skipped} skipped"
        );

        // Regression floor — mirrors the SQLite ratchet at
        // `sof_conformance.rs`. The full SoF v2 corpus (144 fixtures) passes
        // against PostgreSQL; lowering this requires the same justification as the
        // SQLite floor (a fixture genuinely outside the in-DB runner's
        // coverage, listed in `KNOWN_SKIPS` with a reason).
        //
        // 126 -> 124: SoF v2 PR #349 removed two `join()` fixtures from the
        // upstream `fhirpath.json` corpus, shrinking the total fixture count
        // (not a compiler regression).
        //
        // 124 -> 132: the upstream sync added `row_index.json` and the PG in-DB
        // compiler now supports `%rowIndex` (`WITH ORDINALITY` for forEach, a
        // pre-order `ord_path` in the repeat CTE).
        //
        // 132 -> 144: nested/sibling `repeat` and `unionAll` nested inside
        // another `select` now compile, so the whole corpus passes.
        const PG_PASS_FLOOR: usize = 144;
        assert!(
            passed >= PG_PASS_FLOOR,
            "regression: only {passed} fixtures pass (floor: {PG_PASS_FLOOR}). \
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
        let backend = create_backend().await;
        let (tenant, id) = unique_tenant();
        let server = create_test_server(backend.clone()).await;
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::exact_fixture())
                .await;
        assert_eq!(resources.len(), 3);
        sof_ordering::preview_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            &id,
            &resources,
        )
        .await;
    }

    #[tokio::test]
    async fn total_order_filtered_matrix_rest_prefixes() {
        let backend = create_backend().await;
        let (tenant, id) = unique_tenant();
        let server = create_test_server(backend.clone()).await;
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::fixture()).await;
        sof_ordering::preview_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            &id,
            &resources,
        )
        .await;
    }

    #[tokio::test]
    async fn total_order_collection_cells_rest() {
        let backend = create_backend().await;
        let (tenant, id) = unique_tenant();
        let server = create_test_server(backend.clone()).await;
        let resources =
            sof_ordering::seed_fixture(backend.as_ref(), &tenant, sof_ordering::cell_fixture())
                .await;
        sof_ordering::cell_acceptance(
            &server,
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            &id,
            &resources,
        )
        .await;
    }

    #[tokio::test]
    async fn total_order_export_manifest_shards_postgres() {
        use helios_rest::export::{InMemoryController, InMemorySink};
        let backend = create_backend().await;
        let (tenant, id) = unique_tenant();
        let runner = backend.sof_runner().unwrap();
        let controller = InMemoryController::with_shard_rows(
            runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
            Some(7),
        );
        let config = ServerConfig {
            base_url: "http://localhost".into(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(backend.clone(), config)
            .with_sof_runner(runner.clone())
            .with_export_controller(Arc::new(controller));
        let server =
            TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).unwrap();
        let mut fixture = sof_ordering::exact_fixture();
        fixture.extend(sof_ordering::cell_fixture());
        let resources = sof_ordering::seed_fixture(backend.as_ref(), &tenant, fixture).await;
        // Export reads PostgreSQL in keyset pages over the resource id.
        sof_ordering::export_acceptance(
            &server,
            runner.as_ref(),
            &tenant,
            &id,
            &resources,
            sof_ordering::ExportOrder::IdBytes,
        )
        .await;
    }

    // =========================================================================
    // Column order (#1864 A2/A3), PostgreSQL cells
    // =========================================================================

    /// A2: the declared schema is ViewDefinition document order and every
    /// row's keys follow it (`sof_prefix_matrix::schema_order_views`).
    #[tokio::test]
    async fn view_schema_matches_row_keys_in_document_order() {
        use super::sof_prefix_matrix as matrix;
        let backend = create_backend().await;
        let (tenant, _id) = unique_tenant();
        seed_resources(
            &backend,
            &tenant,
            &matrix::union_first_sibling_after_resources(),
        )
        .await;
        matrix::assert_view_schema_matches_row_keys(
            backend.sof_runner().unwrap().as_ref(),
            &tenant,
            "postgres",
        )
        .await;
    }

    /// A3: a `unionAll` before a sibling `select` keeps the base row sequence
    /// (the statement's `ORDER BY 1` key, the first branch column) while every
    /// row's keys are in document order: runner preview and unlimited (one
    /// statement), the export runner (`for_export_job()`, keyset pages), and
    /// `$sql-run`.
    #[tokio::test]
    async fn union_first_sibling_after_keeps_row_order() {
        use super::sof_prefix_matrix as matrix;
        let backend = create_backend().await;
        let (tenant, id) = unique_tenant();
        let server = create_test_server(backend.clone()).await;
        seed_resources(
            &backend,
            &tenant,
            &matrix::union_first_sibling_after_resources(),
        )
        .await;
        let runner = backend.sof_runner().unwrap();
        assert!(
            runner.for_export_job().is_some(),
            "PostgreSQL exports through its keyset-paged runner"
        );
        matrix::assert_union_first_sibling_after_keeps_row_order(
            runner.as_ref(),
            &tenant,
            "postgres",
        )
        .await;
        // The same sequence and key order through `$sql-run` (NDJSON).
        let response = server
            .post("/$sql-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_str(&id).unwrap())
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
            "postgres/$sql-run: base row sequence",
        );
        matrix::assert_row_keys(
            &rows,
            &matrix::UNION_FIRST_DOCUMENT_ORDER,
            "postgres/$sql-run",
        );
    }
}
