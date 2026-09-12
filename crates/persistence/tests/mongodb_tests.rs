//! MongoDB backend tests.
//!
//! Run with:
//! `cargo test -p helios-persistence --features mongodb --test mongodb_tests`
//!
//! The integration tests provision MongoDB automatically: when Docker is
//! available they start an ephemeral standalone Mongo testcontainer (so the
//! suite runs in CI), and they otherwise use `HFS_TEST_MONGODB_URL` if set
//! (e.g. `HFS_TEST_MONGODB_URL=mongodb://localhost:27017` to target a specific
//! instance). When neither is available the integration tests skip.
//!
//! Multi-document transaction tests additionally require a replica set; against
//! a standalone server they skip only the transaction-specific assertions.

#![cfg(feature = "mongodb")]

use std::path::PathBuf;
use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
use helios_persistence::core::{
    Backend, BackendCapability, BackendKind, BundleEntry, BundleMethod, BundleProvider,
    BundleResult, ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage,
    ConditionalUpdateResult, HistoryParams, IncludeProvider, InstanceHistoryProvider, PatchFormat,
    ResourceStorage, RevincludeProvider, SearchProvider, SettingsStore, SystemHistoryProvider,
    TypeHistoryProvider, VersionedStorage,
};
use helios_persistence::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, TransactionError,
};
use helios_persistence::search::SearchParameterStatus;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    IncludeDirective, IncludeType, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, SortDirective,
};
use mongodb::Client;
use mongodb::bson::{Document, doc};
use serde_json::json;

const MONGODB_MAX_DATABASE_NAME_LEN: usize = 63;
const MONGODB_TEST_DB_PREFIX: &str = "hfs_phase2_mongo_";

/// Connection-pool caps for the integration suite.
///
/// mongod is thread-per-connection, so its resident memory scales with the
/// number of open connections. The whole suite shares one standalone container,
/// and ~40 integration tests run in parallel, each holding a backend pool plus
/// occasional raw admin/assertion clients. Left at their defaults (backend
/// pool = 10, driver default raw-client pool = 100) the peak connection count
/// balloons and — on the shared, memory-pressured CI docker host — the host
/// OOM-kills mongod mid-run (seen as "unexpected end of file" then "connection
/// refused" on the last wave of tests). Capping both pools keeps the suite's
/// footprint small; the datasets are tiny so a handful of connections suffice.
const TEST_BACKEND_MAX_POOL: u32 = 4;
const TEST_RAW_CLIENT_MAX_POOL: u32 = 2;

/// Builds a raw `mongodb::Client` with a small pool for test-only assertions and
/// fixture writes, instead of the driver's default `max_pool_size` of 100.
async fn raw_test_client(uri: &str) -> mongodb::error::Result<Client> {
    let mut options = mongodb::options::ClientOptions::parse(uri).await?;
    options.max_pool_size = Some(TEST_RAW_CLIENT_MAX_POOL);
    Client::with_options(options)
}

fn build_test_database_name(test_name: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let reserved_len = MONGODB_TEST_DB_PREFIX.len() + 1 + suffix.len();
    let max_test_name_len = MONGODB_MAX_DATABASE_NAME_LEN.saturating_sub(reserved_len);
    let truncated_test_name: String = test_name.chars().take(max_test_name_len).collect();

    format!("{MONGODB_TEST_DB_PREFIX}{truncated_test_name}_{suffix}")
}

fn extract_resource_id_from_location(location: &str) -> String {
    let resource_path = location.split("/_history").next().unwrap_or(location);
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

#[test]
fn test_mongodb_config_defaults() {
    let config = MongoBackendConfig::default();
    assert_eq!(config.connection_string, "mongodb://localhost:27017");
    assert_eq!(config.database_name, "helios");
    assert_eq!(config.max_connections, 10);
    assert_eq!(config.connect_timeout_ms, 5000);
    // Unchanged from when this was a hardcoded constant — making it configurable
    // must not change the default behaviour of an existing deployment.
    assert_eq!(config.server_selection_timeout_ms, 15_000);
    assert!(!config.search_offloaded);
    assert_eq!(config.fhir_version, FhirVersion::default());
}

#[test]
fn test_mongodb_config_serialization() {
    let config = MongoBackendConfig {
        connection_string: "mongodb://mongo.test:27018".to_string(),
        database_name: "phase2".to_string(),
        max_connections: 24,
        connect_timeout_ms: 7000,
        server_selection_timeout_ms: 9000,
        ..Default::default()
    };

    let serialized = serde_json::to_string(&config).unwrap();
    let decoded: MongoBackendConfig = serde_json::from_str(&serialized).unwrap();

    assert_eq!(decoded.connection_string, "mongodb://mongo.test:27018");
    assert_eq!(decoded.database_name, "phase2");
    assert_eq!(decoded.max_connections, 24);
    assert_eq!(decoded.connect_timeout_ms, 7000);
    assert_eq!(decoded.server_selection_timeout_ms, 9000);
}

#[test]
fn test_mongodb_backend_kind_display() {
    assert_eq!(BackendKind::MongoDB.to_string(), "mongodb");
}

#[test]
fn test_mongodb_integration_database_name_within_limit() {
    let db_name = build_test_database_name("create_or_update");

    assert!(db_name.len() <= MONGODB_MAX_DATABASE_NAME_LEN);

    let (name_without_uuid, uuid_suffix) = db_name.rsplit_once('_').unwrap();
    assert!(name_without_uuid.starts_with(MONGODB_TEST_DB_PREFIX));
    assert_eq!(uuid_suffix.len(), 32);
}

#[test]
fn test_mongodb_phase4_capabilities() {
    let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();

    assert_eq!(backend.kind(), BackendKind::MongoDB);
    assert_eq!(backend.name(), "mongodb");

    assert!(backend.supports(BackendCapability::Crud));
    assert!(backend.supports(BackendCapability::Versioning));
    assert!(backend.supports(BackendCapability::InstanceHistory));
    assert!(backend.supports(BackendCapability::TypeHistory));
    assert!(backend.supports(BackendCapability::SystemHistory));
    assert!(backend.supports(BackendCapability::BasicSearch));
    assert!(backend.supports(BackendCapability::DateSearch));
    assert!(backend.supports(BackendCapability::ReferenceSearch));
    assert!(backend.supports(BackendCapability::Sorting));
    assert!(backend.supports(BackendCapability::OffsetPagination));
    assert!(backend.supports(BackendCapability::CursorPagination));
    assert!(backend.supports(BackendCapability::OptimisticLocking));
    assert!(backend.supports(BackendCapability::SharedSchema));

    assert!(backend.supports(BackendCapability::Transactions));
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_topology_behavior() {
    let Some(backend) = create_backend("bundle_topology_behavior").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_topology_behavior (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-topology");

    match backend
        .process_transaction(&tenant, vec![], FhirVersion::default())
        .await
    {
        Ok(bundle_result) => assert!(bundle_result.entries.is_empty()),
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_topology_behavior (MongoDB topology does not support transactions)"
            );
        }
        Err(other) => panic!("Unexpected transaction result: {}", other),
    }
}

async fn process_transaction_or_skip(
    backend: &MongoBackend,
    tenant: &TenantContext,
    entries: Vec<BundleEntry>,
    test_name: &str,
) -> Option<BundleResult> {
    match backend
        .process_transaction(tenant, entries, FhirVersion::default())
        .await
    {
        Ok(result) => Some(result),
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping {} (MongoDB topology does not support transactions)",
                test_name
            );
            None
        }
        Err(e) => panic!("{} failed: {}", test_name, e),
    }
}

fn test_mongo_url() -> Option<String> {
    std::env::var("HFS_TEST_MONGODB_URL").ok()
}

/// Shared MongoDB endpoint for the integration suite.
///
/// Prefers an externally supplied server (`HFS_TEST_MONGODB_URL`, e.g. for local
/// runs against a specific instance) and otherwise starts a single ephemeral
/// **standalone** Mongo testcontainer shared across the whole test binary, so the
/// suite runs in CI (where Docker is available) instead of silently skipping.
/// Each test still uses a unique database name, so they don't collide on the
/// shared server.
///
/// Multi-document transactions require a replica set, which a standalone server
/// does not provide; the transaction tests detect this and skip that portion
/// (see [`process_transaction_or_skip`]). Everything else — CRUD, search,
/// history, pagination, tenancy, conditional ops — runs. If neither a URL nor
/// Docker is available the tests skip.
mod shared_mongo {
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    struct SharedMongo {
        connection_string: String,
        /// Kept alive for the duration of the test binary; reaped by the
        /// testcontainers watchdog and the CI cleanup step (by `github.run_id`
        /// label). `None` when an external `HFS_TEST_MONGODB_URL` is used.
        _container: Option<testcontainers::ContainerAsync<Mongo>>,
    }

    static SHARED: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

    async fn shared() -> Option<&'static SharedMongo> {
        SHARED
            .get_or_init(|| async {
                // Prefer an explicitly provided mongo (fast local runs).
                if let Some(url) = super::test_mongo_url() {
                    return Some(SharedMongo {
                        connection_string: url,
                        _container: None,
                    });
                }
                // Otherwise start an ephemeral standalone Mongo container; if
                // Docker is unavailable, `start()` errors and the suite skips.
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = Mongo::default()
                    .with_label("github.run_id", &run_id)
                    // Cap WiredTiger's cache. By default mongod sizes it to
                    // ~50% of *host* RAM (ignoring container limits), so on CI
                    // — where this container runs alongside ES/Postgres plus
                    // coverage-instrumented test binaries — it balloons and the
                    // host OOM-kills mongod mid-run (observed as connections
                    // refused / "unexpected end of file" on the last wave of
                    // tests). 0.25 GB is WiredTiger's floor and ample for the
                    // suite's tiny datasets. `--bind_ip_all` matches the stock
                    // image default and keeps the mapped port reachable once we
                    // supply our own command.
                    .with_cmd(["mongod", "--bind_ip_all", "--wiredTigerCacheSizeGB", "0.25"])
                    // Every test creates its own uniquely-named database, and
                    // WiredTiger holds file handles open per collection/index
                    // across all of them. With 50+ test databases the stock
                    // container nofile limit is exhausted and index builds die
                    // with TooManyFilesOpen (error 264) late in the run. 64000
                    // is mongod's own recommended minimum.
                    .with_ulimit("nofile", 64000, Some(64000))
                    .with_startup_timeout(std::time::Duration::from_secs(120))
                    .start()
                    .await
                    .ok()?;
                let host = container.get_host().await.ok()?;
                let port = container.get_host_port_ipv4(27017).await.ok()?;
                Some(SharedMongo {
                    connection_string: format!("mongodb://{host}:{port}/"),
                    _container: Some(container),
                })
            })
            .await
            .as_ref()
    }

    /// Connection string for the shared mongo, or `None` when no mongo is
    /// available (neither `HFS_TEST_MONGODB_URL` nor a startable container).
    pub(super) async fn connection_string() -> Option<String> {
        Some(shared().await?.connection_string.clone())
    }
}

/// The backend-agnostic tenant-id fidelity scenarios (issue #447), shared
/// verbatim with the SQLite and PostgreSQL suites.
///
/// `#[path]`-included rather than placed in `tests/common/`, which no test
/// target declares and cargo therefore never compiles (issue #306).
#[path = "multitenancy/tenant_id_fidelity_suite.rs"]
mod tenant_id_fidelity_suite;

/// The backend-agnostic day-precision date-boundary suite (issue #519) — the
/// #456 table that #463 pinned for SQLite only. Same `#[path]` arrangement.
#[path = "search/date_boundary_suite.rs"]
mod date_boundary_suite;

/// #519: MongoDB's date handler was explicitly unverified. Needs the full
/// registry so `birthdate` extracts into the search index at write time.
#[tokio::test]
async fn mongodb_day_precision_date_boundaries() {
    let Some(backend) = create_backend_with_full_registry("date_boundary").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    date_boundary_suite::day_precision_boundaries(&backend, "date-boundary-519").await;
}

/// #1062: a comma-separated value list on one `SearchParameter` is OR per
/// FHIR (https://build.fhir.org/search.html#combining) — for date same as
/// every other type. Drives the real `SearchProvider::search` /
/// `search_count` path so the assertion is on behavior, not just the filter
/// document shape (see `value_list_tests` in `search_impl.rs` for that
/// Docker-free half of the pin).
///
/// This is deliberately distinct from the *repeated*-parameter form
/// (`?birthdate=ge...&birthdate=le...`), which is two separate
/// `SearchParameter` entries and is intersected, not OR-ed — case (d) below
/// guards that it is untouched (MANUAL_TESTING_MATRIX row 4.3 relies on it).
#[tokio::test]
async fn mongodb_comma_separated_date_values_are_ored() {
    let Some(backend) = create_backend_with_full_registry("comma_or_date").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let tenant = create_tenant("tenant-comma-or-date");

    const BIRTHDATES: [&str; 3] = ["1985-05-05", "1995-10-02", "2005-01-01"];
    for (i, birth) in BIRTHDATES.iter().enumerate() {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": format!("cod-{i}"), "birthDate": birth}),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }

    // One `birthdate` SearchParameter carrying all the values
    // (a comma list), as opposed to `repeated_birthdate_query` below which
    // builds one SearchParameter per value.
    fn comma_birthdate_query(values: &[&str]) -> SearchQuery {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            values: values.iter().map(|v| SearchValue::parse(v)).collect(),
            ..Default::default()
        })
    }

    fn repeated_birthdate_query(values: &[&str]) -> SearchQuery {
        let mut query = SearchQuery::new("Patient");
        for v in values {
            query = query.with_parameter(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                values: vec![SearchValue::parse(v)],
                ..Default::default()
            });
        }
        query
    }

    // Eventually-consistent search backends need the seed to land in the
    // index first; poll on the broadest query until all 3 are visible (same
    // idiom as `date_boundary_suite::day_precision_boundaries`).
    let visibility_probe = comma_birthdate_query(&["ge1900-01-01"]);
    for attempt in 0..60 {
        let visible = backend
            .search(&tenant, &visibility_probe)
            .await
            .expect("visibility probe")
            .resources
            .items
            .len();
        if visible == BIRTHDATES.len() {
            break;
        }
        assert!(
            attempt < 59,
            "cohort never became searchable: {visible}/{} visible",
            BIRTHDATES.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // (a) Precondition: extraction really produced 3 rows, so a failure
    // below is never blamed on extraction instead of the fix under test.
    let precondition = backend
        .search(&tenant, &comma_birthdate_query(&["ge1900-01-01"]))
        .await
        .expect("precondition search");
    assert_eq!(
        precondition.resources.items.len(),
        3,
        "precondition: birthdate=ge1900-01-01 must see all 3 patients"
    );

    // (b) Disjoint comma list -> the union, not the empty set. Before the
    // fix the $and over one search_index row can never be satisfied by a
    // disjoint pair, `matching_resource_ids` short-circuits to an empty
    // set, and the whole search returns 0.
    let disjoint = comma_birthdate_query(&["1985-05-05", "2005-01-01"]);
    let disjoint_result = backend
        .search(&tenant, &disjoint)
        .await
        .expect("disjoint comma search");
    assert_eq!(
        disjoint_result.resources.items.len(),
        2,
        "birthdate=1985-05-05,2005-01-01 must return the union (FHIR comma = OR)"
    );
    let disjoint_count = backend
        .search_count(&tenant, &disjoint)
        .await
        .expect("disjoint comma count");
    assert_eq!(
        disjoint_count, 2,
        "search_count must agree with search() on the same query"
    );

    // (c) Range-shaped comma list -> the deliberate widening. Before the
    // fix this behaved as a closed range (1: only the 1995-10-02 patient).
    // After, it is "any date >= 1995-01-01 OR any date <= 1995-12-31" (in
    // FHIR terms, ge OR le), which every patient in this cohort satisfies —
    // the widening this fix pins on purpose (see PR description / issue
    // #1062 for the migration to the repeated form).
    let range_shaped = comma_birthdate_query(&["ge1995-01-01", "le1995-12-31"]);
    let range_result = backend
        .search(&tenant, &range_shaped)
        .await
        .expect("range-shaped comma search");
    assert_eq!(
        range_result.resources.items.len(),
        3,
        "birthdate=ge1995-01-01,le1995-12-31 widens to OR across all 3 patients"
    );

    // (d) Guard: the repeated-parameter form is a different mechanism (two
    // `SearchParameter` entries, intersected in `matching_resource_ids`)
    // and must be unaffected — it stays a closed range.
    let repeated = repeated_birthdate_query(&["ge1995-01-01", "le1995-12-31"]);
    let repeated_result = backend
        .search(&tenant, &repeated)
        .await
        .expect("repeated-parameter search");
    assert_eq!(
        repeated_result.resources.items.len(),
        1,
        "repeated birthdate parameters (ge&le) must still AND to a closed range"
    );
}

/// #1062, Number: same defect class as the date case above. `ChargeItem`
/// is chosen deliberately over `RiskAssessment.probability`: the latter is
/// an uncast choice element (`RiskAssessment.prediction.probability`) that
/// the schema-less extractor does not resolve, so a failure there would be
/// ambiguous between "extraction didn't run" and "the fix is wrong".
/// `ChargeItem.factorOverride` is a plain decimal path.
#[tokio::test]
async fn mongodb_comma_separated_number_values_are_ored() {
    let Some(backend) = create_backend_with_full_registry("comma_or_number").await else {
        eprintln!("skipping: no MongoDB container available");
        return;
    };
    let tenant = create_tenant("tenant-comma-or-number");

    const FACTORS: [f64; 3] = [0.25, 0.75, 1.5];
    for (i, factor) in FACTORS.iter().enumerate() {
        backend
            .create(
                &tenant,
                "ChargeItem",
                json!({
                    "id": format!("con-{i}"),
                    "status": "billable",
                    "factorOverride": factor,
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed chargeitem");
    }

    fn factor_query(values: &[&str]) -> SearchQuery {
        SearchQuery::new("ChargeItem").with_parameter(SearchParameter {
            name: "factor-override".to_string(),
            param_type: SearchParamType::Number,
            values: values.iter().map(|v| SearchValue::parse(v)).collect(),
            ..Default::default()
        })
    }

    let visibility_probe = factor_query(&["ge0"]);
    for attempt in 0..60 {
        let visible = backend
            .search(&tenant, &visibility_probe)
            .await
            .expect("visibility probe")
            .resources
            .items
            .len();
        if visible == FACTORS.len() {
            break;
        }
        assert!(
            attempt < 59,
            "cohort never became searchable: {visible}/{} visible",
            FACTORS.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // (a) Precondition: proves ChargeItem.factorOverride actually extracts
    // into a `value_number` row, isolating that risk from the fix itself.
    let precondition = backend
        .search(&tenant, &visibility_probe)
        .await
        .expect("precondition search");
    assert_eq!(
        precondition.resources.items.len(),
        3,
        "precondition: factor-override=ge0 must see all 3 ChargeItems \
         (a failure here means ChargeItem.factorOverride did not extract, \
         which is unrelated to #1062 — see comma_separated_number_values_are_ored \
         in search_impl.rs for the extraction-free pin of the same fix)"
    );

    // (b) Disjoint comma list -> the union, not the empty set.
    let disjoint = factor_query(&["0.25", "1.5"]);
    let disjoint_result = backend
        .search(&tenant, &disjoint)
        .await
        .expect("disjoint comma search");
    assert_eq!(
        disjoint_result.resources.items.len(),
        2,
        "factor-override=0.25,1.5 must return the union (FHIR comma = OR)"
    );
}

fn create_tenant(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

async fn create_backend(test_name: &str) -> Option<MongoBackend> {
    create_backend_with_search_offloaded(test_name, false).await
}

/// True when a schema-init error means "the mongo server is not reachable"
/// (as opposed to a genuine schema/logic bug). On the shared CI docker host the
/// standalone container is periodically killed mid-run — the host is memory
/// pressured by other concurrent jobs — after which every remaining backend
/// can't select a server. We treat that as "mongo unavailable → skip", the same
/// way the suite already skips when Docker isn't present, rather than reporting
/// it as a test failure. A real schema bug produces a different error (e.g. a
/// command failure) and still fails hard.
fn msg_is_mongo_unavailable(msg: &str) -> bool {
    msg.contains("Server selection timeout")
        || msg.contains("No available servers")
        || msg.contains("Connection refused")
        || msg.contains("Connection reset by peer")
        || msg.contains("unexpected end of file")
        || msg.contains("connection closed")
        || msg.contains("SystemOverloadedError") // driver: server too busy to respond
        || msg.contains("os error 111") // Linux: connection refused
        || msg.contains("os error 104") // Linux: connection reset by peer
        || msg.contains("os error 10061") // Windows: connection refused
}

fn is_mongo_unavailable(err: &BackendError) -> bool {
    msg_is_mongo_unavailable(&format!("{err:?}"))
}

/// `StorageError` variant of [`is_mongo_unavailable`], for errors surfaced by a
/// resource/settings operation (not just schema init) once the shared container
/// has died mid-test. The backend already retries *transient* blips internally
/// (see the settings-store `retry_transient`), so an unavailability error that
/// still reaches a test means the container is genuinely gone — a legitimate
/// skip, not a masked bug (a real logic bug surfaces as a wrong value/assertion).
fn storage_err_is_mongo_unavailable(err: &StorageError) -> bool {
    msg_is_mongo_unavailable(&format!("{err:?}"))
}

/// Builds a `MongoBackend` and runs schema initialization.
///
/// Returns `None` when the shared mongo has become unreachable, so callers skip
/// (see [`is_mongo_unavailable`]); a genuine schema-init failure still panics.
///
/// A short bounded retry absorbs a transient dropped handshake ("unexpected end
/// of file") that can hit `createCollection`/`createIndexes` (not retryable
/// writes) under load, without masking real bugs — those fail every attempt.
async fn build_backend(mut config: MongoBackendConfig) -> Option<MongoBackend> {
    const MAX_ATTEMPTS: u32 = 3;
    config.max_connections = config.max_connections.min(TEST_BACKEND_MAX_POOL);
    let mut attempt = 1;
    loop {
        let backend = MongoBackend::new(config.clone())
            .expect("failed to create MongoBackend for mongodb integration tests");
        match backend.initialize().await {
            Ok(()) => return Some(backend),
            Err(err) if attempt < MAX_ATTEMPTS && is_mongo_unavailable(&err) => {
                eprintln!(
                    "MongoDB schema init attempt {attempt}/{MAX_ATTEMPTS} failed \
                     ({err}); retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)))
                    .await;
                attempt += 1;
            }
            Err(err) if is_mongo_unavailable(&err) => {
                eprintln!(
                    "Skipping mongodb integration test: shared mongo unreachable after \
                     {MAX_ATTEMPTS} attempts ({err})"
                );
                return None;
            }
            Err(err) => {
                panic!("failed to initialize MongoDB schema for integration tests: {err:?}")
            }
        }
    }
}

async fn create_backend_with_search_offloaded(
    test_name: &str,
    search_offloaded: bool,
) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;

    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        search_offloaded,
        ..Default::default()
    };

    build_backend(config).await
}

/// Creates a backend whose registry is loaded from the repo's spec files, so
/// non-embedded search parameters (e.g. `value-quantity`) are active.
async fn create_backend_with_full_registry(test_name: &str) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))?;
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(data_dir),
        ..Default::default()
    };
    build_backend(config).await
}

async fn search_index_entry_count(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    resource_id: &str,
) -> u64 {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for search_index assertions");
    let database = client.database(&backend.config().database_name);
    let search_index = database.collection::<Document>("search_index");

    search_index
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "resource_id": resource_id,
        })
        .await
        .expect("failed to count search_index entries")
}

async fn mongodb_total_created_connections(connection_string: &str) -> Option<i64> {
    let client = raw_test_client(connection_string).await.ok()?;
    let status = client
        .database("admin")
        .run_command(doc! { "serverStatus": 1_i32 })
        .await
        .ok()?;
    let connections = status.get_document("connections").ok()?;

    connections
        .get_i64("totalCreated")
        .or_else(|_| connections.get_i32("totalCreated").map(i64::from))
        .ok()
}

#[tokio::test]
async fn mongodb_integration_readiness_check() {
    let Some(backend) = create_backend("readiness_check").await else {
        eprintln!(
            "Skipping mongodb_integration_readiness_check (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    // The `/_readiness` probe delegates to `Backend::health_check` via the
    // `ResourceStorage::readiness_check` override; a live mongo must report ready.
    let readiness = ResourceStorage::readiness_check(&backend).await;
    assert!(
        readiness.is_ok(),
        "readiness_check failed on a live mongodb: {:?}",
        readiness.err()
    );
}

#[tokio::test]
async fn mongodb_integration_create_read_update_delete() {
    let Some(backend) = create_backend("crud").await else {
        eprintln!(
            "Skipping mongodb_integration_create_read_update_delete (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Phase2"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());

    let updated = backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Updated"}]
            }),
        )
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Updated");

    backend
        .delete(&tenant, "Patient", updated.id())
        .await
        .unwrap();

    let read_after_delete = backend.read(&tenant, "Patient", updated.id()).await;
    assert!(matches!(
        read_after_delete,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

/// A `PUT` onto a deleted id restores the resource instead of failing.
///
/// FHIR permits a deleted resource to be brought back by a subsequent update
/// (http.html#delete). The restore continues the existing version chain — v1
/// create, v2 delete, v3 restore — rather than resetting to "1", and the
/// resource is readable again afterwards. Mirrors
/// `crud::delete_tests::test_delete_is_soft_delete`, which covers this path on
/// SQLite.
#[tokio::test]
async fn mongodb_integration_create_or_update_restores_deleted() {
    let Some(backend) = create_backend("restore_deleted").await else {
        eprintln!(
            "Skipping mongodb_integration_create_or_update_restores_deleted (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-restore");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Original"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let id = created.id().to_string();
    assert_eq!(created.version_id(), "1");

    backend.delete(&tenant, "Patient", &id).await.unwrap();

    let (restored, _created_new) = backend
        .create_or_update(
            &tenant,
            "Patient",
            &id,
            json!({"resourceType": "Patient", "name": [{"family": "Restored"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(restored.content()["name"][0]["family"], "Restored");
    assert_eq!(
        restored.version_id(),
        "3",
        "restore should continue the version chain (v1 create, v2 delete, v3 restore)"
    );
    assert!(!restored.is_deleted());

    let read = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .expect("restored resource must be readable");
    assert_eq!(read.version_id(), "3");
    assert_eq!(read.content()["name"][0]["family"], "Restored");
    assert!(backend.exists(&tenant, "Patient", &id).await.unwrap());

    // History keeps every version, including the deletion — which is only
    // returned when `include_deleted` is set (deleted versions are filtered out
    // by default on every backend).
    let history = backend
        .history_instance(
            &tenant,
            "Patient",
            &id,
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert_eq!(
        history.items.len(),
        3,
        "history should hold create, delete and restore"
    );
    assert_eq!(history.items[0].resource.version_id(), "3");
    assert!(!history.items[0].resource.is_deleted());
    assert!(
        history.items[1].resource.is_deleted(),
        "the middle version is the deletion"
    );
}

/// Restoring a deleted resource requires update permission.
#[tokio::test]
async fn mongodb_integration_restore_deleted_requires_permission() {
    let Some(backend) = create_backend("restore_deleted_permission").await else {
        eprintln!(
            "Skipping mongodb_integration_restore_deleted_requires_permission (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-restore-perm");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let id = created.id().to_string();
    backend.delete(&tenant, "Patient", &id).await.unwrap();

    let read_only = TenantContext::new(tenant.tenant_id().clone(), TenantPermissions::read_only());
    let result = backend
        .create_or_update(
            &read_only,
            "Patient",
            &id,
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await;
    assert!(
        matches!(result, Err(StorageError::Tenant(_))),
        "restore without update permission must be refused"
    );
}

/// `:exact` is case-sensitive; the default and `:contains` string matches are
/// not. MongoDB stores `value_string` as written and asks the server for a
/// case-insensitive regex for the insensitive variants.
#[tokio::test]
async fn mongodb_integration_string_exact_is_case_sensitive() {
    let Some(backend) = create_backend_with_full_registry("string_exact_case").await else {
        eprintln!(
            "Skipping mongodb_integration_string_exact_is_case_sensitive (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-exact");
    for family in ["Smith", "SMITH", "Smithson"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": family}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let exact = |value: &str| {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Exact),
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        })
    };

    // Exactly the one resource spelled "Smith".
    let result = backend.search(&tenant, &exact("Smith")).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        ":exact=Smith must match only the 'Smith' spelling"
    );
    assert_eq!(
        result.resources.items[0].content()["name"][0]["family"],
        "Smith"
    );

    // A different casing must not match.
    let result = backend.search(&tenant, &exact("smith")).await.unwrap();
    assert!(
        result.resources.items.is_empty(),
        ":exact is case-sensitive, so 'smith' must not match 'Smith'"
    );

    // `:contains` is a case-insensitive substring match: "MITH" finds every
    // spelling regardless of case.
    let contains = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Contains),
        values: vec![SearchValue::eq("MITH")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &contains).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        3,
        ":contains is case-insensitive"
    );

    // The default match stays case-insensitive (prefix), so it still finds all
    // three spellings.
    let insensitive = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("smith")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &insensitive).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        3,
        "default string search is case-insensitive"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reuses_client_pool_under_concurrent_read_search() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let config = MongoBackendConfig {
        connection_string: connection_string.clone(),
        database_name: build_test_database_name("client_pool_reuse"),
        max_connections: 8,
        ..Default::default()
    };
    let Some(backend) = build_backend(config).await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (shared mongo unreachable)"
        );
        return;
    };

    let tenant = create_tenant("tenant-client-pool");
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-client-pool",
                "identifier": [{
                    "system": "http://hospital.org/mrn",
                    "value": "MRN-CLIENT-POOL"
                }],
                "name": [{ "family": "Pool" }],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let Some(before) = mongodb_total_created_connections(&connection_string).await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (serverStatus unavailable)"
        );
        return;
    };

    let backend = Arc::new(backend);
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let backend = backend.clone();
        let tenant = tenant.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..20 {
                backend
                    .read(&tenant, "Patient", "patient-client-pool")
                    .await
                    .unwrap();

                let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                    name: "identifier".to_string(),
                    param_type: SearchParamType::Token,
                    modifier: None,
                    values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-CLIENT-POOL")],
                    chain: vec![],
                    components: vec![],
                });
                backend.search(&tenant, &query).await.unwrap();
            }
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    let Some(after) = mongodb_total_created_connections(&connection_string).await else {
        eprintln!(
            "Skipping mongodb_integration_reuses_client_pool_under_concurrent_read_search (serverStatus unavailable)"
        );
        return;
    };

    let created_during_test = after - before;
    // `totalCreated` is a server-global counter on the SHARED mongo, so
    // concurrently running neighbor tests (each with its own client pool)
    // inflate it — observed spilling past 50 on wide runners. The regression
    // this guards against (a fresh client per operation) creates at least one
    // connection per iteration: 8 tasks × 20 ops ≥ 160. A 120 ceiling keeps
    // that detectable while tolerating neighbor noise.
    assert!(
        created_during_test <= 120,
        "MongoDB backend should reuse one client pool; created {} connections during concurrent read/search",
        created_during_test
    );
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_create_and_resolve_references() {
    let Some(backend) = create_backend("bundle_create_resolve_references").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_create_and_resolve_references (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-resolve");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "BundleRefPatient"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-patient".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "subject": {"reference": "urn:uuid:new-patient"}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-observation".to_string()),
        },
    ];

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        entries,
        "mongodb_integration_transaction_bundle_create_and_resolve_references",
    )
    .await
    else {
        return;
    };

    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.entries[0].status, 201);
    assert_eq!(result.entries[1].status, 201);

    let patient_location = result.entries[0]
        .location
        .as_deref()
        .expect("patient location should be present");
    let expected_patient_reference = patient_location
        .split("/_history")
        .next()
        .unwrap_or(patient_location)
        .to_string();

    let observation_location = result.entries[1]
        .location
        .as_deref()
        .expect("observation location should be present");
    let observation_id = extract_resource_id_from_location(observation_location);

    let observation = backend
        .read(&tenant, "Observation", &observation_id)
        .await
        .unwrap()
        .unwrap();

    let resolved_reference = observation.content()["subject"]["reference"]
        .as_str()
        .expect("resolved subject reference should be present");

    assert_eq!(resolved_reference, expected_patient_reference);
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete() {
    let Some(backend) = create_backend("bundle_mixed_operations").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-mixed");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "update-me",
                "name": [{"family": "BeforeUpdate"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "delete-me",
                "name": [{"family": "BeforeDelete"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Delete,
            url: "Patient/delete-me".to_string(),
            resource: None,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "new-from-transaction",
                "name": [{"family": "CreatedInTransaction"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-created".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Put,
            url: "Patient/update-me".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "update-me",
                "name": [{"family": "AfterUpdate"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
    ];

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        entries,
        "mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete",
    )
    .await
    else {
        return;
    };

    assert_eq!(result.entries.len(), 3);
    assert_eq!(result.entries[0].status, 204);
    assert_eq!(result.entries[1].status, 201);
    assert_eq!(result.entries[2].status, 200);

    let updated = backend
        .read(&tenant, "Patient", "update-me")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.content()["name"][0]["family"], "AfterUpdate");

    let deleted = backend.read(&tenant, "Patient", "delete-me").await;
    assert!(matches!(
        deleted,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));

    let created = backend
        .read(&tenant, "Patient", "new-from-transaction")
        .await
        .unwrap();
    assert!(created.is_some());

    let idempotent_delete = vec![BundleEntry {
        method: BundleMethod::Delete,
        url: "Patient/non-existent-delete".to_string(),
        resource: None,
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let Some(idempotent_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        idempotent_delete,
        "mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete/idempotent",
    )
    .await
    else {
        return;
    };

    assert_eq!(idempotent_result.entries.len(), 1);
    assert_eq!(idempotent_result.entries[0].status, 204);
}

#[tokio::test]
async fn mongodb_integration_transaction_if_none_exist_match_resolves_urn_references() {
    let Some(backend) = create_backend("if_none_exist_urn").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_if_none_exist_match_resolves_urn_references (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-none-exist-urn");

    let existing = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-URN-1"}],
                "name": [{"family": "AlreadyThere"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-URN-1"}],
                "name": [{"family": "Duplicate"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some("identifier=http://example.org/mrn|MRN-URN-1".to_string()),
            full_url: Some("urn:uuid:patient".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"text": "test"},
                "subject": {"reference": "urn:uuid:patient"}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:observation".to_string()),
        },
    ];

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        entries,
        "mongodb_integration_transaction_if_none_exist_match_resolves_urn_references",
    )
    .await
    else {
        return;
    };

    assert_eq!(
        result.entries[0].status, 200,
        "the match is answered, not duplicated"
    );
    assert_eq!(result.entries[1].status, 201);

    let observation = result.entries[1]
        .resource
        .as_ref()
        .expect("created observation is echoed");
    assert_eq!(
        observation["subject"]["reference"],
        json!(format!("Patient/{}", existing.id())),
        "a urn:uuid reference to a matched ifNoneExist entry must resolve to the match"
    );
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_conditional_headers() {
    let Some(backend) = create_backend("bundle_conditional_headers").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_conditional_headers (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-conditional");

    let conditional_create = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN-TX-COND-1"}],
            "name": [{"family": "ConditionalCreate"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org/mrn|MRN-TX-COND-1".to_string()),
        full_url: Some("urn:uuid:conditional-create".to_string()),
    }];

    let Some(first_create) = process_transaction_or_skip(
        &backend,
        &tenant,
        conditional_create.clone(),
        "mongodb_integration_transaction_bundle_conditional_headers/create-first",
    )
    .await
    else {
        return;
    };
    assert_eq!(first_create.entries[0].status, 201);

    let Some(second_create) = process_transaction_or_skip(
        &backend,
        &tenant,
        conditional_create,
        "mongodb_integration_transaction_bundle_conditional_headers/create-second",
    )
    .await
    else {
        return;
    };
    assert_eq!(second_create.entries[0].status, 200);
    // A matched `ifNoneExist` names the match in `location`, exactly as a
    // fresh create names the row it wrote; that is what the transaction's
    // fullUrl → id map is built from (#511).
    assert_eq!(
        second_create.entries[0].location, first_create.entries[0].location,
        "the 200 entry must point at the resource the 201 entry created"
    );

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "if-match-target",
                "name": [{"family": "BeforeIfMatch"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let good_if_match = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/if-match-target".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "if-match-target",
            "name": [{"family": "AfterIfMatch"}]
        })),
        if_match: Some("W/\"1\"".to_string()),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let Some(good_if_match_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        good_if_match,
        "mongodb_integration_transaction_bundle_conditional_headers/if-match-good",
    )
    .await
    else {
        return;
    };
    assert_eq!(good_if_match_result.entries[0].status, 200);

    let bad_if_match = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/if-match-target".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "if-match-target",
            "name": [{"family": "ShouldNotPersist"}]
        })),
        if_match: Some("W/\"999\"".to_string()),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    match backend
        .process_transaction(&tenant, bad_if_match, FhirVersion::default())
        .await
    {
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_conditional_headers/if-match-bad (MongoDB topology does not support transactions)"
            );
            return;
        }
        Err(TransactionError::BundleError { .. }) => {}
        Err(other) => panic!("Unexpected transaction error: {}", other),
        Ok(_) => panic!("Expected if-match failure transaction to return BundleError"),
    }

    let read_after_bad_if_match = backend
        .read(&tenant, "Patient", "if-match-target")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read_after_bad_if_match.content()["name"][0]["family"],
        "AfterIfMatch"
    );
}

#[tokio::test]
async fn mongodb_integration_transaction_bundle_rolls_back_on_failure() {
    let Some(backend) = create_backend("bundle_rollback_failure").await else {
        eprintln!(
            "Skipping mongodb_integration_transaction_bundle_rolls_back_on_failure (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-bundle-rollback");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "already-exists",
                "name": [{"family": "PreExisting"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "should-rollback",
                "name": [{"family": "ShouldRollback"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:rollback-created".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "already-exists",
                "name": [{"family": "Duplicate"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:rollback-fail".to_string()),
        },
    ];

    match backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
    {
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!(
                "Skipping mongodb_integration_transaction_bundle_rolls_back_on_failure (MongoDB topology does not support transactions)"
            );
            return;
        }
        Err(TransactionError::BundleError { .. }) => {}
        Err(other) => panic!("Unexpected transaction error: {}", other),
        Ok(_) => panic!("Expected rollback scenario to fail transaction"),
    }

    let rolled_back = backend
        .read(&tenant, "Patient", "should-rollback")
        .await
        .unwrap();
    assert!(rolled_back.is_none());
}

#[tokio::test]
async fn mongodb_integration_tenant_isolation() {
    let Some(backend) = create_backend("tenant").await else {
        eprintln!(
            "Skipping mongodb_integration_tenant_isolation (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let created = backend
        .create(
            &tenant_a,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "shared-id",
                "name": [{"family": "TenantA"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let read_a = backend
        .read(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_a.is_some());

    let read_b = backend
        .read(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_b.is_none());

    let exists_a = backend
        .exists(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    let exists_b = backend
        .exists(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(exists_a);
    assert!(!exists_b);
}

#[tokio::test]
async fn mongodb_integration_count_and_batch() {
    let Some(backend) = create_backend("count_batch").await else {
        eprintln!(
            "Skipping mongodb_integration_count_and_batch (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-count");

    let mut ids = Vec::new();
    for idx in 0..3 {
        let created = backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": format!("obs-{}", idx),
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        ids.push(created.id().to_string());
    }

    let count = backend.count(&tenant, Some("Observation")).await.unwrap();
    assert_eq!(count, 3);

    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let batch = backend
        .read_batch(&tenant, "Observation", &id_refs)
        .await
        .unwrap();
    assert_eq!(batch.len(), 3);
}

// ============================================================================
// Console Dashboard count_* tests
// ============================================================================

#[tokio::test]
async fn mongodb_integration_count_by_types() {
    let Some(backend) = create_backend("console_count_by_types").await else {
        eprintln!("Skipping mongodb_integration_count_by_types (set HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-console-count-by-types");

    // Seed a small deterministic dataset: 2 Patients, 1 Observation.
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Observation", json!({}), FhirVersion::default())
        .await
        .unwrap();

    let counts = backend
        .count_by_types(&tenant, &["Patient", "Observation", "Encounter"])
        .await
        .unwrap();
    let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
    assert_eq!(map.get("Patient"), Some(&2));
    assert_eq!(map.get("Observation"), Some(&1));
    // A type with zero rows is ABSENT from the result, not a 0 row.
    assert!(!map.contains_key("Encounter"));
}

#[tokio::test]
async fn mongodb_integration_count_all_types() {
    let Some(backend) = create_backend("console_count_all_types").await else {
        eprintln!("Skipping mongodb_integration_count_all_types (set HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-console-count-all-types");

    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Observation", json!({}), FhirVersion::default())
        .await
        .unwrap();

    let counts = backend.count_all_types(&tenant).await.unwrap();
    let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
    assert_eq!(map.get("Patient"), Some(&2));
    assert_eq!(map.get("Observation"), Some(&1));
}

#[tokio::test]
async fn mongodb_integration_count_by_day() {
    let Some(backend) = create_backend("console_count_by_day").await else {
        eprintln!("Skipping mongodb_integration_count_by_day (set HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-console-count-by-day");

    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();

    // `since` = start of today (UTC midnight), built the same way the handler
    // does; `today` is derived from the same clock so this stays date-robust.
    let since = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let today = chrono::Utc::now().date_naive();

    let rows = backend
        .count_by_day(&tenant, "Patient", since)
        .await
        .unwrap();
    let today_row = rows
        .iter()
        .find(|r| r.day == today)
        .expect("today bucket should be present");
    assert_eq!(today_row.count, 2);
}

#[tokio::test]
async fn mongodb_integration_count_deltas_by_bucket() {
    let Some(backend) = create_backend("console_count_deltas").await else {
        eprintln!("Skipping mongodb_integration_count_deltas_by_bucket (set HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-console-count-deltas");

    let first = backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    // An update writes a v2 history row, which must contribute no delta — the
    // aggregation pipeline's `$switch` has to agree with the SQL backends' CASE.
    backend
        .update(&tenant, &first, json!({"active": true}))
        .await
        .unwrap();

    let since = chrono::Utc::now() - chrono::Duration::minutes(5);
    let rows = backend
        .count_deltas_by_bucket(&tenant, "Patient", since, 60)
        .await
        .unwrap();

    assert_eq!(
        rows.iter().map(|r| r.delta).sum::<i64>(),
        2,
        "two creates and one update net to +2"
    );
    assert!(
        rows.iter().all(|r| r.bucket_start.timestamp() % 60 == 0),
        "buckets are epoch-aligned to their width"
    );

    backend
        .delete(&tenant, "Patient", first.id())
        .await
        .unwrap();
    let rows = backend
        .count_deltas_by_bucket(&tenant, "Patient", since, 60)
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|r| r.delta).sum::<i64>(), 1);
}

#[tokio::test]
async fn mongodb_integration_activity_histogram() {
    let Some(backend) = create_backend("console_activity_histogram").await else {
        eprintln!("Skipping mongodb_integration_activity_histogram (set HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-console-activity-histogram");

    // 3 writes for this tenant -> 3 resource_history rows.
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Observation", json!({}), FhirVersion::default())
        .await
        .unwrap();

    let since = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();

    let cells = backend.activity_histogram(&tenant, since).await.unwrap();
    assert!(!cells.is_empty());
    // Total across returned cells equals the number of writes seeded.
    let total: u64 = cells.iter().map(|c| c.count).sum();
    assert_eq!(total, 3);
}

#[tokio::test]
async fn mongodb_integration_count_by_tenant() {
    let Some(backend) = create_backend("console_count_by_tenant").await else {
        eprintln!("Skipping mongodb_integration_count_by_tenant (set HFS_TEST_MONGODB_URL)");
        return;
    };
    // Each test runs against a freshly-named database, so fixed tenant IDs are
    // isolated to this test's cross-tenant aggregate.
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // tenant-a: 3 resources, tenant-b: 2 resources.
    backend
        .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant_a, "Observation", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant_b, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant_b, "Observation", json!({}), FhirVersion::default())
        .await
        .unwrap();

    // Cross-tenant admin aggregate: takes NO TenantContext.
    let counts = backend.count_by_tenant().await.unwrap();
    let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
    assert_eq!(map.get("tenant-a"), Some(&3));
    assert_eq!(map.get("tenant-b"), Some(&2));
}

#[tokio::test]
async fn mongodb_integration_tenant_registry_crud() {
    let Some(backend) = create_backend("tenant_registry_crud").await else {
        eprintln!("Skipping mongodb_integration_tenant_registry_crud (set HFS_TEST_MONGODB_URL)");
        return;
    };

    assert!(backend.supports_tenant_registry());

    // Register in id order so the (created_at, id) sort is deterministic even
    // when both inserts land in the same second.
    let alpha = backend
        .register_tenant("tenant-alpha", Some("Acme Corp"))
        .await
        .unwrap();
    assert_eq!(alpha.id, "tenant-alpha");
    assert_eq!(alpha.display_name.as_deref(), Some("Acme Corp"));
    assert!(!alpha.created_at.is_empty());

    let beta = backend.register_tenant("tenant-beta", None).await.unwrap();
    assert_eq!(beta.id, "tenant-beta");
    assert_eq!(beta.display_name, None);
    assert!(!beta.created_at.is_empty());

    // get_tenant round-trips the registered records.
    let fetched_alpha = backend.get_tenant("tenant-alpha").await.unwrap().unwrap();
    assert_eq!(fetched_alpha, alpha);
    let fetched_beta = backend.get_tenant("tenant-beta").await.unwrap().unwrap();
    assert_eq!(fetched_beta, beta);
    assert_eq!(backend.get_tenant("tenant-missing").await.unwrap(), None);

    // Each test gets a fresh database, so the listing is exhaustive.
    let listed = backend.list_tenants().await.unwrap();
    assert_eq!(listed, vec![alpha.clone(), beta.clone()]);

    // Duplicate id hits the unique index on `id`.
    let duplicate = backend.register_tenant("tenant-alpha", None).await;
    assert!(duplicate.is_err());

    // Deregister removes the row once; repeating reports nothing deleted.
    assert!(backend.deregister_tenant("tenant-alpha").await.unwrap());
    assert!(!backend.deregister_tenant("tenant-alpha").await.unwrap());
    assert_eq!(backend.get_tenant("tenant-alpha").await.unwrap(), None);

    let remaining = backend.list_tenants().await.unwrap();
    assert_eq!(remaining, vec![beta]);
}

#[tokio::test]
async fn mongodb_integration_purge_tenant_data() {
    let Some(backend) = create_backend("purge_tenant_data").await else {
        eprintln!("Skipping mongodb_integration_purge_tenant_data (set HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant_a = create_tenant("tenant-purge-a");
    let tenant_b = create_tenant("tenant-purge-b");

    let a1 = backend
        .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    let a2 = backend
        .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();
    let b1 = backend
        .create(&tenant_b, "Patient", json!({}), FhirVersion::default())
        .await
        .unwrap();

    let removed = backend.purge_tenant_data("tenant-purge-a").await.unwrap();
    assert_eq!(removed, 2);

    // Tenant A's resources are hard-deleted (not merely soft-deleted).
    assert!(
        backend
            .read(&tenant_a, "Patient", a1.id())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .read(&tenant_a, "Patient", a2.id())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 0);

    // Tenant B is untouched.
    assert!(
        backend
            .read(&tenant_b, "Patient", b1.id())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 1);
}

#[tokio::test]
async fn mongodb_integration_is_cluster_shared() {
    let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
    assert!(backend.is_cluster_shared());
}

#[tokio::test]
async fn mongodb_integration_create_or_update() {
    let Some(backend) = create_backend("create_or_update").await else {
        eprintln!(
            "Skipping mongodb_integration_create_or_update (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-cou");

    let (created, was_created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-1",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "First"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(was_created);
    assert_eq!(created.version_id(), "1");

    let (updated, was_created_again) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-1",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Second"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(!was_created_again);
    assert_eq!(updated.version_id(), "2");
}

#[tokio::test]
async fn mongodb_integration_versioned_storage_vread_and_list_versions() {
    let Some(backend) = create_backend("versioned_vread").await else {
        eprintln!(
            "Skipping mongodb_integration_versioned_storage_vread_and_list_versions (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-versioned");

    let v1 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-v",
                "name": [{"family": "Version1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let v2 = backend
        .update(
            &tenant,
            &v1,
            json!({
                "resourceType": "Patient",
                "id": "patient-v",
                "name": [{"family": "Version2"}]
            }),
        )
        .await
        .unwrap();

    backend.delete(&tenant, "Patient", v2.id()).await.unwrap();

    let read_v1 = backend
        .vread(&tenant, "Patient", v1.id(), "1")
        .await
        .unwrap()
        .unwrap();
    let read_v2 = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap()
        .unwrap();
    let read_v3 = backend
        .vread(&tenant, "Patient", v1.id(), "3")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read_v1.version_id(), "1");
    assert_eq!(read_v1.content()["name"][0]["family"], "Version1");
    assert_eq!(read_v2.version_id(), "2");
    assert_eq!(read_v2.content()["name"][0]["family"], "Version2");
    assert_eq!(read_v3.version_id(), "3");
    assert!(read_v3.deleted_at().is_some());

    let versions = backend
        .list_versions(&tenant, "Patient", v1.id())
        .await
        .unwrap();
    assert_eq!(versions, vec!["1", "2", "3"]);
}

#[tokio::test]
async fn mongodb_integration_update_with_match_and_delete_with_match() {
    let Some(backend) = create_backend("if_match").await else {
        eprintln!(
            "Skipping mongodb_integration_update_with_match_and_delete_with_match (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-match");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-if-match",
                "name": [{"family": "Original"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            "W/\"1\"",
            json!({
                "resourceType": "Patient",
                "id": created.id(),
                "name": [{"family": "Updated"}]
            }),
        )
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Updated");

    let stale_update = backend
        .update_with_match(
            &tenant,
            "Patient",
            created.id(),
            "1",
            json!({
                "resourceType": "Patient",
                "id": created.id(),
                "name": [{"family": "ShouldFail"}]
            }),
        )
        .await;

    assert!(matches!(
        stale_update,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));

    let stale_delete = backend
        .delete_with_match(&tenant, "Patient", created.id(), "1")
        .await;
    assert!(matches!(
        stale_delete,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));

    backend
        .delete_with_match(&tenant, "Patient", created.id(), "2")
        .await
        .unwrap();
}

#[tokio::test]
async fn mongodb_integration_history_providers() {
    let Some(backend) = create_backend("history_providers").await else {
        eprintln!(
            "Skipping mongodb_integration_history_providers (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-history");

    let patient_v1 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-history",
                "name": [{"family": "One"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let patient_v2 = backend
        .update(
            &tenant,
            &patient_v1,
            json!({
                "resourceType": "Patient",
                "id": "patient-history",
                "name": [{"family": "Two"}]
            }),
        )
        .await
        .unwrap();

    backend
        .delete(&tenant, "Patient", patient_v2.id())
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-history",
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let params = HistoryParams::new().count(20).include_deleted(true);

    let instance_history = backend
        .history_instance(&tenant, "Patient", patient_v1.id(), &params)
        .await
        .unwrap();
    assert_eq!(instance_history.items.len(), 3);
    assert_eq!(instance_history.items[0].resource.version_id(), "3");

    let type_history = backend
        .history_type(&tenant, "Patient", &params)
        .await
        .unwrap();
    assert!(type_history.items.len() >= 3);

    let system_history = backend.history_system(&tenant, &params).await.unwrap();
    assert!(system_history.items.len() >= 4);

    let instance_count = backend
        .history_instance_count(&tenant, "Patient", patient_v1.id())
        .await
        .unwrap();
    assert_eq!(instance_count, 3);

    let type_count = backend
        .history_type_count(&tenant, "Patient")
        .await
        .unwrap();
    assert_eq!(type_count, 3);

    let system_count = backend.history_system_count(&tenant).await.unwrap();
    assert!(system_count >= 4);
}

// ---------------------------------------------------------------------------
// #1053: history_type / history_system must page via a server-side sort +
// limit + cursor predicate rather than draining the whole history corpus into
// memory before sorting/paging in Rust. See the inline comments in
// history_type / history_system in mongodb/storage.rs.
// ---------------------------------------------------------------------------

use mongodb::Collection;

/// Seeds `obs_count` Observations with 3 versions each (create + 2 updates)
/// and returns the (id, version_id) history keys in creation order:
/// `hist-obs-0` v1, v2, v3, `hist-obs-1` v1, v2, v3, ...
async fn seed_type_history_corpus(
    backend: &MongoBackend,
    tenant: &TenantContext,
    obs_count: usize,
) -> Vec<(String, String)> {
    let mut keys = Vec::new();
    for i in 0..obs_count {
        let id = format!("hist-obs-{i}");
        let v1 = backend
            .create(
                tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": id, "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        keys.push((id.clone(), "1".to_string()));

        let v2 = backend
            .update(
                tenant,
                &v1,
                json!({"resourceType": "Observation", "id": id, "status": "amended"}),
            )
            .await
            .unwrap();
        keys.push((id.clone(), "2".to_string()));

        backend
            .update(
                tenant,
                &v2,
                json!({"resourceType": "Observation", "id": id, "status": "final"}),
            )
            .await
            .unwrap();
        keys.push((id.clone(), "3".to_string()));
    }
    keys
}

/// Overwrites `last_updated` on one `resource_history` row directly, so tests
/// can pin a deterministic, wall-clock-independent ordering instead of
/// trusting back-to-back millisecond-resolution writes not to tie.
async fn stamp_history_last_updated(
    history: &Collection<Document>,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
    version_id: &str,
    ts: mongodb::bson::DateTime,
) {
    let result = history
        .update_one(
            doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "resource_type": resource_type,
                "id": id,
                "version_id": version_id,
            },
            doc! { "$set": { "last_updated": ts } },
        )
        .await
        .expect("failed to stamp resource_history.last_updated");
    assert_eq!(
        result.matched_count, 1,
        "expected exactly one history row for ({resource_type}, {id}, v{version_id})"
    );
}

fn history_ts(base_millis: i64, offset_secs: i64) -> mongodb::bson::DateTime {
    mongodb::bson::DateTime::from_millis(base_millis + offset_secs * 1000)
}

/// Pin for #1053: `history_type` must not read rows outside the requested
/// page. A `data`-less sentinel row sits at the *oldest* timestamp (well
/// outside the first two pages); today's unbounded `find` drains it into
/// `parse_history_row`, which errors on the missing payload even though the
/// sentinel is nowhere near the page being requested. After the fix the
/// server-side `sort + limit` means the sentinel is never fetched for a
/// small page, and assertion (3) below proves the sentinel is still live
/// (and still errors) once the requested window actually reaches it — so a
/// future tolerant parse that swallowed missing-payload rows could not make
/// this test pass vacuously.
#[tokio::test]
async fn mongodb_history_type_does_not_read_rows_outside_the_page() {
    let Some(backend) = create_backend("history_page_bounds").await else {
        eprintln!(
            "Skipping mongodb_history_type_does_not_read_rows_outside_the_page (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-history-page-bounds");

    // 10 Observations x 3 versions = 30 good history rows.
    seed_type_history_corpus(&backend, &tenant, 10).await;

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect raw MongoDB client");
    let database = client.database(&backend.config().database_name);
    let history: Collection<Document> = database.collection("resource_history");

    // Deterministic timestamps: row k (k = i*3 + (v-1)) gets base + k seconds,
    // so descending last_updated order is exactly descending k, with no ties.
    let base_millis = chrono::Utc::now().timestamp_millis() - 1_000_000;
    for i in 0..10usize {
        for v in 1..=3usize {
            let k = (i * 3 + (v - 1)) as i64;
            stamp_history_last_updated(
                &history,
                &tenant,
                "Observation",
                &format!("hist-obs-{i}"),
                &v.to_string(),
                history_ts(base_millis, k),
            )
            .await;
        }
    }

    // Sentinel: oldest timestamp of all, no `data` field, unique id so the
    // (tenant_id, resource_type, id, version_id) unique index is satisfied.
    history
        .insert_one(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "id": "sentinel-1053",
            "version_id": "1",
            "last_updated": history_ts(base_millis, -1000),
            "is_deleted": false,
            "fhir_version": "R4",
        })
        .await
        .expect("failed to insert sentinel history row");

    // Expected global order (newest first) is descending k: 29, 28, ..., 0.
    let expected: Vec<(String, String)> = (0..30i64)
        .rev()
        .map(|k| {
            let i = k / 3;
            let v = (k % 3) + 1;
            (format!("hist-obs-{i}"), v.to_string())
        })
        .collect();

    // --- Assertion (1): THE PIN ---
    let params = HistoryParams::new().count(10).include_deleted(true);
    let page1 = backend
        .history_type(&tenant, "Observation", &params)
        .await
        .expect(
            "history_type must not read past the requested page \
             (a data-less sentinel far outside the page must not surface)",
        );
    assert_eq!(page1.items.len(), 10);
    let page1_keys: Vec<(String, String)> = page1
        .items
        .iter()
        .map(|e| {
            (
                e.resource.id().to_string(),
                e.resource.version_id().to_string(),
            )
        })
        .collect();
    assert_eq!(page1_keys, expected[0..10]);
    assert!(page1.page_info.has_next);
    let next_cursor = page1
        .page_info
        .next_cursor
        .clone()
        .expect("expected a next_cursor for page 1");

    // --- Assertion (2): CURSOR CONSISTENCY ---
    let mut params2 = HistoryParams::new().count(10).include_deleted(true);
    params2.pagination = helios_persistence::types::Pagination::with_cursor(10, next_cursor);
    let page2 = backend
        .history_type(&tenant, "Observation", &params2)
        .await
        .expect("page 2 must succeed (its fetch window does not reach the sentinel)");
    assert_eq!(page2.items.len(), 10);
    let page2_keys: Vec<(String, String)> = page2
        .items
        .iter()
        .map(|e| {
            (
                e.resource.id().to_string(),
                e.resource.version_id().to_string(),
            )
        })
        .collect();
    assert_eq!(page2_keys, expected[10..20]);
    // Disjoint from page 1, contiguous with it.
    for key in &page2_keys {
        assert!(
            !page1_keys.contains(key),
            "page 2 repeated a page 1 row: {key:?}"
        );
    }

    // --- Assertion (3): CANARY PREMISE ---
    // A window wide enough to actually reach the sentinel must still error,
    // proving the sentinel is live and matches the filter (i.e. it is only
    // absent from assertion (1) because the page stopped short of it).
    let wide_params = HistoryParams::new().count(100).include_deleted(true);
    let err = backend
        .history_type(&tenant, "Observation", &wide_params)
        .await
        .expect_err("a window reaching the sentinel must surface its missing payload");
    let message = format!("{err}");
    assert!(
        message.contains("Missing history payload"),
        "expected a missing-payload error, got: {message}"
    );
}

/// Companion to the pin test: no sentinel, walks every page via `next_cursor`
/// and checks the full corpus comes back in the right order with no
/// duplicates or omissions. Includes one deliberate (last_updated, id) tie
/// pair placed strictly inside page 1, to pin that the version_id tie-break
/// (last_updated desc, id desc, version_id desc) survives in Rust once the
/// sort/limit/cursor predicate move server-side.
#[tokio::test]
async fn mongodb_history_type_paging_is_ordered_and_complete() {
    let Some(backend) = create_backend("history_paging_complete").await else {
        eprintln!(
            "Skipping mongodb_history_type_paging_is_ordered_and_complete (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-history-paging-complete");

    // 8 Observations x 3 versions = 24 rows, plus 2 extra versions on
    // "hist-obs-0" (v4, v5) sharing v3's timestamp to create a tie group that
    // straddles nothing but sits inside page 1 (count=5).
    seed_type_history_corpus(&backend, &tenant, 8).await;
    let obs0_v3 = backend
        .read(&tenant, "Observation", "hist-obs-0")
        .await
        .unwrap()
        .unwrap();
    let obs0_v4 = backend
        .update(
            &tenant,
            &obs0_v3,
            json!({"resourceType": "Observation", "id": "hist-obs-0", "status": "final"}),
        )
        .await
        .unwrap();
    backend
        .update(
            &tenant,
            &obs0_v4,
            json!({"resourceType": "Observation", "id": "hist-obs-0", "status": "final"}),
        )
        .await
        .unwrap();

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect raw MongoDB client");
    let database = client.database(&backend.config().database_name);
    let history: Collection<Document> = database.collection("resource_history");

    let base_millis = chrono::Utc::now().timestamp_millis() - 1_000_000;
    // 26 total rows: assign distinct k = 0..25 to every (id, version) except
    // that hist-obs-0's v4 and v5 both get k = 25 (the newest slot), forming
    // the deliberate tie: version_id 5 must sort before version_id 4.
    let mut k = 0i64;
    let mut expected_by_k: Vec<(i64, String, String)> = Vec::new();
    for i in 0..8usize {
        for v in 1..=3usize {
            expected_by_k.push((k, format!("hist-obs-{i}"), v.to_string()));
            stamp_history_last_updated(
                &history,
                &tenant,
                "Observation",
                &format!("hist-obs-{i}"),
                &v.to_string(),
                history_ts(base_millis, k),
            )
            .await;
            k += 1;
        }
    }
    // Tie pair: v4 and v5 of hist-obs-0 share the *next* timestamp (k = 25).
    let tie_k = k;
    for v in [4usize, 5usize] {
        stamp_history_last_updated(
            &history,
            &tenant,
            "Observation",
            "hist-obs-0",
            &v.to_string(),
            history_ts(base_millis, tie_k),
        )
        .await;
    }
    expected_by_k.push((tie_k, "hist-obs-0".to_string(), "5".to_string()));
    expected_by_k.push((tie_k, "hist-obs-0".to_string(), "4".to_string()));

    // Expected global order: descending k; within a k tie, descending
    // version_id (the Rust-side tie-break).
    let mut expected = expected_by_k;
    expected.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            b.2.parse::<i64>()
                .unwrap()
                .cmp(&a.2.parse::<i64>().unwrap())
        })
    });
    let expected: Vec<(String, String)> = expected.into_iter().map(|(_, id, v)| (id, v)).collect();
    assert_eq!(expected.len(), 26);
    // The tie pair (v5, v4) must be strictly inside page 1 (count=5): it's
    // rank 1-2 of 26, well inside the first 5.
    assert_eq!(expected[0], ("hist-obs-0".to_string(), "5".to_string()));
    assert_eq!(expected[1], ("hist-obs-0".to_string(), "4".to_string()));

    let mut all_rows: Vec<(String, String)> = Vec::new();
    let mut params = HistoryParams::new().count(5).include_deleted(true);
    loop {
        let page = backend
            .history_type(&tenant, "Observation", &params)
            .await
            .expect("paging through the full corpus must not error");
        let keys: Vec<(String, String)> = page
            .items
            .iter()
            .map(|e| {
                (
                    e.resource.id().to_string(),
                    e.resource.version_id().to_string(),
                )
            })
            .collect();
        all_rows.extend(keys);

        if page.page_info.has_next {
            let cursor = page
                .page_info
                .next_cursor
                .expect("has_next implies a cursor");
            params = HistoryParams::new().count(5).include_deleted(true);
            params.pagination = helios_persistence::types::Pagination::with_cursor(5, cursor);
        } else {
            break;
        }
    }

    assert_eq!(all_rows.len(), expected.len(), "row count mismatch");
    assert_eq!(all_rows, expected, "row order mismatch");
    let mut dedup_check = all_rows.clone();
    dedup_check.sort();
    dedup_check.dedup();
    assert_eq!(
        dedup_check.len(),
        all_rows.len(),
        "found duplicate rows across pages"
    );
}

/// Mirror of the pin test for `history_system`: sentinel row of a resource
/// type that already appears in the corpus, at the oldest timestamp, no
/// `data` field.
#[tokio::test]
async fn mongodb_history_system_does_not_read_rows_outside_the_page() {
    let Some(backend) = create_backend("history_system_page_bounds").await else {
        eprintln!(
            "Skipping mongodb_history_system_does_not_read_rows_outside_the_page (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-history-system-page-bounds");

    // 5 Patients + 5 Observations, 3 versions each = 30 rows.
    for i in 0..5usize {
        let id = format!("sys-pat-{i}");
        let v1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let v2 = backend
            .update(
                &tenant,
                &v1,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "Y"}]}),
            )
            .await
            .unwrap();
        backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "Z"}]}),
            )
            .await
            .unwrap();
    }
    seed_type_history_corpus(&backend, &tenant, 5).await;

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect raw MongoDB client");
    let database = client.database(&backend.config().database_name);
    let history: Collection<Document> = database.collection("resource_history");

    // Deterministic, distinct timestamps across all 30 rows: Patients get
    // k = 0..14, Observations get k = 15..29 (so ordering is unambiguous by
    // timestamp alone; resource_type/id tie-break is covered by the type
    // history companion test).
    let base_millis = chrono::Utc::now().timestamp_millis() - 1_000_000;
    let mut expected: Vec<(String, String, String)> = Vec::new();
    let mut k = 0i64;
    for i in 0..5usize {
        for v in 1..=3usize {
            let id = format!("sys-pat-{i}");
            stamp_history_last_updated(
                &history,
                &tenant,
                "Patient",
                &id,
                &v.to_string(),
                history_ts(base_millis, k),
            )
            .await;
            expected.push(("Patient".to_string(), id, v.to_string()));
            k += 1;
        }
    }
    for i in 0..5usize {
        for v in 1..=3usize {
            let id = format!("hist-obs-{i}");
            stamp_history_last_updated(
                &history,
                &tenant,
                "Observation",
                &id,
                &v.to_string(),
                history_ts(base_millis, k),
            )
            .await;
            expected.push(("Observation".to_string(), id, v.to_string()));
            k += 1;
        }
    }
    expected.reverse(); // newest (highest k) first

    history
        .insert_one(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Patient",
            "id": "sentinel-1053-system",
            "version_id": "1",
            "last_updated": history_ts(base_millis, -1000),
            "is_deleted": false,
            "fhir_version": "R4",
        })
        .await
        .expect("failed to insert sentinel history row");

    let params = HistoryParams::new().count(10).include_deleted(true);
    let page1 = backend
        .history_system(&tenant, &params)
        .await
        .expect("history_system must not read past the requested page");
    assert_eq!(page1.items.len(), 10);
    let page1_keys: Vec<(String, String, String)> = page1
        .items
        .iter()
        .map(|e| {
            (
                e.resource.resource_type().to_string(),
                e.resource.id().to_string(),
                e.resource.version_id().to_string(),
            )
        })
        .collect();
    assert_eq!(page1_keys, expected[0..10]);
    assert!(page1.page_info.has_next);

    let wide_params = HistoryParams::new().count(100).include_deleted(true);
    let err = backend
        .history_system(&tenant, &wide_params)
        .await
        .expect_err("a window reaching the sentinel must surface its missing payload");
    assert!(
        format!("{err}").contains("Missing history payload"),
        "expected a missing-payload error"
    );
}

/// Scale-free plan guard: the winning plan for the *first* (no-cursor) page
/// must be a bounded index walk — sort and limit pushed to MongoDB, no
/// blocking in-memory SORT stage, and keys/docs examined bounded by
/// `_count`, not by the size of the corpus. Per code review: on a tiny
/// corpus the cursor-page query can legitimately choose a residual-filter
/// plan whose keysExamined scale with the cursor's rank rather than just
/// `_count` (still O(rank + count), still non-blocking) — so this test only
/// asserts the hard bound on the first page, where no cursor predicate is
/// involved and the index alone must satisfy the whole query.
#[tokio::test]
async fn mongodb_history_type_plan_is_a_bounded_index_walk() {
    let Some(backend) = create_backend("history_plan_guard").await else {
        eprintln!(
            "Skipping mongodb_history_type_plan_is_a_bounded_index_walk (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-history-plan-guard");

    seed_type_history_corpus(&backend, &tenant, 10).await; // 30 rows

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect raw MongoDB client");
    let db_name = backend.config().database_name.clone();
    let database = client.database(&db_name);

    if let Err(e) = database.run_command(doc! { "profile": 2_i32 }).await {
        eprintln!(
            "Skipping mongodb_history_type_plan_is_a_bounded_index_walk plan assertions: \
             {{profile: 2}} was refused ({e})"
        );
        return;
    }

    let params = HistoryParams::new().count(10).include_deleted(true);
    backend
        .history_type(&tenant, "Observation", &params)
        .await
        .expect("history_type must succeed");

    let _ = database.run_command(doc! { "profile": 0_i32 }).await;

    let profile: Collection<Document> = database.collection("system.profile");
    let opts = mongodb::options::FindOptions::builder()
        .sort(doc! { "ts": -1_i32 })
        .limit(20)
        .build();
    let mut cursor = profile
        .find(doc! {
            "ns": format!("{db_name}.resource_history"),
            "op": "query",
            "command.find": "resource_history",
        })
        .with_options(opts)
        .await
        .expect("failed to query system.profile");

    // Only the newest matching entry is of interest, so advance once rather
    // than looping (a `while` with an unconditional `break` trips
    // `clippy::never_loop`, which CI denies).
    let entry: Option<Document> = if cursor
        .advance()
        .await
        .expect("failed to advance profile cursor")
    {
        Some(
            cursor
                .deserialize_current()
                .expect("failed to deserialize profile entry"),
        )
    } else {
        None
    };
    let entry = entry.expect("expected a profiled find on resource_history");

    let command = entry
        .get_document("command")
        .expect("profile entry missing command");
    assert_eq!(
        command.get_document("sort").ok(),
        Some(&doc! { "last_updated": -1_i32, "id": -1_i32 })
    );
    let limit = command
        .get_i32("limit")
        .map(i64::from)
        .or_else(|_| command.get_i64("limit"))
        .expect("command missing limit");
    assert_eq!(limit, 11);

    // On MongoDB 5.0 `hasSortStage` is simply absent when there is no sort
    // stage — must not unwrap a missing field as an error.
    let has_sort_stage = entry.get_bool("hasSortStage").unwrap_or(false);
    assert!(
        !has_sort_stage,
        "expected no blocking sort stage on the first page"
    );

    let docs_examined = entry
        .get_i64("docsExamined")
        .or_else(|_| entry.get_i32("docsExamined").map(i64::from));
    let keys_examined = entry
        .get_i64("keysExamined")
        .or_else(|_| entry.get_i32("keysExamined").map(i64::from));
    if let Ok(d) = docs_examined {
        assert!(
            d <= 11,
            "docsExamined {d} exceeds count+1 (11) on the first page"
        );
    }
    if let Ok(k) = keys_examined {
        assert!(
            k <= 11,
            "keysExamined {k} exceeds count+1 (11) on the first page"
        );
    }

    let plan_summary = entry.get_str("planSummary").unwrap_or_default();
    assert!(
        plan_summary.contains("IXSCAN"),
        "expected an IXSCAN plan, got: {plan_summary}"
    );

    // Rebuild the inner find command from exactly what the server recorded
    // (command also carries $db/lsid/$readPreference, which explain rejects).
    let mut inner = Document::new();
    for key in ["find", "filter", "sort", "limit", "projection"] {
        if let Some(v) = command.get(key) {
            inner.insert(key, v.clone());
        }
    }
    let explain = database
        .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
        .await
        .expect("explain of the recorded find command failed");

    let stats = explain
        .get_document("executionStats")
        .expect("explain missing executionStats");
    let total_keys = stats
        .get_i64("totalKeysExamined")
        .or_else(|_| stats.get_i32("totalKeysExamined").map(i64::from))
        .unwrap();
    let total_docs = stats
        .get_i64("totalDocsExamined")
        .or_else(|_| stats.get_i32("totalDocsExamined").map(i64::from))
        .unwrap();
    assert!(
        total_keys <= 11,
        "explain totalKeysExamined {total_keys} exceeds 11"
    );
    assert!(
        total_docs <= 11,
        "explain totalDocsExamined {total_docs} exceeds 11"
    );

    // Only the *winning* plan matters here. `explain` also carries
    // `queryPlanner.rejectedPlans` — other candidates the multi-planner
    // tried and discarded (e.g. via idx_history_identity or
    // idx_history_resource_updated), several of which legitimately contain
    // their own SORT stage. Scanning the whole explain document (including
    // rejected plans) would false-positive on those, so only the winning
    // plan tree is searched for a blocking SORT.
    let winning_plan = explain
        .get_document("queryPlanner")
        .expect("explain missing queryPlanner")
        .get_document("winningPlan")
        .expect("explain missing winningPlan");
    assert!(
        !contains_stage_named(winning_plan, "SORT"),
        "winning plan contains a blocking SORT stage: {winning_plan:?}"
    );
    let mut index_names = Vec::new();
    collect_index_names(winning_plan, &mut index_names);
    assert!(
        index_names.iter().any(|n| n == "idx_history_type_updated"),
        "expected idx_history_type_updated in the winning plan, got: {index_names:?}"
    );
}

/// Recursively searches a BSON document for a nested `"stage"` field equal
/// exactly to `name` (case-sensitive). Used to assert the *absence* of a
/// blocking `SORT` stage without false-positiving on the non-blocking
/// `SORT_MERGE`.
fn contains_stage_named(doc: &Document, name: &str) -> bool {
    if doc.get_str("stage").map(|s| s == name).unwrap_or(false) {
        return true;
    }
    for (_, v) in doc.iter() {
        match v {
            mongodb::bson::Bson::Document(d) => {
                if contains_stage_named(d, name) {
                    return true;
                }
            }
            mongodb::bson::Bson::Array(arr) => {
                for item in arr {
                    if let mongodb::bson::Bson::Document(d) = item {
                        if contains_stage_named(d, name) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Recursively collects every `"indexName"` field found anywhere in a BSON
/// document (explain output nests it under queryPlanner/winningPlan/inputStages).
fn collect_index_names(doc: &Document, out: &mut Vec<String>) {
    if let Ok(name) = doc.get_str("indexName") {
        out.push(name.to_string());
    }
    for (_, v) in doc.iter() {
        match v {
            mongodb::bson::Bson::Document(d) => collect_index_names(d, out),
            mongodb::bson::Bson::Array(arr) => {
                for item in arr {
                    if let mongodb::bson::Bson::Document(d) = item {
                        collect_index_names(d, out);
                    }
                }
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn mongodb_integration_history_delete_trial_use_not_supported() {
    let Some(backend) = create_backend("history_delete_not_supported").await else {
        eprintln!(
            "Skipping mongodb_integration_history_delete_trial_use_not_supported (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-history-not-supported");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-history-delete",
                "name": [{"family": "TrialUse"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let delete_all_history = backend
        .delete_instance_history(&tenant, "Patient", created.id())
        .await;

    assert!(matches!(
        delete_all_history,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));

    let delete_single_version = backend
        .delete_version(&tenant, "Patient", created.id(), "1")
        .await;

    assert!(matches!(
        delete_single_version,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
}

#[tokio::test]
async fn mongodb_integration_contained_search() {
    use helios_persistence::types::{ContainedMode, ContainedReturn};

    let Some(backend) = create_backend_with_full_registry("contained_search").await else {
        eprintln!(
            "Skipping mongodb_integration_contained_search (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-contained");

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs1",
                "status": "final",
                "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                "subject": { "reference": "#p1" },
                "contained": [{
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{ "family": "Smith", "given": ["Contained"] }],
                    "gender": "male"
                }]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "top1", "name": [{ "family": "Smith" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let name_query = |mode: ContainedMode, ret: ContainedReturn| {
        let mut q = SearchQuery::new("Patient");
        q.contained = mode;
        q.contained_return = ret;
        q.parameters.push(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });
        q
    };

    // Default (_contained=off): only the top-level Patient.
    let off = backend
        .search(
            &tenant,
            &name_query(ContainedMode::Off, ContainedReturn::Container),
        )
        .await
        .unwrap();
    let off_urls: Vec<String> = off.resources.items.iter().map(|r| r.url()).collect();
    assert_eq!(off_urls, vec!["Patient/top1"]);

    // _contained=true: the container is returned.
    let on = backend
        .search(
            &tenant,
            &name_query(ContainedMode::On, ContainedReturn::Container),
        )
        .await
        .unwrap();
    let on_urls: Vec<String> = on.resources.items.iter().map(|r| r.url()).collect();
    assert_eq!(on_urls, vec!["Observation/obs1"]);

    // _containedType=contained: the contained Patient itself.
    let contained = backend
        .search(
            &tenant,
            &name_query(ContainedMode::On, ContainedReturn::Contained),
        )
        .await
        .unwrap();
    assert_eq!(contained.resources.items.len(), 1);
    assert_eq!(contained.resources.items[0].resource_type(), "Patient");
    assert_eq!(contained.resources.items[0].id(), "p1");

    // _contained=both: top-level + container.
    let both = backend
        .search(
            &tenant,
            &name_query(ContainedMode::Both, ContainedReturn::Container),
        )
        .await
        .unwrap();
    let mut both_urls: Vec<String> = both.resources.items.iter().map(|r| r.url()).collect();
    both_urls.sort();
    assert_eq!(both_urls, vec!["Observation/obs1", "Patient/top1"]);
}

#[tokio::test]
async fn mongodb_integration_search_quantity() {
    let Some(backend) = create_backend_with_full_registry("search_quantity").await else {
        eprintln!(
            "Skipping mongodb_integration_search_quantity (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-quantity");

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-weight",
                "status": "final",
                "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
                "valueQuantity": { "value": 72.5, "unit": "kg", "system": "http://unitsofmeasure.org" }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let query = |prefix: SearchPrefix, value: &str| {
        SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        })
    };

    // ge70 matches (72.5 >= 70).
    let result = backend
        .search(&tenant, &query(SearchPrefix::Ge, "70"))
        .await
        .unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "value-quantity ge70 → 1 hit"
    );
    assert_eq!(result.resources.items[0].id(), "obs-weight");

    // ge100 does not match.
    let result = backend
        .search(&tenant, &query(SearchPrefix::Ge, "100"))
        .await
        .unwrap();
    assert!(result.resources.items.is_empty(), "ge100 → no hit");

    // Quantity with a matching unit code.
    let result = backend
        .search(
            &tenant,
            &query(SearchPrefix::Ge, "70|http://unitsofmeasure.org|kg"),
        )
        .await
        .unwrap();
    assert_eq!(result.resources.items.len(), 1, "ge70 with kg unit → 1 hit");

    // Non-matching unit code excludes.
    let result = backend
        .search(
            &tenant,
            &query(SearchPrefix::Ge, "70|http://unitsofmeasure.org|lb"),
        )
        .await
        .unwrap();
    assert!(result.resources.items.is_empty(), "wrong unit → no hit");
}

#[tokio::test]
async fn mongodb_integration_compartment_search() {
    // Compartment membership: a resource joins the Patient compartment if it
    // references the patient via ANY of the membership params (`subject` OR
    // `performer`). Resources for another patient must be excluded.
    use helios_persistence::types::CompartmentMembership;

    let Some(backend) = create_backend_with_full_registry("compartment_search").await else {
        eprintln!(
            "Skipping mongodb_integration_compartment_search (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-compartment");

    // In the compartment via `subject`.
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-subject",
                "status": "final",
                "code": {"text": "hr"},
                "subject": {"reference": "Patient/p1"}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // In the compartment via the NON-first param `performer` only.
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-performer",
                "status": "final",
                "code": {"text": "hr"},
                "subject": {"reference": "Patient/p2"},
                "performer": [{"reference": "Patient/p1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Not in the compartment (references another patient).
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-other",
                "status": "final",
                "code": {"text": "hr"},
                "subject": {"reference": "Patient/p2"}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let mut query = SearchQuery::new("Observation");
    query.compartment = Some(CompartmentMembership {
        params: vec!["subject".to_string(), "performer".to_string()],
        reference: "Patient/p1".to_string(),
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    let ids: Vec<String> = result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();

    assert!(
        ids.contains(&"obs-subject".to_string()),
        "compartment must include the resource linked via `subject`"
    );
    assert!(
        ids.contains(&"obs-performer".to_string()),
        "compartment must include the resource linked via `performer`"
    );
    assert!(
        !ids.contains(&"obs-other".to_string()),
        "compartment must exclude resources of another patient"
    );
}

#[tokio::test]
async fn mongodb_integration_search_token_string_and_offset_pagination() {
    let Some(backend) = create_backend_with_full_registry("search_token_string").await else {
        eprintln!(
            "Skipping mongodb_integration_search_token_string_and_offset_pagination (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search");

    for (id, mrn, family) in [
        ("patient-search-1", "MRN-SEARCH-1", "Smith"),
        ("patient-search-2", "MRN-SEARCH-2", "Smiley"),
        ("patient-search-3", "MRN-SEARCH-3", "Jones"),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "identifier": [{"system": "http://hospital.org/mrn", "value": mrn}],
                    "name": [{"family": family}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let token_query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-SEARCH-1")],
        chain: vec![],
        components: vec![],
    });

    let token_result = backend.search(&tenant, &token_query).await.unwrap();
    assert_eq!(token_result.resources.items.len(), 1);
    assert_eq!(token_result.resources.items[0].id(), "patient-search-1");

    let mut string_query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smi")],
            chain: vec![],
            components: vec![],
        })
        .with_sort(SortDirective::parse("_id"))
        .with_count(1);

    let first_page = backend.search(&tenant, &string_query).await.unwrap();
    assert_eq!(first_page.resources.items.len(), 1);
    assert!(first_page.resources.page_info.has_next);
    let first_id = first_page.resources.items[0].id().to_string();

    string_query.offset = Some(1);
    let second_page = backend.search(&tenant, &string_query).await.unwrap();
    assert_eq!(second_page.resources.items.len(), 1);
    assert_ne!(second_page.resources.items[0].id(), first_id);
}

#[tokio::test]
async fn mongodb_integration_search_cursor_pagination_roundtrip() {
    let Some(backend) = create_backend("search_cursor_roundtrip").await else {
        eprintln!(
            "Skipping mongodb_integration_search_cursor_pagination_roundtrip (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-cursor");

    for id in ["patient-cursor-1", "patient-cursor-2", "patient-cursor-3"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{"family": format!("Cursor-{}", id)}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let query = SearchQuery::new("Patient").with_count(1);

    let page1 = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(page1.resources.items.len(), 1);
    assert!(page1.resources.page_info.has_next);
    assert!(!page1.resources.page_info.has_previous);

    let first_id = page1.resources.items[0].id().to_string();
    let next_cursor = page1
        .resources
        .page_info
        .next_cursor
        .clone()
        .expect("first page should include next cursor");

    let page2 = backend
        .search(&tenant, &query.clone().with_cursor(next_cursor))
        .await
        .unwrap();

    assert_eq!(page2.resources.items.len(), 1);
    assert!(page2.resources.page_info.has_previous);
    let second_id = page2.resources.items[0].id().to_string();
    assert_ne!(second_id, first_id);

    let previous_cursor = page2
        .resources
        .page_info
        .previous_cursor
        .clone()
        .expect("second page should include previous cursor");

    let page_back = backend
        .search(&tenant, &query.with_cursor(previous_cursor))
        .await
        .unwrap();

    assert_eq!(page_back.resources.items.len(), 1);
    assert_eq!(page_back.resources.items[0].id(), first_id.as_str());
}

#[tokio::test]
async fn mongodb_integration_search_missing_not_and_param_sort() {
    let Some(backend) = create_backend_with_full_registry("search_missing_not_sort").await else {
        eprintln!(
            "Skipping mongodb_integration_search_missing_not_and_param_sort (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-missing-not-sort");

    for (id, gender, birth_date) in [
        ("patient-mns-1", Some("male"), Some("1990-05-01")),
        ("patient-mns-2", Some("female"), Some("1975-03-20")),
        ("patient-mns-3", None, Some("2001-11-11")),
        ("patient-mns-4", Some("female"), None),
    ] {
        let mut resource = json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": format!("Mns-{}", id)}],
        });
        if let Some(gender) = gender {
            resource["gender"] = json!(gender);
        }
        if let Some(birth_date) = birth_date {
            resource["birthDate"] = json!(birth_date);
        }
        backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
    }

    let ids = |result: &helios_persistence::core::SearchResult| {
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect::<Vec<_>>()
    };

    // :missing=true — only the patient with no gender at all.
    let missing_true = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::eq("true")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &missing_true).await.unwrap();
    assert_eq!(ids(&result), vec!["patient-mns-3"]);

    // :missing=false — everyone with a gender value.
    let missing_false = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::eq("false")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &missing_false).await.unwrap();
    let mut got = ids(&result);
    got.sort();
    assert_eq!(got, vec!["patient-mns-1", "patient-mns-2", "patient-mns-4"]);

    // :not — excludes matches AND includes resources without the value (per spec).
    let not_male = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Not),
        values: vec![SearchValue::eq("male")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &not_male).await.unwrap();
    let mut got = ids(&result);
    got.sort();
    assert_eq!(got, vec!["patient-mns-2", "patient-mns-3", "patient-mns-4"]);

    // _sort by an indexed date parameter, ascending; the patient without a
    // birthDate sorts last.
    let sorted = SearchQuery::new("Patient")
        .with_sort(SortDirective::parse("birthdate").with_param_type(Some(SearchParamType::Date)));
    let result = backend.search(&tenant, &sorted).await.unwrap();
    assert_eq!(
        ids(&result),
        vec![
            "patient-mns-2",
            "patient-mns-1",
            "patient-mns-3",
            "patient-mns-4"
        ]
    );

    // Descending; keyed resources reverse, the unkeyed one still sorts last.
    let sorted_desc = SearchQuery::new("Patient")
        .with_sort(SortDirective::parse("-birthdate").with_param_type(Some(SearchParamType::Date)));
    let result = backend.search(&tenant, &sorted_desc).await.unwrap();
    assert_eq!(
        ids(&result),
        vec![
            "patient-mns-3",
            "patient-mns-1",
            "patient-mns-2",
            "patient-mns-4"
        ]
    );

    // Param sort composes with filters and offset pagination.
    let mut filtered_sorted = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("false")],
            chain: vec![],
            components: vec![],
        })
        .with_sort(SortDirective::parse("birthdate").with_param_type(Some(SearchParamType::Date)))
        .with_count(2);
    let page1 = backend.search(&tenant, &filtered_sorted).await.unwrap();
    assert_eq!(ids(&page1), vec!["patient-mns-2", "patient-mns-1"]);
    assert!(page1.resources.page_info.has_next);
    assert!(page1.resources.page_info.next_cursor.is_none());

    filtered_sorted.offset = Some(2);
    let page2 = backend.search(&tenant, &filtered_sorted).await.unwrap();
    assert_eq!(ids(&page2), vec!["patient-mns-4"]);
    assert!(!page2.resources.page_info.has_next);
    assert!(page2.resources.page_info.has_previous);
}

/// The in-DB runner compiles no compartment predicate, so a run carrying
/// `patient`/`group` filters is handed to the in-process engine over a scan
/// of the same collection instead of failing as uncompilable — and answers
/// the rows the SQL runners answer for the same filters. An unfiltered run
/// still takes the aggregation pipeline.
#[tokio::test]
async fn mongodb_integration_sof_runner_compartment_filters_fall_back_in_process() {
    use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
    use tokio_stream::StreamExt;

    let Some(backend) = create_backend("sof_compartment_fallback").await else {
        eprintln!(
            "Skipping mongodb_integration_sof_runner_compartment_filters_fall_back_in_process (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-sof-compartment-fallback");

    for resource in [
        json!({ "resourceType": "Patient", "id": "sof-p1" }),
        json!({ "resourceType": "Patient", "id": "sof-p2" }),
        json!({
            "resourceType": "Group",
            "id": "sof-g1",
            "type": "person",
            "actual": true,
            "member": [{ "entity": { "reference": "Patient/sof-p1" } }]
        }),
        json!({
            "resourceType": "Observation",
            "id": "sof-o1",
            "status": "final",
            "code": { "text": "x" },
            "subject": { "reference": "Patient/sof-p1" }
        }),
        json!({
            "resourceType": "Observation",
            "id": "sof-o2",
            "status": "final",
            "code": { "text": "x" },
            "subject": { "reference": "Patient/sof-p2" }
        }),
    ] {
        let resource_type = resource["resourceType"].as_str().unwrap().to_string();
        backend
            .create(&tenant, &resource_type, resource, FhirVersion::default())
            .await
            .unwrap();
    }

    async fn observation_ids(
        runner: &dyn SofRunner,
        tenant: &TenantContext,
        filters: ViewFilters,
    ) -> Vec<String> {
        let view = json!({
            "resourceType": "ViewDefinition",
            "resource": "Observation",
            "status": "active",
            "select": [{ "column": [{ "path": "id", "name": "obs_id" }] }]
        });
        let mut stream = runner
            .run_view(tenant, view, filters)
            .await
            .expect("run_view must succeed");
        let mut ids = Vec::new();
        while let Some(row) = stream.next().await {
            let row = row.expect("row must not be an error");
            ids.push(row["obs_id"].as_str().unwrap().to_string());
        }
        ids.sort();
        ids
    }

    let runner = backend.sof_runner().expect("MongoDB provides a SOF runner");

    assert_eq!(
        observation_ids(runner.as_ref(), &tenant, ViewFilters::default()).await,
        vec!["sof-o1", "sof-o2"]
    );
    assert_eq!(
        observation_ids(
            runner.as_ref(),
            &tenant,
            ViewFilters {
                patient: vec!["Patient/sof-p2".to_string()],
                ..Default::default()
            },
        )
        .await,
        vec!["sof-o2"]
    );
    assert_eq!(
        observation_ids(
            runner.as_ref(),
            &tenant,
            ViewFilters {
                group: vec!["Group/sof-g1".to_string()],
                ..Default::default()
            },
        )
        .await,
        vec!["sof-o1"]
    );
}

#[tokio::test]
async fn mongodb_integration_conditional_create_exists() {
    let Some(backend) = create_backend_with_full_registry("conditional_create").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_create_exists (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-create");

    let created = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-1"}],
                "name": [{"family": "Original"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-1",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let created_id = match created {
        ConditionalCreateResult::Created(resource) => resource.id().to_string(),
        other => panic!("expected Created result, got {:?}", other),
    };

    let second = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-1"}],
                "name": [{"family": "Duplicate"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-1",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    match second {
        ConditionalCreateResult::Exists(existing) => assert_eq!(existing.id(), created_id),
        other => panic!("expected Exists result, got {:?}", other),
    }
}

#[tokio::test]
async fn mongodb_integration_conditional_update_delete_and_no_match() {
    let Some(backend) = create_backend_with_full_registry("conditional_update_delete").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_update_delete_and_no_match (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-update-delete");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-cond-update",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-UPDATE"}],
                "name": [{"family": "Before"}],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .conditional_update(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-cond-update",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-COND-UPDATE"}],
                "name": [{"family": "After"}],
            }),
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
            false,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated_id = match updated {
        ConditionalUpdateResult::Updated(resource) => {
            assert_eq!(resource.content()["name"][0]["family"], "After");
            resource.id().to_string()
        }
        other => panic!("expected Updated result, got {:?}", other),
    };

    let deleted = backend
        .conditional_delete(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
        )
        .await
        .unwrap();
    assert!(matches!(deleted, ConditionalDeleteResult::Deleted(_)));

    let no_match = backend
        .conditional_delete(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-UPDATE",
        )
        .await
        .unwrap();
    assert!(matches!(no_match, ConditionalDeleteResult::NoMatch));

    let read_after_delete = backend.read(&tenant, "Patient", &updated_id).await;
    assert!(matches!(
        read_after_delete,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test]
async fn mongodb_integration_conditional_create_multiple_matches() {
    let Some(backend) = create_backend_with_full_registry("conditional_multiple_matches").await
    else {
        eprintln!(
            "Skipping mongodb_integration_conditional_create_multiple_matches (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-multi");

    for (id, system) in [
        ("patient-cond-multi-1", "http://system-a.org"),
        ("patient-cond-multi-2", "http://system-b.org"),
    ] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "identifier": [{"system": system, "value": "SHARED-VALUE"}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let result = backend
        .conditional_create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"value": "SHARED-VALUE"}],
            }),
            "identifier=SHARED-VALUE",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    match result {
        ConditionalCreateResult::MultipleMatches(count) => assert_eq!(count, 2),
        other => panic!("expected MultipleMatches result, got {:?}", other),
    }
}

#[tokio::test]
async fn mongodb_integration_conditional_patch_not_supported() {
    let Some(backend) = create_backend("conditional_patch_not_supported").await else {
        eprintln!(
            "Skipping mongodb_integration_conditional_patch_not_supported (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-conditional-patch");

    let result = backend
        .conditional_patch(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-COND-PATCH",
            &PatchFormat::MergePatch(json!({ "active": true })),
        )
        .await;

    assert!(matches!(
        result,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
}

#[tokio::test]
async fn mongodb_integration_search_parameter_create_registers_active() {
    let Some(backend) = create_backend("search_param_create_active").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_create_registers_active (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-create-active");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-patient-nickname",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-patient-nickname",
                "name": "MongoPatientNickname",
                "status": "active",
                "code": "mongo-nickname",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.name.where(use='nickname').given"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let reg = backend.search_param_registry(&tenant);
    let registry = reg.read();
    let param = registry.get_param("Patient", "mongo-nickname");
    assert!(
        param.is_some(),
        "Active SearchParameter should be registered"
    );

    let param = param.unwrap();
    assert_eq!(
        param.url,
        "http://example.org/fhir/SearchParameter/mongo-custom-patient-nickname"
    );
    assert_eq!(param.status, SearchParameterStatus::Active);
}

/// The TTL-cache refresh (#235) rebuilds the registry's stored parameters from
/// what the database currently holds: a parameter written by a cluster-mate
/// enters resolution on the next refresh, and a deleted one leaves it. This is
/// the SQLite integration test's contract exercised over the Mongo cursor path.
#[tokio::test]
async fn mongodb_integration_refresh_rebuilds_stored_parameters() {
    let Some(backend) = create_backend("search_param_refresh").await else {
        eprintln!(
            "Skipping mongodb_integration_refresh_rebuilds_stored_parameters (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-refresh");
    let other = create_tenant("tenant-search-param-other");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-refresh-nickname",
                "url": "http://example.org/fhir/SearchParameter/mongo-refresh-nickname",
                "name": "MongoRefreshNickname",
                "status": "active",
                "code": "mongo-refresh-nickname",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.name.where(use='nickname').given"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // The write refreshes the stored-param cache: the parameter resolves for its
    // tenant and is isolated from others.
    {
        let reg = backend.search_param_registry(&tenant);
        assert!(
            reg.read()
                .get_param("Patient", "mongo-refresh-nickname")
                .is_some(),
            "visible to the owning tenant after the write"
        );
    }
    {
        let reg = backend.search_param_registry(&other);
        assert!(
            reg.read()
                .get_param("Patient", "mongo-refresh-nickname")
                .is_none(),
            "isolated from other tenants"
        );
    }

    // The TTL refresh drops the cached per-tenant registries; the next access
    // re-reads storage (how a cluster-mate's write becomes visible).
    let _ = backend
        .search_param_registry(&tenant)
        .read()
        .get_param("Patient", "mongo-refresh-nickname");
    backend
        .refresh_stored_search_parameters()
        .await
        .expect("refresh from storage");
    assert_eq!(backend.tenant_registries().cached_tenant_count(), 0);

    // Delete it from storage; it leaves the owning tenant's resolution.
    backend
        .delete(&tenant, "SearchParameter", "mongo-refresh-nickname")
        .await
        .unwrap();
    let reg = backend.search_param_registry(&tenant);
    assert!(
        reg.read()
            .get_param("Patient", "mongo-refresh-nickname")
            .is_none(),
        "gone after the delete"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_create_draft_not_registered() {
    let Some(backend) = create_backend("search_param_create_draft").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_create_draft_not_registered (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-create-draft");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-draft-param",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-draft-param",
                "name": "MongoDraftParam",
                "status": "draft",
                "code": "mongo-draft",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.extension('draft')"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let reg = backend.search_param_registry(&tenant);
    let registry = reg.read();
    let param = registry.get_param("Patient", "mongo-draft");
    assert!(
        param.is_none(),
        "Draft SearchParameter should not be registered"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_update_status_change() {
    let Some(backend) = create_backend("search_param_update_status").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_update_status_change (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-update-status");

    let created = backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-status-change",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-status-change",
                "name": "MongoStatusChange",
                "status": "active",
                "code": "mongo-statuschange",
                "base": ["Condition"],
                "type": "token",
                "expression": "Condition.code"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let reg = backend.search_param_registry(&tenant);
        let registry = reg.read();
        let param = registry.get_param("Condition", "mongo-statuschange");
        assert!(
            param.is_some(),
            "Parameter should be registered after create"
        );
        assert_eq!(
            param.unwrap().status,
            SearchParameterStatus::Active,
            "Initial status should be active"
        );
    }

    backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-status-change",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-status-change",
                "name": "MongoStatusChange",
                "status": "retired",
                "code": "mongo-statuschange",
                "base": ["Condition"],
                "type": "token",
                "expression": "Condition.code"
            }),
        )
        .await
        .unwrap();

    // A per-tenant registry overlays only a tenant's *active* stored params, so
    // retiring the parameter removes it from that tenant's search resolution.
    let reg = backend.search_param_registry(&tenant);
    let registry = reg.read();
    assert!(
        registry
            .get_param("Condition", "mongo-statuschange")
            .is_none(),
        "a retired custom parameter should no longer resolve for the tenant"
    );
}

#[tokio::test]
async fn mongodb_integration_search_parameter_delete_unregisters() {
    let Some(backend) = create_backend("search_param_delete_unregister").await else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_delete_unregisters (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-delete");

    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-custom-to-delete",
                "url": "http://example.org/fhir/SearchParameter/mongo-custom-to-delete",
                "name": "MongoToDelete",
                "status": "active",
                "code": "mongo-todelete",
                "base": ["Observation"],
                "type": "token",
                "expression": "Observation.code"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let reg = backend.search_param_registry(&tenant);
        let registry = reg.read();
        assert!(
            registry
                .get_param("Observation", "mongo-todelete")
                .is_some()
        );
    }

    backend
        .delete(&tenant, "SearchParameter", "mongo-custom-to-delete")
        .await
        .unwrap();

    let reg = backend.search_param_registry(&tenant);
    let registry = reg.read();
    assert!(
        registry
            .get_param("Observation", "mongo-todelete")
            .is_none(),
        "Deleted SearchParameter should be unregistered"
    );
}

#[tokio::test]
async fn mongodb_integration_search_offloaded_prevents_search_index_writes() {
    let Some(backend) =
        create_backend_with_search_offloaded("search_offloaded_no_index", true).await
    else {
        eprintln!(
            "Skipping mongodb_integration_search_offloaded_prevents_search_index_writes (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-offloaded");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "mongo-offloaded-patient",
                "name": [{"family": "Offloaded"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "OFFLOADED-1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let resource_id = created.id().to_string();

    let after_create = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_create, 0,
        "search_index should remain empty when search_offloaded=true (create)"
    );

    let updated = backend
        .update(
            &tenant,
            &created,
            json!({
                "resourceType": "Patient",
                "id": "mongo-offloaded-patient",
                "name": [{"family": "StillOffloaded"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "OFFLOADED-1"}]
            }),
        )
        .await
        .unwrap();

    let after_update = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_update, 0,
        "search_index should remain empty when search_offloaded=true (update)"
    );

    backend
        .delete(&tenant, "Patient", updated.id())
        .await
        .unwrap();

    let after_delete = search_index_entry_count(&backend, &tenant, "Patient", &resource_id).await;
    assert_eq!(
        after_delete, 0,
        "search_index should remain empty when search_offloaded=true (delete)"
    );
}

/// Schema v9 replaces `idx_resources_type_deleted` with a longer index that also
/// carries the `$reindex` page order (#1021). The old index is that index's
/// strict prefix, so keeping both would cost a second B-tree on every write for
/// no reader — the migration has to actually drop it, on a deployment that
/// already has it.
#[tokio::test]
async fn mongodb_integration_schema_v9_swaps_in_the_reindex_scan_index() {
    let Some(backend) = create_backend("schema_v9_index_swap").await else {
        eprintln!(
            "Skipping mongodb_integration_schema_v9_swaps_in_the_reindex_scan_index (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for index assertions");
    let database = client.database(&backend.config().database_name);
    let resources = database.collection::<Document>("resources");

    async fn index_names(collection: &mongodb::Collection<Document>) -> Vec<String> {
        use futures::stream::TryStreamExt;
        collection
            .list_indexes()
            .await
            .expect("list indexes")
            .try_collect::<Vec<_>>()
            .await
            .expect("collect indexes")
            .into_iter()
            .filter_map(|i| i.options.and_then(|o| o.name))
            .collect()
    }

    // Put the collection back into its v8 shape: the superseded index present,
    // the new one absent, as an upgraded deployment finds it.
    let _ = resources.drop_index("idx_resources_type_scan").await;
    resources
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "is_deleted": 1_i32 })
                .options(Some(
                    mongodb::options::IndexOptions::builder()
                        .name(Some("idx_resources_type_deleted".to_string()))
                        .build(),
                ))
                .build(),
        )
        .await
        .expect("recreate the v7 index");
    assert!(
        index_names(&resources)
            .await
            .contains(&"idx_resources_type_deleted".to_string())
    );

    backend.init_schema().await.expect("re-run schema init");

    let names = index_names(&resources).await;
    assert!(
        names.contains(&"idx_resources_type_scan".to_string()),
        "the reindex page order must be indexed, got {names:?}"
    );
    assert!(
        !names.contains(&"idx_resources_type_deleted".to_string()),
        "the superseded prefix must be dropped, not kept beside it, got {names:?}"
    );

    // Idempotent: the second run finds nothing to drop, which is also the
    // fresh-deployment path.
    backend
        .init_schema()
        .await
        .expect("schema init is idempotent");
    let names = index_names(&resources).await;
    assert!(names.contains(&"idx_resources_type_scan".to_string()));
    assert!(!names.contains(&"idx_resources_type_deleted".to_string()));
}

#[tokio::test]
async fn mongodb_integration_standalone_search_writes_search_index() {
    let Some(backend) = create_backend("search_index_written_standalone").await else {
        eprintln!(
            "Skipping mongodb_integration_standalone_search_writes_search_index (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-standalone");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "mongo-standalone-patient",
                "name": [{"family": "Indexed"}],
                "identifier": [{"system": "http://hospital.org/mrn", "value": "INDEXED-1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let count = search_index_entry_count(&backend, &tenant, "Patient", created.id()).await;
    assert!(
        count > 0,
        "search_index should contain entries in standalone mode"
    );
}

/// #1064: `MongoBackend` had no `write_search_entries_page` override, so
/// `$reindex` fell through to `ReindexTarget`'s default loop — per resource,
/// one `delete_many` (`delete_search_entries`) followed by ANOTHER
/// `delete_many` plus one `insert_many` (`write_search_entries` ->
/// `index_resource`). An 8-resource page issued 16 deletes and 8 inserts
/// instead of 1 and 1, strictly sequential.
///
/// MongoDB's per-database profiler is the seam that proves the command
/// count: every test here gets a unique database
/// (`build_test_database_name`), so `system.profile` is immune to the other
/// integration tests running in parallel against the shared container — the
/// reason a `serverStatus` counter would not work here.
#[tokio::test]
async fn mongodb_integration_reindex_page_batches_index_writes() {
    use futures::stream::TryStreamExt;
    use helios_persistence::search::ReindexTarget;
    use helios_persistence::types::StoredResource;

    let Some(backend) = create_backend("reindex_page_batches").await else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_batches_index_writes (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("reindex-page-tenant");

    let make_patient = |n: usize| {
        StoredResource::from_storage(
            "Patient",
            format!("page-{n}"),
            "1",
            tenant.tenant_id().clone(),
            json!({
                "resourceType": "Patient",
                "id": format!("page-{n}"),
                "name": [{"family": format!("Paged{n}")}]
            }),
            chrono::Utc::now(),
            chrono::Utc::now(),
            None,
            FhirVersion::default(),
        )
    };

    let page: Vec<StoredResource> = (0..8).map(make_patient).collect();
    let untouched = make_patient(8);

    // Seed: every resource — the page plus one that will NOT be reindexed —
    // already has search_index rows, as a prior write or reindex would leave
    // them. This also runs the override once before the profiled window, so
    // that window measures only the second rebuild below.
    let mut seed = page.clone();
    seed.push(untouched.clone());
    let seed_outcomes = backend.write_search_entries_page(&tenant, &seed).await;
    assert!(
        seed_outcomes.iter().all(|o| o.is_ok()),
        "seeding failed: {seed_outcomes:?}"
    );

    let mut before_counts = Vec::with_capacity(page.len());
    for resource in &page {
        let count = search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await;
        assert!(
            count > 0,
            "resource {} should already be indexed",
            resource.id()
        );
        before_counts.push(count);
    }
    let untouched_before =
        search_index_entry_count(&backend, &tenant, "Patient", untouched.id()).await;
    assert!(untouched_before > 0);

    let db = backend.get_database().await.unwrap();
    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_integration_reindex_page_batches_index_writes: server refused \
             {{profile: 2}} (likely a managed/shared HFS_TEST_MONGODB_URL); skipping the \
             command-count assertions and running the contract assertions only"
        );
    }

    let outcomes = backend.write_search_entries_page(&tenant, &page).await;

    if profiling_enabled {
        db.run_command(doc! { "profile": 0_i32 })
            .await
            .expect("failed to disable profiling");

        let ns = format!("{}.search_index", db.name());
        let entries: Vec<Document> = db
            .collection::<Document>("system.profile")
            .find(doc! { "ns": ns.as_str() })
            .await
            .expect("failed to read system.profile")
            .try_collect()
            .await
            .expect("failed to collect system.profile");

        assert!(
            !entries.is_empty(),
            "profiler produced no entries for {ns} — the assertions below would be vacuous"
        );

        let removes = entries
            .iter()
            .filter(|e| e.get_str("op").ok() == Some("remove"))
            .count();
        let inserts = entries
            .iter()
            .filter(|e| e.get_str("op").ok() == Some("insert"))
            .count();

        assert!(
            removes <= 1,
            "a reindex page must issue at most one delete_many, got {removes}: {entries:?}"
        );
        assert!(
            inserts <= 1,
            "a reindex page must issue at most one insert_many, got {inserts}: {entries:?}"
        );
        assert!(
            entries.len() <= 2,
            "a reindex page must issue at most 2 write commands total, got {}: {entries:?}",
            entries.len()
        );
    }

    // Contract, checked whether or not profiling was available: one outcome
    // per resource, in order, each reporting the rows actually present, and
    // the rewrite left the row count for each resource unchanged (a
    // delete-then-rewrite of identical content).
    assert_eq!(outcomes.len(), page.len());
    for (i, (outcome, resource)) in outcomes.iter().zip(&page).enumerate() {
        let count = outcome
            .as_ref()
            .unwrap_or_else(|e| panic!("resource {i} failed: {e:?}"));
        let actual = search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await;
        assert_eq!(
            *count as u64, actual,
            "resource {i} reported count must match rows actually present"
        );
        assert_eq!(
            actual, before_counts[i],
            "resource {i} row count drifted across the rebuild"
        );
    }

    // Scoping: the ninth Patient, not in the page, must be untouched — proves
    // the single grouped `delete_many` is scoped by the page's resource ids
    // and did not wipe the tenant's other rows.
    let untouched_after =
        search_index_entry_count(&backend, &tenant, "Patient", untouched.id()).await;
    assert_eq!(
        untouched_after, untouched_before,
        "the page's delete_many must not touch resources outside the page"
    );
}

/// #1064: `write_search_entries` used to report `extract(..).len()` — the
/// container's own extracted values — even though `index_resource` (and,
/// after the fix, `write_search_entries_page`) also inserts `_contained`
/// rows for anything nested under `contained`. The reported count therefore
/// undercounted whenever a resource has `contained` entries. Uses the full
/// registry so both the Observation's own parameters and the contained
/// Patient's `name` are guaranteed to extract, not just whichever handful
/// ship in the embedded default registry.
#[tokio::test]
async fn mongodb_integration_reindex_page_counts_contained_entries() {
    use helios_persistence::search::ReindexTarget;
    use helios_persistence::types::StoredResource;

    let Some(backend) = create_backend_with_full_registry("reindex_page_contained").await else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_counts_contained_entries (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("reindex-contained-tenant");

    let with_contained = StoredResource::from_storage(
        "Observation",
        "obs-contained",
        "1",
        tenant.tenant_id().clone(),
        json!({
            "resourceType": "Observation",
            "id": "obs-contained",
            "status": "final",
            "contained": [{
                "resourceType": "Patient",
                "id": "inner",
                "name": [{"family": "Contained"}]
            }],
            "subject": {"reference": "#inner"},
            "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]}
        }),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );

    let outcomes = backend
        .write_search_entries_page(&tenant, std::slice::from_ref(&with_contained))
        .await;
    assert_eq!(
        outcomes.len(),
        1,
        "one outcome per resource, not per document"
    );
    let reported = outcomes[0]
        .as_ref()
        .unwrap_or_else(|e| panic!("reindex failed: {e:?}"));

    let actual = search_index_entry_count(&backend, &tenant, "Observation", "obs-contained").await;
    assert_eq!(
        *reported as u64, actual,
        "reported entry count must match rows actually written, including _contained rows"
    );

    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for search_index assertions");
    let database = client.database(&backend.config().database_name);
    let contained_rows = database
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": "obs-contained",
            "is_contained": true,
        })
        .await
        .expect("failed to count contained search_index rows");
    assert!(
        contained_rows > 0,
        "the contained Patient's values must be indexed alongside the container"
    );
}

/// Guard for the `is_search_offloaded()` short-circuit the batched override
/// needs. The default loop honors the flag via `delete_search_entries` and
/// `write_search_entries`'s own guards; the page override has to reproduce
/// it directly rather than issuing commands a search-offloaded backend must
/// never run. Passes both before and after #1064 — it pins the guard, not
/// the defect.
#[tokio::test]
async fn mongodb_integration_reindex_page_is_a_no_op_when_search_offloaded() {
    use helios_persistence::search::ReindexTarget;
    use helios_persistence::types::StoredResource;

    let Some(backend) = create_backend_with_search_offloaded("reindex_page_offloaded", true).await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_is_a_no_op_when_search_offloaded (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("reindex-offloaded-tenant");

    let resources: Vec<StoredResource> = (0..2)
        .map(|n| {
            StoredResource::from_storage(
                "Patient",
                format!("offloaded-{n}"),
                "1",
                tenant.tenant_id().clone(),
                json!({
                    "resourceType": "Patient",
                    "id": format!("offloaded-{n}"),
                    "name": [{"family": format!("Offloaded{n}")}]
                }),
                chrono::Utc::now(),
                chrono::Utc::now(),
                None,
                FhirVersion::default(),
            )
        })
        .collect();

    let outcomes = backend.write_search_entries_page(&tenant, &resources).await;
    assert_eq!(outcomes.len(), 2);
    for outcome in &outcomes {
        assert_eq!(*outcome.as_ref().unwrap(), 0usize);
    }

    for resource in &resources {
        let count = search_index_entry_count(&backend, &tenant, "Patient", resource.id()).await;
        assert_eq!(count, 0, "search-offloaded backend must write nothing");
    }
}

#[tokio::test]
async fn mongodb_integration_search_parameter_registry_updates_when_offloaded() {
    let Some(backend) =
        create_backend_with_search_offloaded("search_param_offloaded_registry", true).await
    else {
        eprintln!(
            "Skipping mongodb_integration_search_parameter_registry_updates_when_offloaded (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-search-param-offloaded");

    let created = backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "mongo-offloaded-search-param",
                "url": "http://example.org/fhir/SearchParameter/mongo-offloaded-search-param",
                "name": "MongoOffloadedSearchParam",
                "status": "active",
                "code": "mongo-offloaded-code",
                "base": ["Patient"],
                "type": "token",
                "expression": "Patient.identifier"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    {
        let reg = backend.search_param_registry(&tenant);
        let registry = reg.read();
        let param = registry.get_param("Patient", "mongo-offloaded-code");
        assert!(
            param.is_some(),
            "Active SearchParameter should register when offloaded"
        );
        assert_eq!(param.unwrap().status, SearchParameterStatus::Active);
    }

    let search_index_count =
        search_index_entry_count(&backend, &tenant, "SearchParameter", created.id()).await;
    assert_eq!(
        search_index_count, 0,
        "SearchParameter resources should not write Mongo search_index when offloaded"
    );

    backend
        .delete(&tenant, "SearchParameter", created.id())
        .await
        .unwrap();

    let reg = backend.search_param_registry(&tenant);
    let registry = reg.read();
    assert!(
        registry
            .get_param("Patient", "mongo-offloaded-code")
            .is_none(),
        "Deleted SearchParameter should unregister when offloaded"
    );
}

#[tokio::test]
async fn mongodb_integration_resolve_include_and_revinclude() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping mongodb_integration_resolve_include_and_revinclude (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    // Point at the workspace-root spec file so that the registry knows about
    // Observation.subject — without it, no reference index entries get written
    // and revinclude resolution would have nothing to match.
    let workspace_data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name("includes"),
        data_dir: Some(workspace_data_dir),
        ..Default::default()
    };
    let Some(backend) = build_backend(config).await else {
        eprintln!(
            "Skipping mongodb_integration_resolve_include_and_revinclude (shared mongo unreachable)"
        );
        return;
    };

    let tenant = create_tenant("tenant-includes");

    let patient = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Includer"}],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let observation = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "subject": {"reference": format!("Patient/{}", patient.id())},
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let forward = IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    };
    let included = backend
        .resolve_includes(&tenant, std::slice::from_ref(&observation), &[forward])
        .await
        .expect("forward include resolution must succeed");
    assert_eq!(included.len(), 1, "exactly one Patient should be included");
    assert_eq!(included[0].resource_type(), "Patient");
    assert_eq!(included[0].id(), patient.id());

    let reverse = IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: None,
        iterate: false,
    };
    let revincluded = backend
        .resolve_revincludes(
            &tenant,
            std::slice::from_ref(&patient),
            std::slice::from_ref(&reverse),
        )
        .await
        .expect("revinclude resolution must succeed");
    assert_eq!(
        revincluded.len(),
        1,
        "exactly one Observation should be revincluded"
    );
    assert_eq!(revincluded[0].resource_type(), "Observation");
    assert_eq!(revincluded[0].id(), observation.id());

    let query = SearchQuery::new("Patient").with_include(reverse);
    let result = backend
        .search(&tenant, &query)
        .await
        .expect("search with _revinclude must not be rejected");
    assert!(
        result
            .resources
            .items
            .iter()
            .any(|r| r.resource_type() == "Patient" && r.id() == patient.id()),
        "primary results should still contain the Patient"
    );
    assert!(
        result
            .included
            .iter()
            .any(|r| r.resource_type() == "Observation" && r.id() == observation.id()),
        "search() should populate `included` from revinclude resolution"
    );
}

/// Like [`create_backend_with_full_registry`] but with a custom
/// `max_included_resources` cap (#1061), so cap/truncation tests can use a
/// small fixture instead of exercising the 1000-resource default.
async fn create_backend_with_full_registry_and_cap(
    test_name: &str,
    max_included_resources: usize,
) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))?;
    let config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(data_dir),
        max_included_resources,
        ..Default::default()
    };
    build_backend(config).await
}

/// #1061 part (a): `resolve_revincludes` must resolve the referring id set via
/// a driver-paged `find` against `search_index`, never via
/// `Collection::distinct` — a `distinct` reply is a single BSON document
/// capped at 16 MiB by mongod (error 17217 past ~372k 36-char ids), which is
/// exactly the failure mode this fix removes.
///
/// Enables the per-database profiler around a *direct* `resolve_revincludes`
/// call (not `search()`, which would also route through the fenced
/// `matching_resource_ids` — itself still `distinct`-based pending #999 — and
/// pollute the profiled window with an unrelated `distinct`), then asserts
/// `system.profile` recorded zero `distinct` commands against `search_index`
/// and at least one `find` — the second assertion is what keeps the first
/// from being vacuously true if profiling silently failed to enable.
#[tokio::test]
async fn mongodb_revinclude_streams_ids_without_distinct() {
    let Some(backend) = create_backend_with_full_registry("revinclude_no_distinct").await else {
        eprintln!(
            "Skipping mongodb_revinclude_streams_ids_without_distinct (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-revinclude-profiler");

    let patient = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Streamed"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    for i in 0..12 {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "subject": {"reference": format!("Patient/{}", patient.id())},
                    "code": {"coding": [{"code": format!("stream-{i}")}]}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let Some(connection_string) = shared_mongo::connection_string().await else {
        eprintln!(
            "Skipping mongodb_revinclude_streams_ids_without_distinct (shared mongo unreachable)"
        );
        return;
    };
    let raw_client = raw_test_client(&connection_string)
        .await
        .expect("failed to build raw test client");
    let db = raw_client.database(&backend.config().database_name);

    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_revinclude_streams_ids_without_distinct: server refused {{profile: 2}} \
             (likely a managed/shared HFS_TEST_MONGODB_URL); skipping"
        );
        return;
    }

    let revinclude = IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: None,
        iterate: false,
    };
    let result = backend
        .resolve_revincludes(
            &tenant,
            std::slice::from_ref(&patient),
            std::slice::from_ref(&revinclude),
        )
        .await
        .expect("resolve_revincludes must succeed");

    db.run_command(doc! { "profile": 0_i32 })
        .await
        .expect("failed to disable profiling");

    assert_eq!(
        result.len(),
        12,
        "all 12 Observations should resolve under the default 1000 cap"
    );

    let ns = format!("{}.search_index", db.name());
    let profile = db.collection::<Document>("system.profile");
    let distinct_count = profile
        .count_documents(doc! { "ns": ns.as_str(), "command.distinct": "search_index" })
        .await
        .expect("failed to count distinct commands in system.profile");
    let find_count = profile
        .count_documents(doc! { "ns": ns.as_str(), "command.find": "search_index" })
        .await
        .expect("failed to count find commands in system.profile");

    assert_eq!(
        distinct_count, 0,
        "resolve_revincludes must not issue `distinct` against search_index"
    );
    assert!(
        find_count >= 1,
        "resolve_revincludes must issue `find` against search_index (find_count=0 would mean \
         profiling silently captured nothing, making the distinct_count==0 assertion vacuous)"
    );
}

/// #1061 part (b): the referring set must be bounded at
/// `max_included_resources`, applied per directive, with truncation signaled
/// by a synthetic `OperationOutcome` (`search.mode = outcome`) rather than
/// silently dropped — proven both through the unsorted `search()` path and
/// through the fenced `search_param_sorted` path (#1040/#1056), which carries
/// its own duplicate copy of the include-resolution block and must not be
/// missed by this fix.
#[tokio::test]
async fn mongodb_revinclude_caps_included_and_signals_truncation() {
    let Some(backend) =
        create_backend_with_full_registry_and_cap("revinclude_cap_truncation", 5).await
    else {
        eprintln!(
            "Skipping mongodb_revinclude_caps_included_and_signals_truncation (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-revinclude-cap");

    let patient = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Capped"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    for i in 0..12 {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "subject": {"reference": format!("Patient/{}", patient.id())},
                    "code": {"coding": [{"code": format!("cap-{i}")}]}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let revinclude = IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: None,
        iterate: false,
    };
    let id_filter = SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(patient.id())],
        chain: vec![],
        components: vec![],
    };

    let query = SearchQuery::new("Patient")
        .with_parameter(id_filter.clone())
        .with_include(revinclude.clone());
    let result = backend
        .search(&tenant, &query)
        .await
        .expect("search with _revinclude must succeed");

    assert!(
        result
            .resources
            .items
            .iter()
            .any(|r| r.id() == patient.id()),
        "primary page must still contain the Patient"
    );

    let observation_count = result
        .included
        .iter()
        .filter(|r| r.resource_type() == "Observation")
        .count();
    assert_eq!(
        observation_count, 5,
        "included Observations must be capped at max_included_resources (5), not all 12"
    );

    let outcomes: Vec<_> = result
        .included
        .iter()
        .filter(|r| r.resource_type() == "OperationOutcome")
        .collect();
    assert_eq!(
        outcomes.len(),
        1,
        "exactly one truncation marker, got: {:?}",
        result
            .included
            .iter()
            .map(|r| format!("{}/{}", r.resource_type(), r.id()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        outcomes[0].id(),
        helios_persistence::core::INCLUDE_TRUNCATION_OUTCOME_ID
    );
    let issue = &outcomes[0].content()["issue"][0];
    assert_eq!(issue["severity"], "warning");
    let diagnostics = issue["diagnostics"].as_str().expect("diagnostics text");
    assert!(
        diagnostics.contains('5'),
        "diagnostics should name the limit (5): {diagnostics}"
    );
    assert!(
        diagnostics.contains("HFS_MONGODB_MAX_INCLUDED_RESOURCES"),
        "diagnostics should name the env var: {diagnostics}"
    );

    // The fenced `search_param_sorted` path (lines ~727-831) carries its own
    // duplicate include-resolution block — prove the cap and the marker
    // apply there too, since this fix must not touch that fenced code to get
    // it right.
    let sorted_query = SearchQuery::new("Patient")
        .with_parameter(id_filter)
        .with_include(revinclude)
        .with_sort(SortDirective::parse("birthdate").with_param_type(Some(SearchParamType::Date)));
    let sorted_result = backend
        .search(&tenant, &sorted_query)
        .await
        .expect("sorted search with _revinclude must succeed");
    let sorted_observation_count = sorted_result
        .included
        .iter()
        .filter(|r| r.resource_type() == "Observation")
        .count();
    assert_eq!(
        sorted_observation_count, 5,
        "the sorted path (search_param_sorted) must apply the same per-directive cap"
    );
    let sorted_outcome_count = sorted_result
        .included
        .iter()
        .filter(|r| r.resource_type() == "OperationOutcome")
        .count();
    assert_eq!(
        sorted_outcome_count, 1,
        "the sorted path must carry the truncation marker too"
    );
}

/// #1063: MongoDB's inline `resolve_includes` never even looks at
/// `IncludeDirective::iterate` — it applies every directive against the
/// SAME primary result set on every pass, so a directive whose `source_type`
/// only matches a resource introduced by an earlier hop (here: `Patient`,
/// produced by hop 1) contributes nothing. This proves both halves of that:
/// MongoDB's own `search()` stops at hop 1, and composing it with
/// `resolve_includes_iterate_continuation` — the composition the REST guard
/// performs — reaches hop 2 without re-returning hop 1's Patient.
#[tokio::test]
async fn mongodb_include_iterate_follows_the_second_hop() {
    let Some(backend) = create_backend_with_full_registry("include_iterate_second_hop").await
    else {
        eprintln!(
            "Skipping mongodb_include_iterate_follows_the_second_hop (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-include-iterate");

    let (org, _) = backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-iter-1",
            json!({"resourceType": "Organization", "id": "org-iter-1", "name": "Iterate Org"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let (patient, _) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-iter-1",
            json!({
                "resourceType": "Patient",
                "id": "patient-iter-1",
                "name": [{"family": "Iterate"}],
                "managingOrganization": {"reference": "Organization/org-iter-1"}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let observation = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "subject": {"reference": "Patient/patient-iter-1"},
                "code": {"coding": [{"code": "iter"}]}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let includes = vec![
        IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        },
        IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: Some("Organization".to_string()),
            iterate: true,
        },
    ];

    let mut query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(observation.id())],
        chain: vec![],
        components: vec![],
    });
    for directive in &includes {
        query = query.with_include(directive.clone());
    }

    let result = backend
        .search(&tenant, &query)
        .await
        .expect("search must succeed");

    assert!(
        result
            .included
            .iter()
            .any(|r| r.resource_type() == "Patient"),
        "hop 1 (Observation:subject) should resolve inline"
    );
    assert!(
        !result
            .included
            .iter()
            .any(|r| r.resource_type() == "Organization"),
        "MongoDB's inline resolution must NOT itself chase :iterate hops — that's what makes \
         this the REST guard's job (#1063)"
    );

    // The composition `execute_search_bundle`'s `IterativePass::Continuation`
    // branch performs.
    let continuation = helios_persistence::core::resolve_includes_iterate_continuation(
        &backend,
        &tenant,
        &result.resources.items,
        &includes,
        &result.included,
    )
    .await
    .expect("continuation must succeed");

    assert_eq!(
        continuation.len(),
        1,
        "continuation should return exactly the Organization, got: {:?}",
        continuation
            .iter()
            .map(|r| format!("{}/{}", r.resource_type(), r.id()))
            .collect::<Vec<_>>()
    );
    assert_eq!(continuation[0].resource_type(), "Organization");
    assert_eq!(continuation[0].id(), org.id());

    let mut all_included: Vec<String> = result
        .included
        .iter()
        .map(|r| format!("{}/{}", r.resource_type(), r.id()))
        .collect();
    all_included.extend(
        continuation
            .iter()
            .map(|r| format!("{}/{}", r.resource_type(), r.id())),
    );
    let unique: std::collections::HashSet<&String> = all_included.iter().collect();
    assert_eq!(
        all_included.len(),
        unique.len(),
        "no duplicates across hop 1 + continuation: {all_included:?}"
    );
    assert!(all_included.contains(&format!("Patient/{}", patient.id())));
    assert!(all_included.contains(&format!("Organization/{}", org.id())));
}

// The unreachable-server test that used to live here now sits alongside the same
// contract for every other backend, in `tests/backend_error_handling.rs`. It needs
// no server, so it did not belong in a suite whose tests all skip without one —
// and its `mongodb_integration_*` name implied a Docker dependency it never had.

// ============================================================================
// Per-user settings store
// ============================================================================

/// A user key unique to each test, so tests sharing a database don't collide on
/// the single-document-per-user `user_settings` collection.
fn unique_user_key(prefix: &str) -> String {
    format!("{}|{}", prefix, uuid::Uuid::new_v4().simple())
}

/// Backend provisioning for the settings-store tests.
///
/// Delegates to the suite-wide [`super::create_backend`] so these tests share
/// the single, memory-capped Mongo container (and its pool cap + init gate)
/// rather than starting a *second* standalone container in the same test
/// binary. Two containers doubled the footprint on the shared CI docker host —
/// and this one was uncapped, so its WiredTiger cache sized to ~50% of host RAM
/// — which contributed to mongod being OOM-killed mid-run. The settings store
/// uses version-conditioned writes rather than multi-document transactions, so
/// a standalone (non replica-set) Mongo is sufficient. When neither a URL nor
/// Docker is available the tests skip.
mod settings_mongo {
    use super::MongoBackend;

    /// Returns a schema-initialised backend against the shared mongo, or `None`
    /// when no mongo is available (skip).
    pub(super) async fn backend(test_name: &str) -> Option<MongoBackend> {
        super::create_backend(test_name).await
    }
}

/// `delete_settings` removes the document and reports whether one existed —
/// the primitive the #270 legacy-key migration uses to move a document rather
/// than leave a duplicate copy behind.
#[tokio::test]
async fn mongodb_integration_settings_delete_is_idempotent() {
    let Some(backend) = settings_mongo::backend("settings_delete").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_delete_is_idempotent (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("delete");

    // Absent is not an error, and reports "nothing removed".
    assert!(!backend.delete_settings(&user).await.unwrap());

    backend
        .put_settings(&user, json!({"theme": "dark"}), None)
        .await
        .unwrap();
    assert!(backend.get_settings(&user).await.unwrap().is_some());

    assert!(backend.delete_settings(&user).await.unwrap());
    assert!(backend.get_settings(&user).await.unwrap().is_none());
    assert!(!backend.delete_settings(&user).await.unwrap());
}

#[tokio::test]
async fn mongodb_integration_settings_get_missing_is_none() {
    let Some(backend) = settings_mongo::backend("settings_missing").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_get_missing_is_none (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("missing");
    assert!(backend.get_settings(&user).await.unwrap().is_none());
}

#[tokio::test]
async fn mongodb_integration_settings_put_get_and_version() {
    let Some(backend) = settings_mongo::backend("settings_round_trip").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_put_get_and_version (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("round-trip");
    let doc = json!({"theme": "dark", "recentQueries": {"Patient": ["name=smith"]}});

    let stored = backend
        .put_settings(&user, doc.clone(), None)
        .await
        .unwrap();
    assert_eq!(stored.version, 1);

    let fetched = backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(fetched.document, doc);
    assert_eq!(fetched.version, 1);

    // A second unconditional write replaces the document and bumps the version.
    let second = backend
        .put_settings(&user, json!({"theme": "light"}), None)
        .await
        .unwrap();
    assert_eq!(second.version, 2);
    assert_eq!(second.document, json!({"theme": "light"}));
}

#[tokio::test]
async fn mongodb_integration_settings_patch_merges_and_deletes() {
    let Some(backend) = settings_mongo::backend("settings_patch").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_patch_merges_and_deletes (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("patch");
    backend
        .put_settings(
            &user,
            json!({"theme": "dark", "defaultTenant": "acme"}),
            None,
        )
        .await
        .unwrap();

    let patched = backend
        .patch_settings(
            &user,
            json!({"theme": "light", "defaultTenant": null}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(patched.document, json!({"theme": "light"}));
    assert_eq!(patched.version, 2);
}

#[tokio::test]
async fn mongodb_integration_settings_patch_on_missing_creates_document() {
    let Some(backend) = settings_mongo::backend("settings_patch_create").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_patch_on_missing_creates_document (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("patch-create");
    let patched = backend
        .patch_settings(&user, json!({"theme": "dark"}), None)
        .await
        .unwrap();
    assert_eq!(patched.document, json!({"theme": "dark"}));
    assert_eq!(patched.version, 1);
}

#[tokio::test]
async fn mongodb_integration_settings_optimistic_lock() {
    let Some(backend) = settings_mongo::backend("settings_lock").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_optimistic_lock (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("lock");

    // `Some(0)` asserts "does not exist yet" — succeeds for the first write.
    backend
        .put_settings(&user, json!({"a": 1}), Some(0))
        .await
        .unwrap(); // version 1

    // A stale precondition against an existing document is rejected.
    let err = backend
        .put_settings(&user, json!({"a": 2}), Some(0))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
    ));

    // A matching precondition succeeds and bumps the version.
    let ok = backend
        .put_settings(&user, json!({"a": 2}), Some(1))
        .await
        .unwrap();
    assert_eq!(ok.version, 2);
}

#[tokio::test]
async fn mongodb_integration_settings_concurrent_patches_serialize() {
    let Some(backend) = settings_mongo::backend("settings_concurrent").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_concurrent_patches_serialize (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let backend = Arc::new(backend);
    let user = unique_user_key("concurrent");

    // Deliberately do NOT seed a document: the racers start from version 0, so
    // exactly one wins the initial insert and the rest hit the unique-index
    // duplicate-key path and retry into the version-conditioned update. This
    // exercises both the insert race and the update race in one test.
    //
    // Fire many unconditional single-key merge-patches concurrently. The
    // version-conditioned write + retry loop must serialize them so every key
    // survives (no lost updates) despite the read-modify-write race.
    let mut handles = Vec::new();
    for i in 0..12 {
        let backend = backend.clone();
        let user = user.clone();
        handles.push(tokio::spawn(async move {
            backend
                .patch_settings(&user, json!({ format!("k{i}"): i }), None)
                .await
        }));
    }
    for h in handles {
        match h.await.expect("patch task panicked") {
            Ok(_) => {}
            // The container died mid-test (it survived the backend's transient
            // retry). That is infra, not a serialization bug — which would show
            // up as a lost update in the assertions below — so skip.
            Err(err) if storage_err_is_mongo_unavailable(&err) => {
                eprintln!(
                    "Skipping mongodb_integration_settings_concurrent_patches_serialize \
                     (shared mongo died mid-test: {err:?})"
                );
                return;
            }
            Err(err) => panic!("patch_settings failed: {err:?}"),
        }
    }

    let final_doc = match backend.get_settings(&user).await {
        Ok(Some(doc)) => doc,
        Ok(None) => panic!("settings document missing after 12 successful patches"),
        Err(err) if storage_err_is_mongo_unavailable(&err) => {
            eprintln!(
                "Skipping mongodb_integration_settings_concurrent_patches_serialize \
                 (shared mongo died mid-test: {err:?})"
            );
            return;
        }
        Err(err) => panic!("get_settings failed: {err:?}"),
    };
    let obj = final_doc.document.as_object().unwrap();
    for i in 0..12 {
        assert_eq!(
            obj.get(&format!("k{i}")),
            Some(&json!(i)),
            "key k{i} was lost to a read-modify-write race"
        );
    }
    // 12 patches, each a distinct successful write from version 0 upward.
    assert_eq!(final_doc.version, 12);
}

/// A row whose `data` is not valid JSON must surface a backend error on read,
/// rather than panicking or silently returning an empty document. Mirrors the
/// SQLite/PostgreSQL `user_settings` decode-error coverage.
#[tokio::test]
async fn mongodb_integration_settings_get_surfaces_decode_error() {
    let Some(backend) = settings_mongo::backend("settings_decode_err").await else {
        eprintln!(
            "Skipping mongodb_integration_settings_get_surfaces_decode_error (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("corrupt");

    // Write a row whose `data` blob is not valid JSON, bypassing the store.
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("connect raw mongo client");
    let db = client.database(&backend.config().database_name);
    db.collection::<Document>("user_settings")
        .insert_one(doc! {
            "user_key": &user,
            "data": "not json",
            "version": 1_i64,
            "updated_at": mongodb::bson::DateTime::from_millis(0),
        })
        .await
        .expect("insert corrupt user_settings row");

    let err = backend.get_settings(&user).await.unwrap_err();
    assert!(matches!(err, StorageError::Backend(_)));
}

/// Issue #313: a tenant purge must reach the PHI-derived query strings a client
/// stores in its settings document, which are keyed by *user* and so are not
/// touched by the tenant-scoped deletes in `purge_tenant_data`.
///
/// The MongoDB-specific risk this proves is the write shape: with no
/// multi-document transaction available, the sweep rewrites each document with a
/// version-conditioned update, and it must remove exactly the named tenant's
/// subtree — which is why it edits the parsed document rather than `$unset`ing a
/// dotted path (`admin_tenants::validate_tenant_id` permits tenant ids containing
/// `.`, which a dotted path would misparse).
#[tokio::test]
async fn mongodb_integration_purge_tenant_settings() {
    let Some(backend) = settings_mongo::backend("settings_tenant_purge").await else {
        eprintln!(
            "Skipping mongodb_integration_purge_tenant_settings (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let user = unique_user_key("tenant-purge");
    let dotted = unique_user_key("tenant-purge-dotted");

    backend
        .put_settings(
            &user,
            json!({
                "theme": "dark",
                "byTenant": {
                    "acme": {"savedQueries": {"Patient": {"q": {"query": "name=smith"}}}},
                    "beta": {"savedQueries": {"Patient": {"q": {"query": "name=jones"}}}}
                }
            }),
            None,
        )
        .await
        .unwrap();
    // A tenant id containing a dot: `$unset: {"byTenant.a.b": 1}` would target
    // `byTenant.a` → `b`, the wrong node.
    backend
        .put_settings(
            &dotted,
            json!({"byTenant": {"a.b": {"savedQueries": {"Patient": {"q": {}}}}}}),
            None,
        )
        .await
        .unwrap();

    let before = backend.get_settings(&user).await.unwrap().unwrap();
    backend.purge_tenant_settings("acme").await.unwrap();

    let after = backend.get_settings(&user).await.unwrap().unwrap();
    assert_eq!(after.document["theme"], "dark");
    assert!(after.document["byTenant"].get("acme").is_none());
    assert_eq!(
        after.document["byTenant"]["beta"]["savedQueries"]["Patient"]["q"]["query"],
        "name=jones"
    );
    assert!(
        !serde_json::to_string(&after.document)
            .unwrap()
            .contains("smith"),
        "purged content must not survive in the stored document"
    );
    assert_eq!(
        after.version,
        before.version + 1,
        "the version must bump so a stale ETag cannot write the content back"
    );

    // The dotted tenant is purgeable, and only by its own id.
    backend.purge_tenant_settings("a").await.unwrap();
    let dotted_doc = backend.get_settings(&dotted).await.unwrap().unwrap();
    assert!(
        dotted_doc.document["byTenant"].get("a.b").is_some(),
        "purging 'a' must not touch the tenant literally named 'a.b'"
    );
    backend.purge_tenant_settings("a.b").await.unwrap();
    let dotted_doc = backend.get_settings(&dotted).await.unwrap().unwrap();
    assert_eq!(dotted_doc.document, json!({}));
}

// ============================================================================
// Issue #447 — tenant-id fidelity
// ============================================================================
//
// The #447 defect is S3's: it *derives* a key prefix from the tenant id, and
// `trim_matches('/')` made that derivation many-to-one. MongoDB derives
// nothing — documents carry a `tenant_id` field matched exactly, under the
// default (case-sensitive) collation — so the scoping is the identity mapping
// and the defect cannot occur here.
//
// That is a code reading, and a code reading is exactly what let the same
// defect class sit undiscovered in the two backends that *do* derive (#384 on
// Elasticsearch, #447 on S3). So it is checked against a real server, where
// BSON matching and collation are the engine's behaviour rather than the Rust's.
//
// Each test gets its own database (`build_test_database_name`), so a fixed base
// id is safe here.

#[tokio::test]
async fn mongodb_integration_distinct_tenant_ids_never_share_data() {
    let Some(backend) = create_backend("tenant_fidelity").await else {
        eprintln!(
            "Skipping mongodb_integration_distinct_tenant_ids_never_share_data (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    tenant_id_fidelity_suite::distinct_tenant_ids_never_share_data(&backend, "acme").await;
}

#[tokio::test]
async fn mongodb_integration_purging_one_tenant_leaves_the_look_alikes_intact() {
    let Some(backend) = create_backend("tenant_fidelity_purge").await else {
        eprintln!(
            "Skipping mongodb_integration_purging_one_tenant_leaves_the_look_alikes_intact (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    tenant_id_fidelity_suite::purging_one_tenant_leaves_the_look_alikes_intact(&backend, "acme")
        .await;
}

// ===========================================================================
// Bulk Data Submit ($bulk-submit)
//
// MongoDB hosts the whole submit surface — the ingestion engine and the REST
// worker's job store — so these cover both halves: ingest through the streaming
// engine, and claim/heartbeat/fence/finish through the lease layer.
// ===========================================================================

mod bulk_submit {
    use super::*;

    use helios_persistence::core::bulk_submit::{CANCELLED_ABORT_REASON, CancelToken};
    use helios_persistence::core::{
        BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider, ChangeType,
        DefaultSubmitWorker, IMPORT_MODE_PARAMETER_URL, LeaseError, ManifestFetchParams,
        ManifestPublicationStatus, ManifestStatus, NdjsonEntry, RemoteFile, RemoteManifest,
        StreamingBulkSubmitProvider, SubmissionId, SubmissionStatus, SubmitClaimStrategy,
        SubmitFileRecord, SubmitInputFetcher, SubmitWorkerStorage, WorkerId,
    };
    use helios_persistence::error::StorageResult;
    use std::collections::HashMap;
    use std::time::Duration;

    fn lease_duration() -> Duration {
        Duration::from_secs(60)
    }

    /// Creates a submission with one fetchable manifest — the shape the REST
    /// kickoff handler produces.
    async fn seed(backend: &MongoBackend, tenant: &TenantContext) -> (SubmissionId, String) {
        let id = SubmissionId::generate("data-provider");
        backend.create_submission(tenant, &id, None).await.unwrap();
        let manifest = backend
            .add_manifest(
                tenant,
                &id,
                Some("https://provider.example/manifest.json"),
                None,
            )
            .await
            .unwrap();
        (id, manifest.manifest_id)
    }

    /// Serves a fixed manifest and its NDJSON files from memory.
    struct MockFetcher {
        manifest: RemoteManifest,
        files: HashMap<String, Vec<u8>>,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for MockFetcher {
        async fn fetch_manifest(
            &self,
            _url: &str,
            _headers: &[(String, String)],
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<RemoteManifest> {
            Ok(self.manifest.clone())
        }

        async fn open_file_stream(
            &self,
            url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            let data = self.files.get(url).cloned().unwrap_or_default();
            let len = data.len() as u64;
            Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
                Some(len),
            ))
        }
    }

    #[tokio::test]
    async fn test_process_entries_creates_and_updates() {
        let Some(backend) = create_backend("submit_process_entries").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "gender": "female"}),
            ),
            NdjsonEntry::new(2, "Patient", json!({"resourceType": "Patient"})),
        ];
        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_success()));
        assert!(results[0].created && results[1].created);

        // The resource really landed in the FHIR store, not just the job store.
        let stored = backend
            .read(&tenant, "Patient", "p1")
            .await
            .unwrap()
            .expect("submitted Patient is readable");
        assert_eq!(stored.content()["gender"], json!("female"));

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.success, 2);
        assert_eq!(counts.total, 2);

        // Re-submitting the same id is an update, and the change log records it
        // so an abort can roll it back.
        let update = vec![NdjsonEntry::new(
            1,
            "Patient",
            json!({"resourceType": "Patient", "id": "p1", "gender": "male"}),
        )];
        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                update,
                &BulkProcessingOptions::new()
                    .with_file_url("https://provider.example/second.ndjson"),
            )
            .await
            .unwrap();
        assert!(results[0].is_success() && !results[0].created);

        let changes = backend.list_changes(&tenant, &id, 100, 0).await.unwrap();
        assert!(
            changes.iter().any(|c| c.change_type == ChangeType::Update),
            "an update must be recorded for rollback, got {changes:?}"
        );
    }

    /// `HFS_BULK_SUBMIT_DEFER_INDEXING` (#903) was a no-op on this backend: the
    /// per-entry path reached the search index through `create`/`update`, which
    /// have no way to be told to skip it, so a "fast load" still wrote ~21
    /// `search_index` documents per resource (#1000). The batched ingest owns
    /// the index write, so the switch now actually defers it — and the resource,
    /// its history and its receipt still land, because deferring is only ever
    /// about the derived index.
    #[tokio::test]
    async fn test_defer_indexing_skips_the_search_index_but_stores_everything_else() {
        let Some(backend) = create_backend("submit_defer_indexing").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": "deferred",
                        "name": [{"family": "Deferred"}]
                    }),
                )],
                &BulkProcessingOptions::new().with_defer_indexing(true),
            )
            .await
            .unwrap();
        assert!(results[0].is_success());

        assert_eq!(
            search_index_entry_count(&backend, &tenant, "Patient", "deferred").await,
            0,
            "deferred indexing must not write search_index documents"
        );
        assert!(
            backend
                .read(&tenant, "Patient", "deferred")
                .await
                .unwrap()
                .is_some(),
            "the resource itself is stored either way"
        );
        assert_eq!(
            backend
                .list_versions(&tenant, "Patient", "deferred")
                .await
                .unwrap()
                .len(),
            1,
            "history is stored either way"
        );

        // The default still indexes, so the switch is what makes the difference.
        backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    2,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": "indexed",
                        "name": [{"family": "Indexed"}]
                    }),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert!(
            search_index_entry_count(&backend, &tenant, "Patient", "indexed").await > 0,
            "without the switch the batch still indexes inline"
        );
    }

    /// A batch is planned against an overlay of its own staged writes, so an id
    /// repeated inside one batch versions forward exactly as it did when every
    /// entry was its own round trip: one history row per entry, the last
    /// entry's content stored, and one rollback record per entry.
    #[tokio::test]
    async fn test_repeated_id_in_one_batch_versions_forward() {
        let Some(backend) = create_backend("submit_repeated_id").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let entries = (1..=3)
            .map(|n| {
                NdjsonEntry::new(
                    n,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": "repeated",
                        "gender": if n == 3 { "male" } else { "female" },
                    }),
                )
            })
            .collect();
        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_success()));
        assert!(
            results[0].created && !results[1].created && !results[2].created,
            "only the first entry creates, got {results:?}"
        );

        let stored = backend
            .read(&tenant, "Patient", "repeated")
            .await
            .unwrap()
            .expect("the repeated id is stored once");
        assert_eq!(stored.version_id(), "3");
        assert_eq!(stored.content()["gender"], json!("male"));

        let mut versions = backend
            .list_versions(&tenant, "Patient", "repeated")
            .await
            .unwrap();
        versions.sort();
        assert_eq!(
            versions,
            vec!["1".to_string(), "2".to_string(), "3".to_string()],
            "every entry contributes its own history version"
        );

        let changes = backend.list_changes(&tenant, &id, 100, 0).await.unwrap();
        assert_eq!(
            changes.len(),
            3,
            "the rollback log records one change per entry, got {changes:?}"
        );
        assert_eq!(
            changes
                .iter()
                .filter(|c| c.change_type == ChangeType::Create)
                .count(),
            1
        );

        // The index reflects the final version only, not one row set per entry.
        let indexed = search_index_entry_count(&backend, &tenant, "Patient", "repeated").await;
        backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    4,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "single", "gender": "male"}),
                )],
                &BulkProcessingOptions::new().with_file_url("second.ndjson"),
            )
            .await
            .unwrap();
        assert_eq!(
            indexed,
            search_index_entry_count(&backend, &tenant, "Patient", "single").await,
            "three entries for one id leave the same index rows as one entry would"
        );
    }

    /// One bad entry must not take the rest of its batch down with it, and every
    /// entry — success, validation error or skip — still gets a receipt at its
    /// own line number.
    #[tokio::test]
    async fn test_a_mixed_batch_keeps_results_aligned_to_lines() {
        let Some(backend) = create_backend("submit_mixed_batch").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "ok-1"}),
                    ),
                    // Payload disagrees with the file's declared type.
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType": "Observation", "id": "wrong"}),
                    ),
                    NdjsonEntry::new(
                        3,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "ok-2"}),
                    ),
                ],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(
            results.iter().map(|r| r.line_number).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(results[0].is_success() && results[2].is_success());
        assert!(results[1].is_error(), "line 2 is a validation error");
        assert!(
            backend
                .read(&tenant, "Patient", "ok-2")
                .await
                .unwrap()
                .is_some(),
            "an entry after the bad one still lands"
        );

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 3);
        assert_eq!(counts.success, 2);
        assert_eq!(counts.validation_error, 1);
    }

    /// With `allow_updates` off, an id that already exists is skipped rather
    /// than versioned — and the pre-existing row keeps its content.
    #[tokio::test]
    async fn test_create_only_skips_existing_ids() {
        let Some(backend) = create_backend("submit_create_only").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "existing", "gender": "female"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "existing", "gender": "male"}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "fresh"}),
                    ),
                ],
                &BulkProcessingOptions::create_only(),
            )
            .await
            .unwrap();

        assert!(
            !results[0].is_success() && !results[0].is_error(),
            "skipped"
        );
        assert!(results[1].is_success() && results[1].created);
        let stored = backend
            .read(&tenant, "Patient", "existing")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.version_id(), "1");
        assert_eq!(stored.content()["gender"], json!("female"));
    }

    /// A tombstoned id is not an update target, and `create` refuses it too —
    /// its existence probe does not filter `is_deleted`, so it reports
    /// `AlreadyExists`. That is a pre-existing defect in the resource layer, not
    /// in the ingest; this pins the behaviour so the batched path is known to
    /// reproduce it rather than to have quietly changed it.
    #[tokio::test]
    async fn test_reimporting_a_deleted_resource_reports_already_exists() {
        let Some(backend) = create_backend("submit_deleted_reimport").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "tombstoned"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "tombstoned"}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert!(results[0].is_error());
        let diagnostics = results[0].operation_outcome.as_ref().unwrap()["issue"][0]["diagnostics"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            diagnostics.contains("already exists"),
            "expected an already-exists diagnostic, got {diagnostics}"
        );
    }

    /// A tenant that may not write the type gets a per-entry processing error,
    /// not a failed batch — the permission check is per entry, as it was when
    /// each entry called `create`/`update` for itself.
    #[tokio::test]
    async fn test_entries_the_tenant_may_not_write_fail_individually() {
        let Some(backend) = create_backend("submit_permission_denied").await else {
            return;
        };
        let writer = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &writer).await;
        let reader = TenantContext::new(writer.tenant_id().clone(), TenantPermissions::read_only());

        let results = backend
            .process_entries(
                &reader,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "forbidden"}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert!(results[0].is_error(), "a read-only tenant cannot create");
        assert!(
            backend
                .read(&writer, "Patient", "forbidden")
                .await
                .unwrap()
                .is_none(),
            "and nothing was written"
        );
    }

    /// `max_errors` with `continue_on_error` off aborts the batch — but the
    /// entries processed before the abort keep their receipts, which is what the
    /// per-entry path left behind and what a re-fetch relies on.
    #[tokio::test]
    async fn test_max_errors_aborts_the_batch_but_keeps_earlier_receipts() {
        let Some(backend) = create_backend("submit_max_errors_abort").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let outcome = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Observation", "id": "wrong"}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType": "Patient", "id": "never-reached"}),
                    ),
                ],
                &BulkProcessingOptions::strict(),
            )
            .await;
        assert!(
            matches!(
                outcome,
                Err(StorageError::BulkSubmit(
                    helios_persistence::error::BulkSubmitError::MaxErrorsExceeded { .. }
                ))
            ),
            "expected MaxErrorsExceeded, got {outcome:?}"
        );

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 1, "only the entry that ran has a receipt");
        assert_eq!(counts.validation_error, 1);
        assert!(
            backend
                .read(&tenant, "Patient", "never-reached")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// With `continue_on_error` on, everything past `max_errors` is recorded as
    /// skipped rather than attempted.
    #[tokio::test]
    async fn test_max_errors_with_continue_on_error_skips_the_rest() {
        let Some(backend) = create_backend("submit_max_errors_skip").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Observation", "id": "wrong"}),
                    ),
                    NdjsonEntry::new(2, "Patient", json!({"resourceType": "Patient", "id": "a"})),
                    NdjsonEntry::new(3, "Patient", json!({"resourceType": "Patient", "id": "b"})),
                ],
                &BulkProcessingOptions::new()
                    .with_max_errors(1)
                    .with_continue_on_error(true),
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        assert!(results[0].is_error());
        assert!(
            results[1..]
                .iter()
                .all(|r| !r.is_success() && !r.is_error()),
            "entries past max_errors are skipped, got {results:?}"
        );
        assert!(
            backend
                .read(&tenant, "Patient", "a")
                .await
                .unwrap()
                .is_none()
        );

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.skipped, 2);
    }

    /// Two batches ingesting the same ids at once — the shape
    /// `HFS_BULK_SUBMIT_FILE_CONCURRENCY > 1` produces when two of a manifest's
    /// files carry the same resource. Whichever loses the race must lose it
    /// cleanly: no failed call, no duplicated row, no receipt lost.
    ///
    /// Half the ids exist beforehand and half do not, so both collision paths
    /// are in play: two concurrent inserts of a new id (one loses on the unique
    /// index) and two concurrent updates of an existing one (one loses the
    /// version guard, which the batch resolves by re-reading).
    ///
    /// The assertions are invariants rather than an expected winner, so the test
    /// does not depend on how the two interleave.
    #[tokio::test]
    async fn test_two_batches_racing_for_the_same_ids_stay_consistent() {
        let Some(backend) = create_backend("submit_concurrent_same_ids").await else {
            return;
        };
        let backend = std::sync::Arc::new(backend);
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(backend.as_ref(), &tenant).await;

        const IDS: usize = 60;
        for n in (0..IDS).step_by(2) {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("racer-{n}")}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let entries = || {
            (0..IDS)
                .map(|n| {
                    NdjsonEntry::new(
                        n as u64 + 1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": format!("racer-{n}")}),
                    )
                })
                .collect::<Vec<_>>()
        };

        let mut handles = Vec::new();
        for file in ["race-a.ndjson", "race-b.ndjson"] {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let id = id.clone();
            let manifest_id = manifest_id.clone();
            let entries = entries();
            handles.push(tokio::spawn(async move {
                backend
                    .process_entries(
                        &tenant,
                        &id,
                        &manifest_id,
                        entries,
                        &BulkProcessingOptions::new().with_file_url(file),
                    )
                    .await
            }));
        }
        for handle in handles {
            let results = handle.await.unwrap().expect("a losing batch still returns");
            assert_eq!(results.len(), IDS);
        }

        for n in 0..IDS {
            let stored = backend
                .read(&tenant, "Patient", &format!("racer-{n}"))
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("racer-{n} must exist exactly once"));
            // Pre-existing ids start at 1 and can be bumped once or twice
            // depending on how the two batches interleaved; new ids start at 1.
            assert!(
                ["1", "2", "3"].contains(&stored.version_id()),
                "racer-{n} landed at version {}",
                stored.version_id()
            );
        }

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(
            counts.total,
            (IDS * 2) as u64,
            "every line of both files keeps its own receipt"
        );
    }

    /// #1007: `mark_entries_unindexed` flips only the named `(type, id)`
    /// entry results to `processing-error`, leaving the rest untouched, and
    /// is a no-op on an empty entry list.
    #[tokio::test]
    async fn mark_entries_unindexed_flips_only_the_named_resources() {
        use helios_persistence::core::{BulkEntryOutcome, UnindexedEntry};

        let Some(backend) = create_backend("submit_mark_unindexed").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "id": "mongo-unidx-1"}),
            ),
            NdjsonEntry::new(
                2,
                "Patient",
                json!({"resourceType": "Patient", "id": "mongo-unidx-2"}),
            ),
            NdjsonEntry::new(
                3,
                "Patient",
                json!({"resourceType": "Patient", "id": "mongo-unidx-3"}),
            ),
        ];
        let results = backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert!(results.iter().all(|r| r.is_success()));

        assert_eq!(
            backend
                .mark_entries_unindexed(&tenant, &id, &manifest_id, &[])
                .await
                .unwrap(),
            0,
            "an empty entry list is a no-op"
        );

        let oo = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "incomplete",
                "diagnostics": "Patient/mongo-unidx-2 was stored but could not be indexed for \
                                search on es: timeout. Run POST /Patient/$reindex to repair."
            }]
        });
        let changed = backend
            .mark_entries_unindexed(
                &tenant,
                &id,
                &manifest_id,
                &[UnindexedEntry {
                    resource_type: "Patient".to_string(),
                    resource_id: "mongo-unidx-2".to_string(),
                    operation_outcome: oo.clone(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(changed, 1);

        let page = backend
            .get_entry_results_page(&tenant, &id, &manifest_id, None, 10, None)
            .await
            .unwrap();
        for paged in &page.entries {
            let result = &paged.result;
            if result.resource_id.as_deref() == Some("mongo-unidx-2") {
                assert_eq!(result.outcome, BulkEntryOutcome::ProcessingError);
                assert_eq!(result.operation_outcome.as_ref(), Some(&oo));
            } else {
                assert_eq!(result.outcome, BulkEntryOutcome::Success);
            }
        }
    }

    /// Line numbers restart in every manifest output file, so the file is part
    /// of an entry result's identity (#457). Without it the second file's line 1
    /// overwrites the first file's.
    #[tokio::test]
    async fn test_entry_results_from_two_files_do_not_collide() {
        let Some(backend) = create_backend("submit_file_discriminator").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        for (file, patient) in [("a.ndjson", "pa"), ("b.ndjson", "pb")] {
            backend
                .process_entries(
                    &tenant,
                    &id,
                    &manifest_id,
                    vec![NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType": "Patient", "id": patient}),
                    )],
                    &BulkProcessingOptions::new().with_file_url(file),
                )
                .await
                .unwrap();
        }

        let counts = backend
            .get_entry_counts(&tenant, &id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(
            counts.total, 2,
            "both files' line 1 must survive as separate results"
        );
        let mut next = None;
        let mut ids = Vec::new();
        for page_index in 0..3 {
            let page = backend
                .get_entry_results_page(&tenant, &id, &manifest_id, None, 1, next.as_ref())
                .await
                .unwrap();
            assert!(
                page.entries
                    .iter()
                    .all(|entry| entry.stored_identity.is_none())
            );
            ids.extend(
                page.entries
                    .into_iter()
                    .map(|entry| entry.result.resource_id.unwrap()),
            );
            next = page.next;
            if page_index < 2 {
                assert_eq!(
                    next,
                    Some(helios_persistence::core::EntryResultContinuation::Offset(
                        page_index + 1
                    ))
                );
            } else {
                assert!(next.is_none(), "exact multiple must terminate");
            }
        }
        assert_eq!(ids, ["pa", "pb"]);
        assert!(
            backend
                .get_entry_results_page(&tenant, &id, &manifest_id, None, 0, None)
                .await
                .is_err()
        );
        assert!(
            backend
                .get_entry_results_page(
                    &tenant,
                    &id,
                    &manifest_id,
                    None,
                    1,
                    Some(&helios_persistence::core::EntryResultContinuation::Keyset(
                        helios_persistence::core::EntryResultCursor {
                            file_url: String::new(),
                            line_number: 0,
                        }
                    ))
                )
                .await
                .is_err()
        );
        let primary = Arc::new(backend);
        let config = helios_persistence::composite::config::CompositeConfig::builder()
            .primary("mongo", BackendKind::MongoDB)
            .build()
            .unwrap();
        let storage = Arc::new(
            helios_persistence::composite::storage::CompositeStorage::new(
                config,
                std::collections::HashMap::from([(
                    "mongo".to_string(),
                    primary.clone() as helios_persistence::composite::storage::DynStorage,
                )]),
            )
            .unwrap(),
        );
        let jobs =
            helios_persistence::composite::bulk_submit::CompositeSubmitJobs::new(primary, storage);
        let delegated = jobs
            .get_entry_results_page(&tenant, &id, &manifest_id, None, 1, None)
            .await
            .unwrap();
        assert!(delegated.entries[0].stored_identity.is_none());
        assert_eq!(
            delegated.next,
            Some(helios_persistence::core::EntryResultContinuation::Offset(1))
        );
        eprintln!("Verified MongoDB bulk-submit receipt pagination and Composite delegation");
    }

    /// The manifest counters are cumulative across every run of a manifest, so
    /// both writers into them — the worker's `add_manifest_progress` and the
    /// ingestion engine's per-batch bookkeeping — must add rather than assign.
    /// Assigning would stomp the other writer and walk the status endpoint's
    /// numbers backwards on resume (#969). The SQLite backend pins the same
    /// invariant in `test_progress_counters_only_move_forward`; this is its
    /// MongoDB twin, guarding the `$inc` documents.
    #[tokio::test]
    async fn test_progress_counters_only_move_forward() {
        let Some(backend) = create_backend("submit_progress_accumulates").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .unwrap()
            .expect("the seeded manifest is claimable");

        // The worker's own contributions accumulate across calls...
        backend
            .add_manifest_progress(&lease, 5, 1, 6)
            .await
            .unwrap();
        backend
            .add_manifest_progress(&lease, 2, 0, 2)
            .await
            .unwrap();
        assert_eq!(
            backend
                .get_manifest_for_worker(&lease)
                .await
                .unwrap()
                .last_processed_line,
            8
        );

        // ...and the ingestion engine's per-batch bookkeeping adds on top of
        // them instead of replacing them.
        backend
            .process_entries(
                &tenant,
                &id,
                &manifest_id,
                vec![
                    NdjsonEntry::new(1, "Patient", json!({"resourceType": "Patient"})),
                    NdjsonEntry::new(2, "Patient", json!({"resourceType": "Patient"})),
                ],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        let manifests = backend.list_manifests(&tenant, &id).await.unwrap();
        assert_eq!(manifests[0].processed_entries, 9);
        assert_eq!(manifests[0].failed_entries, 1);
        // `add_manifest_progress` never touches `total_entries`, so only the two
        // batched entries are counted here.
        assert_eq!(manifests[0].total_entries, 2);
        assert_eq!(
            backend
                .get_manifest_for_worker(&lease)
                .await
                .unwrap()
                .last_processed_line,
            10
        );
    }

    #[tokio::test]
    async fn test_claim_heartbeat_and_finish() {
        let Some(backend) = create_backend("submit_claim_lifecycle").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let worker = WorkerId::new("worker-1");
        let lease = backend
            .claim_next_manifest(&worker, lease_duration())
            .await
            .unwrap()
            .expect("the seeded manifest is claimable");
        assert_eq!(lease.manifest_id, manifest_id);
        assert_eq!(lease.fencing_token, 1, "the first claim bumps 0 -> 1");

        // A second worker cannot take a manifest that is still leased.
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-2"), lease_duration())
                .await
                .unwrap()
                .is_none(),
            "a live lease must not be claimable by another worker"
        );

        backend.heartbeat(&lease).await.unwrap();
        backend.finish_manifest(&lease).await.unwrap();

        let manifests = backend.list_manifests(&tenant, &id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        assert!(
            backend
                .claim_next_manifest(&worker, lease_duration())
                .await
                .unwrap()
                .is_none(),
            "a completed manifest must leave the queue"
        );
    }

    #[tokio::test]
    async fn test_fencing_blocks_a_zombie_worker() {
        let Some(backend) = create_backend("submit_fencing").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let _ = seed(&backend, &tenant).await;

        let stale = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .unwrap()
            .unwrap();
        // Release puts it back; the next claim bumps the token past the stale one.
        // Disambiguated because `Backend::release` (connection pooling) shares
        // the name with the lease release.
        SubmitClaimStrategy::release(&backend, stale.clone())
            .await
            .unwrap();
        let fresh = backend
            .claim_next_manifest(&WorkerId::new("worker-2"), lease_duration())
            .await
            .unwrap()
            .unwrap();
        assert!(fresh.fencing_token > stale.fencing_token);

        for outcome in [
            backend
                .heartbeat(&stale)
                .await
                .err()
                .map(|e| format!("{e:?}")),
            backend
                .add_manifest_progress(&stale, 1, 0, 1)
                .await
                .err()
                .map(|e| format!("{e:?}")),
            backend
                .finish_manifest(&stale)
                .await
                .err()
                .map(|e| format!("{e:?}")),
        ] {
            let message = outcome.expect("a zombie worker's fenced write must fail");
            assert!(
                message.contains("LeaseLost"),
                "expected LeaseLost, got {message}"
            );
        }
    }

    #[tokio::test]
    async fn test_poll_token_and_kickoff_metadata() {
        let Some(backend) = create_backend("submit_poll_token").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, _manifest_id) = seed(&backend, &tenant).await;

        backend
            .set_submission_kickoff_meta(
                &tenant,
                &id,
                Some("client-subject"),
                "https://hfs.example/$bulk-submit",
                true,
            )
            .await
            .unwrap();

        let token = backend.ensure_poll_token(&tenant, &id).await.unwrap();
        assert_eq!(
            backend.ensure_poll_token(&tenant, &id).await.unwrap(),
            token,
            "ensure_poll_token must be idempotent"
        );

        let target = backend
            .resolve_poll_token(&token)
            .await
            .unwrap()
            .expect("the token resolves to its submission");
        assert_eq!(target.submission_id, id);
        assert_eq!(target.tenant.tenant_id().as_str(), "submit-tenant");
        assert_eq!(target.owner_subject.as_deref(), Some("client-subject"));

        // A transaction time is minted once and then stable.
        let first = backend.ensure_transaction_time(&tenant, &id).await.unwrap();
        assert_eq!(
            backend.ensure_transaction_time(&tenant, &id).await.unwrap(),
            first
        );

        backend.clear_poll_token(&tenant, &id).await.unwrap();
        assert!(
            backend.resolve_poll_token(&token).await.unwrap().is_none(),
            "a cleared token must stop resolving, so a deleted submission 404s"
        );
    }

    #[tokio::test]
    async fn test_submit_files_are_recorded_and_deletable() {
        let Some(backend) = create_backend("submit_files").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, _manifest_id) = seed(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .unwrap()
            .unwrap();
        let record = SubmitFileRecord {
            manifest_url: Some("https://provider.example/manifest.json".to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "tenant/job/output/Patient-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 120,
            count_severity: None,
        };
        backend.record_submit_file(&lease, &record).await.unwrap();
        // Re-recording the same artifact must not double it in the manifest.
        backend.record_submit_file(&lease, &record).await.unwrap();

        let rows = backend.list_submit_files(&tenant, &id).await.unwrap();
        assert_eq!(rows.len(), 1, "record_submit_file must be idempotent");
        assert_eq!(rows[0].line_count, 3);
        assert_eq!(rows[0].fencing_token, lease.fencing_token);

        backend
            .delete_submission_artifacts(&tenant, &id)
            .await
            .unwrap();
        assert!(
            backend
                .list_submit_files(&tenant, &id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_active_submission_count_and_expiry_scan() {
        let Some(backend) = create_backend("submit_counts").await else {
            return;
        };
        let tenant = create_tenant("submit-counts");
        let (id, _manifest_id) = seed(&backend, &tenant).await;

        assert_eq!(
            backend.count_active_submissions(&tenant).await.unwrap(),
            1,
            "an in-progress submission counts toward the per-tenant cap"
        );
        backend.complete_submission(&tenant, &id).await.unwrap();
        assert_eq!(
            backend.count_active_submissions(&tenant).await.unwrap(),
            0,
            "a completed submission must free its slot"
        );

        // A zero TTL makes every submission expired; a long one, none.
        let expired = backend
            .list_expired_submissions(chrono::Utc::now(), Duration::from_secs(0), 10)
            .await
            .unwrap();
        assert!(expired.iter().any(|(_, sub)| sub == &id));
        let fresh = backend
            .list_expired_submissions(chrono::Utc::now(), Duration::from_secs(86_400), 10)
            .await
            .unwrap();
        assert!(!fresh.iter().any(|(_, sub)| sub == &id));
    }

    #[tokio::test]
    async fn test_aborted_submission_is_not_claimable() {
        let Some(backend) = create_backend("submit_abort").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, _manifest_id) = seed(&backend, &tenant).await;

        backend
            .abort_submission(&tenant, &id, "provider cancelled")
            .await
            .unwrap();
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .unwrap()
                .is_none(),
            "an aborted submission's manifests must not be picked up"
        );

        let summary = backend.get_submission(&tenant, &id).await.unwrap().unwrap();
        assert_eq!(summary.status, SubmissionStatus::Aborted);
    }

    /// End-to-end through the shared worker: claim → fetch → ingest → artifacts.
    #[tokio::test]
    async fn test_worker_ingests_a_manifest_end_to_end() {
        let Some(backend) = create_backend("submit_worker_e2e").await else {
            return;
        };
        let backend = Arc::new(backend);
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;
        backend
            .set_manifest_fetch_params(
                &tenant,
                &id,
                &manifest_id,
                ManifestFetchParams {
                    import_directives: &[(
                        IMPORT_MODE_PARAMETER_URL.to_string(),
                        "replace".to_string(),
                    )],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"w1\",\"gender\":\"female\"}\n",
            "not-json\n"
        );
        let mut files = HashMap::new();
        files.insert(
            "https://provider.example/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "https://provider.example/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
            files,
        });

        let tmp = tempfile::tempdir().unwrap();
        let output = Arc::new(
            helios_persistence::backends::local_fs::LocalFsOutputStore::new(
                tmp.path().to_path_buf(),
                "http://localhost:8080",
            ),
        );
        let worker_id = WorkerId::new("e2e-worker");
        let worker = DefaultSubmitWorker::new(backend.clone(), fetcher, output, worker_id.clone());

        let lease = backend
            .claim_next_manifest(&worker_id, lease_duration())
            .await
            .unwrap()
            .unwrap();
        worker.run_job(lease).await.unwrap();

        // Partial success: the good line ingested, the malformed one counted.
        let stored = backend.read(&tenant, "Patient", "w1").await.unwrap();
        assert!(stored.is_some(), "the valid NDJSON line must be ingested");
        let manifests = backend.list_manifests(&tenant, &id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        assert!(manifests[0].failed_entries >= 1);

        let files = backend.list_submit_files(&tenant, &id).await.unwrap();
        assert!(
            files
                .iter()
                .any(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient")),
            "an output receipt must be recorded, got {files:?}"
        );
        assert!(
            files.iter().any(|f| f.file_type == "error"),
            "the malformed line must surface in the error artifact, got {files:?}"
        );
    }

    /// A status-only kickoff registers no fetchable manifest, so the worker has
    /// nothing to claim — the same predicate the SQL job stores encode in their
    /// claim query's `WHERE manifest_url IS NOT NULL`.
    #[tokio::test]
    async fn test_manifest_without_url_is_never_claimed() {
        let Some(backend) = create_backend("submit_status_only").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let id = SubmissionId::generate("data-provider");
        backend.create_submission(&tenant, &id, None).await.unwrap();
        backend
            .add_manifest(&tenant, &id, None, None)
            .await
            .unwrap();

        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_replaces_manifest_url_supersedes_and_dequeues() {
        let Some(backend) = create_backend("submit_replaces").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let superseded = backend
            .replace_manifest_by_url(&tenant, &id, "https://provider.example/manifest.json")
            .await
            .unwrap();
        assert_eq!(superseded, vec![manifest_id]);

        let manifests = backend.list_manifests(&tenant, &id).await.unwrap();
        assert_eq!(manifests[0].status, ManifestStatus::Replaced);
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .unwrap()
                .is_none(),
            "a replaced manifest is no longer work"
        );
    }

    /// The streaming engine is what the worker actually drives, so cover it
    /// directly too: wrong-typed lines are counted, not ingested.
    #[tokio::test]
    async fn test_stream_rejects_lines_of_the_wrong_type() {
        let Some(backend) = create_backend("submit_stream_types").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (id, manifest_id) = seed(&backend, &tenant).await;

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"ok\"}\n",
            "{\"resourceType\":\"Observation\",\"id\":\"wrong\"}\n"
        );
        let reader = Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            ndjson.as_bytes().to_vec(),
        )));
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &id,
                &manifest_id,
                "Patient",
                reader,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        assert_eq!(result.counts.success, 1);
        assert_eq!(result.counts.validation_error, 1);
        assert!(
            backend
                .read(&tenant, "Observation", "wrong")
                .await
                .unwrap()
                .is_none(),
            "a line of the wrong type must not be stored"
        );
    }

    /// Wraps a reader so that `token` is tripped the first time the ingest
    /// actually reads from the stream.
    struct CancelOnFirstRead<R> {
        inner: R,
        token: CancelToken,
        tripped: bool,
    }

    impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for CancelOnFirstRead<R> {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let this = self.get_mut();
            if !this.tripped {
                this.tripped = true;
                this.token.cancel();
            }
            std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }

    /// Six one-per-line Patients, enough for three batches of two.
    fn six_patient_lines() -> Vec<u8> {
        (1..=6)
            .map(|i| format!("{{\"resourceType\":\"Patient\",\"id\":\"cancel-{i}\"}}\n"))
            .collect::<String>()
            .into_bytes()
    }

    /// #968, MongoDB: cancelling mid-manifest stops at the next batch boundary,
    /// keeping the batches already committed and skipping the rest. Mongo has no
    /// enclosing transaction around the manifest, so "durable partial progress"
    /// is a property of the backend, not of a rollback that never happened.
    #[tokio::test]
    async fn cancelled_mid_stream_keeps_committed_batches_and_stops() {
        let Some(backend) = create_backend("submit_cancel_mid_stream").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (sub_id, manifest_id) = seed(&backend, &tenant).await;

        let cancel = CancelToken::new();
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_cancel(cancel.clone());

        let reader = Box::new(tokio::io::BufReader::new(CancelOnFirstRead {
            inner: std::io::Cursor::new(six_patient_lines()),
            token: cancel,
            tripped: false,
        }));
        let result = backend
            .process_ndjson_stream(&tenant, &sub_id, &manifest_id, "Patient", reader, &options)
            .await
            .unwrap();

        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some(CANCELLED_ABORT_REASON));
        assert_eq!(
            result.counts.success, 2,
            "the batch already committed when the token tripped is kept"
        );
        assert_eq!(
            result.lines_processed, 2,
            "the remaining four lines were never read"
        );

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 2, "the partial counts are durable");
        assert!(
            backend
                .read(&tenant, "Patient", "cancel-2")
                .await
                .unwrap()
                .is_some(),
            "the first batch really landed"
        );
        assert!(
            backend
                .read(&tenant, "Patient", "cancel-3")
                .await
                .unwrap()
                .is_none(),
            "nothing after the cancellation point was ingested"
        );
    }

    /// #968, MongoDB: `abort_submission` fails in-flight manifests without
    /// clearing the lease, so the worker's late verdict must lose rather than
    /// resurrect the manifest as `completed`. Mongo enforces this with a
    /// `status: processing` clause added to the fenced-write filter, so this
    /// also pins that the status is spelled the way the filter expects.
    #[tokio::test]
    async fn abort_beats_a_late_finish_manifest() {
        let Some(backend) = create_backend("submit_abort_beats_finish").await else {
            return;
        };
        let tenant = create_tenant("submit-tenant");
        let (sub_id, _manifest_id) = seed(&backend, &tenant).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("w1"), lease_duration())
            .await
            .unwrap()
            .expect("a manifest should be claimable");
        backend.mark_manifest_processing(&lease).await.unwrap();

        // The submitter aborts while the worker still holds a valid lease.
        backend
            .abort_submission(&tenant, &sub_id, "user cancelled")
            .await
            .unwrap();

        // The worker's verdicts arrive too late and change nothing.
        assert!(
            matches!(
                backend.finish_manifest(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a finish after an abort must not win"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(
            stored.status,
            ManifestStatus::Failed,
            "the abort's verdict stands"
        );

        assert!(
            matches!(
                backend.fail_manifest(&lease, "worker gave up").await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a late failure verdict is equally a no-op"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(stored.status, ManifestStatus::Failed);
    }

    /// Reconstructs the pre-v8 Mongo artifact shape, migrates it through the
    /// public backend migration, and verifies manifest-aware v8 identity for
    /// two generations under one submission.
    #[tokio::test]
    async fn test_v7_to_v8_manifest_aware_submit_file_identity() {
        use futures::TryStreamExt;

        let Some(backend) = create_backend("submit_v7_to_v8_manifest_identity").await else {
            eprintln!("skipping: no MongoDB container available");
            return;
        };
        let tenant = create_tenant("submit-v7");
        let submission_id = SubmissionId::new("v7-provider", "legacy-generation");
        let db = backend.get_database().await.unwrap();
        let files = db.collection::<Document>("bulk_submit_files");

        files
            .drop_index("idx_bulk_submit_files_manifest")
            .await
            .unwrap();
        files
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {
                        "tenant_id": 1_i32,
                        "submitter": 1_i32,
                        "submission_id": 1_i32,
                        "file_type": 1_i32,
                        "resource_type": 1_i32,
                        "part_index": 1_i32,
                        "fencing_token": 1_i32,
                    })
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name(Some("idx_bulk_submit_files_part".to_string()))
                            .unique(Some(true))
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        db.collection::<Document>("schema_version")
            .update_one(
                doc! { "_id": "schema_version" },
                doc! { "$set": { "version": 7_i32 } },
            )
            .await
            .unwrap();
        files
            .insert_one(doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "submitter": &submission_id.submitter,
                "submission_id": &submission_id.submission_id,
                "manifest_url": "https://provider.example/legacy.json",
                "file_type": "output",
                "resource_type": "Patient",
                "part_index": 0_i64,
                "fencing_token": 1_i64,
                "file_path": "legacy/output/Patient-0.ndjson",
                "line_count": 2_i64,
                "byte_count": 19_i64,
                "created_at": mongodb::bson::DateTime::from_millis(
                    chrono::Utc::now().timestamp_millis(),
                ),
            })
            .await
            .unwrap();

        backend.migrate().await.unwrap();
        let schema_version = db
            .collection::<Document>("schema_version")
            .find_one(doc! { "_id": "schema_version" })
            .await
            .unwrap()
            .expect("schema version document");
        assert_eq!(schema_version.get_i32("version").unwrap(), 9_i32);

        let indexes = files
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        let manifest_index = indexes
            .iter()
            .find(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.name.as_deref())
                    == Some("idx_bulk_submit_files_manifest")
            })
            .expect("v8 submit-file identity index");
        assert_eq!(
            manifest_index.keys,
            doc! {
                "tenant_id": 1_i32,
                "submitter": 1_i32,
                "submission_id": 1_i32,
                "manifest_id": 1_i32,
                "file_type": 1_i32,
                "resource_type": 1_i32,
                "part_index": 1_i32,
                "fencing_token": 1_i32,
            }
        );
        assert_eq!(manifest_index.options.as_ref().unwrap().unique, Some(true));
        assert!(
            indexes.iter().all(|index| index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                != Some("idx_bulk_submit_files_part")),
            "legacy submit-file index must be dropped, got {indexes:?}"
        );

        // A second pass on the already-migrated database is a no-op.
        backend.migrate().await.unwrap();
        let legacy_rows = backend
            .list_submit_files(&tenant, &submission_id)
            .await
            .unwrap();
        assert_eq!(legacy_rows.len(), 1);
        assert!(legacy_rows[0].manifest_id.is_none());
        assert!(legacy_rows[0].legacy_locator);
        assert_eq!(legacy_rows[0].line_count, 2);
        assert_eq!(legacy_rows[0].byte_count, 19);

        let new_submission_id = SubmissionId::new("v8-provider", "manifest-aware-generation");
        backend
            .create_submission(&tenant, &new_submission_id, None)
            .await
            .unwrap();
        let first = backend
            .add_manifest(
                &tenant,
                &new_submission_id,
                Some("https://provider.example/first.json"),
                None,
            )
            .await
            .unwrap();
        let second = backend
            .add_manifest(
                &tenant,
                &new_submission_id,
                Some("https://provider.example/second.json"),
                None,
            )
            .await
            .unwrap();
        assert_ne!(first.manifest_id, second.manifest_id);

        let worker_id = WorkerId::new("v8-identity-worker");
        let first_lease = backend
            .claim_next_manifest(&worker_id, lease_duration())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_lease.submission_id, new_submission_id);
        assert_eq!(first_lease.manifest_id, first.manifest_id);
        assert_eq!(first_lease.fencing_token, 1);
        let record = SubmitFileRecord {
            manifest_url: Some("https://provider.example/first.json".to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "legacy/output/Patient-0.ndjson".to_string(),
            line_count: 2,
            byte_count: 19,
            count_severity: None,
        };
        backend
            .record_submit_file(&first_lease, &record)
            .await
            .unwrap();
        backend
            .record_submit_file(&first_lease, &record)
            .await
            .unwrap();
        backend
            .publish_manifest_artifacts(
                &first_lease,
                std::slice::from_ref(&record),
                ManifestPublicationStatus::Completed,
            )
            .await
            .unwrap();

        let second_lease = backend
            .claim_next_manifest(&worker_id, lease_duration())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_lease.submission_id, new_submission_id);
        assert_eq!(second_lease.manifest_id, second.manifest_id);
        assert_eq!(second_lease.fencing_token, 1);
        let second_record = SubmitFileRecord {
            manifest_url: Some("https://provider.example/second.json".to_string()),
            ..record.clone()
        };
        backend
            .record_submit_file(&second_lease, &second_record)
            .await
            .unwrap();
        backend
            .publish_manifest_artifacts(
                &second_lease,
                &[second_record],
                ManifestPublicationStatus::Completed,
            )
            .await
            .unwrap();

        let rows = backend
            .list_submit_files(&tenant, &new_submission_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !row.legacy_locator));
        let mut actual_manifest_ids: Vec<_> = rows
            .iter()
            .map(|row| row.manifest_id.as_deref().unwrap())
            .collect();
        actual_manifest_ids.sort_unstable();
        let mut expected_manifest_ids =
            vec![first.manifest_id.as_str(), second.manifest_id.as_str()];
        expected_manifest_ids.sort_unstable();
        assert_eq!(actual_manifest_ids, expected_manifest_ids);
        assert!(rows.iter().all(|row| row.file_type == "output"));
        assert!(
            rows.iter()
                .all(|row| row.resource_type.as_deref() == Some("Patient"))
        );
        assert!(rows.iter().all(|row| row.part_index == 0));
        assert!(rows.iter().all(|row| row.line_count == 2));
        assert!(rows.iter().all(|row| row.byte_count == 19));
        assert!(
            rows.iter()
                .all(|row| row.file_path == "legacy/output/Patient-0.ndjson")
        );
        let manifests = backend
            .list_manifests(&tenant, &new_submission_id)
            .await
            .unwrap();
        assert_eq!(manifests.len(), 2);
        assert_ne!(manifests[0].manifest_id, manifests[1].manifest_id);
        assert!(manifests.iter().all(|manifest| {
            manifest.manifest_id == first.manifest_id || manifest.manifest_id == second.manifest_id
        }));

        let preserved_legacy_rows = backend
            .list_submit_files(&tenant, &submission_id)
            .await
            .unwrap();
        assert_eq!(preserved_legacy_rows.len(), 1);
        assert!(preserved_legacy_rows[0].manifest_id.is_none());
        assert!(preserved_legacy_rows[0].legacy_locator);
    }
}

// ============================================================================
// Bulk Export — the `_since` / `_until` window (#657).
// ============================================================================

/// Pins a stored resource's `last_updated` so a window test does not depend on
/// wall-clock timing.
async fn pin_last_updated(backend: &MongoBackend, id: &str, at: chrono::DateTime<chrono::Utc>) {
    use mongodb::bson::{DateTime as BsonDateTime, Document, doc};
    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    resources
        .update_one(
            doc! { "id": id },
            doc! { "$set": { "last_updated": BsonDateTime::from_millis(at.timestamp_millis()) } },
        )
        .await
        .unwrap();
}

async fn seed_patient_at(
    backend: &MongoBackend,
    tenant: &TenantContext,
    at: chrono::DateTime<chrono::Utc>,
) -> String {
    let stored = backend
        .create(
            tenant,
            "Patient",
            serde_json::json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let id = stored.id().to_string();
    pin_last_updated(backend, &id, at).await;
    id
}

fn instant(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

/// `_until` excludes a resource modified after the bound, and the count agrees
/// with what the fetch emits.
#[tokio::test]
async fn mongodb_integration_export_until_bounds_count_and_fetch() {
    use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};

    let Some(backend) = create_backend("export_until").await else {
        eprintln!(
            "Skipping mongodb_integration_export_until_bounds_count_and_fetch (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("export-until");

    let early = seed_patient_at(&backend, &tenant, instant("2026-01-01T00:00:00Z")).await;
    let _late = seed_patient_at(&backend, &tenant, instant("2026-03-01T00:00:00Z")).await;

    let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));

    let count = backend
        .count_export_resources(&tenant, &request, "Patient")
        .await
        .unwrap();
    let batch = backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 10)
        .await
        .unwrap();

    assert_eq!(count, 1, "count must apply the upper bound");
    assert_eq!(batch.lines.len(), 1, "fetch must apply the upper bound");
    assert_eq!(
        count as usize,
        batch.lines.len(),
        "count and fetch must agree about the window"
    );
    assert!(batch.lines[0].contains(&early));
}

/// `_since` and `_until` together bound the window at both ends.
///
/// This is the case that catches the document-shape trap: `last_updated` is one
/// key, so writing the two bounds as two `insert`s would keep only the second.
#[tokio::test]
async fn mongodb_integration_export_since_and_until_bound_the_window() {
    use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};

    let Some(backend) = create_backend("export_window").await else {
        eprintln!(
            "Skipping mongodb_integration_export_since_and_until_bound_the_window (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("export-window");

    let _before = seed_patient_at(&backend, &tenant, instant("2025-12-01T00:00:00Z")).await;
    let inside = seed_patient_at(&backend, &tenant, instant("2026-01-15T00:00:00Z")).await;
    let _after = seed_patient_at(&backend, &tenant, instant("2026-03-01T00:00:00Z")).await;

    let request = ExportRequest::system()
        .with_since(instant("2026-01-01T00:00:00Z"))
        .with_until(instant("2026-02-01T00:00:00Z"));

    let count = backend
        .count_export_resources(&tenant, &request, "Patient")
        .await
        .unwrap();
    let batch = backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 10)
        .await
        .unwrap();

    assert_eq!(count, 1, "both bounds must survive into one range document");
    assert_eq!(batch.lines.len(), 1);
    assert!(batch.lines[0].contains(&inside));
}

/// The bound is inclusive, matching S3.
#[tokio::test]
async fn mongodb_integration_export_until_is_inclusive() {
    use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};

    let Some(backend) = create_backend("export_until_incl").await else {
        eprintln!(
            "Skipping mongodb_integration_export_until_is_inclusive (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("export-until-incl");

    seed_patient_at(&backend, &tenant, instant("2026-02-01T00:00:00Z")).await;

    let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));
    let batch = backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 10)
        .await
        .unwrap();

    assert_eq!(
        batch.lines.len(),
        1,
        "a resource exactly on the bound is included"
    );
}

#[tokio::test]
async fn mongodb_integration_if_none_exist_multi_param_and_semantics() {
    let Some(backend) = create_backend_with_full_registry("if_none_exist_multi_param").await else {
        eprintln!(
            "Skipping mongodb_integration_if_none_exist_multi_param_and_semantics (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-none-exist-multi");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-MULTI-1"}],
                "active": true
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-MULTI-1"}],
                "active": false
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let active_entry = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN-MULTI-1"}],
            "active": true
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some(
            "identifier=http://example.org/mrn|MRN-MULTI-1&active=true".to_string(),
        ),
        full_url: Some("urn:uuid:active-patient".to_string()),
    };

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![active_entry],
        "mongodb_integration_if_none_exist_multi_param_and_semantics",
    )
    .await
    else {
        return;
    };

    assert_eq!(
        result.entries[0].status, 200,
        "both params match the active patient — should not create a duplicate"
    );

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 2, "no third patient should have been created");
}

#[tokio::test]
async fn mongodb_integration_if_none_exist_same_transaction_read_your_writes() {
    let Some(backend) = create_backend_with_full_registry("if_none_exist_ryw").await else {
        eprintln!(
            "Skipping mongodb_integration_if_none_exist_same_transaction_read_your_writes (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-none-exist-ryw");

    let entry = |full_url: &str| BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN-RYW-1"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org/mrn|MRN-RYW-1".to_string()),
        full_url: Some(full_url.to_string()),
    };

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry("urn:uuid:first"), entry("urn:uuid:second")],
        "mongodb_integration_if_none_exist_same_transaction_read_your_writes",
    )
    .await
    else {
        return;
    };

    assert_eq!(
        result.entries[0].status, 201,
        "first entry creates the patient"
    );
    assert_eq!(
        result.entries[1].status, 200,
        "second entry must see the first entry's write via session"
    );
    assert_eq!(
        result.entries[1].location, result.entries[0].location,
        "second entry must resolve to the same resource as the first"
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

#[tokio::test]
async fn mongodb_integration_if_none_exist_multiple_matches_rolls_back() {
    let Some(backend) = create_backend_with_full_registry("if_none_exist_multi_match").await else {
        eprintln!(
            "Skipping mongodb_integration_if_none_exist_multiple_matches_rolls_back (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-none-exist-ambiguous");

    for family in ["One", "Two"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://example.org/mrn", "value": "MRN-AMB-1"}],
                    "name": [{"family": family}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(
                json!({ "resourceType": "Patient", "name": [{"family": "ShouldRollBack"}] }),
            ),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-AMB-1"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some("identifier=http://example.org/mrn|MRN-AMB-1".to_string()),
            full_url: Some("urn:uuid:ambiguous".to_string()),
        },
    ];

    match backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
    {
        Err(helios_persistence::error::TransactionError::UnsupportedIsolationLevel { .. }) => {
            eprintln!("Skipping multiple_matches_rolls_back (replica-set required)");
            return;
        }
        Err(helios_persistence::error::TransactionError::BundleError { index, message }) => {
            assert_eq!(index, 1);
            assert!(
                message.contains("412"),
                "expected 412 in message, got: {message}"
            );
        }
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("ambiguous ifNoneExist must fail the bundle"),
    }

    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        2,
        "the plain create in entry 0 must have been rolled back"
    );
}

#[tokio::test]
async fn mongodb_integration_if_none_exist_offloaded_search_uses_resource_scan() {
    let Some(mut backend) = create_backend_with_full_registry("if_none_exist_offloaded").await
    else {
        eprintln!(
            "Skipping mongodb_integration_if_none_exist_offloaded_search_uses_resource_scan (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-if-none-exist-offloaded");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-OFFL-1"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend.set_search_offloaded(true);

    let entry = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org/mrn", "value": "MRN-OFFL-1"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org/mrn|MRN-OFFL-1".to_string()),
        full_url: Some("urn:uuid:offloaded".to_string()),
    };

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry],
        "mongodb_integration_if_none_exist_offloaded_search_uses_resource_scan",
    )
    .await
    else {
        return;
    };

    assert_eq!(
        result.entries[0].status, 200,
        "offloaded-search fallback must find the existing resource and suppress the create"
    );
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        1,
        "no duplicate should have been created"
    );
}

#[tokio::test]
async fn mongodb_integration_if_none_exist_broad_param_beyond_probe_limit() {
    let Some(backend) = create_backend_with_full_registry("if_none_exist_broad_param").await else {
        eprintln!(
            "Skipping mongodb_integration_if_none_exist_broad_param_beyond_probe_limit \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    let tenant = create_tenant("tenant-a");

    const NOISE_COUNT: usize = 200;

    for chunk_start in (0..NOISE_COUNT).step_by(50) {
        let entries: Vec<BundleEntry> = (chunk_start..chunk_start + 50)
            .map(|i| BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(serde_json::json!({
                    "resourceType": "Patient",
                    "active": true,
                    "identifier": [{"system": "http://example.org/noise", "value": format!("NOISE-{i}")}]
                })),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: Some(format!("urn:uuid:noise-{i}")),
            })
            .collect();
        let Some(result) = process_transaction_or_skip(
            &backend,
            &tenant,
            entries,
            "mongodb_integration_if_none_exist_broad_param_beyond_probe_limit (setup)",
        )
        .await
        else {
            return;
        };
        assert!(
            result.entries.iter().all(|e| e.status == 201),
            "all noise patients must be created"
        );
    }

    let target_entry = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(serde_json::json!({
            "resourceType": "Patient",
            "active": true,
            "identifier": [{"system": "http://example.org/mrn", "value": "TARGET-BROAD-1"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: Some("urn:uuid:target-broad".to_string()),
    };
    let Some(target_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![target_entry],
        "mongodb_integration_if_none_exist_broad_param_beyond_probe_limit (target create)",
    )
    .await
    else {
        return;
    };
    assert_eq!(
        target_result.entries[0].status, 201,
        "target patient must be created"
    );

    let total = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(total, (NOISE_COUNT + 1) as u64, "all patients present");

    let if_none_exist_entry = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(serde_json::json!({
            "resourceType": "Patient",
            "active": true,
            "identifier": [{"system": "http://example.org/mrn", "value": "TARGET-BROAD-1"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org/mrn|TARGET-BROAD-1".to_string()),
        full_url: Some("urn:uuid:ine-broad".to_string()),
    };
    let Some(ine_result) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![if_none_exist_entry],
        "mongodb_integration_if_none_exist_broad_param_beyond_probe_limit",
    )
    .await
    else {
        return;
    };

    assert_eq!(
        ine_result.entries[0].status, 200,
        "ifNoneExist must find the existing target patient and suppress the create"
    );
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        (NOISE_COUNT + 1) as u64,
        "no duplicate should have been created"
    );
}
