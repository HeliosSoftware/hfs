//! SQL-on-FHIR v2 official conformance test suite — PostgreSQL in-DB runner.
//!
//! Mirrors `sof_conformance.rs` (which targets SQLite) but wires the
//! HTTP server's storage backend to a PostgreSQL container via
//! `testcontainers`. Same fixture set (`crates/sof/tests/sql-on-fhir-v2/tests/`),
//! same comparator, same regression-floor pattern.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).
//! Matches the gating used by `crates/persistence/tests/sof_pg_runner.rs`
//! and the other testcontainers-backed integration tests in this repo —
//! bare `#[tokio::test]`, no `#[ignore]`, no env-var opt-in. CI's
//! self-hosted runner has Docker available; the per-run container label
//! (`github.run_id`) lets the workflow's cleanup job reap it.

#![cfg(feature = "postgres")]

mod sof_conformance_postgres_tests {
    use axum::http::{HeaderName, HeaderValue};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
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

    const KNOWN_SKIPS: &[(&str, &str)] = &[
        (
            "row_index::%rowIndex at top level",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex with forEach",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex with forEachOrNull",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex with nested forEach",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex with repeat",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex with unionAll",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex in unionAll without forEach",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex in unionAll inside forEach",
            "%rowIndex not implemented",
        ),
        (
            "row_index::%rowIndex for surrogate key",
            "%rowIndex not implemented",
        ),
    ];

    // =========================================================================
    // Fixture loading (identical to sof_conformance.rs)
    // =========================================================================

    #[derive(Debug)]
    struct TestCase {
        title: String,
        view: Value,
        expect: Option<Vec<Value>>,
        expect_error: bool,
    }

    #[derive(Debug)]
    struct Fixture {
        title: String,
        resources: Vec<Value>,
        tests: Vec<TestCase>,
    }

    fn load_fixtures() -> Vec<Fixture> {
        let dir = std::path::Path::new("../sof/tests/sql-on-fhir-v2/tests");
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

    fn values_equal(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => x
                .as_f64()
                .zip(y.as_f64())
                .is_some_and(|(xf, yf)| (xf - yf).abs() < 1e-9),
            (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => {
                n.to_string() == *s
            }
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

    // =========================================================================
    // Main conformance test
    // =========================================================================

    #[tokio::test]
    async fn test_sof_v2_conformance_in_db_postgres() {
        let fixtures = load_fixtures();
        let backend = create_backend().await;

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut failure_msgs: Vec<String> = Vec::new();

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
                    .post("/ViewDefinition/$viewdefinition-run?_format=ndjson")
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

                if let Some(expected) = &test.expect {
                    match compare_rows(&actual, expected) {
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
        // `sof_conformance.rs`. The full SoF v2 corpus passes against
        // PostgreSQL; lowering this requires the same justification as the
        // SQLite floor (a fixture genuinely outside the in-DB runner's
        // coverage, listed in `KNOWN_SKIPS` with a reason).
        const PG_PASS_FLOOR: usize = 125;
        assert!(
            passed >= PG_PASS_FLOOR,
            "regression: only {passed} fixtures pass (floor: {PG_PASS_FLOOR}). \
             Failures:\n  {}",
            failure_msgs.join("\n  "),
        );
    }
}
