//! The backend failure contract.
//!
//! Every backend that implements [`ResourceStorage`] must fail *gracefully* when
//! its store is unreachable or misconfigured: it must surface a
//! `StorageError::Backend`, rather than panicking, hanging, or — worst of all —
//! returning a misleading success.
//!
//! The misleading-success case is the one that matters most, and it is why these
//! tests assert more than "an error came back". A `read` that reports `Ok(None)`
//! when it never actually reached the database is indistinguishable from a `read`
//! against a store where the resource genuinely is absent. For a FHIR server that
//! is not a cosmetic difference: "this patient has no recorded allergies" and "I
//! could not reach the database" are different clinical statements, and only one
//! of them is safe to act on. The same goes for `count` returning `Ok(0)`.
//!
//! So the contract is:
//!
//! > An operation against an unreachable or misconfigured store MUST return
//! > `Err(StorageError::Backend(_))`. It MUST NOT return `Ok(None)`, `Ok(0)`, or
//! > any other success, and it MUST NOT hang.
//!
//! Unlike the rest of the persistence suite these tests need **no server and no
//! Docker**: each backend is pointed at a dead address (or, for the embedded and
//! object stores, put into an equivalently broken state), so they always run — in
//! CI and locally.
//!
//! Backends differ in *where* the failure surfaces, and the tests below are shaped
//! accordingly rather than being copy-pasted:
//!
//! | Backend       | Construction | Failure surfaces at |
//! |---------------|--------------|---------------------|
//! | sqlite        | eager, sync  | construction (bad path) *and* operation (unmigrated schema) |
//! | postgres      | **eager**    | construction — `new()` verifies connectivity |
//! | mongodb       | lazy         | operation |
//! | elasticsearch | lazy         | operation |
//! | s3            | lazy or eager, per `validate_buckets_on_startup` | either |
//!
//! Adding a backend? Add a module here. A backend with no section in this file has
//! no evidence that it fails safely.

/// Shared contract helpers.
///
/// Deliberately inlined rather than placed in `tests/common/`: that directory is
/// not wired into any test target (no test file declares `mod common;`), so cargo
/// never compiles it. Reviving it would drag `harness`/`fixtures` and their
/// testcontainer dependencies into a test whose entire point is that it needs no
/// infrastructure.
mod contract {
    #![allow(dead_code)] // not every helper is used by every feature combination

    use std::fmt::Debug;
    use std::future::Future;
    use std::time::Duration;

    use helios_fhir::FhirVersion;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::error::StorageError;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::{Value, json};

    /// Hard upper bound for any single contract test.
    ///
    /// A backend that *hangs* violates the contract just as surely as one that
    /// returns `Ok`, and a hang in CI is far more expensive than a failure. Every
    /// test here is wrapped in this deadline so that "hangs forever" is reported
    /// as a loud, diagnosable test failure instead of a stalled job.
    ///
    /// This is generous on purpose. Every backend here is *also* configured with
    /// its own short internal timeout, so the deadline should never be the thing
    /// that fires; if it does, that is itself the finding. It must stay well clear
    /// of the backends' own timeouts, or it stops being a backstop and starts being
    /// a race — an earlier revision set it to 15s, exactly MongoDB's default
    /// server-selection timeout, and duly flagged the driver's normal (if slow)
    /// failure as a hang.
    pub const DEADLINE: Duration = Duration::from_secs(60);

    pub fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    pub fn patient() -> Value {
        json!({ "resourceType": "Patient", "name": [{ "family": "Unreachable" }] })
    }

    /// Runs `f`, failing the test if it does not complete within [`DEADLINE`].
    pub async fn within_deadline<F: Future>(f: F) -> F::Output {
        tokio::time::timeout(DEADLINE, f)
            .await
            .unwrap_or_else(|_| panic!("contract violation: no error surfaced within {DEADLINE:?} — the backend is hanging instead of failing"))
    }

    /// The floor: the operation failed, and it failed as a *backend* error.
    ///
    /// Intentionally variant-agnostic. Backends legitimately notice a broken store
    /// at different layers, and report accordingly: postgres yields
    /// `ConnectionFailed` from its pool, sqlite `ConnectionFailed` (pool) or
    /// `Internal` (query), mongodb `Internal`, s3 `Unavailable`, elasticsearch
    /// `Unavailable`. Pinning the inner variant per backend would encode today's
    /// taxonomy rather than the contract, and would make the S3 case depend on
    /// whether the AWS SDK happened to fail at credential resolution or at
    /// dispatch. What is not negotiable is `Backend(_)`, and not `Ok`.
    pub fn assert_backend_error<T: Debug>(what: &str, result: Result<T, StorageError>) {
        assert!(
            matches!(result, Err(StorageError::Backend(_))),
            "{what}: expected StorageError::Backend from a broken store, got {result:?}"
        );
    }

    /// The safety invariant, stated separately because it is the whole point.
    ///
    /// A broken store must never answer a `read` with `Ok(None)`. "I could not ask"
    /// is not "the answer is no".
    pub fn assert_read_is_not_silent<T: Debug>(
        what: &str,
        result: Result<Option<T>, StorageError>,
    ) {
        if matches!(&result, Ok(None)) {
            panic!(
                "{what}: read against a broken store returned Ok(None) — a store we \
                 could not reach must never be indistinguishable from one where the \
                 resource is genuinely absent"
            );
        }
        assert_backend_error(what, result);
    }

    /// The one genuinely shared driver.
    ///
    /// The constructors differ wildly between backends; driving and asserting do
    /// not. `ResourceStorage` is `#[async_trait]` and therefore object-safe, so a
    /// single `&dyn` driver covers every backend.
    ///
    /// The three operations are issued **concurrently**, so a backend's internal
    /// network timeout is paid once for the whole test rather than once per call.
    pub async fn assert_core_ops_fail(storage: &dyn ResourceStorage, backend: &str) {
        let t = tenant("tenant-contract");

        let read = storage.read(&t, "Patient", "does-not-exist");
        let count = storage.count(&t, Some("Patient"));
        let create = storage.create(&t, "Patient", patient(), FhirVersion::default());

        let (read_result, count_result, create_result) = tokio::join!(read, count, create);

        // A read must error, never quietly report the resource as absent.
        assert_read_is_not_silent(&format!("{backend} read"), read_result);

        // A count must error, never quietly report zero. `Ok(0)` from a store we
        // never reached is the same lie as `Ok(None)`, wearing different clothes.
        if let Ok(n) = &count_result {
            panic!(
                "{backend} count: returned Ok({n}) against a broken store — a count \
                 is a factual claim about the data, and we never reached it"
            );
        }
        assert_backend_error(&format!("{backend} count"), count_result);

        assert_backend_error(&format!("{backend} create"), create_result);
    }
}

// ============================================================================
// SQLite — embedded, so there is no "unreachable server" to point at.
// ============================================================================
//
// The analogous failures are a store we cannot open, and a store that is open but
// misconfigured. Both are covered.
//
// Two approaches were considered and rejected, and are recorded here so nobody
// re-derives them:
//
//   * Deleting the database file after construction. This does NOT fail: the pool
//     keeps `min_idle` live connections whose file descriptors pin the unlinked
//     inode, and r2d2's checkout validation is `execute_batch("")`, which never
//     touches the file. Operations keep succeeding against the deleted file.
//   * `chmod`-ing the database read-only. CI containers routinely run as root, and
//     root ignores the read-only bit, so the test would silently pass for the wrong
//     reason on the machine we most care about.

#[cfg(feature = "sqlite")]
mod sqlite_backend {
    use super::contract::*;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

    /// Operation-time: a store that opened fine but was never migrated.
    ///
    /// `with_config` deliberately runs no DDL — `init_schema()` is a separate,
    /// opt-in call — so a backend that skips it is a genuinely misconfigured store.
    /// Every operation then hits `no such table: resources`, exercising the
    /// per-operation `map_err` arms in `sqlite/storage.rs` that are otherwise
    /// completely uncovered.
    ///
    /// Fully deterministic, no filesystem, no timeouts, runs in microseconds.
    #[tokio::test]
    async fn unmigrated_store_surfaces_backend_error() {
        let backend = SqliteBackend::in_memory().expect("opening an in-memory DB must succeed");
        // init_schema() is deliberately NOT called.

        within_deadline(assert_core_ops_fail(&backend, "sqlite")).await;
    }

    /// Construction-time: a path that cannot be opened as a database.
    ///
    /// A directory is used rather than a missing file, because SQLite is always
    /// opened with `SQLITE_OPEN_CREATE` (the open flags are never customised), so a
    /// missing file would simply be created. A directory reliably yields
    /// `SQLITE_CANTOPEN` on every platform, as root or otherwise.
    ///
    /// `connection_timeout_ms` MUST be lowered: r2d2 treats the permanent
    /// `CANTOPEN` as transient and retries it with backoff until the timeout
    /// elapses. At the 30s default this would be a 30-second test.
    ///
    /// Plain `#[test]`, not `#[tokio::test]`: `with_config` is synchronous and
    /// blocks its thread, which would stall a runtime worker.
    #[test]
    fn unopenable_path_surfaces_backend_error() {
        let config = SqliteBackendConfig {
            connection_timeout_ms: 250,
            ..Default::default()
        };

        let result = SqliteBackend::with_config(std::env::temp_dir(), config);

        // Map away the backend, which is not `Debug`.
        assert_backend_error("sqlite construction", result.map(|_| ()));
    }
}

// ============================================================================
// PostgreSQL — `new()` eagerly verifies connectivity, so it fails at construction
// ============================================================================
//
// NOTE the module name: `postgres_backend`, NOT `postgres_integration`. The
// Docker-backed suite in `postgres_tests.rs` lives in a `postgres_integration`
// module, and the documented no-Docker escape hatch is
// `cargo test -- --skip postgres_integration` (README.md:546). That is a substring
// match on the test path, so naming this test `postgres_integration_*` would cause
// the single Postgres test that needs no Docker to be skipped by the very command
// people run when they have no Docker.

#[cfg(feature = "postgres")]
mod postgres_backend {
    use super::contract::*;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};

    fn unreachable_config(port: u16) -> PostgresConfig {
        PostgresConfig {
            host: "127.0.0.1".to_string(),
            port,
            connect_timeout_secs: 2,
            max_connections: 1,
            ..Default::default()
        }
    }

    /// Connection refused: nothing is listening on the port.
    ///
    /// Unlike MongoDB, `PostgresBackend::new` eagerly checks out a connection to
    /// verify connectivity, so an unreachable server fails at *construction* — the
    /// caller never gets a backend to drive. That is the correct behaviour, and it
    /// means there is no object on which to call `read`/`count`/`create` here.
    ///
    /// Port 1 is a privileged loopback port nothing listens on, so the connection
    /// is refused deterministically rather than depending on external network state.
    #[tokio::test]
    async fn unreachable_server_surfaces_backend_error() {
        let result = within_deadline(PostgresBackend::new(unreachable_config(1))).await;

        assert_backend_error("postgres construction", result.map(|_| ()));
    }

    /// A server that accepts the TCP connection and then never speaks.
    ///
    /// This is the failure the previous test does *not* cover, and the one that
    /// actually bites in production: a hung database, or a load balancer / security
    /// group that accepts into a blackhole. The kernel completes the handshake from
    /// the listen backlog, so `TcpStream::connect` *succeeds* and the client then
    /// blocks awaiting the Postgres startup/auth response.
    ///
    /// It is a regression test for a real bug: `connect_timeout_secs` used to be
    /// dead config (declared, defaulted, serialized — and never read), and even
    /// wiring it into tokio-postgres would not save us here, because
    /// tokio-postgres applies `connect_timeout` only to `TcpStream::connect` and
    /// leaves DNS, TLS and the auth exchange unbounded. Only deadpool's
    /// `create_timeout` bounds the whole of connection establishment.
    ///
    /// Delete `.create_timeout(..)` from `create_pool` and this test hangs — which
    /// the deadline turns into a failure. That is precisely its job.
    #[tokio::test]
    async fn blackholed_server_is_bounded_and_surfaces_backend_error() {
        // Bind a listener and never accept(). Binding port 0 lets the OS pick a
        // free port, so this cannot collide with anything else on the machine.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("binding a loopback listener must succeed");
        let port = listener
            .local_addr()
            .expect("a bound listener has a local address")
            .port();

        let result = within_deadline(PostgresBackend::new(unreachable_config(port))).await;

        assert_backend_error("postgres blackholed", result.map(|_| ()));
    }
}

// ============================================================================
// MongoDB — lazy construction, so the failure surfaces at operation time
// ============================================================================

#[cfg(feature = "mongodb")]
mod mongodb_backend {
    use super::contract::*;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};

    /// The driver's client is initialised lazily, so `new` succeeds without ever
    /// contacting a server; the error appears when an operation actually tries to
    /// reach one.
    ///
    /// `server_selection_timeout_ms` — not `connect_timeout_ms` — is what bounds
    /// this. The driver keeps re-attempting server selection while its monitor
    /// reports no healthy server, so an operation against a dead address takes the
    /// *server-selection* timeout to fail; the connect timeout only bounds an
    /// individual TCP handshake. Left at its 15s default this test would sit for
    /// 15 seconds.
    #[tokio::test]
    async fn unreachable_server_surfaces_backend_error() {
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:1/".to_string(),
            database_name: "hfs_contract_unreachable".to_string(),
            connect_timeout_ms: 500,
            server_selection_timeout_ms: 500,
            ..Default::default()
        })
        .expect("mongo client construction is lazy and must not connect");

        within_deadline(assert_core_ops_fail(&backend, "mongodb")).await;
    }
}

// ============================================================================
// Elasticsearch — lazy construction; failure surfaces at operation time
// ============================================================================
//
// The issue that prompted these tests declared Elasticsearch out of scope
// ("search-only, never a standalone primary"). That is a statement about how it is
// *deployed*, not about the code: `ElasticsearchBackend` implements
// `ResourceStorage`, four of the eight supported `HFS_STORAGE_BACKEND` modes pair
// it with a primary, and the composite layer *prefers* it for search. It is
// therefore very much on the path a real query takes, and it is the backend where
// the misleading-success failure mode was actually live: `read` mapped every
// transport error to `Ok(None)`, and `search`/`count` reported an unreachable
// cluster as an empty result set. These tests lock that shut.

#[cfg(feature = "elasticsearch")]
mod elasticsearch_backend {
    use super::contract::*;
    use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
    use helios_persistence::core::SearchProvider;
    use helios_persistence::types::SearchQuery;

    fn unreachable_backend() -> ElasticsearchBackend {
        ElasticsearchBackend::new(ElasticsearchConfig {
            nodes: vec!["http://127.0.0.1:1".to_string()],
            request_timeout_ms: 500,
            ..Default::default()
        })
        .expect("ES client construction is lazy and must not connect")
    }

    #[tokio::test]
    async fn unreachable_cluster_surfaces_backend_error() {
        let backend = unreachable_backend();

        within_deadline(assert_core_ops_fail(&backend, "elasticsearch")).await;
    }

    /// Search is the operation that matters most for Elasticsearch, because search
    /// is the role it actually plays in a composite deployment — and an empty
    /// `Bundle` with `total: 0` is a success-shaped answer that a caller will act on.
    ///
    /// A cluster we could not reach must not produce one.
    #[tokio::test]
    async fn unreachable_cluster_search_does_not_report_empty_results() {
        let backend = unreachable_backend();
        let t = tenant("tenant-contract");
        let query = SearchQuery::new("Patient");

        let (search_result, count_result) = within_deadline(async {
            tokio::join!(backend.search(&t, &query), backend.search_count(&t, &query))
        })
        .await;

        if let Ok(page) = &search_result {
            panic!(
                "elasticsearch search: returned Ok with {} result(s) against an \
                 unreachable cluster — a search that never reached the cluster must \
                 not report an empty (or any) result set",
                page.resources.items.len()
            );
        }
        assert_backend_error("elasticsearch search", search_result);

        if let Ok(n) = &count_result {
            panic!(
                "elasticsearch search_count: returned Ok({n}) against an unreachable \
                 cluster — 'zero matches' is a claim we are in no position to make"
            );
        }
        assert_backend_error("elasticsearch search_count", count_result);
    }
}

// ============================================================================
// S3 — object store; lazy or eager depending on `validate_buckets_on_startup`
// ============================================================================
//
// No AWS credentials are configured here, and none are needed. Every `SdkError`
// variant — including the `ConstructionFailure` the SDK raises when the credential
// chain finds nothing — is mapped into `StorageError::Backend(_)`, so the contract
// holds whether the SDK gives up at credential resolution or at dispatch. That is
// why this file does not call `std::env::set_var`, which is `unsafe` in edition
// 2024 precisely because it is process-global and would race the other tests
// sharing this binary.

#[cfg(feature = "s3")]
mod s3_backend {
    use super::contract::*;
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig};

    fn dead_endpoint_config(validate_buckets_on_startup: bool) -> S3BackendConfig {
        S3BackendConfig {
            endpoint_url: Some("http://127.0.0.1:1".to_string()),
            // `validate()` rejects a plaintext endpoint unless this is set.
            allow_http: true,
            // Pin the region so the SDK does not go looking for one (which can mean
            // probing the EC2 instance-metadata endpoint).
            region: Some("us-east-1".to_string()),
            validate_buckets_on_startup,
            ..Default::default()
        }
    }

    /// With startup validation disabled, construction is lazy and the failure
    /// surfaces in the storage operations — where a dispatch failure is mapped to
    /// `BackendError::Unavailable`.
    ///
    /// This is the highest-value assertion for S3: `read` legitimately returns
    /// `Ok(None)` for a genuinely missing object, so it is exactly the operation
    /// where an unreachable endpoint could most plausibly be mistaken for an empty
    /// one.
    #[tokio::test]
    async fn unreachable_endpoint_surfaces_backend_error() {
        let backend = within_deadline(S3Backend::from_env_async(dead_endpoint_config(false)))
            .await
            .expect("construction is lazy when validate_buckets_on_startup is false");

        within_deadline(assert_core_ops_fail(&backend, "s3")).await;
    }

    /// The eager path: `validate_buckets_on_startup` (the default) issues a
    /// `HeadBucket` per configured bucket, so construction itself must fail cleanly
    /// rather than yielding a backend that looks usable.
    #[tokio::test]
    async fn bucket_validation_against_dead_endpoint_surfaces_backend_error() {
        let result = within_deadline(S3Backend::from_env_async(dead_endpoint_config(true))).await;

        assert_backend_error("s3 bucket validation", result.map(|_| ()));
    }
}
