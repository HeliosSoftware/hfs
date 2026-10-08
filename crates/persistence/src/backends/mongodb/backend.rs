//! MongoDB backend implementation.

use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mongodb::{Client, Database, bson::doc, options::ClientOptions};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use helios_fhir::FhirVersion;

use super::search_index_builder::{BuildOutcome, IndexBuildMode, SearchIndexBuilder};
use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::search::{
    SearchParameterDefinition, SearchParameterExtractor, SearchParameterLoader,
    SearchParameterRegistry, TenantSearchRegistries,
};

/// Sync in-memory cache of each tenant's stored active SearchParameter
/// definitions. MongoDB queries are async but the registry loader must be sync,
/// so async paths populate this map and the loader reads it.
type StoredByTenant =
    Arc<RwLock<std::collections::HashMap<String, Vec<SearchParameterDefinition>>>>;

use super::schema;

/// Builds a MongoDB client from a backend configuration.
///
/// Standalone (rather than a `&self` method) so both [`MongoBackend`] and the
/// in-DB SOF runner can initialise the shared client cell from a cloned config.
pub(crate) async fn connect_client(config: &MongoBackendConfig) -> StorageResult<Client> {
    let mut client_options = ClientOptions::parse(&config.connection_string)
        .await
        .map_err(|e| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: e.to_string(),
            })
        })?;

    client_options.max_pool_size = Some(config.max_connections);
    client_options.connect_timeout = Some(Duration::from_millis(config.connect_timeout_ms));
    client_options.app_name = Some(config.app_name.clone());

    // Fail fast when no healthy server can be selected. `connect_timeout`
    // only covers new TCP handshakes; this caps requests once server
    // monitoring has marked the server unavailable.
    client_options.server_selection_timeout =
        Some(Duration::from_millis(config.server_selection_timeout_ms));
    // Recycle idle connections so stale ones (e.g. silently dropped by a
    // NAT/firewall on a long-lived network path) are not handed out.
    client_options.max_idle_time = Some(MAX_CONNECTION_IDLE_TIME);

    Client::with_options(client_options).map_err(|e| {
        StorageError::Backend(BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message: format!("Failed to create MongoDB client: {}", e),
            source: None,
        })
    })
}

/// Connections idle longer than this are closed rather than reused, so a
/// connection silently dropped by a NAT/firewall is never handed to a request.
const MAX_CONNECTION_IDLE_TIME: Duration = Duration::from_secs(60);

/// MongoDB backend for FHIR resource storage.
///
/// The Phase 4 implementation provides backend wiring, schema bootstrap,
/// core ResourceStorage behavior for CRUD/count + tenant isolation,
/// [`crate::core::VersionedStorage`] support, and history providers.
///
/// Basic search and conditional create/update/delete are available.
/// Composite search parameters are served via a grouped
/// `(resource_id, composite_group)` pair check (#1206); forward/reverse
/// chains remain unsupported.
pub struct MongoBackend {
    config: MongoBackendConfig,
    /// Lazily initialized MongoDB client. MongoDB clients own their connection
    /// pools, so each backend instance must reuse one client. Wrapped in an
    /// `Arc` so the in-DB SOF runner can share the same pooled client (it is
    /// constructed from `&self` but outlives the borrow).
    client: Arc<OnceCell<Client>>,
    /// Per-tenant search parameter registries (shared base + per-tenant overlay).
    registries: Arc<TenantSearchRegistries>,
    /// Sync cache of each tenant's stored params, read by the registry loader.
    stored_by_tenant: StoredByTenant,
    /// Publishes the post-boot `search_index` build's outcome once, from the
    /// task `init_schema` spawns. `None` until that task finishes.
    search_index_rx: tokio::sync::watch::Receiver<Option<BuildOutcome>>,
    /// The sending half, taken (leaving `None`) by `init_schema` the one time
    /// it runs; `wait_for_search_index_build` reads "already taken" as
    /// "`init_schema` has run". A `Mutex` only to make `take()` safe from
    /// `&self`; never held across an `.await`.
    search_index_tx:
        Arc<tokio::sync::Mutex<Option<tokio::sync::watch::Sender<Option<BuildOutcome>>>>>,
    /// Rayon pool for `$reindex` sub-batch extraction; built lazily and only
    /// on a multi-thread runtime (#1403). `Err` remembers a failed build so
    /// the one warning fires once.
    prepare_pool: std::sync::OnceLock<Result<rayon::ThreadPool, String>>,
    /// The one-permit admission gate for `prepare_pool` (#1403).
    prepare_gate: tokio::sync::Semaphore,
    /// Permits for potentially broad standard searches (#1748), sized by
    /// [`MongoBackendConfig::broad_search_concurrency`]; `None` when it is
    /// unset, so no search waits.
    broad_search_gate: Option<tokio::sync::Semaphore>,
    /// Whether `mongodb reindex writer configuration` has already been logged
    /// for this instance (#1403).
    reindex_mode_logged: std::sync::atomic::AtomicBool,
    /// Bounds how many transaction Bundles run at once (#1776).
    transaction_bundle_gate: super::transaction_bundle_gate::TransactionBundleGate,
    /// `(resources, docs)` of each resource type's previous successful
    /// overlapped page, for the sub-batch planner's seed (#1403).
    reindex_docs_per_resource: std::sync::Mutex<std::collections::HashMap<String, (u64, u64)>>,
}

impl Debug for MongoBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoBackend")
            .field("config", &self.config)
            .field("base_registry_len", &self.registries.base().read().len())
            .finish_non_exhaustive()
    }
}

impl MongoBackend {
    /// The rayon pool for `$reindex` sub-batch extraction, or `None` when the
    /// resolved width is below 2 or the pool failed to build (#1403). Built
    /// lazily, once per backend instance. Callers must only call this on a
    /// multi-thread Tokio runtime.
    pub(super) fn reindex_prepare_pool(&self) -> Option<&rayon::ThreadPool> {
        crate::search::reindex_prepare::prepare_pool(
            &self.prepare_pool,
            self.config.reindex_prepare_threads,
            "hfs-mongo-reindex",
            "MongoDB",
        )
    }

    /// The admission gate for transaction Bundles (#1776).
    pub(super) fn transaction_bundle_gate(
        &self,
    ) -> &super::transaction_bundle_gate::TransactionBundleGate {
        &self.transaction_bundle_gate
    }

    /// The one-permit admission gate for [`Self::reindex_prepare_pool`] (#1403).
    pub(super) fn reindex_prepare_gate(&self) -> &tokio::sync::Semaphore {
        &self.prepare_gate
    }

    /// Permits for potentially broad standard searches (#1748); `None` when
    /// no limit is configured.
    pub(super) fn broad_search_gate(&self) -> Option<&tokio::sync::Semaphore> {
        self.broad_search_gate.as_ref()
    }

    /// `(resources, docs)` of this type's previous successful overlapped
    /// page, if any (#1403).
    pub(super) fn reindex_docs_seed(&self, resource_type: &str) -> Option<(u64, u64)> {
        self.reindex_docs_per_resource
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(resource_type)
            .copied()
    }

    /// Records `(resources, docs)` for a successful single-type overlapped
    /// page (#1403).
    pub(super) fn record_reindex_docs(&self, resource_type: &str, resources: u64, docs: u64) {
        self.reindex_docs_per_resource
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(resource_type.to_string(), (resources, docs));
    }

    /// Whether `mongodb reindex writer configuration` has already been logged
    /// for this backend instance (#1403).
    pub(super) fn reindex_mode_logged(&self) -> &std::sync::atomic::AtomicBool {
        &self.reindex_mode_logged
    }
}

/// Configuration for the MongoDB backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoBackendConfig {
    /// MongoDB connection string.
    #[serde(default = "default_connection_string")]
    pub connection_string: String,

    /// MongoDB database name used by this backend.
    #[serde(default = "default_database_name")]
    pub database_name: String,

    /// Maximum number of connections in the driver pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Connection timeout in milliseconds.
    ///
    /// This bounds a new TCP handshake only. It does NOT bound how long an
    /// operation waits for a usable server to be selected — see
    /// [`Self::server_selection_timeout_ms`].
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,

    /// Upper bound, in milliseconds, for selecting a usable server before an
    /// operation gives up.
    ///
    /// This — not `connect_timeout_ms` — is what determines how long an
    /// operation against an unreachable server takes to fail, because the driver
    /// keeps re-attempting server selection while its monitor reports no healthy
    /// server. Tests that point a backend at a dead address should lower it.
    #[serde(default = "default_server_selection_timeout_ms")]
    pub server_selection_timeout_ms: u64,

    /// FHIR version for this backend instance.
    #[serde(default = "crate::default_fhir_version")]
    pub fhir_version: FhirVersion,

    /// Directory containing FHIR SearchParameter spec files.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    /// When true, search indexing is offloaded to a secondary backend.
    #[serde(default)]
    pub search_offloaded: bool,

    /// Upper bound on the number of resources `_include`/`_revinclude`
    /// resolution will add to a searchset's `included`, applied per directive
    /// (`_revinclude=Observation:patient&_revinclude=Condition:patient` can
    /// therefore each independently reach this cap; forward and reverse
    /// resolution are separate passes, each subject to this bound).
    ///
    /// Reverse resolution in particular has no other bound: the referring set
    /// for a page of primary matches can be arbitrarily large regardless of
    /// `_count` (#1061). Hitting the cap on any directive appends one
    /// synthetic `OperationOutcome` warning entry (`search.mode = outcome`)
    /// to the Bundle rather than silently dropping resources. Clamped to at
    /// least 1.
    #[serde(default = "default_max_included_resources")]
    pub max_included_resources: usize,

    /// `appName` the driver sends on every connection. Visible in the server's
    /// `currentOp`/logs and usable to scope a `failCommand` failpoint to one
    /// client in tests.
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// When the generation-2 `search_index` indexes are built relative to
    /// boot: `background` (default) spawns the builder and serves at once,
    /// `inline` awaits it, `off` only warns about missing indexes so an
    /// operator can build them out of band (`HFS_MONGODB_INDEX_BUILD`).
    #[serde(default)]
    pub index_build: IndexBuildMode,

    /// Clock-skew and commit-lag allowance of the `$reindex` walk's catch-up
    /// rounds, in milliseconds (#1403). A resource stamped within this much of
    /// a walk's start, or written while it runs, is re-read in `last_updated`
    /// order after the id-order pass. Default 120 000 (two minutes); values
    /// are clamped to 1 000 ..= 86 400 000. No `HFS_*` variable sets it;
    /// tests construct a shorter one directly.
    #[serde(default = "default_reindex_catch_up_margin_ms")]
    pub reindex_catch_up_margin_ms: u64,

    /// Overlaps search-parameter extraction with `search_index` inserts
    /// inside a `$reindex` page (#1403; `HFS_MONGODB_REINDEX_OVERLAP`).
    /// `false` restores the serial writer.
    #[serde(default = "default_reindex_overlap")]
    pub reindex_overlap: bool,
    /// Extraction threads for `$reindex` pages (#1403;
    /// `HFS_MONGODB_REINDEX_PREPARE_THREADS`); `0` = cores − 1 clamped to
    /// 1..=4, `1` = the page's own thread, `N` = `min(N, 64)`.
    #[serde(default)]
    pub reindex_prepare_threads: usize,
    /// Lets the reindex driver fetch the next id-phase page while it writes
    /// the current one (#1403; `HFS_MONGODB_REINDEX_PREFETCH`). Never used
    /// when search is offloaded.
    #[serde(default = "default_reindex_prefetch")]
    pub reindex_prefetch: bool,

    /// Wall-clock budget for one transaction Bundle, replays and pauses
    /// included (#1586). When the server aborts a transaction with a
    /// `TransientTransactionError` the backend runs the Bundle again, but never
    /// starts another run once the time already spent plus the pause plus
    /// another run as long as the last one would pass this budget; the request
    /// then ends as a retryable `TransactionError::Transient` instead.
    ///
    /// Default 120 s. A server that answers requests under a timeout should set
    /// this to fit inside it: a replay still running when the timeout fires is
    /// cut off and answered `408`, where the budget would have had it give up
    /// in time for a `503` with `Retry-After`. The `hfs` binary sets it to
    /// `HFS_REQUEST_TIMEOUT` less 2 s, capped at the default. It counts from
    /// admission when `bundle_transaction_deadline` is set, and from the call,
    /// time spent waiting for an admission slot
    /// (`max_concurrent_transaction_bundles`) included, when it is not. A zero
    /// budget disables replays. Not read from the environment
    /// here; the embedder decides.
    #[serde(default = "default_bundle_transaction_budget")]
    pub bundle_transaction_budget: Duration,

    /// Upper bound on one transaction Bundle call, admission wait included,
    /// measured from when the call reaches the admission gate (#1806). Time
    /// spent before the backend (middleware, auth, body parsing) is covered
    /// only by the embedder's margin, which is 2 s in `hfs`.
    ///
    /// When set, `bundle_transaction_budget` counts from admission instead, and
    /// a replay starts only if it also ends within this bound, so time spent
    /// queued does not use up the replays but no replay runs into the
    /// embedder's request timeout. `None` (default) makes the budget this bound
    /// too, counted from the call: the admission wait counts against the
    /// budget, as in #1776. It does not cut the admission wait short; the
    /// embedder's own request timeout does that. The `hfs` binary sets it to
    /// `HFS_REQUEST_TIMEOUT` less 2 s, uncapped. Not read from the environment.
    #[serde(default)]
    pub bundle_transaction_deadline: Option<Duration>,

    /// The most standard transaction Bundles (of
    /// `transaction_bundle_weight_entries` entries) this backend runs at once
    /// (#1776; `HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES`).
    ///
    /// A Bundle is one multi-document transaction whose uncommitted writes
    /// WiredTiger holds in cache until commit. Past what the cache holds, the
    /// server rolls back the oldest transaction, and replaying it only adds
    /// pressure. Bundles past the limit wait first come first served before
    /// their session starts; the wait is bounded only by the request timeout.
    ///
    /// Default 4. `0` removes the limit. Applies per backend instance.
    #[serde(default = "default_max_concurrent_transaction_bundles")]
    pub max_concurrent_transaction_bundles: usize,

    /// How many potentially broad standard searches may run at once in this
    /// process (#1748; `HFS_MONGODB_BROAD_SEARCH_CONCURRENCY`). Others wait for
    /// a slot. `None` (the default) sets no limit. This does not reserve
    /// connections for other requests; it limits how much of the pool broad
    /// searches can hold at once. A value at or above `max_connections` is
    /// accepted with a warning; zero is refused.
    #[serde(default)]
    pub broad_search_concurrency: Option<usize>,

    /// The entry count of one standard transaction Bundle, the unit of
    /// `max_concurrent_transaction_bundles` (#1806;
    /// `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES`).
    ///
    /// Bundles are admitted while their entries fit in
    /// `max_concurrent_transaction_bundles` × this; each counts at least 1
    /// entry and at most the whole room, so a Bundle larger than the room is
    /// admitted alone. `0` counts every Bundle as one slot (the #1776
    /// behaviour).
    ///
    /// Default 1000.
    #[serde(default = "default_transaction_bundle_weight_entries")]
    pub transaction_bundle_weight_entries: usize,

    /// Server execution budget, in milliseconds, sent as `maxTimeMS` on the
    /// cross-tenant `count_by_tenant` aggregate and on no other command (#1828;
    /// `HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS`).
    ///
    /// That aggregate groups every live resource of every tenant, and none of
    /// the `resources` indexes leads with `is_deleted`, so its cost grows with
    /// the whole store. When the server spends longer than this on it, it
    /// stops the aggregate with `MaxTimeMSExpired` and the call fails with
    /// [`BackendError::Timeout`], which callers must read as "count
    /// unavailable", never as zero. Every caller is affected: `GET
    /// /admin/tenants`, the existence probe of `DELETE /admin/tenants/{id}` and
    /// the console tenant metrics then answer `504` instead of waiting, and the
    /// Tenants UI shows its error state.
    ///
    /// The server checks the budget at interrupt points during execution
    /// only. It does not bound server selection
    /// ([`Self::server_selection_timeout_ms`]), waiting for a pooled
    /// connection, or network time, so it is not a wall-clock deadline on the
    /// call.
    ///
    /// Default 30 000 (30 s), the same policy as PostgreSQL's
    /// `statement_timeout_ms` and the default HTTP request timeout; it is not
    /// derived from a large-store measurement. Must be between 1 and
    /// 2 147 483 647 (`i32::MAX`, the server's limit); [`MongoBackend::new`]
    /// rejects anything else.
    #[serde(default = "default_count_by_tenant_max_time_ms")]
    pub count_by_tenant_max_time_ms: u64,
}

/// Environment variable that sets
/// [`MongoBackendConfig::count_by_tenant_max_time_ms`] (#1828).
pub(crate) const COUNT_BY_TENANT_MAX_TIME_MS_ENV: &str = "HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS";

/// Default [`MongoBackendConfig::count_by_tenant_max_time_ms`]: 30 s, by policy
/// (parity with PostgreSQL `statement_timeout_ms` and the HTTP request timeout).
const DEFAULT_COUNT_BY_TENANT_MAX_TIME_MS: u64 = 30_000;

/// Checks a `count_by_tenant` budget and returns it as a [`Duration`]. Zero
/// would mean "no limit" to the server, and values above `i32::MAX` are
/// refused by it, so both are errors naming the variable.
fn check_count_by_tenant_max_time_ms(ms: u64) -> Result<Duration, String> {
    if ms == 0 || ms > i32::MAX as u64 {
        return Err(format!(
            "{COUNT_BY_TENANT_MAX_TIME_MS_ENV} must be between 1 and {}; got {ms}",
            i32::MAX
        ));
    }
    Ok(Duration::from_millis(ms))
}

impl MongoBackendConfig {
    /// Applies `HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES` (#1776) and
    /// `HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES` (#1806) from `env`. Each
    /// value is trimmed; an unset variable, or one empty after trimming, leaves
    /// its field unchanged; anything but a non-negative integer is an `Err`
    /// naming the variable.
    pub fn apply_transaction_bundle_env(
        &mut self,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        if let Some(raw) = env("HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.max_concurrent_transaction_bundles = raw.parse::<usize>().map_err(|_| {
                    format!(
                        "HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES must be a non-negative integer; got {raw:?}"
                    )
                })?;
            }
        }
        if let Some(raw) = env("HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.transaction_bundle_weight_entries = raw.parse::<usize>().map_err(|_| {
                    format!(
                        "HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES must be a non-negative integer; got {raw:?}"
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Applies `HFS_MONGODB_BROAD_SEARCH_CONCURRENCY` from `env` (#1748). The
    /// value is trimmed; an unset variable, or one empty after trimming,
    /// leaves the field unchanged. Anything but a positive integer is an `Err`
    /// naming the variable.
    pub fn apply_search_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        use super::search_admission::BROAD_SEARCH_CONCURRENCY_ENV;
        if let Some(raw) = env(BROAD_SEARCH_CONCURRENCY_ENV) {
            let raw = raw.trim();
            if !raw.is_empty() {
                let limit = raw
                    .parse::<usize>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| {
                        format!(
                            "{BROAD_SEARCH_CONCURRENCY_ENV} must be a positive integer; got {raw:?}"
                        )
                    })?;
                self.broad_search_concurrency = Some(limit);
            }
        }
        Ok(())
    }

    /// Applies `HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS` from `env` (#1828).
    /// The value is trimmed; an unset variable, or one empty after trimming,
    /// leaves the field unchanged. Anything but an integer from 1 to
    /// `i32::MAX` is an `Err` naming the variable.
    pub fn apply_count_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        if let Some(raw) = env(COUNT_BY_TENANT_MAX_TIME_MS_ENV) {
            let raw = raw.trim();
            if !raw.is_empty() {
                let ms = raw.parse::<u64>().map_err(|_| {
                    format!(
                        "{COUNT_BY_TENANT_MAX_TIME_MS_ENV} must be a positive integer \
                         (milliseconds); got {raw:?}"
                    )
                })?;
                check_count_by_tenant_max_time_ms(ms)?;
                self.count_by_tenant_max_time_ms = ms;
            }
        }
        Ok(())
    }

    /// Applies `HFS_MONGODB_REINDEX_{OVERLAP,PREPARE_THREADS,PREFETCH}` from
    /// `env` (#1403). Each value is trimmed; an unset variable, or one empty
    /// after trimming, leaves its field unchanged. Booleans accept
    /// `true|1|yes|on` / `false|0|no|off`, case-insensitively; anything else
    /// is an `Err` naming the variable. The thread count parses as `usize`;
    /// values above 64 are accepted here and clamped later by
    /// `resolve_prepare_width`.
    pub fn apply_reindex_env(
        &mut self,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        fn parse_bool(name: &str, raw: &str) -> Result<bool, String> {
            match raw.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(true),
                "false" | "0" | "no" | "off" => Ok(false),
                _ => Err(format!("{name} must be true or false; got {raw:?}")),
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_OVERLAP") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_overlap = parse_bool("HFS_MONGODB_REINDEX_OVERLAP", raw)?;
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_PREPARE_THREADS") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_prepare_threads = raw.parse::<usize>().map_err(|_| {
                    format!(
                        "HFS_MONGODB_REINDEX_PREPARE_THREADS must be a non-negative integer; got {raw:?}"
                    )
                })?;
            }
        }
        if let Some(raw) = env("HFS_MONGODB_REINDEX_PREFETCH") {
            let raw = raw.trim();
            if !raw.is_empty() {
                self.reindex_prefetch = parse_bool("HFS_MONGODB_REINDEX_PREFETCH", raw)?;
            }
        }
        Ok(())
    }
}

fn default_connection_string() -> String {
    "mongodb://localhost:27017".to_string()
}

fn default_database_name() -> String {
    "helios".to_string()
}

fn default_max_connections() -> u32 {
    10
}

fn default_connect_timeout_ms() -> u64 {
    5000
}

fn default_server_selection_timeout_ms() -> u64 {
    15_000
}

fn default_max_included_resources() -> usize {
    1000
}

fn default_app_name() -> String {
    "helios-persistence".to_string()
}

fn default_reindex_catch_up_margin_ms() -> u64 {
    120_000
}

fn default_reindex_overlap() -> bool {
    true
}

fn default_reindex_prefetch() -> bool {
    true
}

fn default_bundle_transaction_budget() -> Duration {
    super::retry::DEFAULT_BUNDLE_TRANSACTION_BUDGET
}

fn default_max_concurrent_transaction_bundles() -> usize {
    super::transaction_bundle_gate::DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES
}

fn default_transaction_bundle_weight_entries() -> usize {
    super::transaction_bundle_gate::DEFAULT_TRANSACTION_BUNDLE_WEIGHT_ENTRIES
}

fn default_count_by_tenant_max_time_ms() -> u64 {
    DEFAULT_COUNT_BY_TENANT_MAX_TIME_MS
}

impl Default for MongoBackendConfig {
    fn default() -> Self {
        Self {
            connection_string: default_connection_string(),
            database_name: default_database_name(),
            max_connections: default_max_connections(),
            connect_timeout_ms: default_connect_timeout_ms(),
            server_selection_timeout_ms: default_server_selection_timeout_ms(),
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
            max_included_resources: default_max_included_resources(),
            app_name: default_app_name(),
            index_build: IndexBuildMode::default(),
            reindex_catch_up_margin_ms: default_reindex_catch_up_margin_ms(),
            reindex_overlap: default_reindex_overlap(),
            reindex_prepare_threads: 0,
            reindex_prefetch: default_reindex_prefetch(),
            bundle_transaction_budget: default_bundle_transaction_budget(),
            bundle_transaction_deadline: None,
            max_concurrent_transaction_bundles: default_max_concurrent_transaction_bundles(),
            broad_search_concurrency: None,
            transaction_bundle_weight_entries: default_transaction_bundle_weight_entries(),
            count_by_tenant_max_time_ms: default_count_by_tenant_max_time_ms(),
        }
    }
}

impl MongoBackend {
    pub(crate) const RESOURCES_COLLECTION: &'static str = "resources";
    pub(crate) const RESOURCE_HISTORY_COLLECTION: &'static str = "resource_history";
    pub(crate) const SEARCH_INDEX_COLLECTION: &'static str = "search_index";
    pub(crate) const SEARCH_INDEX_CONTAINED_COLLECTION: &'static str =
        super::search_index_catalog::SEARCH_INDEX_CONTAINED_COLLECTION;
    pub(crate) const TENANTS_COLLECTION: &'static str = "tenants";

    /// The capabilities this backend declares.
    ///
    /// Invariant across every valid configuration, so it is exposed as a
    /// constructor-free associated function. Both [`Backend::supports`] and
    /// [`Backend::capabilities`] delegate here so the two answers cannot drift
    /// apart.
    ///
    /// # Tenancy
    ///
    /// `SharedSchema` only: all tenants share one set of collections, separated
    /// by a `tenant_id` discriminator that every query filters on. There is no
    /// per-tenant database or collection namespace, so it declares neither
    /// `SchemaPerTenant` nor `DatabasePerTenant`.
    pub fn declared_capabilities() -> Vec<BackendCapability> {
        vec![
            BackendCapability::Crud,
            BackendCapability::Versioning,
            BackendCapability::InstanceHistory,
            BackendCapability::TypeHistory,
            BackendCapability::SystemHistory,
            BackendCapability::BasicSearch,
            BackendCapability::DateSearch,
            BackendCapability::QuantitySearch,
            BackendCapability::ReferenceSearch,
            BackendCapability::ChainedSearch,
            BackendCapability::ReverseChaining,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::Sorting,
            BackendCapability::OffsetPagination,
            BackendCapability::CursorPagination,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::BulkExport,
            BackendCapability::BulkSubmitIngest,
            BackendCapability::BulkSubmitRestWorker,
            BackendCapability::InDbSofRunner,
            BackendCapability::ConditionalCreate,
            BackendCapability::ConditionalUpdate,
            BackendCapability::ConditionalDelete,
            BackendCapability::ConditionalPatch,
            BackendCapability::SharedSchema,
        ]
    }

    /// Creates a new MongoDB backend from the provided configuration.
    pub fn new(config: MongoBackendConfig) -> StorageResult<Self> {
        Self::validate_connection_string(&config.connection_string)?;
        check_count_by_tenant_max_time_ms(config.count_by_tenant_max_time_ms).map_err(
            |message| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message,
                    source: None,
                })
            },
        )?;

        let stored_by_tenant: StoredByTenant =
            Arc::new(RwLock::new(std::collections::HashMap::new()));
        let loader_cache = stored_by_tenant.clone();
        let registries = Arc::new(TenantSearchRegistries::new(Arc::new(
            move |tenant_id: &str| {
                // A HashMap lookup cannot fail the way a live query can — a
                // missing entry legitimately means "no stored params yet",
                // not "load failed" — so this always reports success (#787).
                Some(
                    loader_cache
                        .read()
                        .get(tenant_id)
                        .cloned()
                        .unwrap_or_default(),
                )
            },
        )));
        Self::initialize_search_registry(registries.base(), &config);

        let broad_search_gate = match config.broad_search_concurrency {
            None => None,
            Some(limit) => {
                let limit = super::search_admission::check_broad_search_limit(limit).map_err(
                    |message| {
                        StorageError::Backend(BackendError::Internal {
                            backend_name: "mongodb".to_string(),
                            message,
                            source: None,
                        })
                    },
                )?;
                if limit >= config.max_connections as usize {
                    tracing::warn!(
                        broad_search_concurrency = limit,
                        max_connections = config.max_connections,
                        "MongoDB broad search limit is not below the connection pool size; \
                         broad searches can hold every pooled connection"
                    );
                }
                Some(tokio::sync::Semaphore::new(limit))
            }
        };

        let (search_index_tx, search_index_rx) = tokio::sync::watch::channel(None::<BuildOutcome>);

        let transaction_bundle_gate = super::transaction_bundle_gate::TransactionBundleGate::new(
            config.max_concurrent_transaction_bundles,
            config.transaction_bundle_weight_entries,
        );

        Ok(Self {
            config,
            client: Arc::new(OnceCell::new()),
            registries,
            stored_by_tenant,
            search_index_rx,
            search_index_tx: Arc::new(tokio::sync::Mutex::new(Some(search_index_tx))),
            prepare_pool: std::sync::OnceLock::new(),
            prepare_gate: tokio::sync::Semaphore::new(1),
            transaction_bundle_gate,
            broad_search_gate,
            reindex_mode_logged: std::sync::atomic::AtomicBool::new(false),
            reindex_docs_per_resource: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Creates a backend from a MongoDB connection string.
    pub fn from_connection_string(connection_string: impl Into<String>) -> StorageResult<Self> {
        let config = MongoBackendConfig {
            connection_string: connection_string.into(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Creates a backend from environment variables.
    ///
    /// Supported variables:
    /// - `HFS_MONGODB_URL` (preferred)
    /// - `HFS_MONGODB_URI` (alias)
    /// - `HFS_DATABASE_URL` (fallback)
    /// - `HFS_MONGODB_DATABASE` (default: `helios`)
    /// - `HFS_MONGODB_MAX_CONNECTIONS` (default: `10`)
    /// - `HFS_MONGODB_CONNECT_TIMEOUT_MS` (default: `5000`)
    /// - `HFS_MONGODB_MAX_INCLUDED_RESOURCES` (default: `1000`)
    /// - `HFS_MONGODB_INDEX_BUILD` (default: `background`; `inline` | `off`)
    /// - `HFS_MONGODB_REINDEX_OVERLAP` (default: `true`)
    /// - `HFS_MONGODB_REINDEX_PREPARE_THREADS` (default: `0` = cores − 1, 1–4)
    /// - `HFS_MONGODB_REINDEX_PREFETCH` (default: `true`)
    /// - `HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES` (default: 4; 0 = no limit)
    /// - `HFS_MONGODB_BROAD_SEARCH_CONCURRENCY` (default: unset, no limit)
    /// - `HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS` (default: `30000`)
    pub fn from_env() -> StorageResult<Self> {
        let connection_string = std::env::var("HFS_MONGODB_URL")
            .or_else(|_| std::env::var("HFS_MONGODB_URI"))
            .or_else(|_| std::env::var("HFS_DATABASE_URL"))
            .unwrap_or_else(|_| default_connection_string());

        let database_name =
            std::env::var("HFS_MONGODB_DATABASE").unwrap_or_else(|_| default_database_name());

        let max_connections = std::env::var("HFS_MONGODB_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_else(default_max_connections);

        let connect_timeout_ms = std::env::var("HFS_MONGODB_CONNECT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(default_connect_timeout_ms);

        let max_included_resources = std::env::var("HFS_MONGODB_MAX_INCLUDED_RESOURCES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(default_max_included_resources)
            .max(1);

        let index_build = IndexBuildMode::from_env().map_err(|message| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message,
                source: None,
            })
        })?;

        let mut config = MongoBackendConfig {
            connection_string,
            database_name,
            max_connections,
            connect_timeout_ms,
            max_included_resources,
            index_build,
            ..Default::default()
        };

        config
            .apply_reindex_env(|n| std::env::var(n).ok())
            .and_then(|()| config.apply_search_env(|n| std::env::var(n).ok()))
            .and_then(|()| config.apply_count_env(|n| std::env::var(n).ok()))
            .map_err(|message| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message,
                    source: None,
                })
            })?;

        config
            .apply_transaction_bundle_env(|n| std::env::var(n).ok())
            .map_err(|message| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message,
                    source: None,
                })
            })?;

        Self::new(config)
    }

    fn validate_connection_string(connection_string: &str) -> StorageResult<()> {
        let uri = connection_string.trim();
        if uri.is_empty() {
            return Err(StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: "MongoDB connection string cannot be empty".to_string(),
            }));
        }

        if !Self::looks_like_mongodb_uri(uri) {
            tracing::warn!(
                uri = %uri,
                "MongoDB connection string does not start with mongodb:// or mongodb+srv://"
            );
        }

        Ok(())
    }

    fn looks_like_mongodb_uri(connection_string: &str) -> bool {
        connection_string.starts_with("mongodb://")
            || connection_string.starts_with("mongodb+srv://")
    }

    fn initialize_search_registry(
        registry: &Arc<RwLock<SearchParameterRegistry>>,
        config: &MongoBackendConfig,
    ) {
        let loader = SearchParameterLoader::new(config.fhir_version);
        let mut reg = registry.write();

        let mut fallback_count = 0;
        let mut spec_count = 0;
        let mut spec_file: Option<PathBuf> = None;
        let mut custom_count = 0;
        let mut custom_files: Vec<String> = Vec::new();

        // 1. Load minimal embedded fallback params.
        match loader.load_embedded() {
            Ok(params) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        fallback_count += 1;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to load embedded SearchParameters: {}", e);
            }
        }

        // 2. Load spec file params.
        let data_dir = config
            .data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./data"));
        let spec_filename = loader.spec_filename();
        let spec_path = data_dir.join(spec_filename);
        match loader.load_from_spec_file(&data_dir) {
            Ok(params) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        spec_count += 1;
                    }
                }
                if spec_count > 0 {
                    spec_file = Some(spec_path);
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Could not load spec SearchParameters from {}: {}. Using minimal fallback.",
                    spec_path.display(),
                    e
                );
            }
        }

        // 3. Load custom SearchParameters.
        match loader.load_custom_from_directory_with_files(&data_dir) {
            Ok((params, files)) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        custom_count += 1;
                    }
                }
                custom_files = files;
            }
            Err(e) => {
                tracing::warn!(
                    "Error loading custom SearchParameters from {}: {}",
                    data_dir.display(),
                    e
                );
            }
        }

        let resource_type_count = reg.resource_types().len();
        let spec_info = spec_file
            .map(|p| format!(" from {}", p.display()))
            .unwrap_or_default();
        let custom_info = if custom_files.is_empty() {
            String::new()
        } else {
            format!(" [{}]", custom_files.join(", "))
        };

        tracing::info!(
            "MongoDB SearchParameter registry initialized: {} total ({} spec{}, {} fallback, {} custom{}) covering {} resource types",
            reg.len(),
            spec_count,
            spec_info,
            fallback_count,
            custom_count,
            custom_info,
            resource_type_count
        );
    }

    /// Initializes the MongoDB schema/index bootstrap for this backend.
    ///
    /// Inline-class indexes are created before this returns. The
    /// generation-4 `search_index` indexes are built by `SearchIndexBuilder`,
    /// which moves any contained rows out of `search_index` first, in every
    /// mode (#1160): spawned and left running in `background` mode, awaited
    /// in `inline` mode, and only inspected in `off` mode (see
    /// `IndexBuildMode`).
    pub async fn init_schema(&self) -> StorageResult<()> {
        let db = self.get_database().await?;
        schema::initialize_schema_async(&db).await?;

        // Taken once: `init_schema` runs once per backend instance (boot). A
        // second call finds `None` here and skips spawning another build —
        // `search_index_rx` still carries the first build's outcome.
        let tx = self.search_index_tx.lock().await.take();
        if let Some(tx) = tx {
            let builder = SearchIndexBuilder::new(db.clone(), self.config.index_build);
            let handle = tokio::spawn(async move {
                let outcome = builder.run().await;
                let _ = tx.send(Some(outcome));
            });
            if self.config.index_build == IndexBuildMode::Inline {
                handle.await.map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "mongodb".to_string(),
                        message: format!("search_index build task panicked: {e}"),
                        source: None,
                    })
                })?;
                if let Some(BuildOutcome::Failed { message }) =
                    self.search_index_rx.borrow().clone()
                {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "mongodb".to_string(),
                        message,
                        source: None,
                    }));
                }
            }
        }

        // Populate the per-tenant stored-param cache so the registries can build
        // each tenant's overlay lazily.
        self.reload_stored_cache().await?;
        Ok(())
    }

    /// Waits for the post-boot `search_index` build started by `init_schema`
    /// and returns its outcome; `None` if `init_schema` has not run. Safe to
    /// call repeatedly: the outcome is kept. Holds no lock across an `.await`,
    /// so it is safe to cancel (e.g. the caller's future is dropped mid-wait).
    pub async fn wait_for_search_index_build(&self) -> Option<BuildOutcome> {
        {
            let sender = self.search_index_tx.lock().await;
            if sender.is_some() {
                // init_schema hasn't taken the sender yet, so it hasn't run.
                return None;
            }
        }
        let mut rx = self.search_index_rx.clone();
        loop {
            if let Some(outcome) = rx.borrow().clone() {
                return Some(outcome);
            }
            // `Err` means the sender was dropped without ever sending: the
            // spawned task ended (e.g. panicked) before publishing an outcome.
            if rx.changed().await.is_err() {
                return Some(BuildOutcome::Failed {
                    message: "search_index build task ended without reporting".to_string(),
                });
            }
        }
    }

    /// The filter [`Self::reload_stored_cache_for_tenant`] reads one tenant's
    /// stored SearchParameters with. It bounds `tenant_id`, so it is answered
    /// by `idx_resources_type_scan` (`tenant_id, resource_type, is_deleted,
    /// …`) instead of a `COLLSCAN` over every tenant's resources (#1764).
    ///
    /// `#[doc(hidden)] pub` only so the integration tests can `explain` the
    /// exact filter and pin that it seeks by `tenant_id`.
    #[doc(hidden)]
    pub fn stored_search_parameters_filter(tenant_id: &str) -> mongodb::bson::Document {
        mongodb::bson::doc! {
            "tenant_id": tenant_id,
            "resource_type": "SearchParameter",
            "is_deleted": false,
        }
    }

    /// Reloads every tenant's stored active SearchParameters into the sync
    /// `stored_by_tenant` cache (grouped by tenant), then drops the cached
    /// per-tenant registries so they rebuild against the fresh overlay.
    ///
    /// Used at startup and by the TTL refresh. Every index on `resources` leads
    /// with `tenant_id`, so a single tenant-less query would scan the whole
    /// collection (#1764). Instead this lists the tenants with a `distinct` on
    /// `tenant_id` (a `DISTINCT_SCAN` of the identity index) and then seeks
    /// each tenant's SearchParameters on its own, so the cost follows the
    /// number of tenants and SearchParameters, not the size of `resources`.
    pub(crate) async fn reload_stored_cache(&self) -> StorageResult<usize> {
        use mongodb::bson::{Bson, Document, doc};

        let db = self.get_database().await?;
        let collection = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let tenant_ids = collection
            .distinct("tenant_id", doc! {})
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message: format!("Failed to list tenants for SearchParameters: {}", e),
                    source: None,
                })
            })?;

        let mut by_tenant: std::collections::HashMap<String, Vec<SearchParameterDefinition>> =
            std::collections::HashMap::new();
        let mut count = 0;
        for tenant_id in tenant_ids {
            let Bson::String(tenant_id) = tenant_id else {
                continue;
            };
            let defs = self
                .load_stored_search_parameters(&collection, &tenant_id)
                .await?;
            if !defs.is_empty() {
                count += defs.len();
                by_tenant.insert(tenant_id, defs);
            }
        }

        *self.stored_by_tenant.write() = by_tenant;
        self.registries.invalidate_all();
        Ok(count)
    }

    /// Reloads one tenant's stored active SearchParameters into the sync
    /// `stored_by_tenant` cache, then drops only that tenant's cached registry
    /// so it rebuilds against the fresh overlay on next access.
    ///
    /// The SearchParameter write paths call this with the writing tenant: a
    /// write in one tenant cannot change another tenant's overlay, so there is
    /// no reason to reload (or invalidate) the others (#1764).
    pub(crate) async fn reload_stored_cache_for_tenant(
        &self,
        tenant_id: &str,
    ) -> StorageResult<usize> {
        use mongodb::bson::Document;

        let db = self.get_database().await?;
        let collection = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let defs = self
            .load_stored_search_parameters(&collection, tenant_id)
            .await?;
        let count = defs.len();

        {
            let mut stored = self.stored_by_tenant.write();
            if defs.is_empty() {
                stored.remove(tenant_id);
            } else {
                stored.insert(tenant_id.to_string(), defs);
            }
        }
        self.registries.invalidate(tenant_id);
        Ok(count)
    }

    /// Reads `tenant_id`'s live, `active` stored SearchParameters, skipping
    /// any document that cannot be read or parsed.
    async fn load_stored_search_parameters(
        &self,
        collection: &mongodb::Collection<mongodb::bson::Document>,
        tenant_id: &str,
    ) -> StorageResult<Vec<SearchParameterDefinition>> {
        use crate::search::registry::{SearchParameterSource, SearchParameterStatus};

        let mut cursor = collection
            .find(Self::stored_search_parameters_filter(tenant_id))
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message: format!("Failed to query SearchParameters: {}", e),
                    source: None,
                })
            })?;

        let loader = SearchParameterLoader::new(self.config.fhir_version);
        let mut defs = Vec::new();
        while cursor.advance().await.map_err(|e| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("SearchParameter cursor advance: {}", e),
                source: None,
            })
        })? {
            let document = match cursor.deserialize_current() {
                Ok(document) => document,
                Err(e) => {
                    tracing::warn!("Failed to read SearchParameter document: {}", e);
                    continue;
                }
            };
            let Ok(payload) = document.get_document("data") else {
                tracing::warn!("Stored SearchParameter document has no data payload");
                continue;
            };
            let json = match super::storage::document_to_value(payload) {
                Ok(json) => json,
                Err(e) => {
                    tracing::warn!("Failed to convert SearchParameter payload: {}", e);
                    continue;
                }
            };
            if let Ok(mut def) = loader.parse_resource(&json)
                && def.status == SearchParameterStatus::Active
            {
                def.source = SearchParameterSource::Stored;
                defs.push(def);
            }
        }
        Ok(defs)
    }

    /// TTL-cache refresh (#235): reload the stored-param cache and drop the
    /// cached per-tenant registries.
    pub async fn refresh_stored_search_parameters(&self) -> StorageResult<usize> {
        self.reload_stored_cache().await
    }

    /// Creates a MongoDB client from backend configuration.
    /// Returns the shared MongoDB client for this backend.
    pub(crate) async fn get_client(&self) -> StorageResult<Client> {
        self.client
            .get_or_try_init(|| connect_client(&self.config))
            .await
            .cloned()
    }

    /// Returns a handle to the shared client cell, so collaborators constructed
    /// from `&self` (e.g. the in-DB SOF runner) can lazily initialise and reuse
    /// the same pooled client.
    pub(crate) fn client_cell(&self) -> Arc<OnceCell<Client>> {
        Arc::clone(&self.client)
    }

    /// Returns the configured MongoDB database handle.
    ///
    /// `#[doc(hidden)] pub` rather than `pub(crate)` only so the out-of-crate
    /// bulk-export window tests (`tests/mongodb_tests.rs`) can pin a stored
    /// resource's `last_updated` and assert on `_since`/`_until` without
    /// depending on wall-clock timing. It hands out a raw database handle that
    /// bypasses tenant scoping, so it is not stable API — workspace callers
    /// should use the `ResourceStorage`/`SearchProvider` methods instead.
    #[doc(hidden)]
    pub async fn get_database(&self) -> StorageResult<Database> {
        let client = self.get_client().await?;
        Ok(client.database(&self.config.database_name))
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &MongoBackendConfig {
        &self.config
    }

    /// The per-tenant registry container (shared with a co-located ES backend).
    pub fn tenant_registries(&self) -> &Arc<TenantSearchRegistries> {
        &self.registries
    }

    /// The shared base registry (embedded + spec + custom), tenant-independent.
    pub(crate) fn base_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        self.registries.base()
    }

    /// The registry for a tenant (base + that tenant's stored overlay).
    pub(crate) fn tenant_registry(&self, tenant_id: &str) -> Arc<RwLock<SearchParameterRegistry>> {
        self.registries.for_tenant(tenant_id)
    }

    /// A value extractor over a tenant's registry.
    pub(crate) fn tenant_extractor(&self, tenant_id: &str) -> SearchParameterExtractor {
        SearchParameterExtractor::new(self.tenant_registry(tenant_id))
    }

    /// Returns whether search indexing is offloaded to a secondary backend.
    pub fn is_search_offloaded(&self) -> bool {
        self.config.search_offloaded
    }

    /// Sets the search-offloaded flag.
    pub fn set_search_offloaded(&mut self, offloaded: bool) {
        self.config.search_offloaded = offloaded;
    }
}

/// Connection wrapper for MongoDB.
#[derive(Clone)]
pub struct MongoConnection {
    pub(crate) database: Database,
}

impl Debug for MongoConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoConnection")
            .field("database", &self.database.name())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for MongoBackend {
    type Connection = MongoConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::MongoDB
    }

    fn name(&self) -> &'static str {
        "mongodb"
    }

    fn supports(&self, capability: BackendCapability) -> bool {
        Self::declared_capabilities().contains(&capability)
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        Self::declared_capabilities()
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        let client = self
            .get_client()
            .await
            .map_err(|e| BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: e.to_string(),
            })?;
        let database = client.database(&self.config.database_name);
        Ok(MongoConnection { database })
    }

    async fn release(&self, _conn: Self::Connection) {
        // MongoDB connection pooling is managed by the client internally.
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        if !Self::looks_like_mongodb_uri(&self.config.connection_string) {
            return Err(BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: "Invalid MongoDB connection string format".to_string(),
            });
        }

        let db = self
            .get_database()
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: format!("Unable to create database handle: {}", e),
            })?;

        db.run_command(doc! { "ping": 1_i32 })
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: format!("Health check failed: {}", e),
            })?;

        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to initialize schema: {}", e),
                source: None,
            })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        let db = self
            .get_database()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to acquire database for migration: {}", e),
                source: None,
            })?;

        schema::migrate_schema_async(&db)
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to run migrations: {}", e),
                source: None,
            })
    }
}

// ============================================================================
// SearchCapabilityProvider Implementation
// ============================================================================

use crate::core::capabilities::{
    GlobalSearchCapabilities, ResourceSearchCapabilities, SearchCapabilityProvider,
};
use crate::types::{
    IncludeCapability, PaginationCapability, ResultModeCapability, SearchParamFullCapability,
    SearchParamType, SpecialSearchParam,
};

impl MongoBackend {
    /// Returns the search modifiers the MongoDB backend actually honors for a
    /// parameter type.
    ///
    /// `:missing` (true and false) and token `:not` ARE implemented (#881):
    /// both are resolved generically in `search_impl::matching_resource_ids`
    /// from search-index-entry presence/absence and positive-filter
    /// complement respectively, *before* any type-specific value parsing
    /// runs — which is why `:missing` is safe to advertise on date/number/
    /// quantity even though a literal `"true"`/`"false"` value would fail
    /// their parsers. They are advertised on exactly the types
    /// `SearchModifier::is_valid_for` permits (the same gate
    /// `search_query_builder` uses to reject a modifier before it reaches any
    /// backend): `:missing` on every index-backed type (string, token, date,
    /// number, quantity, reference, uri), `:not` on token only. Composite
    /// search itself is served (#1206) by the grouped `(resource_id,
    /// composite_group)` pair check in `matching_resource_ids`;
    /// `missing_presence_filter` would serve `:missing` on a composite for a
    /// direct `SearchProvider` caller, but over REST every modifier on a
    /// composite is rejected upstream by `SearchModifier::is_valid_for`; the
    /// backend itself rejects every composite modifier other than
    /// `:missing` in `validate_query_support`. Special params stay
    /// unadvertised for both because they are never indexed by value.
    ///
    /// `_id` and `_lastUpdated` take a different route and are no longer
    /// imprecise here (#1055). `matching_resource_ids` skips them by name and
    /// hands them to `build_resource_id_condition` /
    /// `build_resource_last_updated_conditions`, which now honour the
    /// modifiers this function advertises for them: `:not` and `:missing` on
    /// `_id`, `:missing` on `_lastUpdated`. Anything else on those two names
    /// is rejected with `UnsupportedModifier` rather than silently degrading
    /// to a positive match — which is what `_id:not` used to do, returning
    /// exactly the resource the caller asked to exclude.
    ///
    /// One residual imprecision: this function is keyed on parameter *type*,
    /// so the Token arm advertises `text`/`code-text` for every token param
    /// including `_id` (via its Token fallback), while
    /// `validate_query_support` now rejects `_id:text`. Advertised-but-
    /// rejected is a visible 400 rather than the old silent wrong answer;
    /// narrowing it would need a name-aware capability path.
    ///
    /// `:above`/`:below` are implemented for `uri` (#1002) and `reference`
    /// (#1408): `build_uri_filter` / `build_reference_filter` resolve them
    /// segment-aware, mirroring SQLite and Elasticsearch, so they are
    /// advertised there. Token `:of-type` and reference `:identifier` are
    /// served too (#1408) — the latter by `resolve_reference_identifier` in
    /// `matching_resource_ids`, ahead of the filter builders — as is the
    /// reference `:[type]` qualifier, which like on the other backends is not
    /// a named modifier and so is not listed. `:in`/`:not-in` stay
    /// unimplemented and unadvertised for every type (rejected outright by
    /// `validate_query_support`), as do token `:above`/`:below` (terminology
    /// subsumption) and `:text-advanced` on token and reference — each still
    /// hits an `UnsupportedModifier` catch-all in its type-specific builder.
    pub(super) fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text", "missing"],
            SearchParamType::Token => vec!["text", "code-text", "of-type", "not", "missing"],
            SearchParamType::Reference => vec![
                "identifier",
                "contains",
                "text",
                "code-text",
                "below",
                "above",
                "missing",
            ],
            SearchParamType::Uri => vec!["exact", "contains", "below", "above", "missing"],
            SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity => {
                vec!["missing"]
            }
            SearchParamType::Composite | SearchParamType::Special => vec![],
        }
    }
}

impl SearchCapabilityProvider for MongoBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        let params = {
            let registry = self.base_registry().read();
            registry.get_active_params(resource_type)
        };
        let common_params = {
            let registry = self.base_registry().read();
            registry.get_active_params("Resource")
        };
        if params.is_empty() && common_params.is_empty() {
            return None;
        }

        let mut search_params = Vec::new();
        for param in &params {
            let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                .with_definition(&param.url)
                .with_modifiers(Self::modifiers_for_type(param.param_type));
            if let Some(ref targets) = param.target {
                cap = cap.with_targets(targets.iter().map(|s| s.as_str()));
            }
            search_params.push(cap);
        }
        for param in &common_params {
            if !search_params.iter().any(|p| p.name == param.code) {
                search_params.push(
                    SearchParamFullCapability::new(&param.code, param.param_type)
                        .with_definition(&param.url)
                        .with_modifiers(Self::modifiers_for_type(param.param_type)),
                );
            }
        }

        Some(
            ResourceSearchCapabilities::new(resource_type)
                .with_special_params(vec![SpecialSearchParam::Id])
                .with_include_capabilities(vec![
                    IncludeCapability::Include,
                    IncludeCapability::Revinclude,
                ])
                .with_pagination_capabilities(vec![
                    PaginationCapability::Count,
                    PaginationCapability::Offset,
                    PaginationCapability::MaxPageSize(1000),
                    PaginationCapability::DefaultPageSize(20),
                ])
                .with_result_mode_capabilities(vec![ResultModeCapability::Total])
                .with_param_list(search_params),
        )
    }

    fn global_search_capabilities(&self) -> GlobalSearchCapabilities {
        GlobalSearchCapabilities::new().with_special_params(vec![SpecialSearchParam::Id])
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::types::{SearchModifier, SearchParameter, SearchValue};

    #[test]
    fn test_modifiers_for_type_reflects_mongo_support() {
        // String honors exact/contains/text, and :missing (#881, presence-only).
        let s = MongoBackend::modifiers_for_type(SearchParamType::String);
        assert!(s.contains(&"exact"));
        assert!(s.contains(&"text"));
        assert!(s.contains(&"missing"));

        // Token honors text/code-text but not the non-spec :code; :not and
        // :missing ARE honored (#881, generically in `matching_resource_ids`),
        // and so is :of-type (#1408). Terminology-backed modifiers are not.
        let t = MongoBackend::modifiers_for_type(SearchParamType::Token);
        assert!(!t.contains(&"code"));
        assert!(t.contains(&"code-text"));
        assert!(t.contains(&"not"));
        assert!(t.contains(&"missing"));
        assert!(t.contains(&"of-type"));
        assert!(!t.contains(&"in"));
        assert!(!t.contains(&"above"));

        // Reference honors contains/text/code-text/missing, and :identifier
        // and :above/:below (#1408).
        let r = MongoBackend::modifiers_for_type(SearchParamType::Reference);
        assert!(r.contains(&"contains"));
        assert!(r.contains(&"text"));
        assert!(r.contains(&"missing"));
        assert!(r.contains(&"identifier"));
        assert!(r.contains(&"above"));
        assert!(r.contains(&"below"));

        // Uri honors exact/contains/missing and :above/:below (#1002,
        // segment-aware in build_uri_filter).
        let u = MongoBackend::modifiers_for_type(SearchParamType::Uri);
        assert!(u.contains(&"contains"));
        assert!(u.contains(&"missing"));
        assert!(u.contains(&"above"));
        assert!(u.contains(&"below"));
    }

    /// Every `SearchParamType` variant paired with a value `matching_resource_ids`
    /// (`search_impl.rs`) can build a *positive* index filter from. Written as
    /// an exhaustive match with no wildcard arm: adding a new `SearchParamType`
    /// variant makes this fail to compile, which is the point — it forces the
    /// author to also decide what `modifiers_for_type` should advertise for it,
    /// rather than silently skipping it in these tests.
    fn sample_value_for(param_type: SearchParamType) -> &'static str {
        match param_type {
            SearchParamType::String => "smith",
            SearchParamType::Uri => "http://example.org/x",
            SearchParamType::Number => "5",
            SearchParamType::Date => "2020-01-01",
            SearchParamType::Quantity => "5|http://unitsofmeasure.org|mg",
            SearchParamType::Token => "http://loinc.org|1234-5",
            SearchParamType::Reference => "Patient/123",
            SearchParamType::Composite => "unused-composite-has-no-single-value",
            SearchParamType::Special => "unused-special-has-no-single-value",
        }
    }

    /// All nine `SearchParamType` variants, built through `sample_value_for` so
    /// the exhaustiveness guarantee above actually applies to this list.
    fn all_param_types() -> Vec<SearchParamType> {
        [
            SearchParamType::String,
            SearchParamType::Uri,
            SearchParamType::Number,
            SearchParamType::Date,
            SearchParamType::Quantity,
            SearchParamType::Token,
            SearchParamType::Reference,
            SearchParamType::Composite,
            SearchParamType::Special,
        ]
        .into_iter()
        .inspect(|t| {
            let _ = sample_value_for(*t);
        })
        .collect()
    }

    /// #1054: the CapabilityStatement must advertise `:missing`/`:not` on
    /// exactly the parameter types where a client is actually allowed to send
    /// them. `SearchModifier::is_valid_for` is that gate in production — it is
    /// what `crates/rest/src/extractors/search_query_builder.rs` calls to 400 a
    /// modifier before it ever reaches a backend — so the expected answer is
    /// derived from it rather than hardcoded here. Both modifiers are resolved
    /// generically (type-agnostically) in
    /// `MongoBackend::matching_resource_ids` (`search_impl.rs`, #881): `:missing`
    /// from search-index-entry presence/absence, `:not` from the positive
    /// filter's complement. Neither reaches type-specific value parsing, which
    /// is why `:missing` is safe on date/number/quantity (see
    /// `query_support_tests::missing_is_supported_without_type_specific_value_parsing`
    /// in search_impl.rs) and is not gated here on anything Mongo-specific.
    #[test]
    fn missing_and_not_are_advertised_wherever_the_gate_allows_them() {
        for param_type in all_param_types() {
            let advertised = MongoBackend::modifiers_for_type(param_type);

            assert_eq!(
                advertised.contains(&"missing"),
                SearchModifier::Missing.is_valid_for(param_type),
                "advertised `missing` for {param_type} disagrees with \
                 SearchModifier::Missing::is_valid_for"
            );
            assert_eq!(
                advertised.contains(&"not"),
                SearchModifier::Not.is_valid_for(param_type),
                "advertised `not` for {param_type} disagrees with \
                 SearchModifier::Not::is_valid_for"
            );
        }
    }

    /// Companion guard to the test above: for every modifier `modifiers_for_type`
    /// actually advertises for a type, the Mongo index-filter path must be able
    /// to build a filter from it (never advertise something the query builders
    /// reject), the string must round-trip through `SearchModifier::parse` (no
    /// stray/typo'd modifier spellings), and it must never be one of the four
    /// modifiers `MongoBackend::validate_query_support` (`search_impl.rs`,
    /// private to that module) hard-rejects for every type regardless of
    /// per-type support. This is the closest hermetic stand-in for a literal
    /// cross-check against `validate_query_support`: that method is private to
    /// `search_impl`, which is off-limits for edits this wave, so it cannot be
    /// called directly from here.
    #[test]
    fn advertised_modifiers_are_honored_by_the_mongo_index_path() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();

        for param_type in all_param_types() {
            for &modifier_str in &MongoBackend::modifiers_for_type(param_type) {
                // `validate_query_support` (search_impl.rs) rejects `:in`/
                // `:not-in` outright for every parameter type; advertising
                // either would be a straightforward regression back to
                // over-promising. `:above`/`:below` are rejected for every
                // type EXCEPT uri (#1002) and reference (#1408), whose
                // builders resolve them segment-aware.
                assert!(
                    !matches!(modifier_str, "in" | "not-in"),
                    "{param_type} advertises `{modifier_str}`, which \
                     validate_query_support rejects unconditionally"
                );
                assert!(
                    !matches!(modifier_str, "above" | "below")
                        || matches!(
                            param_type,
                            SearchParamType::Uri | SearchParamType::Reference
                        ),
                    "{param_type} advertises `{modifier_str}`, which \
                     validate_query_support rejects for every type except uri and reference"
                );

                // The advertised string must be a real, round-trippable
                // modifier spelling, not a typo or a legacy alias.
                let parsed = SearchModifier::parse(modifier_str).unwrap_or_else(|| {
                    panic!(
                        "{param_type} advertises `{modifier_str}`, which \
                         SearchModifier::parse does not recognize"
                    )
                });
                // `Display` writes `of-type` in its legacy camelCase, which
                // `parse` reads back; every other spelling is its own.
                assert_eq!(
                    SearchModifier::parse(&parsed.to_string()),
                    Some(parsed.clone()),
                    "{param_type}'s advertised `{modifier_str}` does not round-trip \
                     through SearchModifier::parse/Display"
                );
                assert!(
                    parsed.to_string() == modifier_str || modifier_str == "of-type",
                    "{param_type} advertises `{modifier_str}`, which is not the \
                     spelling SearchModifier writes"
                );

                // `:missing` and `:not` are resolved generically in
                // `matching_resource_ids` *before* any type-specific value
                // filter is built (#881: presence-only / positive-complement),
                // so the faithful probe for them clears the modifier rather
                // than setting it — setting it would wrongly fail (e.g. token's
                // `build_token_filter` has no `Missing`/`Not` arm and would hit
                // its `Some(other) => Err(UnsupportedModifier)` catch-all).
                // Reference `:identifier` is resolved there too (#1408), by
                // `resolve_reference_identifier`: it needs the database, which
                // a filter builder does not have, so the builder never sees it.
                let probe_modifier = match modifier_str {
                    "missing" | "not" | "identifier" => None,
                    other => Some(SearchModifier::parse(other).unwrap()),
                };

                let param = SearchParameter {
                    name: "test_param".to_string(),
                    param_type,
                    modifier: probe_modifier,
                    values: vec![SearchValue::eq(sample_value_for(param_type))],
                    ..Default::default()
                };

                let result = backend.build_search_index_filter("t1", "Patient", &param);
                assert!(
                    result.is_ok(),
                    "{param_type} advertises `{modifier_str}` but the Mongo index \
                     filter path rejected it: {:?}",
                    result.err()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_defaults_to_the_historical_constant() {
        assert_eq!(MongoBackendConfig::default().app_name, "helios-persistence");
    }

    #[test]
    fn config_index_build_defaults_to_background_and_reads_env() {
        assert_eq!(
            MongoBackendConfig::default().index_build,
            IndexBuildMode::Background
        );
        // from_env is process-global; guard the variable.
        //
        // SAFETY: this is the only test in this module that touches the
        // environment; it runs single-threaded relative to itself and
        // restores the variable before returning, so there is no
        // cross-test data race on the process environment.
        unsafe { std::env::set_var("HFS_MONGODB_INDEX_BUILD", "inline") };
        let backend = MongoBackend::from_env().expect("from_env");
        assert_eq!(backend.config().index_build, IndexBuildMode::Inline);
        unsafe { std::env::set_var("HFS_MONGODB_INDEX_BUILD", "nonsense") };
        let err = MongoBackend::from_env().expect_err("invalid mode must be rejected");
        assert!(format!("{err}").contains("HFS_MONGODB_INDEX_BUILD"));
        unsafe { std::env::remove_var("HFS_MONGODB_INDEX_BUILD") };
    }

    #[test]
    fn config_reindex_pipeline_defaults() {
        let default = MongoBackendConfig::default();
        assert!(default.reindex_overlap);
        assert_eq!(default.reindex_prepare_threads, 0);
        assert!(default.reindex_prefetch);

        let from_empty: MongoBackendConfig =
            serde_json::from_str("{}").expect("every field must have a serde default");
        assert!(from_empty.reindex_overlap);
        assert_eq!(from_empty.reindex_prepare_threads, 0);
        assert!(from_empty.reindex_prefetch);
    }

    #[test]
    fn config_bundle_transaction_budget_defaults_to_120_seconds_and_is_settable() {
        let default = MongoBackendConfig::default();
        assert_eq!(default.bundle_transaction_budget, Duration::from_secs(120));

        // An older serialized config without the field still loads.
        let from_empty: MongoBackendConfig =
            serde_json::from_str("{}").expect("every field must have a serde default");
        assert_eq!(
            from_empty.bundle_transaction_budget,
            Duration::from_secs(120)
        );

        // The field survives a round trip and `..Default::default()` keeps it.
        let config = MongoBackendConfig {
            bundle_transaction_budget: Duration::from_secs(28),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serializes");
        let back: MongoBackendConfig = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.bundle_transaction_budget, Duration::from_secs(28));
    }

    #[test]
    fn apply_search_env_reads_and_rejects() {
        let mut config = MongoBackendConfig::default();
        config
            .apply_search_env(|_| Some(" 4 ".to_string()))
            .expect("valid value");
        assert_eq!(config.broad_search_concurrency, Some(4));

        let mut config = MongoBackendConfig {
            broad_search_concurrency: Some(2),
            ..Default::default()
        };
        config
            .apply_search_env(|_| Some("  ".to_string()))
            .expect("a blank value is ignored");
        assert_eq!(config.broad_search_concurrency, Some(2));
        config.apply_search_env(|_| None).expect("unset is ignored");
        assert_eq!(config.broad_search_concurrency, Some(2));

        for invalid in ["0", "-1", "two", "1.5"] {
            let err = MongoBackendConfig::default()
                .apply_search_env(|_| Some(invalid.to_string()))
                .expect_err("invalid value");
            assert!(
                err.contains("HFS_MONGODB_BROAD_SEARCH_CONCURRENCY"),
                "{err}"
            );
        }
    }

    #[test]
    fn count_by_tenant_max_time_defaults_to_30_seconds_and_survives_serde() {
        let default = MongoBackendConfig::default();
        assert_eq!(default.count_by_tenant_max_time_ms, 30_000);
        assert_eq!(
            default.count_by_tenant_max_time_ms,
            DEFAULT_COUNT_BY_TENANT_MAX_TIME_MS
        );

        // An older serialized config without the field still loads.
        let from_empty: MongoBackendConfig =
            serde_json::from_str("{}").expect("every field must have a serde default");
        assert_eq!(from_empty.count_by_tenant_max_time_ms, 30_000);

        let config = MongoBackendConfig {
            count_by_tenant_max_time_ms: 1_500,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serializes");
        let back: MongoBackendConfig = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.count_by_tenant_max_time_ms, 1_500);
    }

    #[test]
    fn apply_count_env_reads_and_rejects() {
        let mut config = MongoBackendConfig::default();
        config
            .apply_count_env(|name| {
                (name == "HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS").then(|| " 45000 ".to_string())
            })
            .expect("valid value");
        assert_eq!(config.count_by_tenant_max_time_ms, 45_000);

        let mut config = MongoBackendConfig {
            count_by_tenant_max_time_ms: 2_000,
            ..Default::default()
        };
        config
            .apply_count_env(|_| Some("  ".to_string()))
            .expect("a blank value is ignored");
        assert_eq!(config.count_by_tenant_max_time_ms, 2_000);
        config.apply_count_env(|_| None).expect("unset is ignored");
        assert_eq!(config.count_by_tenant_max_time_ms, 2_000);

        let mut config = MongoBackendConfig::default();
        config
            .apply_count_env(|_| Some(i32::MAX.to_string()))
            .expect("the server's maximum is accepted");
        assert_eq!(config.count_by_tenant_max_time_ms, i32::MAX as u64);

        for invalid in ["0", "-1", "thirty", "1.5", "2147483648"] {
            let mut config = MongoBackendConfig::default();
            let err = config
                .apply_count_env(|_| Some(invalid.to_string()))
                .expect_err("invalid value");
            assert!(
                err.contains("HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS"),
                "{invalid}: {err}"
            );
            assert_eq!(
                config.count_by_tenant_max_time_ms, 30_000,
                "{invalid}: a rejected value leaves the field unchanged"
            );
        }
    }

    #[test]
    fn new_rejects_an_out_of_range_count_by_tenant_budget() {
        for invalid in [0, i32::MAX as u64 + 1] {
            let err = MongoBackend::new(MongoBackendConfig {
                count_by_tenant_max_time_ms: invalid,
                ..Default::default()
            })
            .expect_err("out-of-range budget must be refused");
            assert!(
                err.to_string()
                    .contains("HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS"),
                "{invalid}: {err}"
            );
        }
        assert!(
            MongoBackend::new(MongoBackendConfig {
                count_by_tenant_max_time_ms: 1,
                ..Default::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn broad_search_gate_is_sized_from_the_config() {
        let permits = |max_connections, broad_search_concurrency| {
            MongoBackend::new(MongoBackendConfig {
                max_connections,
                broad_search_concurrency,
                ..Default::default()
            })
            .expect("valid config")
            .broad_search_gate()
            .map(tokio::sync::Semaphore::available_permits)
        };
        assert_eq!(permits(10, None), None, "unset means no limit");
        assert_eq!(permits(1, None), None);
        assert_eq!(permits(10, Some(3)), Some(3));
        assert_eq!(permits(10, Some(12)), Some(12));
        assert!(
            MongoBackend::new(MongoBackendConfig {
                broad_search_concurrency: Some(0),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn broad_searches_wait_for_a_permit_and_narrow_ones_do_not() {
        use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

        let backend = MongoBackend::new(MongoBackendConfig {
            broad_search_concurrency: Some(1),
            ..Default::default()
        })
        .expect("valid config");
        let token = |name: &str, param_type, value: &str| SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        };
        let mut broad = SearchQuery::new("Observation");
        broad.parameters = vec![token("code", SearchParamType::Token, "1234-5")];
        let mut narrow = SearchQuery::new("Observation");
        narrow.parameters = vec![token("subject", SearchParamType::Reference, "Patient/p-1")];

        let held = backend
            .admit_broad_search(&broad)
            .await
            .expect("gate open")
            .expect("a broad search takes a permit");
        let waiting = backend.admit_broad_search(&broad);
        tokio::pin!(waiting);
        assert!(futures::poll!(&mut waiting).is_pending());
        assert!(
            backend
                .admit_broad_search(&narrow)
                .await
                .expect("gate open")
                .is_none(),
            "a narrow search never waits"
        );
        drop(held);
        let next = waiting.await.expect("gate open");
        assert!(
            next.is_some(),
            "the waiting search gets the released permit"
        );
        drop(next);
        assert_eq!(
            backend
                .broad_search_gate()
                .map(tokio::sync::Semaphore::available_permits),
            Some(1)
        );
    }

    #[tokio::test]
    async fn broad_search_error_returns_its_permit() {
        use crate::core::SearchProvider;
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:1".into(),
            broad_search_concurrency: Some(1),
            ..Default::default()
        })
        .unwrap();
        let mut query = crate::types::SearchQuery::new("Observation");
        query.total = Some(crate::types::TotalMode::Accurate);
        query.cursor = Some("invalid-cursor".into());
        let tenant = crate::tenant::TenantContext::system();
        let result = tokio::time::timeout(Duration::from_secs(1), backend.search(&tenant, &query))
            .await
            .expect("cursor validation does not need a live database");
        assert!(matches!(
            result,
            Err(StorageError::Search(
                crate::error::SearchError::InvalidCursor { .. }
            ))
        ));
        assert_eq!(backend.broad_search_gate().unwrap().available_permits(), 1);
    }

    #[tokio::test]
    async fn broad_search_count_waits_without_a_total_option_and_releases_on_cancel() {
        use crate::core::SearchProvider;
        use crate::types::SearchQuery;
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:1".into(),
            broad_search_concurrency: Some(1),
            ..Default::default()
        })
        .unwrap();
        let gate = backend.broad_search_gate().unwrap();
        let held = gate.acquire().await.unwrap();
        let tenant = crate::tenant::TenantContext::new(
            crate::tenant::TenantId::new("admission"),
            crate::tenant::TenantPermissions::full_access(),
        );
        let query = SearchQuery::new("Observation");
        let mut counting = Box::pin(backend.search_count(&tenant, &query));
        assert!(futures::poll!(&mut counting).is_pending());
        assert!(
            backend.client.get().is_none(),
            "a queued count must not open the database client"
        );
        drop(held);
        assert!(futures::poll!(&mut counting).is_pending());
        assert_eq!(
            gate.available_permits(),
            0,
            "the admitted count holds its permit"
        );
        drop(counting);
        assert_eq!(
            gate.available_permits(),
            1,
            "cancelling the count returns its permit"
        );
    }

    #[tokio::test]
    async fn without_a_configured_limit_no_search_waits() {
        use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

        let backend = MongoBackend::new(MongoBackendConfig::default()).expect("valid config");
        let mut broad = SearchQuery::new("Observation");
        broad.parameters = vec![SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("1234-5")],
            chain: vec![],
            components: vec![],
        }];
        let permits: Vec<_> =
            futures::future::join_all((0..64).map(|_| backend.admit_broad_search(&broad))).await;
        assert!(
            permits.iter().all(|permit| matches!(permit, Ok(None))),
            "every broad search proceeds without a permit"
        );
    }

    #[test]
    fn config_bundle_transaction_deadline_defaults_to_none_and_round_trips() {
        assert_eq!(
            MongoBackendConfig::default().bundle_transaction_deadline,
            None
        );

        let from_empty: MongoBackendConfig =
            serde_json::from_str("{}").expect("every field must have a serde default");
        assert_eq!(from_empty.bundle_transaction_deadline, None);

        let config = MongoBackendConfig {
            bundle_transaction_deadline: Some(Duration::from_secs(598)),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serializes");
        let back: MongoBackendConfig = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(
            back.bundle_transaction_deadline,
            Some(Duration::from_secs(598))
        );
    }

    #[test]
    fn apply_reindex_env_reads_and_rejects() {
        let mut config = MongoBackendConfig::default();
        config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_OVERLAP" => Some(" On ".to_string()),
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("8".to_string()),
                _ => None,
            })
            .expect("valid values");
        assert!(
            config.reindex_overlap,
            "\" On \" trims and parses case-insensitively"
        );
        assert_eq!(config.reindex_prepare_threads, 8);
        assert!(
            config.reindex_prefetch,
            "an unset variable leaves the default"
        );

        let mut config = MongoBackendConfig {
            reindex_prepare_threads: 3,
            ..Default::default()
        };
        config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some(String::new()),
                _ => None,
            })
            .expect("an empty value leaves the field unchanged");
        assert_eq!(config.reindex_prepare_threads, 3);

        let mut config = MongoBackendConfig::default();
        let err = config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("x".to_string()),
                _ => None,
            })
            .expect_err("a non-integer must be rejected");
        assert!(err.contains("HFS_MONGODB_REINDEX_PREPARE_THREADS"));

        let mut config = MongoBackendConfig::default();
        let err = config
            .apply_reindex_env(|name| match name {
                "HFS_MONGODB_REINDEX_OVERLAP" => Some("sideways".to_string()),
                _ => None,
            })
            .expect_err("a non-boolean must be rejected");
        assert!(err.contains("HFS_MONGODB_REINDEX_OVERLAP"));
    }

    #[test]
    fn config_max_concurrent_transaction_bundles_defaults_to_4_and_round_trips() {
        assert_eq!(
            MongoBackendConfig::default().max_concurrent_transaction_bundles,
            4
        );
        let from_empty: MongoBackendConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty.max_concurrent_transaction_bundles, 4);

        let set = MongoBackendConfig {
            max_concurrent_transaction_bundles: 2,
            ..Default::default()
        };
        let back: MongoBackendConfig =
            serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back.max_concurrent_transaction_bundles, 2);
    }

    #[test]
    fn config_transaction_bundle_weight_entries_defaults_to_1000_and_round_trips() {
        assert_eq!(
            MongoBackendConfig::default().transaction_bundle_weight_entries,
            1000
        );
        let from_empty: MongoBackendConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty.transaction_bundle_weight_entries, 1000);

        let set = MongoBackendConfig {
            transaction_bundle_weight_entries: 50,
            ..Default::default()
        };
        let back: MongoBackendConfig =
            serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back.transaction_bundle_weight_entries, 50);
    }

    #[test]
    fn apply_transaction_bundle_env_reads_the_weight() {
        const VAR: &str = "HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES";
        const LIMIT_VAR: &str = "HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES";
        let with = |name: &'static str, value: &'static str| {
            move |n: &str| (n == name).then(|| value.to_string())
        };

        let mut config = MongoBackendConfig::default();
        config
            .apply_transaction_bundle_env(with(VAR, " 250 "))
            .unwrap();
        assert_eq!(config.transaction_bundle_weight_entries, 250);
        // Only the weight variable is set: the limit is untouched.
        assert_eq!(config.max_concurrent_transaction_bundles, 4);

        config.apply_transaction_bundle_env(with(VAR, "0")).unwrap();
        assert_eq!(config.transaction_bundle_weight_entries, 0);

        let mut config = MongoBackendConfig {
            transaction_bundle_weight_entries: 300,
            ..Default::default()
        };
        config.apply_transaction_bundle_env(with(VAR, "")).unwrap();
        assert_eq!(config.transaction_bundle_weight_entries, 300);
        config.apply_transaction_bundle_env(|_| None).unwrap();
        assert_eq!(config.transaction_bundle_weight_entries, 300);

        // Only the limit variable is set: the weight is untouched.
        config
            .apply_transaction_bundle_env(with(LIMIT_VAR, "7"))
            .unwrap();
        assert_eq!(config.max_concurrent_transaction_bundles, 7);
        assert_eq!(config.transaction_bundle_weight_entries, 300);

        for bad in ["x", "-1"] {
            let err = config
                .apply_transaction_bundle_env(with(VAR, bad))
                .expect_err("a non-negative integer is required");
            assert!(err.contains(VAR), "{err}");
        }
    }

    #[test]
    fn apply_transaction_bundle_env_reads_and_rejects() {
        const VAR: &str = "HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES";
        let with = |value: &'static str| move |name: &str| (name == VAR).then(|| value.to_string());

        let mut config = MongoBackendConfig::default();
        config.apply_transaction_bundle_env(with("8")).unwrap();
        assert_eq!(config.max_concurrent_transaction_bundles, 8);

        config.apply_transaction_bundle_env(with(" 0 ")).unwrap();
        assert_eq!(config.max_concurrent_transaction_bundles, 0);

        let mut config = MongoBackendConfig {
            max_concurrent_transaction_bundles: 3,
            ..Default::default()
        };
        config.apply_transaction_bundle_env(with("")).unwrap();
        assert_eq!(config.max_concurrent_transaction_bundles, 3);
        config.apply_transaction_bundle_env(|_| None).unwrap();
        assert_eq!(config.max_concurrent_transaction_bundles, 3);

        for bad in ["x", "-1"] {
            let err = config
                .apply_transaction_bundle_env(with(bad))
                .expect_err("a non-negative integer is required");
            assert!(err.contains(VAR), "{err}");
        }
    }
}
