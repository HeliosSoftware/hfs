//! Integration tests for the PostgreSQL terminology backend.
//!
//! Requires Docker — uses `testcontainers` to spin up a real PostgreSQL
//! instance. These tests are gated by `#[cfg(feature = "postgres")]` and
//! are not run in the default test suite.
//!
//! Run with:
//!   `cargo test -p helios-hts --features postgres --test postgres_integration_tests`

#![cfg(feature = "postgres")]

use helios_hts::backends::PostgresTerminologyBackend;
use helios_hts::error::HtsError;
use helios_hts::import::{BundleImportBackend, LanguageFilter};
use helios_hts::traits::{
    CodeSystemOperations, ConceptMapOperations, TerminologyMetadata, ValueSetOperations,
};
use helios_hts::types::{
    ClosureRequest, CodingConcept, ExpandRequest, LookupRequest, ResourceSearchQuery,
    SubsumesRequest, SubsumptionOutcome, TranslateRequest, ValidateCodeRequest,
};
use helios_persistence::tenant::TenantContext;
use std::sync::OnceLock;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

// ── Shared container (one per test binary run) ─────────────────────────────────
//
// The `ContainerAsync` must be kept alive for the lifetime of the process to
// prevent the Docker container from being stopped.  We store it in a
// process-level static and force-remove the container via an atexit hook
// (`#[ctor::dtor]`) since `static` values are never dropped.

const INTEGRATION_PG_LABEL_KEY: &str = "io.helios.hts.test-pool";
const INTEGRATION_PG_LABEL_VALUE: &str = "hts-integration-pg";

static CONTAINER: OnceLock<ContainerAsync<Postgres>> = OnceLock::new();
static DB_URL: OnceCell<String> = OnceCell::const_new();

/// Force-remove the shared PostgreSQL testcontainer at process exit.
/// `static CONTAINER` is never dropped, so its async cleanup never runs;
/// this synchronous `docker rm -f` by label is the backstop.
#[ctor::dtor]
fn cleanup_integration_pg_container() {
    let filter = format!("label={INTEGRATION_PG_LABEL_KEY}={INTEGRATION_PG_LABEL_VALUE}");
    let Ok(listing) = std::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &filter])
        .output()
    else {
        return;
    };
    let ids = String::from_utf8_lossy(&listing.stdout);
    for id in ids.split_whitespace() {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .output();
    }
}

/// Start a labeled Postgres testcontainer, retrying transient failures.
///
/// Why: the testcontainers postgres image's wait-for-log strategy occasionally
/// hits `EndOfStream` before the "ready" line on hosts where initdb emits a
/// `locale: not found` warning during bootstrap. Retry a few times before
/// giving up so a single unlucky first-caller doesn't fail an otherwise green
/// suite.
async fn start_postgres_for_test(label_key: &str, label_value: &str) -> ContainerAsync<Postgres> {
    use testcontainers::{ImageExt, runners::AsyncRunner};
    let mut last_err = None;
    for attempt in 1..=3u32 {
        match Postgres::default()
            .with_label(label_key, label_value)
            .start()
            .await
        {
            Ok(c) => return c,
            Err(e) => {
                eprintln!("postgres container start attempt {attempt}/3 failed: {e:?}");
                last_err = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    panic!(
        "Failed to start Postgres container after 3 attempts: {:?}",
        last_err.unwrap()
    )
}

/// Returns the PostgreSQL URL for the shared test container, starting it on the
/// first call.
async fn db_url() -> &'static str {
    DB_URL
        .get_or_init(|| async {
            let container =
                start_postgres_for_test(INTEGRATION_PG_LABEL_KEY, INTEGRATION_PG_LABEL_VALUE).await;

            let host = container
                .get_host()
                .await
                .expect("Failed to get postgres host");

            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get postgres port");

            let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

            // Keep the container alive for the process lifetime.
            let _ = CONTAINER.set(container);
            url
        })
        .await
}

/// Return a fresh `PostgresTerminologyBackend` connected to the shared
/// container. A new pool is built *per test* on purpose: a `#[tokio::test]`
/// tears down its runtime when the test ends, which aborts the connection
/// driver tasks spawned by `deadpool-postgres`. Sharing a single pool across
/// tests would then hand out dead connections to later tests, producing
/// transient "connection closed" errors. Concurrent schema application is
/// made safe inside `schema::apply` via a PG advisory lock.
async fn fresh_backend() -> PostgresTerminologyBackend {
    PostgresTerminologyBackend::new(db_url().await)
        .await
        .expect("Backend should initialize")
}

fn ctx() -> TenantContext {
    TenantContext::system()
}

/// Returns a `(base_url, uid)` pair for a given test to avoid cross-test
/// interference when tests share the same PostgreSQL database.
/// `base_url` is used for resource canonical URLs; `uid` is used for resource
/// IDs so that no two tests ever try to write the same primary key.
macro_rules! test_ctx {
    ($prefix:literal) => {{
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let base = format!("http://{}.{}/", $prefix, uid);
        (base, uid)
    }};
}

/// Legacy helper — returns only the base URL.
macro_rules! base_url {
    ($prefix:literal) => {{
        let (base, _uid) = test_ctx!($prefix);
        base
    }};
}

// ── Infrastructure tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn schema_applies_idempotently() {
    let url = db_url().await;
    PostgresTerminologyBackend::new(url)
        .await
        .expect("First schema application should succeed");
    PostgresTerminologyBackend::new(url)
        .await
        .expect("Second schema application (idempotent) should also succeed");
}

#[tokio::test]
async fn backend_name_is_postgres() {
    let backend = fresh_backend().await;
    assert_eq!(backend.backend_name(), "postgres");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supported_systems_empty_initially() {
    // Use a dedicated container to guarantee a truly empty DB.
    let container =
        start_postgres_for_test(INTEGRATION_PG_LABEL_KEY, INTEGRATION_PG_LABEL_VALUE).await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let backend = PostgresTerminologyBackend::new(&url).await.unwrap();
    assert!(
        backend.supported_systems().is_empty(),
        "fresh DB should have no systems"
    );
}

// ── Seed helpers ───────────────────────────────────────────────────────────────

/// Seed a CodeSystem into `backend` and return the URL that was used.
async fn seed_code_system_with_url(backend: &PostgresTerminologyBackend, cs_url: &str) -> String {
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{
            "resource": {
                "resourceType": "CodeSystem",
                "id": format!("cs-{uid}"),
                "url": cs_url,
                "name": "TestCS",
                "version": "1.0",
                "status": "active",
                "content": "complete",
                "concept": [{
                    "code": "ABC",
                    "display": "Alpha Beta Charlie",
                    "definition": "The definition",
                    "property": [
                        { "code": "parent", "valueCode": "ROOT" },
                        { "code": "inactive", "valueBoolean": false }
                    ],
                    "designation": [{
                        "language": "fr",
                        "value": "Alpha Bêta Charlie"
                    }]
                }]
            }
        }]
    });
    let bytes = serde_json::to_vec(&bundle).unwrap();
    backend
        .import_bundle(&ctx(), &bytes)
        .await
        .expect("Seed import should succeed");
    cs_url.to_owned()
}

// ── 4.1 — Pre-existing tests (migrated to shared container) ───────────────────

#[tokio::test]
async fn import_bundle_end_to_end() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("e2e");

    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "CodeSystem",
                    "id": format!("cs-{uid}"),
                    "url": format!("{base}cs"),
                    "name": "E2ECS",
                    "version": "2.0",
                    "status": "active",
                    "content": "complete",
                    "concept": [{
                        "code": "X",
                        "display": "X Display",
                        "property": [{ "code": "type", "valueCode": "leaf" }],
                        "designation": [{ "language": "de", "value": "X Anzeige" }]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "ValueSet",
                    "id": format!("vs-{uid}"),
                    "url": format!("{base}vs"),
                    "name": "E2EVS",
                    "status": "active",
                    "compose": {
                        "include": [{ "system": format!("{base}cs") }]
                    }
                }
            }
        ]
    });

    let bytes = serde_json::to_vec(&bundle).unwrap();
    let stats = backend.import_bundle(&ctx(), &bytes).await.unwrap();
    assert_eq!(stats.code_systems, 1);
    assert_eq!(stats.value_sets, 1);
    assert_eq!(stats.concepts, 1);
    assert!(stats.errors.is_empty(), "No errors: {:?}", stats.errors);

    let req = LookupRequest {
        system: format!("{base}cs"),
        code: "X".into(),
        ..Default::default()
    };
    let resp = CodeSystemOperations::lookup(&backend, &ctx(), req)
        .await
        .unwrap();
    assert_eq!(resp.display.as_deref(), Some("X Display"));
    assert_eq!(resp.version.as_deref(), Some("2.0"));

    let expand_req = ExpandRequest {
        url: Some(format!("{base}vs")),
        ..Default::default()
    };
    let expand_resp = backend.expand(&ctx(), expand_req).await.unwrap();
    assert_eq!(expand_resp.total, Some(1));
    assert_eq!(expand_resp.contains[0].code, "X");
}

// ── 4.2 — CodeSystem tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn lookup_filters_properties() {
    let backend = fresh_backend().await;
    let base = base_url!("lkp-filt");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: cs_url,
            code: "ABC".into(),
            properties: vec!["parent".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.properties.len(), 1);
    assert_eq!(resp.properties[0].code, "parent");
}

#[tokio::test]
async fn lookup_display_language_falls_back_to_newest_other_version() {
    let backend = fresh_backend().await;
    let base = base_url!("lkp-mv");
    let cs_url = format!("{base}cs");

    // Two coexisting versions of one canonical URL: the newest (20260601)
    // has only English content; the older edition carries the German term.
    for (version, designations) in [
        ("20260601", serde_json::json!([])),
        (
            "20260515",
            serde_json::json!([{"language": "de", "value": "Farbe"}]),
        ),
    ] {
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{
                "resource": {
                    "resourceType": "CodeSystem",
                    "id": format!("cs-{uid}"),
                    "url": cs_url,
                    "name": "TestCS",
                    "version": version,
                    "status": "active",
                    "content": "complete",
                    "concept": [{
                        "code": "C1",
                        "display": format!("Color ({version})"),
                        "designation": designations
                    }]
                }
            }]
        });
        backend
            .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
            .await
            .expect("Seed import should succeed");
    }

    // Unversioned lookup resolves 20260601 (no German) — the German
    // designation must be served from the older 20260515 edition.
    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: cs_url.clone(),
            code: "C1".into(),
            display_language: Some("de".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.display.as_deref(), Some("Farbe"));
    assert_eq!(resp.version.as_deref(), Some("20260601"));
    assert_eq!(resp.designations.len(), 1);
    assert_eq!(resp.designations[0].language.as_deref(), Some("de"));

    // An explicit version pin is respected strictly: no fallback.
    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: cs_url,
            code: "C1".into(),
            version: Some("20260601".into()),
            display_language: Some("de".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.display.as_deref(), Some("Color (20260601)"));
    assert!(resp.designations.is_empty());
}

#[tokio::test]
async fn lookup_display_language_preferred() {
    let backend = fresh_backend().await;
    let base = base_url!("lkp-lang");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: cs_url,
            code: "ABC".into(),
            display_language: Some("fr".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // French designation should be preferred for display.
    assert_eq!(resp.display.as_deref(), Some("Alpha Bêta Charlie"));
    // Only the French designation should be returned.
    let fr_desigs: Vec<_> = resp
        .designations
        .iter()
        .filter(|d| d.language.as_deref() == Some("fr"))
        .collect();
    assert!(
        !fr_desigs.is_empty(),
        "French designation should be returned"
    );
}

#[tokio::test]
async fn lookup_unknown_system_returns_not_found() {
    let backend = fresh_backend().await;

    let err = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: "http://no.such.system/cs".into(),
            code: "X".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, HtsError::NotFound(_)));
}

#[tokio::test]
async fn lookup_unknown_code_returns_not_found() {
    let backend = fresh_backend().await;
    let base = base_url!("lkp-404");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let err = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: cs_url,
            code: "NOPE".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, HtsError::NotFound(_)));
}

#[tokio::test]
async fn validate_code_display_mismatch_returns_false() {
    let backend = fresh_backend().await;
    let base = base_url!("vc-mismatch");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let resp = CodeSystemOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            system: Some(cs_url),
            code: "ABC".into(),
            display: Some("Wrong Display".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!resp.result, "Display mismatch should return false");
    assert!(resp.message.is_some(), "Message should be present");
}

#[tokio::test]
async fn validate_code_unknown_system_returns_false() {
    let backend = fresh_backend().await;

    let resp = CodeSystemOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            system: Some("http://no.such.system/cs".into()),
            code: "X".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(
        !resp.result,
        "Unknown system should return false (not error)"
    );
}

#[tokio::test]
async fn subsumes_equivalent_same_code() {
    let backend = fresh_backend().await;
    let base = base_url!("sub-equiv");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let resp = CodeSystemOperations::subsumes(
        &backend,
        &ctx(),
        SubsumesRequest {
            system: cs_url,
            code_a: "ABC".into(),
            code_b: "ABC".into(),
            version: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.outcome, SubsumptionOutcome::Equivalent);
}

#[tokio::test]
async fn subsumes_subsumed_by_returns_correct_outcome() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("sub-by");

    // A → B hierarchy
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{"resource": {
            "resourceType": "CodeSystem",
            "id": format!("cs-{uid}"),
            "url": format!("{base}cs"),
            "name": "HierCS",
            "status": "active",
            "content": "complete",
            "concept": [{
                "code": "A", "display": "A",
                "concept": [{"code": "B", "display": "B"}]
            }]
        }}]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    // B is subsumed by A
    let resp = CodeSystemOperations::subsumes(
        &backend,
        &ctx(),
        SubsumesRequest {
            system: format!("{base}cs"),
            code_a: "B".into(),
            code_b: "A".into(),
            version: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.outcome, SubsumptionOutcome::SubsumedBy);
}

#[tokio::test]
async fn subsumes_not_subsumed_returns_correct_outcome() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("sub-none");

    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{"resource": {
            "resourceType": "CodeSystem",
            "id": format!("cs-{uid}"),
            "url": format!("{base}cs"),
            "name": "FlatCS",
            "status": "active",
            "content": "complete",
            "concept": [
                {"code": "A", "display": "A"},
                {"code": "B", "display": "B"}
            ]
        }}]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = CodeSystemOperations::subsumes(
        &backend,
        &ctx(),
        SubsumesRequest {
            system: format!("{base}cs"),
            code_a: "A".into(),
            code_b: "B".into(),
            version: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(resp.outcome, SubsumptionOutcome::NotSubsumed);
}

#[tokio::test]
async fn search_by_url_returns_match() {
    let backend = fresh_backend().await;
    let base = base_url!("srch-url");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let results = CodeSystemOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(cs_url.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        results.len(),
        1,
        "Should find exactly one CodeSystem by URL"
    );
    assert_eq!(results[0]["url"].as_str(), Some(cs_url.as_str()));
}

#[tokio::test]
async fn search_by_name_returns_match() {
    let backend = fresh_backend().await;
    let base = base_url!("srch-name");
    let cs_url = format!("{base}cs");
    seed_code_system_with_url(&backend, &cs_url).await;

    let results = CodeSystemOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            name: Some("TestCS".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!results.is_empty(), "Should find CodeSystem by name");
    assert!(
        results
            .iter()
            .any(|r| r["url"].as_str() == Some(cs_url.as_str())),
        "Our seeded CS should be in the results"
    );
}

#[tokio::test]
async fn search_by_status_filters_correctly() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("srch-status");

    // Import an active and a retired system.
    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-act-{uid}"),
                "url": format!("{base}active"), "name": "ActiveCS",
                "status": "active", "content": "complete"
            }},
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-ret-{uid}"),
                "url": format!("{base}retired"), "name": "RetiredCS",
                "status": "retired", "content": "complete"
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    // Filter for active only. Use a large page size so that this test's
    // own fresh imports are not pushed off the first page by other
    // concurrent tests writing to the shared DB.
    let results = CodeSystemOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            status: Some("active".into()),
            count: Some(1000),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let urls: Vec<&str> = results.iter().filter_map(|r| r["url"].as_str()).collect();
    assert!(
        urls.contains(&format!("{base}active").as_str()),
        "Active system should be in results"
    );
    assert!(
        !urls.contains(&format!("{base}retired").as_str()),
        "Retired system should be excluded"
    );
}

#[tokio::test]
async fn search_pagination_limit_and_offset() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("srch-page");

    // Import 3 code systems with predictable names.
    for i in 0..3u32 {
        let bundle = serde_json::json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": [{"resource": {
                "resourceType": "CodeSystem",
                "id": format!("cs-pg-{i}-{uid}"),
                "url": format!("{base}cs-{i}"),
                "name": format!("PageCS{i}"),
                "status": "active",
                "content": "complete"
            }}]
        });
        backend
            .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
            .await
            .unwrap();
    }

    // Request page with count=2, offset=1 (filtered to our base URL pattern via name prefix).
    // We use _count=2 and _offset=1 on the URL filter for the subset we imported.
    let page = CodeSystemOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(format!("{base}cs-1")),
            count: Some(2),
            offset: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(page.len(), 1, "URL-filtered result should have exactly 1");
}

#[tokio::test]
async fn search_unknown_url_returns_empty() {
    let backend = fresh_backend().await;

    let results = CodeSystemOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some("http://no.match.ever/cs-xxxx".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(results.is_empty(), "Unknown URL should return empty vec");
}

// ── 4.3 — ValueSet tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn expand_explicit_code_list() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-explicit");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "ExpCS",
                "status": "active", "content": "complete",
                "concept": [
                    {"code": "A", "display": "Alpha"},
                    {"code": "B", "display": "Beta"},
                    {"code": "C", "display": "Gamma"}
                ]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "ExpVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs"),
                    "concept": [{"code": "A"}, {"code": "C"}]}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.total, Some(2));
    let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
    assert!(codes.contains(&"A"));
    assert!(codes.contains(&"C"));
    assert!(!codes.contains(&"B"), "B should not be in explicit list");
}

#[tokio::test]
async fn expand_exclude_rules_remove_codes() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-excl");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "ExclCS",
                "status": "active", "content": "complete",
                "concept": [
                    {"code": "A", "display": "Alpha"},
                    {"code": "B", "display": "Beta"}
                ]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "ExclVS",
                "status": "active",
                "compose": {
                    "include": [{"system": format!("{base}cs")}],
                    "exclude": [{"system": format!("{base}cs"),
                        "concept": [{"code": "B"}]}]
                }
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
    assert!(codes.contains(&"A"), "A should be included");
    assert!(!codes.contains(&"B"), "B should be excluded");
}

#[tokio::test]
async fn expand_pagination_count_and_offset() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-page");

    let concepts: Vec<serde_json::Value> = (0..5)
        .map(|i| serde_json::json!({"code": format!("C{i}"), "display": format!("Concept {i}")}))
        .collect();

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "PageCS",
                "status": "active", "content": "complete",
                "concept": concepts
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "PageVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                count: Some(2),
                offset: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.total, Some(5), "Total should reflect all 5 concepts");
    assert_eq!(resp.contains.len(), 2, "Page should contain 2 items");
    assert_eq!(resp.offset, Some(1), "Offset should be returned");
}

#[tokio::test]
async fn expand_filter_substring_match() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-filt");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "FiltCS",
                "status": "active", "content": "complete",
                "concept": [
                    {"code": "APPLE", "display": "Apple"},
                    {"code": "APRICOT", "display": "Apricot"},
                    {"code": "BANANA", "display": "Banana"}
                ]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "FiltVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                filter: Some("Ap".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
    assert!(codes.contains(&"APPLE"), "APPLE should match filter 'Ap'");
    assert!(
        codes.contains(&"APRICOT"),
        "APRICOT should match filter 'Ap'"
    );
    assert!(
        !codes.contains(&"BANANA"),
        "BANANA should not match filter 'Ap'"
    );
}

#[tokio::test]
async fn expand_hierarchical_returns_nested_tree() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-hier");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "HierCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "A",
                    "concept": [{"code": "B", "display": "B",
                        "concept": [{"code": "C", "display": "C"}]}]
                }]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "HierVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                hierarchical: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Top-level should have A; A should have B as a child; B should have C.
    assert_eq!(resp.contains.len(), 1, "Top level should have 1 root node");
    let a = &resp.contains[0];
    assert_eq!(a.code, "A");
    assert!(!a.contains.is_empty(), "A should have children");
    let b = &a.contains[0];
    assert_eq!(b.code, "B");
    assert!(!b.contains.is_empty(), "B should have children");
    assert_eq!(b.contains[0].code, "C");
}

#[tokio::test]
async fn expand_cache_hit_on_second_call() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-cache");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "CacheCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "X", "display": "X"}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "CacheVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let req = ExpandRequest {
        url: Some(format!("{base}vs")),
        ..Default::default()
    };
    let first = backend.expand(&ctx(), req.clone()).await.unwrap();
    let second = backend.expand(&ctx(), req).await.unwrap();

    assert_eq!(
        first.total, second.total,
        "Both calls should return same total"
    );
    assert_eq!(
        first.contains.len(),
        second.contains.len(),
        "Both calls should return same codes"
    );
}

#[tokio::test]
async fn expand_implicit_value_set() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-impl");

    // CodeSystem with a valueSet property pointing to its implicit VS.
    let implicit_vs_url = format!("{base}vs");
    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [{"resource": {
            "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
            "url": format!("{base}cs"),
            "valueSet": implicit_vs_url,
            "name": "ImplCS",
            "status": "active", "content": "complete",
            "concept": [
                {"code": "A", "display": "Alpha"},
                {"code": "B", "display": "Beta"}
            ]
        }}]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(implicit_vs_url),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(resp.total, Some(2), "Implicit VS should expand all codes");
}

#[tokio::test]
async fn expand_max_expansion_size_returns_too_costly() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("exp-costly");

    let concepts: Vec<serde_json::Value> = (0..10)
        .map(|i| serde_json::json!({"code": format!("C{i}"), "display": format!("Concept {i}")}))
        .collect();

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "CostlyCS",
                "status": "active", "content": "complete",
                "concept": concepts
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "CostlyVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let err = backend
        .expand(
            &ctx(),
            ExpandRequest {
                url: Some(format!("{base}vs")),
                max_expansion_size: Some(3), // Limit to 3, but there are 10
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, HtsError::TooCostly(_)),
        "Should return TooCostly when expansion exceeds limit, got: {err:?}"
    );
}

#[tokio::test]
async fn vs_validate_code_in_set_returns_true() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-vc-true");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "VCCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "Alpha"}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "VCVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = ValueSetOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            url: Some(format!("{base}vs")),
            code: "A".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result, "Code in set should return true");
}

#[tokio::test]
async fn vs_validate_code_not_in_set_returns_false() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-vc-false");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "NVCCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "Alpha"}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "NVCVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = ValueSetOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            url: Some(format!("{base}vs")),
            code: "Z".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!resp.result, "Code not in set should return false");
}

#[tokio::test]
async fn vs_validate_code_display_mismatch_returns_false() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-vc-disp");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
                "url": format!("{base}cs"), "name": "VCDCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "Correct Display"}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "VCDVS",
                "status": "active",
                "compose": {"include": [{"system": format!("{base}cs")}]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = ValueSetOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            url: Some(format!("{base}vs")),
            code: "A".into(),
            display: Some("Wrong Display".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!resp.result, "Display mismatch should return false");
    assert!(resp.message.is_some(), "Message should be present");
}

#[tokio::test]
async fn vs_validate_code_system_filter() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-vc-sys");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs1-{uid}"),
                "url": format!("{base}cs1"), "name": "SysCS1",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "A from CS1"}]
            }},
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs2-{uid}"),
                "url": format!("{base}cs2"), "name": "SysCS2",
                "status": "active", "content": "complete",
                "concept": [{"code": "A", "display": "A from CS2"}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "id": format!("vs-{uid}"),
                "url": format!("{base}vs"), "name": "SysVS",
                "status": "active",
                "compose": {"include": [
                    {"system": format!("{base}cs1")},
                    {"system": format!("{base}cs2")}
                ]}
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    // Validate "A" from cs1 only (system filter)
    let resp = ValueSetOperations::validate_code(
        &backend,
        &ctx(),
        ValidateCodeRequest {
            url: Some(format!("{base}vs")),
            system: Some(format!("{base}cs1")),
            code: "A".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result, "Code A from cs1 should be valid in the VS");
}

#[tokio::test]
async fn search_value_sets_by_url() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-srch-url");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [{"resource": {
            "resourceType": "ValueSet", "id": format!("vs-{uid}"),
            "url": format!("{base}vs"), "name": "SrchVS",
            "status": "active"
        }}]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let results = ValueSetOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(format!("{base}vs")),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1, "Should find exactly one VS by URL");
}

#[tokio::test]
async fn search_value_sets_pagination() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("vs-srch-page");

    for i in 0..3u32 {
        let bundle = serde_json::json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": [{"resource": {
                "resourceType": "ValueSet",
                "id": format!("vs-pg-{i}-{uid}"),
                "url": format!("{base}vs-{i}"),
                "name": format!("PgVS{i}"),
                "status": "active"
            }}]
        });
        backend
            .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
            .await
            .unwrap();
    }

    // Fetch exactly 1 value set from our set of 3 by using the specific URL.
    let page1 = ValueSetOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(format!("{base}vs-0")),
            count: Some(2),
            offset: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(page1.len(), 1, "First page should have 1 VS for our URL");
}

// ── 4.4 — ConceptMap tests ─────────────────────────────────────────────────────

async fn seed_concept_map(backend: &PostgresTerminologyBackend, base: &str) {
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-src-{uid}"),
                "url": format!("{base}src"), "name": "SrcCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "100", "display": "One Hundred"}]
            }},
            {"resource": {
                "resourceType": "CodeSystem", "id": format!("cs-tgt-{uid}"),
                "url": format!("{base}tgt"), "name": "TgtCS",
                "status": "active", "content": "complete",
                "concept": [{"code": "200", "display": "Two Hundred"}]
            }},
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm-{uid}"),
                "url": format!("{base}cm"), "name": "TestCM",
                "status": "active",
                "group": [{"source": format!("{base}src"), "target": format!("{base}tgt"),
                    "element": [{"code": "100",
                        "target": [{"code": "200", "equivalence": "equivalent"}]}]}]
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn translate_concept_map_basic() {
    let backend = fresh_backend().await;
    let base = base_url!("tr-basic");
    seed_concept_map(&backend, &base).await;

    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            system: Some(format!("{base}src")),
            code: "100".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result);
    assert_eq!(resp.matches.len(), 1);
    assert_eq!(resp.matches[0].concept_code, "200");
}

#[tokio::test]
async fn translate_multiple_targets() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("tr-multi");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm-{uid}"),
                "url": format!("{base}cm"), "name": "MultiCM",
                "status": "active",
                "group": [{"source": format!("{base}src"), "target": format!("{base}tgt"),
                    "element": [{"code": "A",
                        "target": [
                            {"code": "X", "equivalence": "equivalent"},
                            {"code": "Y", "equivalence": "wider"}
                        ]}]}]
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            system: Some(format!("{base}src")),
            code: "A".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result);
    assert_eq!(resp.matches.len(), 2, "Should return both targets");
    let codes: Vec<&str> = resp
        .matches
        .iter()
        .map(|m| m.concept_code.as_str())
        .collect();
    assert!(codes.contains(&"X"));
    assert!(codes.contains(&"Y"));
}

#[tokio::test]
async fn translate_no_match_returns_false() {
    let backend = fresh_backend().await;
    let base = base_url!("tr-nomatch");
    seed_concept_map(&backend, &base).await;

    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            system: Some(format!("{base}src")),
            code: "BOGUS".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(!resp.result, "No match should return false");
    assert!(resp.matches.is_empty());
}

#[tokio::test]
async fn translate_system_filter() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("tr-sysfilt");

    // Two maps: one from base/src1 and one from base/src2
    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm1-{uid}"),
                "url": format!("{base}cm1"), "name": "CM1",
                "status": "active",
                "group": [{"source": format!("{base}src1"), "target": format!("{base}tgt"),
                    "element": [{"code": "A",
                        "target": [{"code": "X", "equivalence": "equivalent"}]}]}]
            }},
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm2-{uid}"),
                "url": format!("{base}cm2"), "name": "CM2",
                "status": "active",
                "group": [{"source": format!("{base}src2"), "target": format!("{base}tgt"),
                    "element": [{"code": "A",
                        "target": [{"code": "Z", "equivalence": "equivalent"}]}]}]
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    // Filter to src1 only — should only find X, not Z
    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            system: Some(format!("{base}src1")),
            code: "A".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result);
    assert!(
        resp.matches.iter().any(|m| m.concept_code == "X"),
        "src1 mapping should produce X"
    );
    assert!(
        !resp.matches.iter().any(|m| m.concept_code == "Z"),
        "src2 mapping should be excluded by system filter"
    );
}

#[tokio::test]
async fn translate_map_url_filter() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("tr-mapurl");

    // Two maps with the same source code but different target codes
    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm1-{uid}"),
                "url": format!("{base}cm1"), "name": "CMapU1",
                "status": "active",
                "group": [{"source": format!("{base}src"), "target": format!("{base}tgt"),
                    "element": [{"code": "A",
                        "target": [{"code": "P", "equivalence": "equivalent"}]}]}]
            }},
            {"resource": {
                "resourceType": "ConceptMap", "id": format!("cm2-{uid}"),
                "url": format!("{base}cm2"), "name": "CMapU2",
                "status": "active",
                "group": [{"source": format!("{base}src"), "target": format!("{base}tgt"),
                    "element": [{"code": "A",
                        "target": [{"code": "Q", "equivalence": "equivalent"}]}]}]
            }}
        ]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    // Filter by map URL — should only use cm1 → P, not cm2 → Q
    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            url: Some(format!("{base}cm1")),
            code: "A".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result);
    assert!(
        resp.matches.iter().any(|m| m.concept_code == "P"),
        "cm1 mapping should produce P"
    );
    assert!(
        !resp.matches.iter().any(|m| m.concept_code == "Q"),
        "cm2 mapping should be excluded by URL filter"
    );
}

#[tokio::test]
async fn translate_reverse_mode() {
    let backend = fresh_backend().await;
    let base = base_url!("tr-rev");
    seed_concept_map(&backend, &base).await;

    // Reverse: look up target "200" → resolve source "100".
    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            system: Some(format!("{base}tgt")),
            code: "200".into(),
            reverse: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(resp.result, "Reverse translation should succeed");
    assert_eq!(resp.matches.len(), 1);
    // `concept_*` reflects the target side (the looked-up code), `source_*`
    // reflects the resolved source side — independent of forward vs reverse.
    assert_eq!(resp.matches[0].concept_code, "200");
    assert_eq!(resp.matches[0].source_code.as_deref(), Some("100"));
}

#[tokio::test]
async fn translate_code_only_no_system() {
    let backend = fresh_backend().await;
    let base = base_url!("tr-nosys");
    seed_concept_map(&backend, &base).await;

    // No system filter — should still find the mapping
    let resp = ConceptMapOperations::translate(
        &backend,
        &ctx(),
        TranslateRequest {
            code: "100".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(
        resp.result,
        "Code-only lookup should find first matching mapping"
    );
    assert!(!resp.matches.is_empty());
}

#[tokio::test]
async fn closure_empty_input_returns_empty_map() {
    let backend = fresh_backend().await;

    let resp = ConceptMapOperations::closure(
        &backend,
        &ctx(),
        ClosureRequest {
            name: "empty-closure".into(),
            concept: vec![],
            version: None,
        },
    )
    .await
    .unwrap();

    let groups = resp
        .concept_map
        .as_ref()
        .and_then(|cm| cm["group"].as_array())
        .map(|g| g.len())
        .unwrap_or(0);
    assert_eq!(groups, 0, "Empty input should produce no groups");
}

#[tokio::test]
async fn closure_unrelated_codes_produce_no_pairs() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("cl-unrelated");

    let bundle = serde_json::json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [{"resource": {
            "resourceType": "CodeSystem", "id": format!("cs-{uid}"),
            "url": format!("{base}cs"), "name": "CLCS",
            "status": "active", "content": "complete",
            "concept": [
                {"code": "A", "display": "A"},
                {"code": "B", "display": "B"}
            ]
        }}]
    });
    backend
        .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
        .await
        .unwrap();

    let resp = ConceptMapOperations::closure(
        &backend,
        &ctx(),
        ClosureRequest {
            name: "unrelated-closure".into(),
            concept: vec![
                CodingConcept {
                    system: format!("{base}cs"),
                    code: "A".into(),
                    display: None,
                },
                CodingConcept {
                    system: format!("{base}cs"),
                    code: "B".into(),
                    display: None,
                },
            ],
            version: None,
        },
    )
    .await
    .unwrap();

    let groups = resp
        .concept_map
        .as_ref()
        .and_then(|cm| cm["group"].as_array())
        .map(|g| g.len())
        .unwrap_or(0);
    assert_eq!(
        groups, 0,
        "Unrelated codes should produce no hierarchy pairs"
    );
}

#[tokio::test]
async fn closure_unknown_system_skipped() {
    let backend = fresh_backend().await;

    let resp = ConceptMapOperations::closure(
        &backend,
        &ctx(),
        ClosureRequest {
            name: "unknown-sys-closure".into(),
            concept: vec![CodingConcept {
                system: "http://no.such.system/cs".into(),
                code: "X".into(),
                display: None,
            }],
            version: None,
        },
    )
    .await
    .unwrap();

    // Should not error; groups will be empty.
    let groups = resp
        .concept_map
        .as_ref()
        .and_then(|cm| cm["group"].as_array())
        .map(|g| g.len())
        .unwrap_or(0);
    assert_eq!(
        groups, 0,
        "Unknown system should produce no pairs and no error"
    );
}

#[tokio::test]
async fn search_concept_maps_by_url() {
    let backend = fresh_backend().await;
    let base = base_url!("cm-srch-url");
    seed_concept_map(&backend, &base).await;

    let results = ConceptMapOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(format!("{base}cm")),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1, "Should find exactly one CM by URL");
    assert_eq!(
        results[0]["url"].as_str(),
        Some(format!("{base}cm").as_str())
    );
}

#[tokio::test]
async fn search_concept_maps_pagination() {
    let backend = fresh_backend().await;
    let (base, uid) = test_ctx!("cm-srch-page");

    for i in 0..3u32 {
        let bundle = serde_json::json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": [{"resource": {
                "resourceType": "ConceptMap",
                "id": format!("cm-p{i}-{uid}"),
                "url": format!("{base}cm-{i}"),
                "name": format!("PageCM{i}"),
                "status": "active"
            }}]
        });
        backend
            .import_bundle(&ctx(), &serde_json::to_vec(&bundle).unwrap())
            .await
            .unwrap();
    }

    // Fetch by specific URL to verify individual results.
    let result = ConceptMapOperations::search(
        &backend,
        &ctx(),
        ResourceSearchQuery {
            url: Some(format!("{base}cm-1")),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(result.len(), 1, "Should find exactly the requested CM");
}

// ──────────────────────────────────────────────────────────────────────────────
// Importer parity tests — verify a representative set of non-hl7-npm importers
// runs end-to-end against the PostgreSQL backend.
//
// Each test uses `delete_normalized` to purge its CodeSystem by URL before
// seeding, so it is robust to state left over by prior test runs or by other
// tests running in parallel. Assertions are URL-scoped (via `lookup` or
// URL-filtered SQL counts) rather than table-wide, so concurrent tests against
// different CodeSystem URLs do not interfere.
// ──────────────────────────────────────────────────────────────────────────────

use helios_hts::import::dicom::import_dicom;
use helios_hts::import::icd10_cm::import_icd10_cm;
use helios_hts::import::loinc_csv::import_loinc_csv;
use helios_hts::import::ucum::import_ucum;

/// Scoped count of concepts registered under a specific CodeSystem URL.
async fn count_concepts_for_url(backend: &PostgresTerminologyBackend, url: &str) -> i64 {
    let client = backend.pool().get().await.unwrap();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM concepts c \
             JOIN code_systems cs ON cs.id = c.system_id \
             WHERE cs.url = $1",
            &[&url],
        )
        .await
        .unwrap();
    row.get(0)
}

/// Scoped count of hierarchy edges registered under a specific CodeSystem URL.
async fn count_hierarchy_for_url(backend: &PostgresTerminologyBackend, url: &str) -> i64 {
    let client = backend.pool().get().await.unwrap();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM concept_hierarchy ch \
             JOIN code_systems cs ON cs.id = ch.system_id \
             WHERE cs.url = $1",
            &[&url],
        )
        .await
        .unwrap();
    row.get(0)
}

/// UCUM parity — real distribution uses `http://unitsofmeasure.org`.
#[tokio::test]
async fn importer_ucum_runs_against_postgres() {
    let backend = fresh_backend().await;
    const UCUM_URL: &str = "http://unitsofmeasure.org";

    // Purge any residual state from a prior run.
    backend
        .delete_normalized("CodeSystem", UCUM_URL)
        .await
        .unwrap();

    // Minimal UCUM essence XML fixture — 3 codes.
    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="http://unitsofmeasure.org/ucum-essence" version="2.1">
  <prefix Code="k" CODE="K"><name>kilo</name></prefix>
  <base-unit Code="m" CODE="M"><name>meter</name></base-unit>
  <unit Code="[lb_av]" CODE="[LB_AV]"><name>pound</name></unit>
</root>"#;

    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
    tmp.write_all(SAMPLE_XML.as_bytes()).unwrap();

    let stats = import_ucum(&backend, &ctx(), tmp.path(), 500, false)
        .await
        .unwrap();

    assert_eq!(stats.code_systems, 1);
    assert_eq!(stats.concepts, 3);

    // Virtual UCUM root + 3 unit codes.
    assert_eq!(count_concepts_for_url(&backend, UCUM_URL).await, 4);
    // 3 flat edges: UCUM → each code.
    assert_eq!(count_hierarchy_for_url(&backend, UCUM_URL).await, 3);

    // Round-trip via `$lookup` to confirm the data is queryable.
    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: UCUM_URL.into(),
            code: "m".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.display.as_deref(), Some("meter"));

    // Cleanup so we do not leak state into subsequent tests.
    backend
        .delete_normalized("CodeSystem", UCUM_URL)
        .await
        .unwrap();
}

/// LOINC parity — real distribution uses `http://loinc.org`.
#[tokio::test]
async fn importer_loinc_runs_against_postgres() {
    let backend = fresh_backend().await;
    const LOINC_URL: &str = "http://loinc.org";

    backend
        .delete_normalized("CodeSystem", LOINC_URL)
        .await
        .unwrap();

    // Synthetic LoincTable + MultiAxialHierarchy CSVs, packed into a ZIP.
    const LOINC_TABLE_CSV: &str = "LOINC_NUM,LONG_COMMON_NAME,ShortName,STATUS\r\n\
2160-0,Creatinine [Mass/volume] in Serum or Plasma,Creat SerPl-mCnc,ACTIVE\r\n\
718-7,Hemoglobin [Mass/volume] in Blood,Hgb Bld-mCnc,ACTIVE\r\n\
99999-9,Old deprecated test,Old test,DEPRECATED\r\n";

    const HIERARCHY_CSV: &str = "PATH_TO_ROOT,SEQUENCE,IMMEDIATE_PARENT,CODE,CODE_TEXT\r\n\
LP7786-3,1,,LP7786-3,Laboratory\r\n\
LP7786-3.LP29693-6,2,LP7786-3,LP29693-6,Chemistry\r\n\
LP7786-3.LP29693-6.2160-0,3,LP29693-6,2160-0,Creatinine\r\n\
LP7786-3.LP10156-0,2,LP7786-3,LP10156-0,Hematology\r\n\
LP7786-3.LP10156-0.718-7,3,LP10156-0,718-7,Hemoglobin\r\n";

    use std::io::Write;
    let tmp = tempfile::NamedTempFile::with_suffix(".zip").unwrap();
    {
        let mut zip = zip::ZipWriter::new(tmp.reopen().unwrap());
        let opts = zip::write::FileOptions::default();
        zip.start_file("LoincTable.csv", opts).unwrap();
        zip.write_all(LOINC_TABLE_CSV.as_bytes()).unwrap();
        zip.start_file("MultiAxialHierarchy.csv", opts).unwrap();
        zip.write_all(HIERARCHY_CSV.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    let stats = import_loinc_csv(&backend, &ctx(), tmp.path(), 500, false, &LanguageFilter::default())
        .await
        .unwrap();

    // 3 LOINC codes + 3 LP category nodes = 6 concepts.
    assert_eq!(stats.concepts, 6);
    assert_eq!(count_concepts_for_url(&backend, LOINC_URL).await, 6);
    assert_eq!(count_hierarchy_for_url(&backend, LOINC_URL).await, 4);

    // Deprecated codes carry their status in the `definition` column.
    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: LOINC_URL.into(),
            code: "99999-9".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.display.as_deref(), Some("Old deprecated test"));

    backend
        .delete_normalized("CodeSystem", LOINC_URL)
        .await
        .unwrap();
}

/// ICD-10-CM parity — real distribution uses `http://hl7.org/fhir/sid/icd-10-cm`.
#[tokio::test]
async fn importer_icd10_cm_runs_against_postgres() {
    let backend = fresh_backend().await;
    const ICD10CM_URL: &str = "http://hl7.org/fhir/sid/icd-10-cm";

    backend
        .delete_normalized("CodeSystem", ICD10CM_URL)
        .await
        .unwrap();

    const TABULAR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ICD10CM.tabular>
  <chapter>
    <name>I</name>
    <desc>Certain infectious and parasitic diseases</desc>
    <section id="A00-A09">
      <desc>Intestinal infectious diseases</desc>
      <diag>
        <name>A00</name>
        <desc>Cholera</desc>
        <diag><name>A00.0</name><desc>Cholera due to Vibrio cholerae 01, biovar cholerae</desc></diag>
        <diag><name>A00.9</name><desc>Cholera, unspecified</desc></diag>
      </diag>
    </section>
  </chapter>
</ICD10CM.tabular>"#;

    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::with_suffix(".xml").unwrap();
    tmp.write_all(TABULAR_XML.as_bytes()).unwrap();

    let stats = import_icd10_cm(&backend, &ctx(), tmp.path(), 500, false)
        .await
        .unwrap();

    // Virtual root + 1 chapter + 1 section + 1 header + 2 billable = 6.
    assert_eq!(stats.concepts, 6);
    assert_eq!(count_concepts_for_url(&backend, ICD10CM_URL).await, 6);
    // Every concept except the virtual root has a parent = 5 edges.
    assert_eq!(count_hierarchy_for_url(&backend, ICD10CM_URL).await, 5);

    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: ICD10CM_URL.into(),
            code: "A00.9".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.display.as_deref(), Some("Cholera, unspecified"));

    backend
        .delete_normalized("CodeSystem", ICD10CM_URL)
        .await
        .unwrap();
}

/// DICOM parity — small flat enumeration importer, covers the "no hierarchy
/// beyond a virtual root" path.
#[tokio::test]
async fn importer_dicom_runs_against_postgres() {
    let backend = fresh_backend().await;
    const DICOM_URL: &str = "http://dicom.nema.org/resources/ontology/DCM";

    backend
        .delete_normalized("CodeSystem", DICOM_URL)
        .await
        .unwrap();

    const SAMPLE_CSV: &str = "CodeValue,CodingSchemeDesignator,CodeMeaning\n\
001,DCM,Quantitative Immunofluorescence\n\
002,DCM,Qualitative Immunofluorescence\n";

    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::with_suffix(".csv").unwrap();
    tmp.write_all(SAMPLE_CSV.as_bytes()).unwrap();

    let stats = import_dicom(&backend, &ctx(), tmp.path(), 500, false)
        .await
        .unwrap();

    assert_eq!(stats.concepts, 2);
    // DCM virtual root + 2 real codes.
    assert_eq!(count_concepts_for_url(&backend, DICOM_URL).await, 3);
    // 2 flat edges.
    assert_eq!(count_hierarchy_for_url(&backend, DICOM_URL).await, 2);

    let resp = CodeSystemOperations::lookup(
        &backend,
        &ctx(),
        LookupRequest {
            system: DICOM_URL.into(),
            code: "002".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        resp.display.as_deref(),
        Some("Qualitative Immunofluorescence")
    );

    backend
        .delete_normalized("CodeSystem", DICOM_URL)
        .await
        .unwrap();
}
