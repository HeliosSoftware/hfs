//! PostgreSQL backend implementation.

use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::{Config, GenericClient, Pool, Runtime, SslMode};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio_postgres::NoTls;

use helios_fhir::FhirVersion;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageResult};
use crate::search::{
    SearchParameterDefinition, SearchParameterExtractor, SearchParameterLoader,
    SearchParameterRegistry, SearchParameterSource, SearchParameterStatus, TenantSearchRegistries,
};

/// Sync in-memory cache of each tenant's stored (POSTed) active SearchParameter
/// definitions, keyed by tenant id. Postgres queries are async but the per-tenant
/// registry loader must be sync, so async paths (startup, TTL refresh, and
/// SearchParameter writes) populate this map and the loader reads it.
type StoredByTenant = Arc<RwLock<HashMap<String, StoredOverlay>>>;

/// One tenant's cached stored-SearchParameter overlay.
struct StoredOverlay {
    /// The persisted rows this overlay was read from, when a write validated
    /// it (see [`PostgresBackend::write_extractor`]). `None` after the
    /// all-tenant refresh, which does not fingerprint: the next write for the
    /// tenant re-reads and fingerprints once.
    fingerprint: Option<OverlayFingerprint>,
    /// The tenant's stored active definitions, read by the registry loader.
    definitions: Vec<SearchParameterDefinition>,
    /// The registry a validated write extracts with: the shared base when the
    /// overlay is empty, otherwise the base with the overlay registered on
    /// top. Built once per fingerprint, never per write.
    registry: Option<Arc<RwLock<SearchParameterRegistry>>>,
    /// When the fingerprint probe last confirmed this overlay against the
    /// database. A write within the backend's fingerprint TTL of this instant
    /// reuses the overlay without probing. Replaced (so `None`) by every
    /// local SearchParameter write, whose cache reload drops the entry.
    validated_at: Option<std::time::Instant>,
}

/// Default window in which a write trusts a validated overlay without probing
/// the database again: zero, i.e. every write probes (the exact, previous
/// behavior). Opt in with `HFS_PG_SP_FINGERPRINT_TTL_MS=<ms>` (for example
/// `1000`): a SearchParameter committed by this process still invalidates the
/// cache immediately; only one committed by another HFS process can then take
/// up to that long to be seen by ordinary writes.
const DEFAULT_OVERLAY_FINGERPRINT_TTL: Duration = Duration::ZERO;

fn overlay_fingerprint_ttl_from_env() -> Duration {
    std::env::var("HFS_PG_SP_FINGERPRINT_TTL_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_OVERLAY_FINGERPRINT_TTL)
}

/// How fresh the overlay a write extracts with must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayFreshness {
    /// An overlay validated within the fingerprint TTL is reused without a
    /// probe. For ordinary single-resource and Bundle writes, where a
    /// SearchParameter committed by another process may take up to the TTL to
    /// be seen.
    WithinTtl,
    /// Always probe the database fingerprint, whatever the TTL. For callers
    /// whose batch takes its overlay snapshot under the tenant gate and must
    /// see every SearchParameter committed before that point (bulk submit).
    Authoritative,
}

impl OverlayFreshness {
    /// Whether a recently validated cached overlay may stand in for a probe.
    fn may_skip_probe(self, ttl: Duration, pending_search_parameter: bool) -> bool {
        self == Self::WithinTtl && !pending_search_parameter && !ttl.is_zero()
    }
}

/// The tenant's validated overlay registry, if it was confirmed against the
/// database within `ttl`. `None` means the caller must probe.
fn fresh_overlay_registry(
    stored: &StoredByTenant,
    tenant_id: &str,
    ttl: Duration,
) -> Option<Arc<RwLock<SearchParameterRegistry>>> {
    let stored = stored.read();
    let overlay = stored.get(tenant_id)?;
    overlay
        .validated_at
        .is_some_and(|t| t.elapsed() < ttl)
        .then(|| overlay.registry.clone())
        .flatten()
}

/// A cheap summary of a tenant's live `SearchParameter` rows (#1897).
///
/// Every mutation of a live row moves it: a create or update bumps the newest
/// `last_updated`, a delete lowers the live count (or, for the newest row,
/// the maximum). Only a delete paired with a create whose timestamp does not
/// exceed the previous maximum — clock skew between HFS processes within one
/// write — leaves both unchanged, and the TTL refresh bounds that case as it
/// does every other backend's cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OverlayFingerprint {
    live: i64,
    newest: Option<DateTime<Utc>>,
}

/// PostgreSQL backend for FHIR resource storage.
///
/// Cheap to clone: a pool handle, shared registries and the config.
#[derive(Clone)]
pub struct PostgresBackend {
    pool: Pool,
    config: PostgresConfig,
    /// Per-tenant search parameter registries (shared base + per-tenant overlay).
    registries: Arc<TenantSearchRegistries>,
    /// Sync cache of each tenant's stored params, read by the registry loader.
    stored_by_tenant: StoredByTenant,
    /// How long a validated overlay is trusted without a fingerprint probe.
    overlay_fingerprint_ttl: Duration,
    /// `search_index` layout, read from the marker once the schema is initialized.
    /// Unset until then, which reads as [`IndexLayout::Legacy`] — the form that
    /// is correct against either layout.
    index_layout: Arc<std::sync::OnceLock<super::schema::IndexLayout>>,
    /// Set after schema initialization only when the live-row patient
    /// reference GIN index has been verified. Shared by backend clones.
    patient_export_index_ready: Arc<AtomicBool>,
    /// Whether the `resource_fts` table exists, resolved once.
    ///
    /// The FTS write path used to ask `information_schema.tables` on every
    /// single resource write — 254,970 catalog queries in one 5-minute crud
    /// run. The answer only changes when the schema is created or migrated,
    /// both of which happen before the instance serves traffic.
    pub(super) fts_table_exists: Arc<std::sync::OnceLock<bool>>,
    /// Reindex admission is shared by clones and by every reindex entry point.
    pub(super) reindex_admission: Arc<Semaphore>,
    pub(super) reindex_width: usize,
    pub(super) cleanup_tracker: Arc<super::cleanup::CleanupTracker>,
    #[cfg(test)]
    pub(super) reindex_test_hook: Option<Arc<super::storage::ReindexTestHook>>,
    #[cfg(test)]
    pub(super) reindex_test_pause: Option<Arc<tokio::sync::Notify>>,
    #[cfg(test)]
    pub(super) reindex_test_entered: Option<Arc<tokio::sync::Notify>>,
}

impl Debug for PostgresBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresBackend")
            .field("config", &self.config)
            .field("base_registry_len", &self.registries.base().read().len())
            .finish_non_exhaustive()
    }
}

/// Configuration for the PostgreSQL backend.
///
/// `Debug` is hand-written so the password never reaches a log line (#2149).
#[derive(Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    /// PostgreSQL host.
    #[serde(default = "default_host")]
    pub host: String,

    /// PostgreSQL port.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Database name.
    #[serde(default = "default_dbname")]
    pub dbname: String,

    /// Database user.
    #[serde(default = "default_user")]
    pub user: String,

    /// Database password.
    #[serde(default)]
    pub password: Option<String>,

    /// SSL mode.
    #[serde(default)]
    pub ssl_mode: PostgresSslMode,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    /// Upper bound, in seconds, for establishing a connection to the server.
    ///
    /// Bounds the whole of connection establishment — DNS, TCP, TLS and the
    /// Postgres startup/auth exchange — via deadpool's `create_timeout`, not just
    /// the TCP handshake that tokio-postgres' own `connect_timeout` covers.
    ///
    /// Raise it for servers that are slow to answer a fresh connection: a cold or
    /// loaded cluster, a cross-region failover, or a TLS handshake against a
    /// distant proxy. Settable as `HFS_PG_CONNECT_TIMEOUT_SECS`.
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,

    /// Statement timeout in milliseconds.
    #[serde(default = "default_statement_timeout_ms")]
    pub statement_timeout_ms: u64,

    /// Statement budget, in milliseconds, for the cross-tenant
    /// `count_by_tenant` aggregate and no other statement (#1911;
    /// `HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS`).
    ///
    /// That aggregate reads every live row of every tenant, so its cost grows
    /// with the whole store. It runs as `SET LOCAL statement_timeout` in its
    /// own read-only transaction, so the budget ends with that transaction
    /// and never reaches the pooled session's later statements. When it
    /// expires the call fails with [`BackendError::Timeout`], which callers
    /// must read as "counts unavailable", never as zero.
    ///
    /// The budget only tightens: the count runs under the smaller of this and
    /// [`Self::statement_timeout_ms`] (see
    /// [`Self::effective_count_by_tenant_statement_timeout_ms`]), so raising
    /// it past `HFS_PG_STATEMENT_TIMEOUT_MS` has no effect.
    ///
    /// `GET /admin/tenants`, the existence probe of `DELETE
    /// /admin/tenants/{id}` and the console tenant metrics answer `504` only
    /// when the effective budget is below `HFS_REQUEST_TIMEOUT`, as the
    /// defaults are (25 s against 30 s). The request timeout starts when the
    /// request arrives, before the count, so with a budget at or above it the
    /// request timeout answers `408` first; the budget still ends the count in
    /// PostgreSQL a moment later, and its session stays out of the pool until
    /// then (#1826). The Tenants UI, which counts outside the request timeout,
    /// sees the `Timeout` itself.
    ///
    /// Like `statement_timeout`, it counts lock waits and execution, not
    /// waiting for a pooled connection (`pool_wait_timeout_secs`) or network
    /// time. Those, and the handler's work before the count (`GET
    /// /admin/tenants` lists the tenant registry first), spend the request
    /// timeout's headroom: at defaults a slow count answers `504` only while
    /// they take under the 5 s between budget and request timeout. A pool
    /// checkout that waits longer, possible when the pool is saturated (its
    /// wait is up to 10 s by default), still ends in `408`.
    ///
    /// Default 25 000 (25 s), five seconds under the default 30 s HTTP request
    /// timeout; #1828 picks the same default for the MongoDB backend's count
    /// budget. It is policy, not derived from a large-store measurement. Must be between 1 and 2 147 483 647
    /// (PostgreSQL's limit for `statement_timeout`); [`PostgresBackend::new`]
    /// rejects anything else.
    #[serde(default = "default_count_by_tenant_statement_timeout_ms")]
    pub count_by_tenant_statement_timeout_ms: u64,

    /// `plan_cache_mode` shipped to every pooled connection.
    ///
    /// Defaults to [`PostgresPlanCacheMode::ForceCustomPlan`]. Settable as
    /// `HFS_PG_PLAN_CACHE_MODE` (`force_custom_plan` | `auto` |
    /// `force_generic_plan`). See the type's documentation for why the default
    /// is not PostgreSQL's.
    #[serde(default)]
    pub plan_cache_mode: PostgresPlanCacheMode,

    /// How long to wait for a free pooled connection before returning
    /// `BackendError::Unavailable`.
    #[serde(default = "default_pool_wait_timeout_secs")]
    pub pool_wait_timeout_secs: u64,

    /// FHIR version for this backend instance.
    #[serde(default = "crate::default_fhir_version")]
    pub fhir_version: FhirVersion,

    /// Directory containing FHIR SearchParameter spec files.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    /// When true, search indexing is offloaded to a secondary backend.
    #[serde(default)]
    pub search_offloaded: bool,
}

/// SSL mode for PostgreSQL connections.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PostgresSslMode {
    /// Disable SSL.
    Disable,
    /// Prefer SSL, but allow non-SSL.
    #[default]
    Prefer,
    /// Require SSL.
    Require,
}

/// `plan_cache_mode` for pooled connections.
///
/// PostgreSQL promotes a persistent prepared statement to a **generic plan**
/// after five executions, and a generic plan is one built without the parameter
/// values. For most of this backend that is free money — a primary-key lookup
/// plans the same way either way — which is why the write path's statements were
/// cached with the default `auto` and left there.
///
/// The search path is not most of this backend. Two of its predicate shapes are
/// only sargable when the planner can see the value, and both are hot:
///
/// - `value_reference = $3 OR value_reference LIKE $4 ESCAPE '\'` — the prefix
///   bounds `idx_search_reference_pattern` needs exist only while the pattern is
///   a `Const`. Measured against a 21.8M-row `search_index`:
///   **1,499 buffers under a custom plan, 6,005,788 under a generic one.**
/// - `value_token_code = $3 OR value_token_code = $4` (an OR-list such as
///   `?category=laboratory,vital-signs`) — neither plan can make the `OR` an
///   index condition, but only the custom plan estimates it well enough to pick
///   the `last_updated`-ordered index that lets the `LIMIT` stop early.
///   **464 buffers versus 2,453,618.**
///
/// End to end on those two shapes: 6,349 qps today, 7,626 qps cached with
/// `ForceCustomPlan`, **56.8 qps** cached with `Auto`.
///
/// So the default is `ForceCustomPlan`: it keeps every plan byte-identical to
/// the one this backend produces today (a throwaway statement is planned with
/// its values in hand, which *is* a custom plan) while the statement cache still
/// removes the parse, the parse analysis, the Describe and a round trip. It also
/// retires a hazard the write path already carries — a cached point read that
/// has gone generic can pick a prefix-redundant index and filter, which a
/// sibling seat measured at 1,575 buffers for a three-buffer primary-key lookup.
///
/// `Auto` is PostgreSQL's own default and is the larger win — it is what makes
/// the safe shapes 1.7x rather than 1.2x — but it is not safe to switch on until
/// the reference predicate is rewritten as an explicit range, the way schema v34
/// rewrote the string predicate. The knob exists so that can be A/B'd on the
/// benchmark host without a code change, not because operators should tune it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostgresPlanCacheMode {
    /// Plan every execution with the parameter values in hand. The default.
    #[default]
    ForceCustomPlan,
    /// PostgreSQL's own policy: custom for five executions, then generic if the
    /// generic plan does not cost more.
    Auto,
    /// Always use a generic plan. Present for symmetry and A/B measurement;
    /// see the type documentation for what it does to reference search.
    ForceGenericPlan,
}

impl PostgresPlanCacheMode {
    /// The GUC value. A fixed set of literals — nothing derived from
    /// configuration text is ever interpolated into the startup packet.
    fn as_guc(self) -> &'static str {
        match self {
            Self::ForceCustomPlan => "force_custom_plan",
            Self::Auto => "auto",
            Self::ForceGenericPlan => "force_generic_plan",
        }
    }

    /// Parses `HFS_PG_PLAN_CACHE_MODE`. An unrecognised value is ignored rather
    /// than substituted, so a typo cannot silently ship a different plan policy
    /// or a malformed startup packet.
    fn from_env_value(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "force_custom_plan" => Some(Self::ForceCustomPlan),
            "auto" => Some(Self::Auto),
            "force_generic_plan" => Some(Self::ForceGenericPlan),
            _ => None,
        }
    }
}

fn default_host() -> String {
    "localhost".to_string()
}

fn default_port() -> u16 {
    5432
}

fn default_dbname() -> String {
    "helios".to_string()
}

fn default_user() -> String {
    "helios".to_string()
}

/// Default pool size.
///
/// FHIR search is dominated by database round-trips rather than local CPU, so
/// connections spend most of their life waiting on the wire and a pool somewhat
/// larger than the core count is right. The ceiling matters more than the floor:
/// past roughly `db_cores * 2` extra connections do not add throughput, they just
/// move the queue out of the client (cheap: a semaphore) and into Postgres
/// (expensive: a backend process and its `work_mem`).
///
/// The old fixed default of 10 throttled search badly — at 30 concurrent clients
/// with multi-second queries, two-thirds of every request's latency was spent
/// waiting for a connection (issue #224).
fn default_max_connections() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores * 4).clamp(16, 64)
}

fn default_connect_timeout_secs() -> u64 {
    5
}

/// Reads `HFS_PG_CONNECT_TIMEOUT_SECS`, falling back to the default.
///
/// This value bounds deadpool's `create_timeout` and is therefore a hard ceiling
/// on connection establishment. It must be reachable from every construction path
/// — an operator whose server needs longer than the default has no other way to
/// say so, and before this was wired up the field was dead config, so no
/// deployment has ever had to think about it.
fn connect_timeout_secs_from_env() -> u64 {
    std::env::var("HFS_PG_CONNECT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(default_connect_timeout_secs)
}

fn default_statement_timeout_ms() -> u64 {
    30000
}

/// Environment variable that sets
/// [`PostgresConfig::count_by_tenant_statement_timeout_ms`] (#1911).
pub(crate) const COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV: &str =
    "HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS";

/// Default [`PostgresConfig::count_by_tenant_statement_timeout_ms`]: 25 s, by
/// policy (under the default 30 s HTTP request timeout, so REST callers get
/// `504` rather than `408`).
const DEFAULT_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS: u64 = 25_000;

fn default_count_by_tenant_statement_timeout_ms() -> u64 {
    DEFAULT_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS
}

/// Checks a `count_by_tenant` budget. Zero would mean "no limit" to
/// PostgreSQL, and values above `i32::MAX` are refused by it, so both are
/// errors naming the variable.
fn check_count_by_tenant_statement_timeout_ms(ms: u64) -> Result<(), String> {
    if ms == 0 || ms > i32::MAX as u64 {
        return Err(format!(
            "{COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV} must be between 1 and {}; got {ms}",
            i32::MAX
        ));
    }
    Ok(())
}

/// How long a caller waits for a pooled connection before giving up.
///
/// deadpool's default is to wait forever, which turns database saturation into an
/// unbounded latency tail instead of an honest rejection.
fn default_pool_wait_timeout_secs() -> u64 {
    10
}

impl Debug for PostgresConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Destructured without `..` so a new field fails to compile until it
        // is classified as secret or not.
        let Self {
            host,
            port,
            dbname,
            user,
            password,
            ssl_mode,
            max_connections,
            connect_timeout_secs,
            statement_timeout_ms,
            count_by_tenant_statement_timeout_ms,
            plan_cache_mode,
            pool_wait_timeout_secs,
            fhir_version,
            data_dir,
            search_offloaded,
        } = self;
        f.debug_struct("PostgresConfig")
            .field("host", host)
            .field("port", port)
            .field("dbname", dbname)
            .field("user", user)
            .field("password", &password.as_ref().map(|_| "[redacted]"))
            .field("ssl_mode", ssl_mode)
            .field("max_connections", max_connections)
            .field("connect_timeout_secs", connect_timeout_secs)
            .field("statement_timeout_ms", statement_timeout_ms)
            .field(
                "count_by_tenant_statement_timeout_ms",
                count_by_tenant_statement_timeout_ms,
            )
            .field("plan_cache_mode", plan_cache_mode)
            .field("pool_wait_timeout_secs", pool_wait_timeout_secs)
            .field("fhir_version", fhir_version)
            .field("data_dir", data_dir)
            .field("search_offloaded", search_offloaded)
            .finish()
    }
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            dbname: default_dbname(),
            user: default_user(),
            password: None,
            ssl_mode: PostgresSslMode::default(),
            max_connections: default_max_connections(),
            connect_timeout_secs: default_connect_timeout_secs(),
            statement_timeout_ms: default_statement_timeout_ms(),
            count_by_tenant_statement_timeout_ms: default_count_by_tenant_statement_timeout_ms(),
            plan_cache_mode: PostgresPlanCacheMode::default(),
            pool_wait_timeout_secs: default_pool_wait_timeout_secs(),
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
        }
    }
}

impl PostgresConfig {
    /// Applies `HFS_PG_*` tuning overrides.
    ///
    /// Called from every construction path, not just `from_env`. The connection-URL
    /// path (`HFS_DATABASE_URL`, which is how Docker and Kubernetes deployments and
    /// the benchmark all configure the server) previously started from `Default` and
    /// only overwrote host/port/user/password/dbname — so the pool size was pinned
    /// at its default and *no* environment variable could change it (issue #224).
    fn apply_env_overrides(&mut self) {
        if let Some(v) = std::env::var("HFS_PG_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.max_connections = v;
        }
        if let Some(v) = std::env::var("HFS_PG_STATEMENT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.statement_timeout_ms = v;
        }
        // Like its siblings, a value that does not parse is ignored; one that
        // parses but is out of range fails `PostgresBackend::new`.
        if let Some(v) = std::env::var(COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.count_by_tenant_statement_timeout_ms = v;
        }
        if let Some(v) = std::env::var("HFS_PG_POOL_WAIT_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.pool_wait_timeout_secs = v;
        }
        if let Some(v) = std::env::var("HFS_PG_PLAN_CACHE_MODE")
            .ok()
            .and_then(|v| PostgresPlanCacheMode::from_env_value(&v))
        {
            self.plan_cache_mode = v;
        }
    }

    /// The `statement_timeout` the cross-tenant `count_by_tenant` aggregate
    /// runs under (#1911): [`Self::count_by_tenant_statement_timeout_ms`],
    /// capped by [`Self::statement_timeout_ms`] unless that is `0` (no
    /// limit). The count-specific budget can only tighten the general one.
    pub fn effective_count_by_tenant_statement_timeout_ms(&self) -> u64 {
        match self.statement_timeout_ms {
            0 => self.count_by_tenant_statement_timeout_ms,
            general => self.count_by_tenant_statement_timeout_ms.min(general),
        }
    }
}

/// The `options` startup parameter every pooled connection is opened with.
///
/// Both settings must arrive in the startup packet rather than as a post-connect
/// `SET`: a `SET` binds one borrowed session, which says nothing about the
/// connections the pool adds as it grows or replaces after a recycle. For
/// `plan_cache_mode` there is a second reason — the prepared-statement cache is
/// per-connection and a statement first planned under one policy keeps it, so a
/// policy that arrives after the first `prepare_cached` would apply to some
/// statements and not others.
fn startup_options(config: &PostgresConfig) -> String {
    format!(
        "-c statement_timeout={} -c plan_cache_mode={}",
        config.statement_timeout_ms,
        config.plan_cache_mode.as_guc()
    )
}

impl PostgresBackend {
    /// The capabilities this backend declares.
    ///
    /// Invariant across every valid configuration of the backend, so it is
    /// exposed as a constructor-free associated function: [`PostgresBackend::new`]
    /// eagerly verifies connectivity, and without this the declaration would be
    /// unreachable — and therefore untestable — without a live PostgreSQL.
    ///
    /// Both [`Backend::supports`] and [`Backend::capabilities`] delegate here so
    /// the two answers cannot drift apart. They were previously two separately
    /// hand-maintained lists; that duplication is how issue #369's false claim
    /// came to be written twice.
    ///
    /// # Tenancy
    ///
    /// `SharedSchema` **only**, and deliberately so. Every tenant's records live
    /// in one `resources` table keyed by `(tenant_id, resource_type, id)` with a
    /// `tenant_id` discriminator that every query filters on; no schema or
    /// database switching exists in any code path. This is the strategy chosen in
    /// design discussion #28 — it is a decision, not a gap, so please do not
    /// "restore" `SchemaPerTenant` / `DatabasePerTenant` here as a bug fix.
    /// Advertising either of them is what issue #369 corrected.
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
            BackendCapability::FullTextSearch,
            BackendCapability::Sorting,
            BackendCapability::OffsetPagination,
            BackendCapability::CursorPagination,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::PessimisticLocking,
            BackendCapability::BulkExport,
            BackendCapability::BulkSubmitIngest,
            BackendCapability::BulkSubmitRestWorker,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::InDbSofRunner,
            BackendCapability::ConditionalCreate,
            BackendCapability::ConditionalUpdate,
            BackendCapability::ConditionalDelete,
            BackendCapability::ConditionalPatch,
            BackendCapability::SharedSchema,
        ]
    }

    /// Creates a new PostgreSQL backend with the given configuration.
    pub async fn new(config: PostgresConfig) -> StorageResult<Self> {
        check_count_by_tenant_statement_timeout_ms(config.count_by_tenant_statement_timeout_ms)
            .map_err(|message| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message,
                    source: None,
                })
            })?;
        let pool = Self::create_pool(&config)?;

        // Verify connectivity.
        //
        // `statement_timeout` is deliberately NOT set here. It is a session GUC, so
        // a `SET` on one pooled connection binds only that connection — the others,
        // created lazily as the pool grows, inherited the server default (usually
        // 0 = no limit). Under load that meant a runaway query was capped or not
        // depending on which connection it happened to draw. It is now sent in the
        // startup packet by `create_pool`, so every connection carries it.
        let client = pool.get().await.map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })?;
        drop(client);

        // Initialize the per-tenant search parameter registries. Base params
        // (embedded + spec + custom) go into the shared base; each tenant's
        // stored overlay is read from `stored_by_tenant`, populated by
        // `reload_stored_cache` at startup / refresh / SearchParameter writes.
        let stored_by_tenant: StoredByTenant = Arc::new(RwLock::new(HashMap::new()));
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
                        .map(|overlay| overlay.definitions.clone())
                        .unwrap_or_default(),
                )
            },
        )));
        Self::initialize_search_registry(registries.base(), &config);

        Ok(Self {
            pool,
            config,
            registries,
            stored_by_tenant,
            overlay_fingerprint_ttl: overlay_fingerprint_ttl_from_env(),
            index_layout: Arc::new(std::sync::OnceLock::new()),
            patient_export_index_ready: Arc::new(AtomicBool::new(false)),
            fts_table_exists: Arc::new(std::sync::OnceLock::new()),
            reindex_admission: Arc::new(Semaphore::new(1)),
            reindex_width: 1,
            cleanup_tracker: super::cleanup::CleanupTracker::new(),
            #[cfg(test)]
            reindex_test_hook: None,
            #[cfg(test)]
            reindex_test_pause: None,
            #[cfg(test)]
            reindex_test_entered: None,
        })
    }

    /// Sets how long a write trusts a validated SearchParameter overlay
    /// without probing the database (see `HFS_PG_SP_FINGERPRINT_TTL_MS`).
    /// `Duration::ZERO` probes on every write.
    #[doc(hidden)]
    pub fn with_overlay_fingerprint_ttl(mut self, ttl: Duration) -> Self {
        self.overlay_fingerprint_ttl = ttl;
        self
    }

    /// Selects the maximum number of concurrent reindex writers on this instance.
    /// Clones share one semaphore, including manual jobs, automatic jobs, and
    /// per-resource fallback. The effective width is capped by the connection
    /// pool. Ordinary library construction remains serial. HFS calls this only
    /// for standalone PostgreSQL, using the effective bulk-submit file
    /// concurrency; deployments configured above one opt in on upgrade.
    pub fn with_reindex_concurrency(mut self, requested: usize) -> Self {
        let width = requested
            .max(1)
            .min(self.config.max_connections.max(1))
            .min(u32::MAX as usize);
        self.reindex_admission = Arc::new(Semaphore::new(width));
        self.reindex_width = width;
        self
    }

    /// Waits until exceptional rollback tasks have settled or discarded their
    /// managed PostgreSQL sessions. A discarded server session may still be
    /// finishing a request; this is not a server-idle guarantee.
    pub async fn wait_postgres_cleanup(&self) {
        self.cleanup_tracker.wait().await;
    }

    /// Creates a backend from a connection string.
    ///
    /// The URL carries the connection's identity (host, port, database,
    /// credentials); `HFS_PG_CONNECT_TIMEOUT_SECS` still applies on top of it, so
    /// a URL-configured deployment can tune connection establishment exactly as an
    /// `HFS_PG_*`-configured one can.
    pub async fn from_connection_string(url: &str) -> StorageResult<Self> {
        Self::new(Self::config_from_connection_string(url)?).await
    }

    /// Builds the configuration `from_connection_string` would connect with,
    /// without connecting. Callers that need to override config fields the URL
    /// cannot express (e.g. the server's default FHIR version) adjust the
    /// returned config and pass it to [`PostgresBackend::new`].
    pub fn config_from_connection_string(url: &str) -> StorageResult<PostgresConfig> {
        let mut config = Self::parse_connection_string(url)?;
        config.connect_timeout_secs = connect_timeout_secs_from_env();
        Ok(config)
    }

    /// Creates a backend from environment variables.
    ///
    /// Reads the following environment variables:
    /// - `HFS_PG_HOST` (default: "localhost")
    /// - `HFS_PG_PORT` (default: 5432)
    /// - `HFS_PG_DBNAME` (default: "helios")
    /// - `HFS_PG_USER` (default: "helios")
    /// - `HFS_PG_PASSWORD`
    /// - `HFS_PG_MAX_CONNECTIONS` (default: `cores * 4`, clamped to 16..=64)
    /// - `HFS_PG_CONNECT_TIMEOUT_SECS` (default: 5)
    /// - `HFS_PG_STATEMENT_TIMEOUT_MS` (default: 30000)
    /// - `HFS_PG_POOL_WAIT_TIMEOUT_SECS` (default: 10)
    /// - `HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS` (default: 25000)
    pub async fn from_env() -> StorageResult<Self> {
        Self::new(Self::config_from_env()).await
    }

    /// Builds the configuration `from_env` would connect with, without
    /// connecting. Same override contract as
    /// [`PostgresBackend::config_from_connection_string`].
    pub fn config_from_env() -> PostgresConfig {
        let mut config = PostgresConfig {
            host: std::env::var("HFS_PG_HOST").unwrap_or_else(|_| default_host()),
            port: std::env::var("HFS_PG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(default_port),
            dbname: std::env::var("HFS_PG_DBNAME").unwrap_or_else(|_| default_dbname()),
            user: std::env::var("HFS_PG_USER").unwrap_or_else(|_| default_user()),
            password: std::env::var("HFS_PG_PASSWORD").ok(),
            // The other tuning knobs are applied by `apply_env_overrides` below;
            // `connect_timeout_secs` reads its env var through a dedicated helper so
            // the connection-URL path can pick it up the same way.
            connect_timeout_secs: connect_timeout_secs_from_env(),
            ..Default::default()
        };
        // Pool/timeout knobs are applied through the shared helper so this path and
        // the connection-URL path cannot drift apart again.
        config.apply_env_overrides();
        config
    }

    fn create_pool(config: &PostgresConfig) -> StorageResult<Pool> {
        let mut cfg = Config::new();
        cfg.host = Some(config.host.clone());
        cfg.port = Some(config.port);
        cfg.dbname = Some(config.dbname.clone());
        cfg.user = Some(config.user.clone());
        cfg.password = config.password.clone();
        cfg.ssl_mode = Some(match config.ssl_mode {
            PostgresSslMode::Disable => SslMode::Disable,
            PostgresSslMode::Prefer => SslMode::Prefer,
            PostgresSslMode::Require => SslMode::Require,
        });

        let connect_timeout = Duration::from_secs(config.connect_timeout_secs);

        // Bound the TCP handshake. NOTE: tokio-postgres applies `connect_timeout`
        // to `TcpStream::connect` and nothing else — DNS resolution, the TLS
        // handshake and the Postgres startup/auth exchange are all awaited
        // unbounded. So this alone does NOT bound connection establishment; the
        // deadpool `create` timeout below covers the rest.
        cfg.connect_timeout = Some(connect_timeout);

        // Ship `statement_timeout` in the startup packet so it applies to every
        // connection the pool ever creates — including ones added as the pool grows
        // and ones replaced after a recycle. A post-connect `SET` on a single
        // borrowed client cannot make that guarantee (it binds only that session),
        // and a per-connection hook would cost an extra round-trip.
        //
        // `plan_cache_mode` rides along for the same reason, and because it must
        // hold for the connection's whole life: the statement cache is
        // per-connection and a statement prepared before a `SET` would keep
        // whichever policy was in force when it was first planned. See
        // [`PostgresPlanCacheMode`] for what the default protects.
        cfg.options = Some(startup_options(config));
        // Makes HFS connections identifiable in pg_stat_activity.
        cfg.application_name = Some("hfs".to_string());

        let pool = cfg
            .builder(NoTls)
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to create pool builder: {}", e),
                    source: None,
                })
            })?
            .max_size(config.max_connections)
            // deadpool waits forever by default, so a saturated pool — or a database
            // that accepts TCP but stalls the startup handshake — becomes an
            // unbounded latency tail. Bound the wait and surface exhaustion as a
            // fast `Unavailable` (503) instead of a request that hangs for a minute.
            //
            // `create` also bounds the whole of `Manager::create` (DNS + TCP + TLS +
            // auth). Without it, a peer that completes the TCP handshake and then
            // never answers — a hung server, or a load balancer accepting into a
            // blackhole — hangs the caller forever, since the kernel's SYN-retry only
            // covers failures that never complete the handshake.
            .timeouts(deadpool_postgres::Timeouts {
                wait: Some(std::time::Duration::from_secs(
                    config.pool_wait_timeout_secs,
                )),
                create: Some(connect_timeout),
                recycle: Some(connect_timeout),
            })
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: "postgres".to_string(),
                    message: e.to_string(),
                })
            })?;

        Ok(pool)
    }

    fn parse_connection_string(url: &str) -> StorageResult<PostgresConfig> {
        // Parse postgres:// URL format
        // postgres://user:password@host:port/dbname[?params]
        let url = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .unwrap_or(url);

        // Strip any query string before splitting on '/', otherwise it lands in
        // `dbname` verbatim (`.../postgres?sslmode=require` → dbname
        // "postgres?sslmode=require", which no server will resolve).
        let url = url.split('?').next().unwrap_or(url);

        let mut config = PostgresConfig::default();

        // Split user:password@host:port/dbname
        if let Some((userinfo, rest)) = url.split_once('@') {
            if let Some((user, password)) = userinfo.split_once(':') {
                config.user = user.to_string();
                config.password = Some(password.to_string());
            } else {
                config.user = userinfo.to_string();
            }

            if let Some((hostport, dbname)) = rest.split_once('/') {
                if let Some((host, port)) = hostport.split_once(':') {
                    config.host = host.to_string();
                    config.port = port.parse().unwrap_or(5432);
                } else {
                    config.host = hostport.to_string();
                }
                config.dbname = dbname.to_string();
            } else if let Some((host, port)) = rest.split_once(':') {
                config.host = host.to_string();
                config.port = port.parse().unwrap_or(5432);
            } else {
                config.host = rest.to_string();
            }
        }

        config.apply_env_overrides();
        Ok(config)
    }

    fn initialize_search_registry(
        registry: &Arc<RwLock<SearchParameterRegistry>>,
        config: &PostgresConfig,
    ) {
        let loader = SearchParameterLoader::new(config.fhir_version);
        let mut reg = registry.write();

        let mut fallback_count = 0;
        let mut spec_count = 0;
        let mut spec_file: Option<PathBuf> = None;
        let mut custom_count = 0;
        let mut custom_files: Vec<String> = Vec::new();

        // 1. Load minimal embedded fallback params
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

        // 2. Load spec file params
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

        // 3. Load custom SearchParameters
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
            "PostgreSQL SearchParameter registry initialized: {} total ({} spec{}, {} fallback, {} custom{}) covering {} resource types",
            reg.len(),
            spec_count,
            spec_info,
            fallback_count,
            custom_count,
            custom_info,
            resource_type_count
        );
    }

    /// Initialize the database schema.
    ///
    /// Builds the patient export candidate index even when search is
    /// offloaded: `$export` still reads compartments from PostgreSQL (#1663).
    pub async fn init_schema(&self) -> StorageResult<()> {
        self.init_schema_with_patient_export_index(true).await
    }

    /// Initialize the schema for an audit-only backend without the large
    /// patient export acceleration index.
    pub async fn init_schema_without_patient_export_index(&self) -> StorageResult<()> {
        self.init_schema_with_patient_export_index(false).await
    }

    async fn init_schema_with_patient_export_index(&self, build_index: bool) -> StorageResult<()> {
        self.patient_export_index_ready
            .store(false, Ordering::Release);
        let mut client = self.get_client().await?;
        let index_ready =
            super::schema::initialize_schema_with_patient_export_index(&mut client, build_index)
                .await?;
        self.patient_export_index_ready
            .store(index_ready, Ordering::Release);
        let _ = self
            .index_layout
            .set(super::schema::read_index_layout(&client).await);
        // Populate the per-tenant stored-param cache so the registries can build
        // each tenant's overlay lazily.
        self.reload_stored_cache().await?;
        Ok(())
    }

    /// Whether patient-scoped export may use the indexed candidate predicate.
    pub(crate) fn has_patient_export_index(&self) -> bool {
        self.patient_export_index_ready.load(Ordering::Acquire)
    }

    /// The `search_index` layout this database is in.
    pub(crate) fn index_layout(&self) -> super::schema::IndexLayout {
        self.index_layout
            .get()
            .copied()
            .unwrap_or(super::schema::IndexLayout::Legacy)
    }

    /// The statement [`Self::reload_stored_cache`] runs to read every tenant's
    /// stored SearchParameters, at startup and on every refresh.
    ///
    /// `#[doc(hidden)] pub` only so the integration tests can `EXPLAIN` the
    /// exact statement and pin that it seeks by `tenant_id` (#1664).
    #[doc(hidden)]
    pub const STORED_SEARCH_PARAMETERS_SQL: &'static str = "WITH RECURSIVE tenant_ids (tenant_id) AS ( \
             (SELECT tenant_id FROM resources ORDER BY tenant_id LIMIT 1) \
             UNION ALL \
             SELECT (SELECT r.tenant_id FROM resources r \
                     WHERE r.tenant_id > t.tenant_id \
                     ORDER BY r.tenant_id LIMIT 1) \
             FROM tenant_ids t WHERE t.tenant_id IS NOT NULL \
         ) \
         SELECT t.tenant_id, r.data \
         FROM tenant_ids t \
         CROSS JOIN LATERAL ( \
             SELECT r.data FROM resources r \
             WHERE r.tenant_id = t.tenant_id \
               AND r.resource_type = 'SearchParameter' AND r.is_deleted = FALSE \
             OFFSET 0 \
         ) r \
         WHERE t.tenant_id IS NOT NULL";

    /// The cross-tenant live-resource count behind
    /// [`count_by_tenant`](crate::core::ResourceStorage::count_by_tenant).
    /// Its `is_deleted = FALSE` filter implies the predicate of the partial
    /// index `idx_resources_live_tenant (tenant_id)` (schema v49, #1911), so
    /// it can be answered by an index-only scan of that narrow index.
    ///
    /// `#[doc(hidden)] pub` only so tests can `EXPLAIN` the exact statement
    /// and find it in `pg_stat_activity`.
    #[doc(hidden)]
    pub const COUNT_BY_TENANT_SQL: &'static str = "SELECT tenant_id, COUNT(*)::bigint FROM resources \
         WHERE is_deleted = FALSE GROUP BY tenant_id";

    /// Reloads every tenant's stored active SearchParameters into the sync
    /// `stored_by_tenant` cache (grouped by tenant), then drops the cached
    /// per-tenant registries so they rebuild against the fresh overlay.
    pub(crate) async fn reload_stored_cache(&self) -> StorageResult<usize> {
        use crate::search::registry::{SearchParameterSource, SearchParameterStatus};

        fn internal(what: &str, e: tokio_postgres::Error) -> crate::error::StorageError {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("{what}: {e}"),
                source: None,
            })
        }

        // One index probe per tenant instead of a walk over the whole table
        // (#939 C9). The previous `WHERE resource_type = 'SearchParameter'` had
        // no `tenant_id`, which every index on `resources` leads with, so on the
        // 18.9 M-row #939 corpus the hourly refresh scanned the table and died
        // at the 30 s `statement_timeout` every hour of the rebuild ("registry
        // refresh failed; serving the stale cache"). The recursive CTE steps
        // through the distinct tenants along the primary key (a loose index
        // scan), and the lateral join reads each tenant's SearchParameters
        // through the same key. `OFFSET 0` keeps the planner from flattening
        // that join into a hash join over a tenant-less `resource_type` index
        // walk, which it picks whenever its statistics make that walk look
        // cheap (#1664). The timeout is lifted for this one statement, as
        // `count_resources` does for the reindex count (C8): a slow refresh
        // serves stale definitions a little longer, a failed one for ever.
        let mut client = self.get_client().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| internal("Failed to begin SearchParameter refresh", e))?;
        tx.batch_execute("SET LOCAL statement_timeout = 0")
            .await
            .map_err(|e| {
                internal(
                    "Failed to lift statement timeout for SearchParameter refresh",
                    e,
                )
            })?;
        let rows = tx
            .query(Self::STORED_SEARCH_PARAMETERS_SQL, &[])
            .await
            .map_err(|e| internal("Failed to query SearchParameters", e))?;
        tx.commit()
            .await
            .map_err(|e| internal("Failed to finish SearchParameter refresh", e))?;

        let loader = SearchParameterLoader::new(self.config.fhir_version);
        let mut by_tenant: HashMap<String, StoredOverlay> = HashMap::new();
        let mut count = 0;
        for row in rows {
            let tenant_id: String = row.get(0);
            let data: serde_json::Value = row.get(1);
            if let Ok(mut def) = loader.parse_resource(&data)
                && def.status == SearchParameterStatus::Active
            {
                def.source = SearchParameterSource::Stored;
                by_tenant
                    .entry(tenant_id)
                    .or_insert_with(|| StoredOverlay {
                        fingerprint: None,
                        definitions: Vec::new(),
                        registry: None,
                        validated_at: None,
                    })
                    .definitions
                    .push(def);
                count += 1;
            }
        }
        *self.stored_by_tenant.write() = by_tenant;
        self.registries.invalidate_all();
        Ok(count)
    }

    /// The statement a write runs to validate its cached overlay (#1897):
    /// the tenant's live `SearchParameter` count and newest `last_updated`.
    /// Both come from `idx_resources_search (tenant_id, resource_type,
    /// last_updated DESC, id) WHERE is_deleted = FALSE`, so this is an
    /// index-only scan of the tenant's ~1,400 seeded entries, not a read and
    /// decode of their ~1.5 MB of JSONB.
    ///
    /// `#[doc(hidden)] pub` only so the integration tests can `EXPLAIN` the
    /// exact statement and pin that plan.
    #[doc(hidden)]
    pub const OVERLAY_FINGERPRINT_SQL: &'static str = "SELECT COUNT(*)::bigint, MAX(last_updated) \
         FROM resources \
         WHERE tenant_id = $1 AND resource_type = 'SearchParameter' \
           AND is_deleted = FALSE";

    async fn overlay_fingerprint<C>(
        &self,
        client: &C,
        tenant_id: &str,
    ) -> StorageResult<OverlayFingerprint>
    where
        C: GenericClient + Sync + ?Sized,
    {
        let row = client
            .query_one(Self::OVERLAY_FINGERPRINT_SQL, &[&tenant_id])
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to fingerprint tenant SearchParameters: {e}"),
                    source: None,
                })
            })?;
        Ok(OverlayFingerprint {
            live: row.get(0),
            newest: row.get(1),
        })
    }

    /// Reads the tenant's stored overlay on the caller's connection: every
    /// live `SearchParameter` row that is active and whose canonical URL is
    /// not already in the shared base. Seeded core definitions cannot
    /// override their canonical URL in the base, so they are skipped before
    /// parsing.
    async fn read_stored_overlay<C>(
        &self,
        client: &C,
        tenant_id: &str,
    ) -> StorageResult<Vec<SearchParameterDefinition>>
    where
        C: GenericClient + Sync + ?Sized,
    {
        let rows = client
            .query(
                "SELECT data FROM resources
                 WHERE tenant_id = $1 AND resource_type = 'SearchParameter'
                   AND is_deleted = FALSE
                 ORDER BY id",
                &[&tenant_id],
            )
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to query tenant SearchParameters: {e}"),
                    source: None,
                })
            })?;

        let overlays: Vec<serde_json::Value> = {
            let registry = self.registries.base().read();
            rows.into_iter()
                .filter_map(|row| {
                    let data: serde_json::Value = row.get(0);
                    let duplicate = data
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|url| registry.get_by_url(url).is_some());
                    (!duplicate).then_some(data)
                })
                .collect()
        };
        let loader = SearchParameterLoader::new(self.config.fhir_version);
        Ok(overlays
            .iter()
            .filter_map(|data| loader.parse_resource(data).ok())
            .filter(|definition| definition.status == SearchParameterStatus::Active)
            .map(|mut definition| {
                definition.source = SearchParameterSource::Stored;
                definition
            })
            .collect())
    }

    /// The registry for a stored overlay: the shared base itself when the
    /// overlay is empty — the common case, loaded at construction and shared,
    /// so a write need not clone its 1,389 definitions before extracting one
    /// resource — otherwise a clone of the base with the overlay registered.
    fn overlay_registry(
        &self,
        definitions: &[SearchParameterDefinition],
    ) -> Arc<RwLock<SearchParameterRegistry>> {
        let base = self.registries.base();
        if definitions.is_empty() {
            return base.clone();
        }
        let mut registry = base.read().clone();
        for definition in definitions {
            // Match TenantSearchRegistries::build_and_cache: a stored
            // duplicate canonical URL cannot override the base entry.
            let _ = registry.register(definition.clone());
        }
        Arc::new(RwLock::new(registry))
    }

    /// Builds a fresh tenant registry using the caller's guarded transaction.
    ///
    /// The process-local overlay cache can lag writes from another HFS process.
    /// Reindex groups therefore read the persisted overlay after taking the
    /// tenant gate, and never through the cache. The caller keeps this
    /// extractor for one immutable extraction phase; this method never writes
    /// the cache or checks out another connection.
    pub(crate) async fn authoritative_extractor<C>(
        &self,
        client: &C,
        tenant_id: &str,
    ) -> StorageResult<SearchParameterExtractor>
    where
        C: GenericClient + Sync + ?Sized,
    {
        let definitions = self.read_stored_overlay(client, tenant_id).await?;
        Ok(SearchParameterExtractor::new(
            self.overlay_registry(&definitions),
        ))
    }

    /// The extractor a write hands to its index step.
    ///
    /// When `extracts` is false the write indexes nothing in PostgreSQL —
    /// search is offloaded (#939) or deferred to a later rebuild (#903) — so
    /// reading the persisted overlay would be pure cost. The shared base
    /// stands in; nothing extracts with it.
    ///
    /// Otherwise the write validates the cached overlay instead of re-reading
    /// it (#1897). Before this, every indexed write ran the authoritative
    /// read: one query and one JSONB decode per stored SearchParameter (1,372
    /// seeded, ~1.5 MB) per call, 93% of the database's time in the benchmark
    /// crud suite at 24.8 ms a call. Now it fingerprints the tenant's live
    /// `SearchParameter` rows with one index-only probe
    /// ([`Self::OVERLAY_FINGERPRINT_SQL`]) on the same guarded connection, so
    /// a SearchParameter committed by another HFS process is still seen by the
    /// next write here, and only a changed fingerprint pays the full read,
    /// which then re-fingerprints the cache for the tenant.
    ///
    /// `pending_search_parameter` says the caller's transaction has itself
    /// written a `SearchParameter` it has not committed. The fingerprint and
    /// the read both see that row, so the extractor is right either way, but
    /// the cache must not: the transaction may still roll back, and the
    /// search path reads the same cache. The caller reloads the cache after
    /// its commit, as every SearchParameter write path does.
    pub(crate) async fn write_extractor<C>(
        &self,
        client: &C,
        tenant_id: &str,
        extracts: bool,
        pending_search_parameter: bool,
    ) -> StorageResult<SearchParameterExtractor>
    where
        C: GenericClient + Sync + ?Sized,
    {
        self.write_extractor_with(
            client,
            tenant_id,
            extracts,
            pending_search_parameter,
            OverlayFreshness::WithinTtl,
        )
        .await
    }

    /// [`Self::write_extractor`] with the freshness the caller needs. Callers
    /// that must see every SearchParameter committed before their snapshot
    /// pass [`OverlayFreshness::Authoritative`], which always runs the
    /// fingerprint probe (and so fails if the database cannot answer it);
    /// the full overlay read still happens only when the fingerprint moved.
    pub(crate) async fn write_extractor_with<C>(
        &self,
        client: &C,
        tenant_id: &str,
        extracts: bool,
        pending_search_parameter: bool,
        freshness: OverlayFreshness,
    ) -> StorageResult<SearchParameterExtractor>
    where
        C: GenericClient + Sync + ?Sized,
    {
        if !extracts {
            return Ok(SearchParameterExtractor::new(
                self.registries.base().clone(),
            ));
        }
        // Fast path: this overlay was validated moments ago and this
        // transaction has no uncommitted SearchParameter of its own, so skip
        // the probe. Local SearchParameter writes drop the entry on commit;
        // other processes' are seen within the TTL.
        if freshness.may_skip_probe(self.overlay_fingerprint_ttl, pending_search_parameter)
            && let Some(registry) = fresh_overlay_registry(
                &self.stored_by_tenant,
                tenant_id,
                self.overlay_fingerprint_ttl,
            )
        {
            return Ok(SearchParameterExtractor::new(registry));
        }
        let fingerprint = self.overlay_fingerprint(client, tenant_id).await?;
        let cached = self
            .stored_by_tenant
            .read()
            .get(tenant_id)
            .and_then(|overlay| {
                (overlay.fingerprint == Some(fingerprint))
                    .then(|| overlay.registry.clone())
                    .flatten()
            });
        if let Some(registry) = cached {
            if !pending_search_parameter
                && let Some(overlay) = self.stored_by_tenant.write().get_mut(tenant_id)
                && overlay.fingerprint == Some(fingerprint)
            {
                overlay.validated_at = Some(std::time::Instant::now());
            }
            return Ok(SearchParameterExtractor::new(registry));
        }
        let definitions = self.read_stored_overlay(client, tenant_id).await?;
        let registry = self.overlay_registry(&definitions);
        if !pending_search_parameter {
            self.stored_by_tenant.write().insert(
                tenant_id.to_string(),
                StoredOverlay {
                    fingerprint: Some(fingerprint),
                    definitions,
                    registry: Some(registry.clone()),
                    validated_at: Some(std::time::Instant::now()),
                },
            );
            // The search path's registry for this tenant was built from the
            // previous overlay; drop it so it rebuilds from the fresh one.
            self.registries.invalidate(tenant_id);
        }
        Ok(SearchParameterExtractor::new(registry))
    }

    /// TTL-cache refresh (#235): reload the stored-param cache from storage and
    /// drop the cached per-tenant registries. Returns the stored-param count.
    /// PostgreSQL reindex reads authoritative definitions within its guarded
    /// transaction, even if this cache has not yet refreshed.
    pub async fn refresh_stored_search_parameters(&self) -> StorageResult<usize> {
        self.reload_stored_cache().await
    }

    /// Get a client from the pool.
    ///
    /// `#[doc(hidden)] pub` rather than `pub(crate)` only so the out-of-crate
    /// pool-timeout regression test (`tests/postgres_tests.rs`) can hold several
    /// pooled connections at once. It hands out a raw connection that bypasses
    /// tenant scoping and the write lock protocol, so it is not stable API —
    /// workspace callers should use the `ResourceStorage`/`SearchProvider`
    /// methods instead. Maintenance SQL must provide its own transaction,
    /// advisory locks, and current-row checks.
    #[doc(hidden)]
    pub async fn get_client(&self) -> StorageResult<deadpool_postgres::Client> {
        use deadpool_postgres::{PoolError, TimeoutType};

        let _checkout = crate::perf::span(crate::perf::Phase::PostgresPoolCheckout);
        let mut attempt = 1;
        let checked_out = loop {
            match self.pool.get().await {
                Err(e) if attempt < CONNECT_ATTEMPTS && is_transient_connect_failure(&e) => {
                    tracing::warn!(attempt, error = %e, "retrying a failed PostgreSQL connect");
                    tokio::time::sleep(connect_retry_delay(attempt)).await;
                    attempt += 1;
                }
                other => break other,
            }
        };
        checked_out.map_err(|e| match e {
            // Every connection is busy and the wait timeout elapsed. The database
            // is healthy — we are simply over capacity — so this is a retryable
            // 503, not an internal error. Reporting it as a 500 (the previous
            // behavior) both misleads clients and hides saturation in error logs.
            PoolError::Timeout(TimeoutType::Wait) => {
                crate::error::StorageError::Backend(BackendError::Unavailable {
                    backend_name: "postgres".to_string(),
                    message: "connection pool exhausted".to_string(),
                })
            }
            other => crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: other.to_string(),
            }),
        })
    }

    pub(super) async fn guarded_client(&self) -> StorageResult<super::cleanup::GuardedClient> {
        Ok(super::cleanup::GuardedClient::new(
            self.get_client().await?,
            self.cleanup_tracker.clone(),
            None,
        ))
    }

    /// The per-tenant registry container (shared with a co-located ES backend).
    /// Direct cache changes are outside guarded PostgreSQL indexing: those
    /// writes read persisted SearchParameters under the tenant write gate.
    pub fn tenant_registries(&self) -> &Arc<TenantSearchRegistries> {
        &self.registries
    }

    /// The shared base registry (embedded + spec + custom), tenant-independent.
    /// Treat it as immutable after construction while guarded writes run.
    pub(crate) fn base_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        self.registries.base()
    }

    /// The cached registry for a tenant (base + that tenant's stored overlay).
    /// Guarded PostgreSQL indexing reads the persisted overlay instead.
    pub(crate) fn tenant_registry(&self, tenant_id: &str) -> Arc<RwLock<SearchParameterRegistry>> {
        self.registries.for_tenant(tenant_id)
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &PostgresConfig {
        &self.config
    }

    /// Returns a clone of the underlying connection pool.
    ///
    /// `deadpool_postgres::Pool` is `Clone` (Arc-backed), so this is cheap.
    pub(crate) fn pool(&self) -> Pool {
        self.pool.clone()
    }

    /// A point-in-time snapshot of the connection pool: the configured ceiling
    /// (`max_size`), the connections currently open (`size`), the open ones
    /// that are idle (`available`) and the callers parked waiting for one
    /// (`waiting`). Read by the server's `/metrics` scrape (#1907).
    ///
    /// `size` grows lazily up to `max_size`. A caller still opening a new
    /// connection below `max_size` counts as `waiting` until it is open; a
    /// [`get_client`](Self::get_client) caller sleeping between connect
    /// retries is not counted.
    pub fn pool_status(&self) -> super::PoolStatus {
        self.pool.status()
    }

    /// Returns whether search indexing is offloaded to a secondary backend.
    pub fn is_search_offloaded(&self) -> bool {
        self.config.search_offloaded
    }

    /// Sets the search offloaded flag.
    pub fn set_search_offloaded(&mut self, offloaded: bool) {
        self.config.search_offloaded = offloaded;
    }
}

/// Connection wrapper for PostgreSQL.
#[allow(dead_code)]
pub struct PostgresConnection(pub(crate) deadpool_postgres::Client);

impl Debug for PostgresConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConnection").finish()
    }
}

#[async_trait]
impl Backend for PostgresBackend {
    type Connection = PostgresConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::Postgres
    }

    fn name(&self) -> &'static str {
        "postgres"
    }

    fn supports(&self, capability: BackendCapability) -> bool {
        Self::declared_capabilities().contains(&capability)
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        Self::declared_capabilities()
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })?;
        Ok(PostgresConnection(client))
    }

    async fn release(&self, _conn: Self::Connection) {
        // Connection is automatically returned to pool when dropped
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| BackendError::Unavailable {
                backend_name: "postgres".to_string(),
                message: "Failed to get connection".to_string(),
            })?;
        // A failing constant `SELECT 1` means the backend is unreachable/broken,
        // not a bad query — surface it as a retryable `Unavailable` (503) rather
        // than an `Internal` (500) so a down backend reads as "try again", and
        // so the readiness probe classifies it correctly.
        client
            .query_one("SELECT 1", &[])
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "postgres".to_string(),
                message: format!("Health check failed: {}", e),
            })?;
        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("Failed to initialize schema: {}", e),
                source: None,
            })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "postgres".to_string(),
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

impl SearchCapabilityProvider for PostgresBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        let params = {
            let registry = self.base_registry().read();
            registry.get_active_params(resource_type)
        };

        if params.is_empty() {
            let common_params = {
                let registry = self.base_registry().read();
                registry.get_active_params("Resource")
            };
            if common_params.is_empty() {
                return None;
            }
        }

        let mut search_params = Vec::new();
        for param in &params {
            let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                .with_definition(&param.url);
            let modifiers = Self::modifiers_for_type(param.param_type);
            cap = cap.with_modifiers(modifiers);
            if let Some(ref targets) = param.target {
                cap = cap.with_targets(targets.iter().map(|s| s.as_str()));
            }
            search_params.push(cap);
        }

        let common_params = {
            let registry = self.base_registry().read();
            registry.get_active_params("Resource")
        };
        for param in &common_params {
            if !search_params.iter().any(|p| p.name == param.code) {
                let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                    .with_definition(&param.url);
                cap = cap.with_modifiers(Self::modifiers_for_type(param.param_type));
                search_params.push(cap);
            }
        }

        Some(
            ResourceSearchCapabilities::new(resource_type)
                .with_special_params(vec![
                    SpecialSearchParam::Id,
                    SpecialSearchParam::LastUpdated,
                    SpecialSearchParam::Tag,
                    SpecialSearchParam::Profile,
                    SpecialSearchParam::Security,
                ])
                .with_include_capabilities(vec![
                    IncludeCapability::Include,
                    IncludeCapability::Revinclude,
                ])
                .with_pagination_capabilities(vec![
                    PaginationCapability::Count,
                    PaginationCapability::Offset,
                    PaginationCapability::Cursor,
                    PaginationCapability::MaxPageSize(1000),
                    PaginationCapability::DefaultPageSize(20),
                ])
                .with_result_mode_capabilities(vec![
                    ResultModeCapability::Total,
                    ResultModeCapability::TotalNone,
                    ResultModeCapability::TotalAccurate,
                    ResultModeCapability::SummaryCount,
                ])
                .with_param_list(search_params),
        )
    }

    fn global_search_capabilities(&self) -> GlobalSearchCapabilities {
        GlobalSearchCapabilities::new()
            .with_special_params(vec![
                SpecialSearchParam::Id,
                SpecialSearchParam::LastUpdated,
                SpecialSearchParam::Tag,
                SpecialSearchParam::Profile,
                SpecialSearchParam::Security,
            ])
            .with_pagination(vec![
                PaginationCapability::Count,
                PaginationCapability::Offset,
                PaginationCapability::Cursor,
                PaginationCapability::MaxPageSize(1000),
                PaginationCapability::DefaultPageSize(20),
            ])
            .with_system_search()
    }
}

impl PostgresBackend {
    /// Returns supported modifiers for a parameter type.
    pub(super) fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text", "missing"],
            // `not-in` is intentionally omitted: it returns 501 (negated
            // value-set filtering is unimplemented), so it must not be
            // advertised as supported.
            SearchParamType::Token => {
                vec!["not", "text", "code-text", "in", "of-type", "missing"]
            }
            SearchParamType::Reference => vec![
                "identifier",
                "contains",
                "text",
                "code-text",
                "below",
                "above",
                "missing",
            ],
            SearchParamType::Date => vec!["missing"],
            SearchParamType::Number => vec!["missing"],
            SearchParamType::Quantity => vec!["missing"],
            SearchParamType::Uri => vec!["contains", "below", "above", "missing"],
            SearchParamType::Composite | SearchParamType::Special => vec![],
        }
    }
}

/// How many times [`PostgresBackend::get_client`] tries to open a pooled
/// connection when the attempt fails before reaching the server (#1793).
const CONNECT_ATTEMPTS: u32 = 3;

/// Backoff before the next connect attempt: 100 ms, then 300 ms.
fn connect_retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(100 * 3u64.pow(attempt.saturating_sub(1)))
}

/// Whether a pool error is a failed attempt to *open* a connection that a
/// retry can fix: a network-level failure such as a refused or reset socket,
/// with no database error behind it. A rejected login or any other answer from
/// the server is not retried, and neither is a timeout, which already waited
/// its full budget. Nothing has run on the connection, so a retry cannot
/// repeat work.
fn is_transient_connect_failure(e: &deadpool_postgres::PoolError) -> bool {
    match e {
        deadpool_postgres::PoolError::Backend(e) => e.as_db_error().is_none() && !e.is_closed(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_redacts_password() {
        let config = PostgresConfig {
            host: "db.internal".to_string(),
            password: Some("pg-s3cret-pw".to_string()),
            ..Default::default()
        };
        let out = format!("{config:?}");
        assert!(!out.contains("pg-s3cret-pw"), "password leaked: {out}");
        assert!(out.contains("[redacted]"), "{out}");
        assert!(out.contains("db.internal"), "{out}");
    }

    fn overlay(validated_at: Option<std::time::Instant>) -> StoredOverlay {
        StoredOverlay {
            fingerprint: Some(OverlayFingerprint {
                live: 1,
                newest: None,
            }),
            definitions: Vec::new(),
            registry: Some(Arc::new(RwLock::new(SearchParameterRegistry::new()))),
            validated_at,
        }
    }

    /// Only `WithinTtl` may skip the probe, and never with a pending
    /// SearchParameter or a zero TTL; `Authoritative` always probes.
    #[test]
    fn authoritative_freshness_never_skips_the_probe() {
        let ttl = Duration::from_secs(60);
        assert!(OverlayFreshness::WithinTtl.may_skip_probe(ttl, false));
        assert!(!OverlayFreshness::WithinTtl.may_skip_probe(ttl, true));
        assert!(!OverlayFreshness::WithinTtl.may_skip_probe(Duration::ZERO, false));
        assert!(!OverlayFreshness::Authoritative.may_skip_probe(ttl, false));
        assert!(!OverlayFreshness::Authoritative.may_skip_probe(ttl, true));
        assert!(!OverlayFreshness::Authoritative.may_skip_probe(Duration::ZERO, false));
    }

    /// The write-path fast path skips the fingerprint probe only for an
    /// overlay validated within the TTL; a reload (which replaces the entry
    /// with an unvalidated one), an expired validation, a missing tenant and a
    /// zero TTL all force a probe.
    #[test]
    fn fingerprint_probe_is_skipped_only_within_ttl() {
        let stored: StoredByTenant = Arc::new(RwLock::new(HashMap::new()));
        let ttl = Duration::from_secs(60);
        assert!(fresh_overlay_registry(&stored, "t", ttl).is_none());

        stored
            .write()
            .insert("t".into(), overlay(Some(std::time::Instant::now())));
        assert!(fresh_overlay_registry(&stored, "t", ttl).is_some());
        assert!(fresh_overlay_registry(&stored, "other", ttl).is_none());
        assert!(fresh_overlay_registry(&stored, "t", Duration::ZERO).is_none());

        // A SearchParameter write reloads the cache: entry replaced, unvalidated.
        stored.write().insert("t".into(), overlay(None));
        assert!(fresh_overlay_registry(&stored, "t", ttl).is_none());

        // Expired validation.
        let old = std::time::Instant::now() - Duration::from_secs(120);
        stored.write().insert("t".into(), overlay(Some(old)));
        assert!(fresh_overlay_registry(&stored, "t", ttl).is_none());
    }

    /// #1793: only a connect that failed before reaching the server is
    /// retried; pool exhaustion, a closed pool and timeouts are not.
    #[test]
    fn only_network_connect_failures_are_retried() {
        use deadpool_postgres::{PoolError, TimeoutType};
        assert!(!is_transient_connect_failure(&PoolError::Timeout(
            TimeoutType::Wait
        )));
        assert!(!is_transient_connect_failure(&PoolError::Timeout(
            TimeoutType::Create
        )));
        assert!(!is_transient_connect_failure(&PoolError::Closed));
        assert!(!is_transient_connect_failure(
            &PoolError::NoRuntimeSpecified
        ));
        assert_eq!(connect_retry_delay(1), Duration::from_millis(100));
        assert_eq!(connect_retry_delay(2), Duration::from_millis(300));
    }

    #[test]
    fn missing_is_not_advertised_for_composite_or_special_parameters() {
        assert!(PostgresBackend::modifiers_for_type(SearchParamType::Composite).is_empty());
        assert!(PostgresBackend::modifiers_for_type(SearchParamType::Special).is_empty());
        assert!(PostgresBackend::modifiers_for_type(SearchParamType::String).contains(&"missing"));
    }

    // ── config builders (#355) ────────────────────────────────

    #[test]
    fn config_from_connection_string_builds_without_connecting() {
        let cfg = PostgresBackend::config_from_connection_string(
            "postgres://alice:s3cret@db.example.com:5433/clinical",
        )
        .unwrap();
        assert_eq!(cfg.host, "db.example.com");
        assert_eq!(cfg.port, 5433);
        assert_eq!(cfg.dbname, "clinical");
        // The URL cannot express a FHIR version, so the config carries the
        // compile-time default until the caller overrides it — which is the
        // whole point of exposing the builder separately from connecting.
        assert_eq!(cfg.fhir_version, FhirVersion::default_enabled());
    }

    #[test]
    fn config_from_env_builds_without_connecting() {
        let cfg = PostgresBackend::config_from_env();
        assert!(!cfg.host.is_empty());
        assert!(cfg.port > 0);
        assert_eq!(cfg.fhir_version, FhirVersion::default_enabled());
    }

    // ── parse_connection_string ───────────────────────────────────

    #[test]
    fn parses_full_url() {
        let cfg = PostgresBackend::parse_connection_string(
            "postgres://alice:s3cret@db.example.com:5433/clinical",
        )
        .unwrap();
        assert_eq!(cfg.user, "alice");
        assert_eq!(cfg.password.as_deref(), Some("s3cret"));
        assert_eq!(cfg.host, "db.example.com");
        assert_eq!(cfg.port, 5433);
        assert_eq!(cfg.dbname, "clinical");
    }

    #[test]
    fn strips_query_string_so_it_does_not_leak_into_dbname() {
        // Regression for #224: a `?sslmode=...` suffix was previously parsed as
        // part of the database name (`clinical?sslmode=require`), which no server
        // could resolve.
        let cfg = PostgresBackend::parse_connection_string(
            "postgres://alice:s3cret@db.example.com:5433/clinical?sslmode=require&connect_timeout=10",
        )
        .unwrap();
        assert_eq!(cfg.dbname, "clinical");
        assert_eq!(cfg.host, "db.example.com");
        assert_eq!(cfg.port, 5433);
    }

    #[test]
    fn accepts_postgresql_scheme_and_defaults_port() {
        let cfg = PostgresBackend::parse_connection_string("postgresql://bob:pw@localhost/helios")
            .unwrap();
        assert_eq!(cfg.user, "bob");
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.port, 5432); // no explicit port → default
        assert_eq!(cfg.dbname, "helios");
    }

    #[test]
    fn parses_url_without_password() {
        let cfg =
            PostgresBackend::parse_connection_string("postgres://svc@10.0.0.5:5432/db").unwrap();
        assert_eq!(cfg.user, "svc");
        assert_eq!(cfg.password, None);
        assert_eq!(cfg.host, "10.0.0.5");
        assert_eq!(cfg.dbname, "db");
    }

    #[test]
    fn invalid_port_falls_back_to_default() {
        let cfg =
            PostgresBackend::parse_connection_string("postgres://u:p@host:not-a-port/db").unwrap();
        assert_eq!(cfg.host, "host");
        assert_eq!(cfg.port, 5432);
        assert_eq!(cfg.dbname, "db");
    }

    #[test]
    fn parses_host_and_port_without_dbname() {
        // `rest` has no '/', so the host:port branch is taken and dbname keeps
        // its default.
        let cfg = PostgresBackend::parse_connection_string("postgres://u:p@myhost:6000").unwrap();
        assert_eq!(cfg.host, "myhost");
        assert_eq!(cfg.port, 6000);
        assert_eq!(cfg.dbname, default_dbname());
    }

    #[test]
    fn parses_bare_host_only() {
        // No port and no dbname after the host.
        let cfg = PostgresBackend::parse_connection_string("postgres://u:p@onlyhost").unwrap();
        assert_eq!(cfg.host, "onlyhost");
        assert_eq!(cfg.port, default_port());
    }

    // ── default tuning knobs ──────────────────────────────────────

    #[test]
    fn default_pool_size_is_bounded() {
        // Whatever the host's core count, the pool default stays within the band
        // #224 established as useful — and never the old fixed 10.
        let n = default_max_connections();
        assert!((16..=64).contains(&n), "pool default {n} out of band");
    }

    #[test]
    fn default_timeouts_match_documented_values() {
        assert_eq!(default_statement_timeout_ms(), 30_000);
        assert_eq!(default_pool_wait_timeout_secs(), 10);
        assert_eq!(default_connect_timeout_secs(), 5);
    }

    #[test]
    fn config_default_populates_new_knobs() {
        let cfg = PostgresConfig::default();
        assert_eq!(cfg.pool_wait_timeout_secs, default_pool_wait_timeout_secs());
        assert_eq!(cfg.statement_timeout_ms, default_statement_timeout_ms());
        assert!((16..=64).contains(&cfg.max_connections));
    }

    // ── apply_env_overrides ───────────────────────────────────────

    // Serializes the two env-mutating tests below against each other. Other
    // tests in this module never assert on the env-overridable fields
    // (max_connections / statement_timeout_ms / pool_wait_timeout_secs), so they
    // are unaffected by these vars being set transiently.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_overrides_apply_when_set() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: serialized by ENV_GUARD, and every var is removed before the
        // lock is released, so no other thread observes a partial state.
        unsafe {
            std::env::set_var("HFS_PG_MAX_CONNECTIONS", "99");
            std::env::set_var("HFS_PG_STATEMENT_TIMEOUT_MS", "1234");
            std::env::set_var("HFS_PG_POOL_WAIT_TIMEOUT_SECS", "7");
        }

        let mut cfg = PostgresConfig::default();
        cfg.apply_env_overrides();
        assert_eq!(cfg.max_connections, 99);
        assert_eq!(cfg.statement_timeout_ms, 1234);
        assert_eq!(cfg.pool_wait_timeout_secs, 7);

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("HFS_PG_MAX_CONNECTIONS");
            std::env::remove_var("HFS_PG_STATEMENT_TIMEOUT_MS");
            std::env::remove_var("HFS_PG_POOL_WAIT_TIMEOUT_SECS");
        }
    }

    #[test]
    fn count_by_tenant_budget_defaults_below_the_request_timeout_and_survives_serde() {
        let cfg = PostgresConfig::default();
        assert_eq!(cfg.count_by_tenant_statement_timeout_ms, 25_000);
        assert_eq!(cfg.effective_count_by_tenant_statement_timeout_ms(), 25_000);

        // An older serialized config without the field still loads.
        let from_empty: PostgresConfig =
            serde_json::from_str("{}").expect("every field has a serde default");
        assert_eq!(from_empty.count_by_tenant_statement_timeout_ms, 25_000);

        let cfg = PostgresConfig {
            count_by_tenant_statement_timeout_ms: 1_500,
            ..Default::default()
        };
        let back: PostgresConfig =
            serde_json::from_str(&serde_json::to_string(&cfg).expect("serializes"))
                .expect("deserializes");
        assert_eq!(back.count_by_tenant_statement_timeout_ms, 1_500);
    }

    #[test]
    fn count_by_tenant_budget_only_tightens_the_general_statement_timeout() {
        let effective = |count, general| {
            PostgresConfig {
                count_by_tenant_statement_timeout_ms: count,
                statement_timeout_ms: general,
                ..Default::default()
            }
            .effective_count_by_tenant_statement_timeout_ms()
        };
        assert_eq!(effective(25_000, 30_000), 25_000);
        // A lower general timeout still caps the count.
        assert_eq!(effective(25_000, 1_500), 1_500);
        // Raising the count budget past the general one does not lift it.
        assert_eq!(effective(60_000, 30_000), 30_000);
        // A general timeout of 0 means "no limit", so the count budget rules.
        assert_eq!(effective(25_000, 0), 25_000);
    }

    #[test]
    fn count_by_tenant_budget_env_override() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = PostgresConfig::default();
        for (raw, expected) in [
            ("4000", 4_000),
            // Unparseable values are ignored, like the sibling knobs; that
            // includes surrounding whitespace, which none of them trims.
            (" 5000 ", 4_000),
            ("soon", 4_000),
            ("-1", 4_000),
            // Out of range parses; `PostgresBackend::new` rejects it.
            ("0", 0),
        ] {
            // SAFETY: serialized by ENV_GUARD, removed before releasing the lock.
            unsafe {
                std::env::set_var(COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV, raw);
            }
            cfg.apply_env_overrides();
            assert_eq!(
                cfg.count_by_tenant_statement_timeout_ms, expected,
                "{raw:?}"
            );
        }
        // SAFETY: see above.
        unsafe {
            std::env::remove_var(COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV);
        }
        assert_eq!(
            COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS_ENV,
            "HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS"
        );
    }

    #[tokio::test]
    async fn new_rejects_an_out_of_range_count_by_tenant_budget() {
        for invalid in [0, i32::MAX as u64 + 1] {
            let err = PostgresBackend::new(PostgresConfig {
                count_by_tenant_statement_timeout_ms: invalid,
                ..Default::default()
            })
            .await
            .expect_err("out-of-range budget must be refused before connecting");
            assert!(
                err.to_string()
                    .contains("HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS"),
                "{invalid}: {err}"
            );
        }
        assert!(check_count_by_tenant_statement_timeout_ms(1).is_ok());
        assert!(check_count_by_tenant_statement_timeout_ms(i32::MAX as u64).is_ok());
    }

    #[test]
    fn env_overrides_ignore_unparseable_values() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: serialized by ENV_GUARD, removed before releasing the lock.
        unsafe {
            std::env::set_var("HFS_PG_MAX_CONNECTIONS", "not-a-number");
        }

        let mut cfg = PostgresConfig::default();
        let before = cfg.max_connections;
        cfg.apply_env_overrides();
        // A value that fails to parse is ignored (parse → None), leaving the
        // default untouched rather than zeroing the pool.
        assert_eq!(cfg.max_connections, before);

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("HFS_PG_MAX_CONNECTIONS");
        }
    }

    #[test]
    fn the_default_plan_cache_mode_is_the_conservative_one() {
        // Not PostgreSQL's default. `auto` promotes a cached statement to a
        // generic plan after five executions, and the reference predicate
        // (`= $3 OR LIKE $4 ESCAPE`) loses its index bounds when it does:
        // 1,499 buffers becomes 6,005,788 on a 21.8M-row search_index.
        assert_eq!(
            PostgresConfig::default().plan_cache_mode,
            PostgresPlanCacheMode::ForceCustomPlan
        );
    }

    #[test]
    fn both_settings_ride_in_the_startup_packet() {
        let cfg = PostgresConfig::default();
        let opts = startup_options(&cfg);
        assert!(opts.contains("-c statement_timeout=30000"), "{opts}");
        assert!(
            opts.contains("-c plan_cache_mode=force_custom_plan"),
            "{opts}"
        );
    }

    #[test]
    fn plan_cache_mode_parses_only_the_three_guc_values() {
        assert_eq!(
            PostgresPlanCacheMode::from_env_value("AUTO"),
            Some(PostgresPlanCacheMode::Auto)
        );
        assert_eq!(
            PostgresPlanCacheMode::from_env_value(" force_generic_plan "),
            Some(PostgresPlanCacheMode::ForceGenericPlan)
        );
        // Anything else is ignored rather than substituted: a typo must not
        // silently change the plan policy, and nothing operator-supplied may
        // reach the startup packet verbatim.
        assert_eq!(PostgresPlanCacheMode::from_env_value("generic"), None);
        assert_eq!(
            PostgresPlanCacheMode::from_env_value("auto -c work_mem=1GB"),
            None
        );
    }

    #[test]
    fn an_unparseable_plan_cache_mode_leaves_the_default_alone() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by ENV_GUARD, removed before releasing the lock.
        unsafe {
            std::env::set_var("HFS_PG_PLAN_CACHE_MODE", "force_fast_plan");
        }
        let mut cfg = PostgresConfig::default();
        cfg.apply_env_overrides();
        assert_eq!(cfg.plan_cache_mode, PostgresPlanCacheMode::ForceCustomPlan);

        // SAFETY: see above.
        unsafe {
            std::env::set_var("HFS_PG_PLAN_CACHE_MODE", "auto");
        }
        cfg.apply_env_overrides();
        assert_eq!(cfg.plan_cache_mode, PostgresPlanCacheMode::Auto);

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("HFS_PG_PLAN_CACHE_MODE");
        }
    }
}
