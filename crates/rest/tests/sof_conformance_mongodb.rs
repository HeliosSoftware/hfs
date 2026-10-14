//! SQL-on-FHIR v2 official conformance test suite — MongoDB in-DB runner.
//!
//! Mirrors `sof_conformance.rs` (SQLite) and `sof_conformance_postgres.rs` (PG)
//! but wires the HTTP server's storage backend to a MongoDB container via
//! `testcontainers`. Same fixture set (`crates/sof/tests/sql-on-fhir/tests/`),
//! same comparator, same regression-floor pattern.
//!
//! The MongoDB aggregation emitter is at parity with the SQL backends: it
//! passes all 144 fixtures, including nested/sibling `repeat` and `unionAll`
//! nested inside another `select`.
//!
//! The Stage-1 emitter (`$unwind`/`$function`/`$facet`/server-side JS for
//! `repeat`) requires server-side JavaScript to be enabled (the default for the
//! conformance container).
//!
//! Requires Docker (testcontainers spins up a real MongoDB instance), matching
//! the gating used by `sof_conformance_postgres.rs`.

#![cfg(feature = "mongodb")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// The shared column-order fixtures and runner checkers (#1864 A2/A3).
#[path = "../../persistence/tests/common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

mod sof_conformance_mongodb_tests {
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use futures::FutureExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::{Backend, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    // =========================================================================
    // Shared container setup — single Mongo container for the whole suite. Each
    // fixture runs under a unique tenant id inside the same database so the
    // container starts up once (mirrors the PostgreSQL suite).
    // =========================================================================

    struct SharedMongo {
        connection_string: String,
        /// Kept alive for the duration of the test binary; the
        /// `container_cleanup` exit hook removes it at process exit.
        _container: testcontainers::ContainerAsync<Mongo>,
    }

    static SHARED_MONGO: OnceCell<SharedMongo> = OnceCell::const_new();

    async fn shared_mongo() -> &'static SharedMongo {
        SHARED_MONGO
            .get_or_init(|| async {
                // Label with the CI run id (mirrors the PostgreSQL conformance
                // suite); `SHARED_MONGO` is a static and never dropped, so the
                // `container_cleanup` exit hook removes the container at
                // process exit.
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = super::container_cleanup::with_cleanup_label(
                    Mongo::default().with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("failed to start MongoDB container");
                let port = container
                    .get_host_port_ipv4(27017)
                    .await
                    .expect("failed to get host port");
                let host = container
                    .get_host()
                    .await
                    .expect("failed to get host")
                    .to_string();
                SharedMongo {
                    connection_string: format!("mongodb://{host}:{port}"),
                    _container: container,
                }
            })
            .await
    }

    async fn create_backend() -> Arc<MongoBackend> {
        let mongo = shared_mongo().await;
        let config = MongoBackendConfig {
            connection_string: mongo.connection_string.clone(),
            database_name: "sof_mongo_conformance".to_string(),
            ..Default::default()
        };
        let backend = MongoBackend::new(config).expect("failed to create MongoBackend");
        backend
            .initialize()
            .await
            .expect("failed to initialize MongoDB schema");
        Arc::new(backend)
    }

    // Stage-1 Mongo emitter unsupported constructs are absorbed by the floor,
    // not enumerated here (see the module doc).
    const KNOWN_SKIPS: &[(&str, &str)] = &[];

    // =========================================================================
    // Fixture loading (identical to the SQLite/PG suites)
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
    // unique tenant so the shared container hosts the whole suite.
    // =========================================================================

    fn unique_tenant() -> (TenantContext, String) {
        let id = format!("sof_mongo_conf_{}", uuid::Uuid::new_v4().simple());
        let tenant = TenantContext::new(TenantId::new(&id), TenantPermissions::full_access());
        (tenant, id)
    }

    async fn create_test_server(backend: Arc<MongoBackend>) -> TestServer {
        let runner = backend
            .sof_runner()
            .expect("MongoBackend must provide an in-DB SOF runner");
        let config = ServerConfig::for_testing();
        let state =
            helios_rest::AppState::new(Arc::clone(&backend), config).with_sof_runner(runner);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    async fn seed_resources(backend: &MongoBackend, tenant: &TenantContext, resources: &[Value]) {
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

    fn parse_ndjson(body: &str) -> Vec<BTreeMap<String, Value>> {
        body.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let v: Value =
                    serde_json::from_str(l).unwrap_or_else(|e| panic!("invalid NDJSON: {l} — {e}"));
                v.as_object()
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<BTreeMap<_, _>>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    fn row_matches_expected(actual: &BTreeMap<String, Value>, expected: &Value) -> bool {
        let expected_obj = match expected.as_object() {
            Some(o) => o,
            None => return false,
        };
        for (k, ev) in expected_obj {
            match actual.get(k) {
                Some(av) => {
                    if !values_equal(av, ev) {
                        return false;
                    }
                }
                None => {
                    if !ev.is_null() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Strict equality except that numbers are compared as f64: a number is
    /// never equal to its string form (#1769).
    fn values_equal(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => x
                .as_f64()
                .zip(y.as_f64())
                .is_some_and(|(xf, yf)| (xf - yf).abs() < 1e-9),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(xi, yi)| values_equal(xi, yi))
            }
            _ => false,
        }
    }

    fn compare_rows(actual: &[BTreeMap<String, Value>], expected: &[Value]) -> Option<String> {
        if actual.len() != expected.len() {
            return Some(format!(
                "row count mismatch: got {}, expected {}",
                actual.len(),
                expected.len()
            ));
        }
        let mut remaining: Vec<usize> = (0..actual.len()).collect();
        'outer: for exp_row in expected {
            for (pos, &idx) in remaining.iter().enumerate() {
                if row_matches_expected(&actual[idx], exp_row) {
                    remaining.remove(pos);
                    continue 'outer;
                }
            }
            return Some(format!("no matching actual row for expected: {exp_row}"));
        }
        None
    }

    enum Outcome {
        /// Passed; the `String` is an optional trailing note for the log line.
        Pass(String),
        Fail(String),
    }

    /// Runs one fixture test against the server and classifies the result,
    /// plus an `expectColumns` key-order mismatch, if any (#1864). Panics from
    /// `axum-test` (mid-stream runtime errors) propagate to the caller's
    /// `catch_unwind`.
    async fn evaluate_test(
        server: &TestServer,
        tenant_id: &str,
        view_body: &Value,
        test: &TestCase,
    ) -> (Outcome, Option<String>) {
        let resp = server
            .post("/$sql-run?_format=ndjson")
            .add_header(X_TENANT_ID, HeaderValue::from_str(tenant_id).unwrap())
            .add_header(
                axum::http::HeaderName::from_static("content-type"),
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(view_body)
            .await;

        let status = resp.status_code();

        if test.expect_error {
            let outcome = if status.is_success() {
                Outcome::Fail(format!("expected error but got {status}"))
            } else {
                Outcome::Pass(format!(" (expected error, got {status})"))
            };
            return (outcome, None);
        }

        if !status.is_success() {
            return (
                Outcome::Fail(format!("unexpected HTTP {status}: {}", resp.text())),
                None,
            );
        }

        let body = resp.text();
        let actual = parse_ndjson(&body);
        // Key order needs the rows as parsed (`serde_json` keeps insertion
        // order), not the `BTreeMap`s the row comparison uses.
        let column_order = test.expect_columns.as_ref().and_then(|columns| {
            let rows: Vec<Value> = body
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).expect("valid NDJSON row"))
                .collect();
            column_order_mismatch(&rows, columns)
        });
        let outcome = match &test.expect {
            Some(expected) => match compare_rows(&actual, expected) {
                None => Outcome::Pass(String::new()),
                Some(mismatch) => Outcome::Fail(format!(
                    "{mismatch}\n       actual:   {actual:?}\n       expected: {expected:?}"
                )),
            },
            None => Outcome::Pass(" (no assertion)".to_string()),
        };
        (outcome, column_order)
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

    // =========================================================================
    // Main conformance test
    // =========================================================================

    #[tokio::test]
    async fn test_sof_v2_conformance_in_db_mongodb() {
        let fixtures = load_fixtures();
        assert_corpus_intact(&fixtures, "mongodb");
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

                // The Stage-1 Mongo emitter can compile a pipeline that then
                // fails at runtime (e.g. `$arrayElemAt` on a string). Such
                // errors surface mid-stream and make `axum-test` panic, so each
                // request is evaluated under `catch_unwind` and a panic is
                // recorded as a failure rather than aborting the whole suite.
                let view_body = normalise_view(&test.view);
                let (outcome, column_order) =
                    AssertUnwindSafe(evaluate_test(&server, &tenant_id, &view_body, test))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| {
                            (
                                Outcome::Fail(
                                    "request panicked (mid-stream runtime error)".to_string(),
                                ),
                                None,
                            )
                        });
                if let Some(mismatch) = column_order {
                    eprintln!("  COLUMN-ORDER  {key}: {mismatch}");
                    column_order_failures.push(format!("{key}: {mismatch}"));
                }

                match outcome {
                    Outcome::Pass(note) => {
                        eprintln!("  PASS  {key}{note}");
                        passed += 1;
                    }
                    Outcome::Fail(reason) => {
                        let msg = format!("FAIL  {key}: {reason}");
                        eprintln!("  {msg}");
                        failure_msgs.push(msg);
                        failed += 1;
                    }
                }
            }
        }

        eprintln!(
            "\nSoF v2 conformance (MongoDB): {passed} passed, {failed} failed, {skipped} skipped"
        );

        // Regression floor — the Mongo aggregation emitter passes the SAME
        // 144 fixtures as the SQL backends, so the floor matches the SQL
        // suites exactly. A drop signals a Mongo-emitter regression.
        const MONGO_PASS_FLOOR: usize = 144;
        assert!(
            passed >= MONGO_PASS_FLOOR,
            "regression: only {passed} fixtures pass (floor: {MONGO_PASS_FLOOR}). \
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

    /// #1864 A2, MongoDB cell: the declared schema is ViewDefinition document
    /// order and every row's keys follow it, including a `select` after a
    /// `unionAll` (`sof_prefix_matrix::schema_order_views`). The pipeline has
    /// no total row order, so the A3 row-sequence cell is SQLite/PostgreSQL
    /// only; MongoDB's key order is also pinned by `expectColumns` above.
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
            "mongodb",
        )
        .await;
    }
}
