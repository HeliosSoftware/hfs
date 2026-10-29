//! Helios FHIR Server (HFS)
//!
//! A high-performance FHIR R4/R4B/R5/R6 server with pluggable storage backends.
//!
//! # Storage Backends
//!
//! | Backend | Feature Flag | Description |
//! |---------|--------------|-------------|
//! | SQLite (default) | `sqlite` | Zero-config embedded database with FTS5 search |
//! | SQLite + Elasticsearch | `sqlite,elasticsearch` | SQLite for CRUD, Elasticsearch for search |
//! | PostgreSQL | `postgres` | Full-featured RDBMS with JSONB storage and tsvector search |
//! | PostgreSQL + Elasticsearch | `postgres,elasticsearch` | PostgreSQL for CRUD, Elasticsearch for search |
//! | MongoDB | `mongodb` | Document database with native JSON resource storage |
//! | MongoDB + Elasticsearch | `mongodb,elasticsearch` | MongoDB for CRUD, Elasticsearch for search |
//! | S3 | `s3` | AWS S3 object storage for CRUD, versioning, history, and bulk ops (no search) |
//! | S3 + Elasticsearch | `s3,elasticsearch` | S3 for CRUD/history, Elasticsearch for search |
//!
//! Set `HFS_STORAGE_BACKEND` to `sqlite`, `sqlite-elasticsearch`, `postgres`,
//! `postgres-elasticsearch`, `mongodb`, `mongodb-elasticsearch`, `s3`, or `s3-elasticsearch`.

use std::sync::Arc;

mod worker_shutdown;

use helios_audit::{
    AuditBackend, AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter, lifecycle,
};
use helios_auth::{AuthConfig, JwksBearerAuthProvider, JwksCache};
use helios_observability::redact::redact_url;
#[cfg(feature = "mongodb")]
use helios_persistence::core::database_url::redact_database_url;
#[cfg(feature = "postgres")]
use helios_persistence::core::database_url::redact_postgres_location;
use helios_persistence::{BackendKind, ResourceStorage, TenantContext};
use helios_rest::{AuthMiddlewareState, ServerConfig, StorageBackendMode};
use tracing::{info, warn};

// Bulk *export* is only wired on backends that can host (or sidecar) job state,
// so its imports stay narrow. Bulk *submit* is wired on every backend that
// implements the job store — SQLite, PostgreSQL, MongoDB, and S3 — so the types
// it needs are gated on the wider set.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
use helios_persistence::backends::local_fs::LocalFsOutputStore;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_persistence::core::{BulkExportJobStore, DefaultExportWorker};
use helios_persistence::core::{BulkProviderStore, SettingsStore, WriteObserver};
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
use helios_persistence::core::{
    BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, SubmitInputFetcher, WorkerId,
};
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
use helios_rest::bulk_export_auth::BearerScopeAuth;
// Every startup path goes through one builder. The bundles it takes are all
// optional, so a backend that lacks a capability passes `None` rather than
// reaching for a different entry point: S3 has no search index to reindex,
// MongoDB rides an embedded SQLite sidecar for bulk-export job state, and so on.
// Every standalone primary backend (SQLite, PostgreSQL, MongoDB, and now S3)
// *does* host the per-user settings store, so all of them wire one.
use helios_rest::OperationsBundle;
use helios_rest::create_app_with_auth_bulk_settings_and_ops;

use helios_persistence::core::PurgableStorage;
// Only the reindex-capable ops bundles use this; the S3-only build has no
// reindex path.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
use helios_persistence::search::ReindexOperation;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

#[cfg(feature = "mongodb")]
use helios_persistence::backends::mongodb::{IndexBuildMode, MongoBackend, MongoBackendConfig};
fn is_database_audit_dedicated(config: &AuditConfig, backend_kind: BackendKind) -> bool {
    match backend_kind {
        BackendKind::Sqlite | BackendKind::Postgres => config.database_url.is_some(),
        BackendKind::MongoDB => config.database_url.is_some() || config.mongodb_database.is_some(),
        BackendKind::S3 => {
            config.s3_bucket.is_some()
                || config.s3_prefix.is_some()
                || config.s3_region.is_some()
                || config.s3_validate_buckets.is_some()
        }
        _ => false,
    }
}

#[cfg(feature = "s3")]
fn parse_env_bool(var: &str, default_value: bool) -> bool {
    std::env::var(var)
        .map(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            !(normalized == "false" || normalized == "0")
        })
        .unwrap_or(default_value)
}

/// An S3 endpoint override, or `None` when it is unset or blank — so an empty
/// `HFS_*_S3_ENDPOINT=` neither replaces the AWS endpoint nor forces
/// path-style addressing.
/// Configured URLs as safe to log: each passed through [`redact_url`], which
/// drops `user:pass@`, the query and the fragment (#2149). Elasticsearch keeps
/// credentials written into `HFS_ELASTICSEARCH_NODES` and sends them as Basic
/// auth, so the raw node list can carry a password.
// Only the `*-elasticsearch` startups call it; the unit test keeps it
// exercised in every build.
#[cfg_attr(not(feature = "elasticsearch"), allow(dead_code))]
fn redact_urls(urls: &[String]) -> Vec<String> {
    urls.iter().map(|url| redact_url(url)).collect()
}

// Only bulk export's S3 output store calls it, and that is built only with a
// SQL backend; the unit test keeps it exercised in every build.
#[cfg_attr(
    not(all(feature = "s3", any(feature = "sqlite", feature = "postgres"))),
    allow(dead_code)
)]
fn non_blank_endpoint(endpoint: Option<String>) -> Option<String> {
    endpoint
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
}

/// The `HFS_S3_*` variables that decide where tenants live on S3, read as raw
/// strings so [`resolve`](Self::resolve) can be unit-tested without touching
/// the process environment (#1598).
///
/// | Variable | Mode | Meaning |
/// |---|---|---|
/// | `HFS_S3_TENANCY` | both | `prefix-per-tenant` (default) or `bucket-per-tenant` |
/// | `HFS_S3_BUCKET` | prefix-per-tenant | the one shared bucket (default `hfs`) |
/// | `HFS_S3_TENANT_BUCKETS` | bucket-per-tenant | `tenant=bucket,tenant=bucket,…` |
/// | `HFS_S3_SYSTEM_BUCKET` | bucket-per-tenant | the tenant-independent bucket (optional) |
///
/// A variable that belongs to the other mode is an error, not ignored: an
/// operator who sets `HFS_S3_BUCKET` under bucket-per-tenant expects it to
/// mean something, and silently serving from a different bucket is the worst
/// way to tell them it does not.
#[cfg(feature = "s3")]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct S3TenancyEnv {
    tenancy: Option<String>,
    bucket: Option<String>,
    tenant_buckets: Option<String>,
    system_bucket: Option<String>,
}

#[cfg(feature = "s3")]
impl S3TenancyEnv {
    /// Reads the four variables; blank values count as unset.
    fn from_env() -> Self {
        fn read(var: &str) -> Option<String> {
            std::env::var(var)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        }
        Self {
            tenancy: read("HFS_S3_TENANCY"),
            bucket: read("HFS_S3_BUCKET"),
            tenant_buckets: read("HFS_S3_TENANT_BUCKETS"),
            system_bucket: read("HFS_S3_SYSTEM_BUCKET"),
        }
    }

    /// Whether `HFS_S3_TENANCY` selects bucket-per-tenant. Case-insensitive,
    /// `_` and `-` interchangeable; anything but the two modes is an error.
    fn bucket_per_tenant(&self) -> anyhow::Result<bool> {
        match self
            .tenancy
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase().replace('_', "-"))
            .as_deref()
        {
            None | Some("prefix-per-tenant") => Ok(false),
            Some("bucket-per-tenant") => Ok(true),
            Some(other) => anyhow::bail!(
                "HFS_S3_TENANCY '{other}' invalid (expected prefix-per-tenant|bucket-per-tenant)"
            ),
        }
    }

    /// Resolves the tenancy mode the primary S3 backend is built with.
    ///
    /// `default_tenant` is `HFS_DEFAULT_TENANT`: in bucket-per-tenant mode it
    /// must be mapped, because every request without a tenant is served from
    /// it and an unmapped default tenant would fail every such request after a
    /// clean start.
    fn resolve(
        &self,
        default_tenant: &str,
    ) -> anyhow::Result<helios_persistence::backends::s3::S3TenancyMode> {
        use helios_persistence::backends::s3::S3TenancyMode;

        if !self.bucket_per_tenant()? {
            if let Some(var) = [
                ("HFS_S3_TENANT_BUCKETS", &self.tenant_buckets),
                ("HFS_S3_SYSTEM_BUCKET", &self.system_bucket),
            ]
            .into_iter()
            .find_map(|(name, value)| value.as_ref().map(|_| name))
            {
                anyhow::bail!(
                    "{var} applies only to HFS_S3_TENANCY=bucket-per-tenant; the current mode \
                     is prefix-per-tenant (the default), which uses HFS_S3_BUCKET alone"
                );
            }
            return Ok(S3TenancyMode::PrefixPerTenant {
                bucket: self.bucket.clone().unwrap_or_else(|| "hfs".to_string()),
            });
        }

        if let Some(bucket) = &self.bucket {
            anyhow::bail!(
                "HFS_S3_BUCKET='{bucket}' has no meaning under HFS_S3_TENANCY=bucket-per-tenant: \
                 name each tenant's bucket in HFS_S3_TENANT_BUCKETS (tenant=bucket,…) and the \
                 tenant-independent bucket, if any, in HFS_S3_SYSTEM_BUCKET"
            );
        }
        let Some(spec) = &self.tenant_buckets else {
            anyhow::bail!(
                "HFS_S3_TENANT_BUCKETS is required when HFS_S3_TENANCY=bucket-per-tenant \
                 (tenant=bucket,tenant=bucket,…; HFS_DEFAULT_TENANT '{default_tenant}' must be \
                 among them)"
            );
        };
        let tenant_bucket_map = parse_s3_tenant_buckets(spec)?;
        if !tenant_bucket_map.contains_key(default_tenant) {
            anyhow::bail!(
                "HFS_DEFAULT_TENANT '{default_tenant}' has no bucket in HFS_S3_TENANT_BUCKETS; \
                 every request that names no tenant is served from the default tenant, so it \
                 needs one"
            );
        }
        Ok(S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket: self.system_bucket.clone(),
        })
    }
}

/// Parses `HFS_S3_TENANT_BUCKETS`: comma-separated `tenant=bucket` pairs.
/// Blank entries (a trailing comma) are skipped; a tenant id goes through the
/// canonical `TenantId::parse`, so a reserved id such as `__system__` is
/// refused here rather than minted into the map. Distinct buckets per tenant
/// are enforced by `S3BackendConfig::validate` when the backend is built.
#[cfg(feature = "s3")]
fn parse_s3_tenant_buckets(
    spec: &str,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    use helios_persistence::tenant::TenantId;

    let mut map = std::collections::HashMap::new();
    for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((tenant, bucket)) = entry.split_once('=') else {
            anyhow::bail!("HFS_S3_TENANT_BUCKETS entry '{entry}' is not of the form tenant=bucket");
        };
        let (tenant, bucket) = (tenant.trim(), bucket.trim());
        if bucket.is_empty() {
            anyhow::bail!("HFS_S3_TENANT_BUCKETS entry '{entry}' names no bucket");
        }
        let tenant = TenantId::parse(tenant).map_err(|e| {
            anyhow::anyhow!("HFS_S3_TENANT_BUCKETS: tenant id '{tenant}' is not valid: {e}")
        })?;
        if map
            .insert(tenant.as_str().to_string(), bucket.to_string())
            .is_some()
        {
            anyhow::bail!(
                "HFS_S3_TENANT_BUCKETS maps tenant '{}' more than once",
                tenant.as_str()
            );
        }
    }
    if map.is_empty() {
        anyhow::bail!("HFS_S3_TENANT_BUCKETS names no tenant (expected tenant=bucket,…)");
    }
    Ok(map)
}

/// One line saying where tenants live, for the startup log and error context.
/// Deterministic (the map is sorted) so two boots log the same text.
#[cfg(feature = "s3")]
fn describe_s3_tenancy(mode: &helios_persistence::backends::s3::S3TenancyMode) -> String {
    use helios_persistence::backends::s3::S3TenancyMode;
    match mode {
        S3TenancyMode::PrefixPerTenant { bucket } => format!("prefix-per-tenant bucket={bucket}"),
        S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket,
        } => {
            let mut pairs: Vec<String> = tenant_bucket_map
                .iter()
                .map(|(tenant, bucket)| format!("{tenant}={bucket}"))
                .collect();
            pairs.sort();
            format!(
                "bucket-per-tenant tenants=[{}] system_bucket={}",
                pairs.join(","),
                default_system_bucket.as_deref().unwrap_or("(none)")
            )
        }
    }
}

/// Parses `HFS_S3_KEYSPACE_LAYOUT`: the layout new (empty) tenants are created
/// with. Unset or blank is `v1`; anything but `v1`/`v2` is an error rather than
/// a silent fall back to the flat layout.
#[cfg(feature = "s3")]
fn parse_s3_keyspace_layout(
    raw: Option<&str>,
) -> anyhow::Result<helios_persistence::backends::s3::KeyspaceLayout> {
    use helios_persistence::backends::s3::KeyspaceLayout;

    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(KeyspaceLayout::V1),
        Some(value) => KeyspaceLayout::parse(value).ok_or_else(|| {
            anyhow::anyhow!("HFS_S3_KEYSPACE_LAYOUT '{value}' invalid (expected v1|v2)")
        }),
    }
}

/// Parses `HFS_S3_SCAN_CONCURRENCY`: the most `GetObject` requests the S3 read
/// fan-outs may have in flight at once. Unset or blank is the default (32); a
/// value that is not a whole number of at least 1 is an error rather than a
/// silent fall back, since `0` would stall every scan.
#[cfg(feature = "s3")]
fn parse_s3_scan_concurrency(raw: Option<&str>) -> anyhow::Result<usize> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(helios_persistence::backends::s3::DEFAULT_SCAN_CONCURRENCY),
        Some(value) => match value.parse::<usize>() {
            Ok(n) if n >= 1 => Ok(n),
            _ => anyhow::bail!(
                "HFS_S3_SCAN_CONCURRENCY '{value}' invalid (expected a whole number of at least 1)"
            ),
        },
    }
}

/// Builds the primary S3 backend's configuration from the `HFS_S3_*`
/// environment: the one reader for `start_s3`, `start_s3_elasticsearch` and
/// the shared database audit sink, so the three cannot disagree about where
/// tenants live.
#[cfg(feature = "s3")]
fn s3_backend_config_from_env(
    default_tenant: &str,
) -> anyhow::Result<helios_persistence::backends::s3::S3BackendConfig> {
    use helios_persistence::backends::s3::S3BackendConfig;

    let tenancy_mode = S3TenancyEnv::from_env().resolve(default_tenant)?;
    let default_keyspace_layout =
        parse_s3_keyspace_layout(std::env::var("HFS_S3_KEYSPACE_LAYOUT").ok().as_deref())?;
    let scan_concurrency =
        parse_s3_scan_concurrency(std::env::var("HFS_S3_SCAN_CONCURRENCY").ok().as_deref())?;
    Ok(S3BackendConfig {
        tenancy_mode,
        default_keyspace_layout,
        scan_concurrency,
        region: std::env::var("HFS_S3_REGION").ok(),
        prefix: std::env::var("HFS_S3_PREFIX").ok(),
        endpoint_url: std::env::var("HFS_S3_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty()),
        force_path_style: parse_env_bool("HFS_S3_FORCE_PATH_STYLE", false),
        allow_http: parse_env_bool("HFS_S3_ALLOW_HTTP", true),
        validate_buckets_on_startup: parse_env_bool("HFS_S3_VALIDATE_BUCKETS", true),
        ..Default::default()
    })
}

/// Bucket-per-tenant: `HFS_S3_TENANT_BUCKETS` is the provisioning, so the
/// mapped tenants are made real tenants at startup rather than waiting for a
/// `POST /admin/tenants` that would only ever repeat the map (#1598).
///
/// With a system bucket the registry exists, and each mapped tenant is
/// registered there if it is not yet; the regular conformance seed then
/// enumerates the registry and covers them. Without one there is no registry
/// (`/admin/tenants` answers 501), so the mapped tenants are seeded directly
/// here — the only thing registration would otherwise have unlocked. The
/// default tenant is left to the regular seed in both cases. On a composite,
/// `storage` is the composite so the seed also lands in the search index.
///
/// Prefix-per-tenant passes an empty `mapped` and this does nothing.
#[cfg(feature = "s3")]
async fn provision_mapped_tenants<S>(
    storage: &S,
    mapped: &[String],
    config: &ServerConfig,
    observer: Option<&dyn WriteObserver>,
) where
    S: ResourceStorage + ?Sized,
{
    if mapped.is_empty() {
        return;
    }
    if storage.supports_tenant_registry() {
        for tenant in mapped {
            match storage.get_tenant(tenant).await {
                Ok(Some(_)) => {}
                Ok(None) => match storage.register_tenant(tenant, None).await {
                    Ok(_) => {
                        info!(tenant = %tenant, "Registered tenant mapped in HFS_S3_TENANT_BUCKETS")
                    }
                    Err(e) => warn!(tenant = %tenant, "Registering mapped tenant failed: {e}"),
                },
                Err(e) => warn!(tenant = %tenant, "Checking mapped tenant failed: {e}"),
            }
        }
        return;
    }
    if !config.seed_conformance {
        return;
    }
    let data_dir = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    for tenant in mapped.iter().filter(|t| **t != config.default_tenant) {
        helios_persistence::search::seed_tenant_conformance(
            storage,
            config.default_fhir_version,
            &data_dir,
            tenant,
            observer,
        )
        .await;
    }
}

#[cfg(feature = "postgres")]
fn is_postgres_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

#[cfg(feature = "mongodb")]
fn is_mongodb_url(url: &str) -> bool {
    url.starts_with("mongodb://") || url.starts_with("mongodb+srv://")
}

/// Resolves the composite sync mode from `HFS_COMPOSITE_SYNC_MODE`.
///
/// Composite stores route writes to a primary backend (e.g. SQLite, Postgres,
/// MongoDB, S3) and a search backend (Elasticsearch). `Asynchronous` (the
/// default) acks the write as soon as the primary commits and forwards the
/// search-index write on a background worker — lowest latency, but a
/// follow-up search can race the indexing. `Synchronous` blocks the write
/// until the search backend has indexed it, giving read-your-write
/// semantics at the cost of extra latency on each write.
#[cfg(feature = "elasticsearch")]
fn composite_sync_mode_from_env() -> helios_persistence::composite::SyncMode {
    use helios_persistence::composite::SyncMode;
    match std::env::var("HFS_COMPOSITE_SYNC_MODE") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "synchronous" | "sync" => SyncMode::Synchronous,
            "asynchronous" | "async" => SyncMode::Asynchronous,
            "hybrid" => SyncMode::Hybrid {
                sync_for_search: true,
            },
            other => {
                tracing::warn!(
                    value = other,
                    "Unknown HFS_COMPOSITE_SYNC_MODE; defaulting to asynchronous"
                );
                SyncMode::Asynchronous
            }
        },
        Err(_) => SyncMode::Asynchronous,
    }
}

/// Forwards composite secondary sync failures to the Prometheus exporter
/// (#1334): `composite_secondary_sync_failures_total{backend,operation}`, the
/// `composite_secondary_sync_needs_reindex` gauge and the
/// `composite_secondary_sync_queue_depth` gauge (#1545).
#[cfg(feature = "elasticsearch")]
struct CompositeSyncMetrics;

#[cfg(feature = "elasticsearch")]
impl helios_persistence::composite::SecondarySyncObserver for CompositeSyncMetrics {
    fn sync_failed(
        &self,
        backend_id: &str,
        operation: helios_persistence::composite::SyncOperation,
    ) {
        helios_observability::composite_metrics::record_secondary_sync_failure(
            backend_id,
            operation.as_str(),
        );
    }

    fn needs_reindex(&self, outstanding: u64) {
        helios_observability::composite_metrics::set_secondary_sync_needs_reindex(outstanding);
    }

    fn queue_depth(&self, depth: u64) {
        helios_observability::composite_metrics::set_secondary_sync_queue_depth(depth);
    }
}

/// Reads a non-negative integer setting, falling back on anything else.
#[cfg(any(feature = "elasticsearch", feature = "mongodb"))]
fn env_u64(var: &str, default_value: u64) -> u64 {
    match std::env::var(var) {
        Ok(v) => v.trim().parse().unwrap_or_else(|_| {
            tracing::warn!(variable = var, value = %v, default_value, "Not a number; using the default");
            default_value
        }),
        Err(_) => default_value,
    }
}

/// Periodically drains the composite's "needs reindex" records (#1334): each
/// pass re-syncs the primary's current state of up to
/// `HFS_COMPOSITE_SYNC_REPAIR_BATCH` (default 100) recorded resources to the
/// secondary that missed them. Runs every
/// `HFS_COMPOSITE_SYNC_REPAIR_INTERVAL` seconds (default 60; `0` disables),
/// starting right away so records left by an earlier run are picked up.
///
/// A primary without a ledger (S3) has nothing to drain, so no task is
/// spawned for it.
#[cfg(feature = "elasticsearch")]
fn spawn_secondary_sync_repair(
    composite: Arc<helios_persistence::composite::CompositeStorage>,
    has_ledger: bool,
) {
    if !has_ledger {
        info!(
            "This primary keeps no needs-reindex ledger: failed secondary syncs are counted and logged only; repair them with $reindex"
        );
        return;
    }
    let interval = env_u64("HFS_COMPOSITE_SYNC_REPAIR_INTERVAL", 60);
    if interval == 0 {
        info!(
            "HFS_COMPOSITE_SYNC_REPAIR_INTERVAL=0: periodic repair of failed secondary syncs is off"
        );
        return;
    }
    let batch = env_u64("HFS_COMPOSITE_SYNC_REPAIR_BATCH", 100).max(1) as usize;
    let interval = std::time::Duration::from_secs(interval);
    tokio::spawn(async move {
        loop {
            match composite.repair_secondary_sync_failures(batch).await {
                Ok(report) if report.examined > 0 => info!(
                    examined = report.examined,
                    repaired = report.repaired,
                    still_failing = report.still_failing,
                    dropped = report.dropped,
                    remaining = report.remaining,
                    "Secondary sync repair pass finished"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("Repair of failed secondary syncs could not run: {e}"),
            }
            tokio::time::sleep(interval).await;
        }
    });
}

#[cfg(feature = "elasticsearch")]
fn es_write_refresh_from_config(
    config: &ServerConfig,
) -> anyhow::Result<helios_persistence::backends::elasticsearch::WriteRefreshPolicy> {
    config
        .elasticsearch_write_refresh
        .parse()
        .map_err(|e: String| anyhow::anyhow!("{} (from HFS_ELASTICSEARCH_WRITE_REFRESH)", e))
}

/// The refresh policy `$reindex` and the deferred rebuild use, or `None` to
/// follow `HFS_ELASTICSEARCH_WRITE_REFRESH` (unset or blank).
#[cfg(feature = "elasticsearch")]
fn es_reindex_refresh_from_config(
    config: &ServerConfig,
) -> anyhow::Result<Option<helios_persistence::backends::elasticsearch::WriteRefreshPolicy>> {
    match config
        .elasticsearch_reindex_refresh
        .as_deref()
        .map(str::trim)
    {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|e: String| anyhow::anyhow!("{} (from HFS_ELASTICSEARCH_REINDEX_REFRESH)", e)),
    }
}

/// Subscription status-write budget in milliseconds, as the subscriptions
/// engine reads it (`HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS`, default 5 s).
#[cfg(feature = "elasticsearch")]
fn subscription_status_write_timeout_ms() -> u64 {
    std::env::var("HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(5_000)
}

/// Why `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` with this refresh interval
/// would stall interactive writes, or `None` when it would not.
///
/// With `wait_for` every write blocks until the next refresh, so the interval
/// is the price of each write: a disabled interval (`-1`) blocks until
/// something else refreshes the index, and one at or above the Elasticsearch
/// request timeout turns into a failed attempt and a retry (#1749). An
/// interval at or above the subscriptions status-write budget makes every
/// status transition time out when writes are synchronous. That is legal
/// configuration, so it is a warning, not a startup failure.
#[cfg(feature = "elasticsearch")]
fn es_refresh_wait_warning(
    write_refresh: helios_persistence::backends::elasticsearch::WriteRefreshPolicy,
    refresh_interval: &str,
    request_timeout_ms: u64,
    status_write_timeout_ms: u64,
    sync_mode: helios_persistence::composite::SyncMode,
) -> Option<String> {
    use helios_persistence::backends::elasticsearch::{
        RefreshInterval, WriteRefreshPolicy, parse_refresh_interval,
    };
    use helios_persistence::composite::SyncMode;

    if write_refresh != WriteRefreshPolicy::WaitFor {
        return None;
    }
    let ms_of = |d: std::time::Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    let reason = match parse_refresh_interval(refresh_interval)? {
        RefreshInterval::Disabled => "periodic refresh is disabled (-1), so a wait_for write \
             blocks until something else refreshes the index"
            .to_string(),
        RefreshInterval::Every(interval) => {
            let interval_ms = ms_of(interval);
            if interval_ms >= request_timeout_ms {
                format!(
                    "the interval ({interval_ms} ms) is not below \
                     HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS ({request_timeout_ms} ms), so a write \
                     can time out waiting for the refresh and be retried"
                )
            } else if interval_ms >= status_write_timeout_ms
                && matches!(
                    sync_mode,
                    SyncMode::Synchronous
                        | SyncMode::Hybrid {
                            sync_for_search: true
                        }
                )
            {
                format!(
                    "the interval ({interval_ms} ms) is not below \
                     HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS ({status_write_timeout_ms} ms), so \
                     a subscription status transition can time out and not be persisted"
                )
            } else {
                return None;
            }
        }
    };
    Some(format!(
        "HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for with HFS_ELASTICSEARCH_REFRESH_INTERVAL=\
         {refresh_interval}: {reason}. Use a short interval (the default is 1s) or \
         HFS_ELASTICSEARCH_WRITE_REFRESH=false"
    ))
}

/// Startup work on a freshly built Elasticsearch backend: warns about a refresh
/// configuration that stalls writes, then applies the configured
/// `refresh_interval` to indices that already exist (they are otherwise only
/// given it when created). Never fails startup.
#[cfg(feature = "elasticsearch")]
async fn es_startup_refresh_setup(
    es: &helios_persistence::backends::elasticsearch::ElasticsearchBackend,
    config: &ServerConfig,
) {
    let es_config = es.config();
    if let Some(message) = es_refresh_wait_warning(
        es_config.write_refresh,
        &es_config.refresh_interval,
        es_config.request_timeout_ms,
        subscription_status_write_timeout_ms(),
        composite_sync_mode_from_env(),
    ) {
        warn!("{message}");
    }
    match es.apply_refresh_interval().await {
        Ok(0) => {}
        Ok(changed) => info!(
            indices = changed,
            refresh_interval = %config.elasticsearch_refresh_interval,
            "applied the Elasticsearch refresh interval to existing indices"
        ),
        Err(e) => warn!(
            error = %e,
            refresh_interval = %config.elasticsearch_refresh_interval,
            "could not apply the Elasticsearch refresh interval to existing indices"
        ),
    }
}

/// The indexes `$reindex` rebuilds on `sqlite-elasticsearch`.
///
/// Elasticsearch always, because it serves every search here. The SQLite
/// primary only when it still indexes locally: once search is offloaded no
/// query reads its `search_index`/FTS tables (the composite routes the whole
/// query to Elasticsearch, #1012), so rebuilding them is a second FHIRPath
/// extraction, a write transaction on the ingest file, and rows that pile up on
/// every rerun because the matching delete is a no-op (#1125).
#[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
fn sqlite_es_reindex_targets(
    sqlite: &Arc<SqliteBackend>,
    es: &Arc<helios_persistence::backends::elasticsearch::ElasticsearchBackend>,
) -> (
    Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    &'static [&'static str],
) {
    es_only_when_offloaded(
        sqlite.is_search_offloaded(),
        sqlite.clone(),
        &["sqlite", "elasticsearch"],
        es.clone(),
    )
}

/// The indexes `$reindex` rebuilds on `postgres-elasticsearch` (pg-es).
///
/// Same rule as [`sqlite_es_reindex_targets`]: Elasticsearch always, the
/// PostgreSQL primary only while it still indexes locally. pg-es offloads
/// search at startup, so no query reads its `search_index`/`resource_fts`
/// rows, yet the deferred rebuild after `$bulk-submit` wrote them anyway: per
/// group of 128 resources a transaction, 129 advisory locks, two more reads of
/// every body, the 1.5 MB SearchParameter overlay, a second FHIRPath
/// extraction and the `to_tsvector` upsert. On the 18.9 M-resource #939 corpus
/// that grew the database from 98 GB to 229 GB (plus 241 GB of WAL) in a
/// rebuild that had not reached 50 % after 11.6 h.
#[cfg(all(feature = "postgres", feature = "elasticsearch"))]
fn postgres_es_reindex_targets(
    pg: &Arc<helios_persistence::backends::postgres::PostgresBackend>,
    es: &Arc<helios_persistence::backends::elasticsearch::ElasticsearchBackend>,
) -> (
    Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    &'static [&'static str],
) {
    es_only_when_offloaded(
        pg.is_search_offloaded(),
        pg.clone(),
        &["postgres", "elasticsearch"],
        es.clone(),
    )
}

/// The composite rule behind [`sqlite_es_reindex_targets`] and
/// [`postgres_es_reindex_targets`]: with search offloaded only Elasticsearch
/// is a reindex target; otherwise the primary comes first, then Elasticsearch,
/// named by `both`.
#[cfg(all(
    feature = "elasticsearch",
    any(feature = "sqlite", feature = "postgres")
))]
fn es_only_when_offloaded(
    primary_offloaded: bool,
    primary: Arc<dyn helios_persistence::search::ReindexTarget>,
    both: &'static [&'static str],
    es: Arc<dyn helios_persistence::search::ReindexTarget>,
) -> (
    Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    &'static [&'static str],
) {
    if primary_offloaded {
        (vec![es], &["elasticsearch"])
    } else {
        (vec![primary, es], both)
    }
}

/// Room left for the response itself when `HFS_REQUEST_TIMEOUT` expires: the
/// MongoDB backend stops this long before the timeout layer answers `408`.
#[cfg(feature = "mongodb")]
const BUNDLE_TRANSACTION_MARGIN: std::time::Duration = std::time::Duration::from_secs(2);

/// The longest one request may spend in a MongoDB transaction Bundle call,
/// admission wait included (`HFS_REQUEST_TIMEOUT` less a 2 s margin, #1806).
///
/// The request timeout layer drops the handler at `HFS_REQUEST_TIMEOUT` and
/// answers `408`. A replay that started late enough to overrun it would be cut
/// off mid-flight, where the backend would otherwise have given up in time to
/// answer `503 Retry-After`. The margin leaves the response itself room to be
/// written; a timeout of 2 s or less leaves no time, so a transient abort is
/// answered at once without a replay.
///
/// Unlike the replay budget this is not capped, so with a long
/// `HFS_REQUEST_TIMEOUT` a Bundle that queued keeps its full replay budget
/// while no replay can run into the `408`.
#[cfg(feature = "mongodb")]
fn bundle_transaction_deadline(request_timeout_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(request_timeout_secs).saturating_sub(BUNDLE_TRANSACTION_MARGIN)
}

/// How much of one request's time the MongoDB backend may spend re-running a
/// transaction Bundle after transient aborts: the deadline, capped at the
/// backend's own default of 120 s.
///
/// The budget counts from admission, so time spent queued behind other Bundles
/// does not use it up; the deadline ([`bundle_transaction_deadline`]) bounds the
/// whole call, queue wait included.
#[cfg(feature = "mongodb")]
fn bundle_transaction_budget(request_timeout_secs: u64) -> std::time::Duration {
    bundle_transaction_deadline(request_timeout_secs)
        .min(MongoBackendConfig::default().bundle_transaction_budget)
}

#[cfg(feature = "mongodb")]
fn build_mongodb_config(
    config: &ServerConfig,
    search_offloaded: bool,
) -> anyhow::Result<MongoBackendConfig> {
    build_mongodb_config_with_env(config, search_offloaded, |name| std::env::var(name).ok())
}

#[cfg(feature = "mongodb")]
fn build_mongodb_config_with_env<F>(
    config: &ServerConfig,
    search_offloaded: bool,
    env: F,
) -> anyhow::Result<MongoBackendConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let mongo_specific_url = env("HFS_MONGODB_URL").or_else(|| env("HFS_MONGODB_URI"));
    let connection_string = match config.database_url.as_deref() {
        Some(url) if is_mongodb_url(url) => url.to_string(),
        Some(_) => mongo_specific_url.unwrap_or_else(|| "mongodb://localhost:27017".to_string()),
        None => mongo_specific_url
            .or_else(|| env("HFS_DATABASE_URL").filter(|url| is_mongodb_url(url)))
            .unwrap_or_else(|| "mongodb://localhost:27017".to_string()),
    };

    let database_name = env("HFS_MONGODB_DATABASE").unwrap_or_else(|| "helios".to_string());
    let max_connections = env("HFS_MONGODB_MAX_CONNECTIONS")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(10);
    let connect_timeout_ms = env("HFS_MONGODB_CONNECT_TIMEOUT_MS")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5000);
    // Bounds how long an operation waits for a usable server. `connect_timeout_ms`
    // only bounds a TCP handshake, so this is what actually decides how quickly an
    // unreachable MongoDB surfaces an error.
    let server_selection_timeout_ms = env("HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000);
    // Bounds `_include`/`_revinclude` resolution per directive (#1061), so a
    // wide reverse-reference set can't grow a searchset Bundle without limit.
    let max_included_resources = env("HFS_MONGODB_MAX_INCLUDED_RESOURCES")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1000)
        .max(1);
    // The only way to reach `off` (the escape hatch that stops the builder
    // touching the collection at all) or `inline`, so an invalid value must
    // fail startup rather than silently fall back to `background` — same
    // "bail with the invalid value" shape as HFS_BULK_EXPORT_OUTPUT_BACKEND.
    let index_build = match env("HFS_MONGODB_INDEX_BUILD") {
        Some(raw) => raw
            .parse::<IndexBuildMode>()
            .map_err(|message| anyhow::anyhow!(message))?,
        None => IndexBuildMode::default(),
    };

    let mut config = MongoBackendConfig {
        connection_string,
        database_name,
        max_connections,
        connect_timeout_ms,
        server_selection_timeout_ms,
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        search_offloaded,
        max_included_resources,
        index_build,
        reindex_catch_up_margin_ms: MongoBackendConfig::default().reindex_catch_up_margin_ms,
        app_name: MongoBackendConfig::default().app_name,
        bundle_transaction_budget: bundle_transaction_budget(config.request_timeout),
        bundle_transaction_deadline: Some(bundle_transaction_deadline(config.request_timeout)),
        ..Default::default()
    };
    config
        .apply_reindex_env(&env)
        .map_err(|message| anyhow::anyhow!(message))?;
    config
        .apply_transaction_bundle_env(&env)
        .map_err(|message| anyhow::anyhow!(message))?;
    config
        .apply_search_env(&env)
        .map_err(|message| anyhow::anyhow!(message))?;
    config
        .apply_count_env(&env)
        .map_err(|message| anyhow::anyhow!(message))?;
    Ok(config)
}

#[cfg(feature = "sqlite")]
fn validate_shared_sqlite_audit_path(path: &str, dedicated: bool) -> anyhow::Result<()> {
    if !dedicated && path == ":memory:" {
        anyhow::bail!(
            "Database audit backend in shared SQLite mode cannot use :memory: because it cannot be shared \
             across separate backend instances. Set HFS_AUDIT_DATABASE_URL to a file path for a dedicated \
             audit store, or use HFS_AUDIT_BACKEND=file."
        );
    }
    Ok(())
}

struct HfsAuditStorageAdapter {
    storage: Arc<dyn ResourceStorage>,
    tenant: TenantContext,
}

impl HfsAuditStorageAdapter {
    fn new(storage: Arc<dyn ResourceStorage>) -> Self {
        Self {
            storage,
            tenant: TenantContext::system(),
        }
    }
}

#[async_trait::async_trait]
impl helios_audit::sinks::database::AuditStorage for HfsAuditStorageAdapter {
    async fn create_resource(
        &self,
        resource_type: &str,
        resource: serde_json::Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> Result<(), helios_audit::sinks::database::AuditStorageError> {
        use helios_audit::sinks::database::AuditStorageError;
        self.storage
            .create(&self.tenant, resource_type, resource, fhir_version)
            .await
            .map(|_| ())
            .map_err(|e| {
                // Pool exhausted, connection refused, server saturated: the
                // sink retries these before giving the event up (#2234).
                if e.is_unavailable() {
                    AuditStorageError::transient(e.to_string())
                } else {
                    AuditStorageError::permanent(e.to_string())
                }
            })
    }
}

async fn create_database_audit_storage(
    server_config: &ServerConfig,
    backend_mode: StorageBackendMode,
    audit_config: &AuditConfig,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let backend_kind = backend_mode.primary_backend_kind();
    let dedicated = is_database_audit_dedicated(audit_config, backend_kind);

    info!(
        backend_kind = %backend_kind,
        storage_backend = %backend_mode,
        dedicated = dedicated,
        "Initializing database audit sink storage"
    );

    match backend_kind {
        BackendKind::Sqlite => {
            create_audit_sqlite_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::Postgres => {
            create_audit_postgres_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::MongoDB => {
            create_audit_mongodb_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::S3 => create_audit_s3_storage(server_config, audit_config, dedicated).await,
        _ => anyhow::bail!(
            "Database audit sink is unsupported for primary backend kind '{}'",
            backend_kind
        ),
    }
}

#[cfg(feature = "sqlite")]
async fn create_audit_sqlite_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let database_url = if dedicated {
        audit_config.database_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Dedicated SQLite audit mode requires HFS_AUDIT_DATABASE_URL to be set")
        })?
    } else {
        server_config.database_url.as_deref().unwrap_or("fhir.db")
    };

    validate_shared_sqlite_audit_path(database_url, dedicated)?;

    let mut sqlite_config = server_config.clone();
    sqlite_config.database_url = Some(database_url.to_string());
    let backend = create_sqlite_backend(&sqlite_config)?;

    Ok(Arc::new(backend))
}

#[cfg(not(feature = "sqlite"))]
async fn create_audit_sqlite_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with sqlite storage requires the 'sqlite' feature. \
         Build with: cargo build -p helios-hfs --features sqlite"
    )
}

#[cfg(feature = "postgres")]
async fn create_audit_postgres_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    use helios_persistence::backends::postgres::PostgresBackend;

    let backend = if dedicated {
        let url = audit_config.database_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Dedicated PostgreSQL audit mode requires HFS_AUDIT_DATABASE_URL to be set"
            )
        })?;
        if !is_postgres_url(url) {
            anyhow::bail!(
                "HFS_AUDIT_DATABASE_URL must be a PostgreSQL connection string when primary backend is postgres"
            );
        }
        let mut backend_config = PostgresBackend::config_from_connection_string(url)?;
        backend_config.fhir_version = server_config.default_fhir_version;
        backend_config.data_dir = server_config.data_dir.clone();
        PostgresBackend::new(backend_config).await?
    } else {
        create_postgres_backend(server_config).await?
    };

    backend.init_schema_without_patient_export_index().await?;
    // Shared mode still opens a second pool to the primary database, so both
    // branches publish it as its own `audit` pool.
    let backend = Arc::new(backend);
    register_pg_pool_metrics("audit", backend.clone());
    Ok(backend)
}

#[cfg(not(feature = "postgres"))]
async fn create_audit_postgres_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with postgres storage requires the 'postgres' feature. \
         Build with: cargo build -p helios-hfs --features postgres"
    )
}

#[cfg(feature = "mongodb")]
async fn create_audit_mongodb_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let backend = if dedicated {
        let connection_string = match audit_config.database_url.as_deref() {
            Some(url) => {
                if !is_mongodb_url(url) {
                    anyhow::bail!(
                        "HFS_AUDIT_DATABASE_URL must be a MongoDB connection string when primary backend is mongodb"
                    );
                }
                url.to_string()
            }
            None => {
                if let Some(url) = server_config.database_url.as_deref() {
                    if is_mongodb_url(url) {
                        url.to_string()
                    } else {
                        std::env::var("HFS_MONGODB_URL")
                            .or_else(|_| std::env::var("HFS_MONGODB_URI"))
                            .or_else(|_| std::env::var("HFS_DATABASE_URL"))
                            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
                    }
                } else {
                    std::env::var("HFS_MONGODB_URL")
                        .or_else(|_| std::env::var("HFS_MONGODB_URI"))
                        .or_else(|_| std::env::var("HFS_DATABASE_URL"))
                        .unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
                }
            }
        };

        let database_name = audit_config
            .mongodb_database
            .clone()
            .or_else(|| std::env::var("HFS_MONGODB_DATABASE").ok())
            .unwrap_or_else(|| "helios".to_string());

        let max_connections = std::env::var("HFS_MONGODB_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(10);
        let connect_timeout_ms = std::env::var("HFS_MONGODB_CONNECT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5000);
        let server_selection_timeout_ms = std::env::var("HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(15_000);
        let index_build = match std::env::var("HFS_MONGODB_INDEX_BUILD") {
            Ok(raw) => raw
                .parse::<IndexBuildMode>()
                .map_err(|message| anyhow::anyhow!(message))?,
            Err(_) => IndexBuildMode::default(),
        };

        let config = MongoBackendConfig {
            connection_string,
            database_name,
            max_connections,
            connect_timeout_ms,
            server_selection_timeout_ms,
            fhir_version: server_config.default_fhir_version,
            data_dir: server_config.data_dir.clone(),
            search_offloaded: false,
            index_build,
            // Audit storage doesn't resolve _include/_revinclude; the default
            // (and any future new field) is fine here.
            ..Default::default()
        };
        MongoBackend::new(config)?
    } else {
        MongoBackend::new(build_mongodb_config(server_config, false)?)?
    };

    backend.init_schema().await?;
    Ok(Arc::new(backend))
}

#[cfg(not(feature = "mongodb"))]
async fn create_audit_mongodb_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with mongodb storage requires the 'mongodb' feature. \
         Build with: cargo build -p helios-hfs --features mongodb"
    )
}

#[cfg(feature = "s3")]
async fn create_audit_s3_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};

    if audit_config.database_url.is_some() {
        anyhow::bail!(
            "HFS_AUDIT_DATABASE_URL is not supported for S3-backed database audit sink. \
             Use HFS_AUDIT_S3_BUCKET / HFS_AUDIT_S3_PREFIX / HFS_AUDIT_S3_REGION instead."
        );
    }

    let s3_config = if dedicated {
        // A dedicated audit store is its own prefix-per-tenant bucket whatever
        // the primary's tenancy: the AuditEvent trail is one cross-tenant
        // record, written as `__system__`.
        let tenancy = S3TenancyEnv::from_env();
        let bucket = match audit_config.s3_bucket.clone() {
            Some(bucket) => bucket,
            None if tenancy.bucket_per_tenant()? => anyhow::bail!(
                "HFS_AUDIT_S3_BUCKET is required for a dedicated S3 audit store when \
                 HFS_S3_TENANCY=bucket-per-tenant: the primary has no shared bucket to fall \
                 back to"
            ),
            None => tenancy.bucket.clone().unwrap_or_else(|| "hfs".to_string()),
        };
        let region = audit_config
            .s3_region
            .clone()
            .or_else(|| std::env::var("HFS_S3_REGION").ok());
        let prefix = audit_config
            .s3_prefix
            .clone()
            .or_else(|| std::env::var("HFS_S3_PREFIX").ok());
        let validate_buckets = audit_config
            .s3_validate_buckets
            .unwrap_or_else(|| parse_env_bool("HFS_S3_VALIDATE_BUCKETS", true));
        S3BackendConfig {
            tenancy_mode: S3TenancyMode::PrefixPerTenant { bucket },
            region,
            prefix,
            validate_buckets_on_startup: validate_buckets,
            default_keyspace_layout: parse_s3_keyspace_layout(
                std::env::var("HFS_S3_KEYSPACE_LAYOUT").ok().as_deref(),
            )?,
            ..Default::default()
        }
    } else {
        // Shared with the primary: the same configuration the primary is built
        // from, tenancy and endpoint included, so the trail lands in the store
        // the operator is looking at. AuditEvents are written as `__system__`,
        // which bucket-per-tenant keeps in the system bucket — without one the
        // sink would fail every write, so refuse at startup instead.
        let config = s3_backend_config_from_env(&server_config.default_tenant)?;
        if let S3TenancyMode::BucketPerTenant {
            default_system_bucket: None,
            ..
        } = &config.tenancy_mode
        {
            anyhow::bail!(
                "HFS_AUDIT_BACKEND=database on S3 bucket-per-tenant needs a bucket for the \
                 __system__ tenant's AuditEvents: set HFS_S3_SYSTEM_BUCKET, or give the audit \
                 store its own bucket with HFS_AUDIT_S3_BUCKET"
            );
        }
        config
    };

    let tenancy = describe_s3_tenancy(&s3_config.tenancy_mode);
    info!(
        tenancy = %tenancy,
        region = ?s3_config.region,
        prefix = ?s3_config.prefix,
        validate_buckets = s3_config.validate_buckets_on_startup,
        keyspace_layout = ?s3_config.default_keyspace_layout,
        dedicated = dedicated,
        "Initializing S3 storage for database audit sink"
    );

    let backend = S3Backend::from_env_async(s3_config).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize S3 storage for database audit sink ({}, region={:?}): {}",
            tenancy,
            std::env::var("AWS_REGION").ok(),
            e
        )
    })?;

    Ok(Arc::new(
        backend.with_fhir_version(server_config.default_fhir_version),
    ))
}

#[cfg(not(feature = "s3"))]
async fn create_audit_s3_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with s3 storage requires the 's3' feature. \
         Build with: cargo build -p helios-hfs --features s3"
    )
}

/// Creates a PostgreSQL backend from the server configuration.
///
/// Like the SQLite and MongoDB paths, the server's configured default FHIR
/// version and data directory are applied on top of the URL/env-derived
/// connection config, so `HFS_DEFAULT_FHIR_VERSION` reaches the backend.
#[cfg(feature = "postgres")]
async fn create_postgres_backend(
    config: &ServerConfig,
) -> anyhow::Result<helios_persistence::backends::postgres::PostgresBackend> {
    use helios_persistence::backends::postgres::PostgresBackend;

    let mut backend_config = if let Some(ref url) = config.database_url {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            let backend_config = PostgresBackend::config_from_connection_string(url)?;
            // Log the server the parsed config will contact, which can differ
            // from the driver's reading of `url` (#2142).
            info!(
                url = %redact_postgres_location(
                    url,
                    &backend_config.host,
                    backend_config.port,
                    &backend_config.dbname
                ),
                "Initializing PostgreSQL backend from connection string"
            );
            backend_config
        } else {
            info!("Initializing PostgreSQL backend from environment variables");
            PostgresBackend::config_from_env()
        }
    } else {
        info!("Initializing PostgreSQL backend from environment variables");
        PostgresBackend::config_from_env()
    };
    backend_config.fhir_version = config.default_fhir_version;
    backend_config.data_dir = config.data_dir.clone();
    if let Some(message) = count_budget_race_warning(&backend_config, config.request_timeout) {
        warn!("{message}");
    }

    Ok(PostgresBackend::new(backend_config).await?)
}

/// Maps a PostgreSQL pool snapshot onto the `/metrics` pool gauges (#1907).
#[cfg(feature = "postgres")]
fn pg_pool_gauges(
    status: helios_persistence::backends::postgres::PoolStatus,
) -> helios_observability::pool_metrics::PoolGauges {
    helios_observability::pool_metrics::PoolGauges {
        max_size: status.max_size,
        size: status.size,
        available: status.available,
        waiting: status.waiting,
    }
}

/// Publishes `backend`'s connection pool on `/metrics` as the `postgres_pool_*`
/// gauges labelled `pool="<name>"`, sampled at every scrape (#1907). `name`
/// comes from a fixed set: `primary` for the storage backend, `audit` for the
/// database audit sink's own pool. The registration keeps the backend alive
/// for the life of the process.
#[cfg(feature = "postgres")]
fn register_pg_pool_metrics(
    name: &'static str,
    backend: Arc<helios_persistence::backends::postgres::PostgresBackend>,
) {
    helios_observability::pool_metrics::register_postgres_pool(name, move || {
        pg_pool_gauges(backend.pool_status())
    });
}

/// The startup warning for a PostgreSQL `count_by_tenant` budget that does
/// not end before the HTTP request timeout (#1911), or `None` when it does.
///
/// The request timeout starts when the request arrives, before the count, so
/// a budget at or above it means a slow count on `GET /admin/tenants` or the
/// console answers the request timeout's generic `408` instead of the `504`
/// the budget would give. That is legal configuration, so it is a warning,
/// not a startup failure.
///
/// It compares the budget alone. The pool checkout before the count
/// (`HFS_PG_POOL_WAIT_TIMEOUT_SECS`) and the handler's earlier work also
/// spend the request timeout, so a saturated pool can still turn a slow count
/// into `408`. Adding the worst-case pool wait would warn at the defaults
/// (25 s + 10 s against 30 s) on every start, so that case is documented on
/// the setting instead.
#[cfg(feature = "postgres")]
fn count_budget_race_warning(
    backend_config: &helios_persistence::backends::postgres::PostgresConfig,
    request_timeout_secs: u64,
) -> Option<String> {
    let budget_ms = backend_config.effective_count_by_tenant_statement_timeout_ms();
    let request_timeout_ms = request_timeout_secs.saturating_mul(1_000);
    (budget_ms >= request_timeout_ms).then(|| {
        format!(
            "PostgreSQL count_by_tenant budget ({budget_ms} ms, the smaller of \
             HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS and HFS_PG_STATEMENT_TIMEOUT_MS) \
             is not below HFS_REQUEST_TIMEOUT ({request_timeout_ms} ms): a slow tenant \
             count on /admin/tenants or the console will answer 408 instead of 504"
        )
    })
}

/// Creates and initializes a SQLite backend from the server configuration.
#[cfg(feature = "sqlite")]
fn create_sqlite_backend(config: &ServerConfig) -> anyhow::Result<SqliteBackend> {
    let db_path = config.database_url.as_deref().unwrap_or("fhir.db");
    info!(database = %db_path, "Initializing SQLite backend");

    let backend_config = SqliteBackendConfig {
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        ..Default::default()
    };

    let backend = if db_path == ":memory:" {
        SqliteBackend::with_config(":memory:", backend_config)?
    } else {
        SqliteBackend::with_config(db_path, backend_config)?
    };
    backend.init_schema()?;

    Ok(backend)
}

/// Starts the server with MongoDB backend.
#[cfg(feature = "mongodb")]
async fn start_mongodb(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let backend_config = build_mongodb_config(&config, false)?;
    info!(
        url = %redact_database_url(&backend_config.connection_string, "mongodb"),
        database = %backend_config.database_name,
        "Initializing MongoDB backend"
    );
    let backend = MongoBackend::new(backend_config)?;
    info!(
        deferred_index_min_entries = ?backend.deferred_search_index_min_entries(),
        "MongoDB transaction Bundles of at least this many entries write their search rows after the commit (None: never)"
    );

    backend.init_schema().await?;
    let backend = Arc::new(backend);
    attach_login_sessions(auth_state.as_ref(), backend.clone());
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*backend, &config, Some(observability.observers.as_ref())).await;
    spawn_mongodb_search_param_refresh(backend.clone(), &config);
    spawn_mongodb_deferred_index_repair(
        backend.clone(),
        deferred_index_repair_min_age(config.request_timeout),
    );
    let serve_audit_state = audit_state.clone();

    // MongoDB is a full standalone primary, so it also hosts the per-user
    // settings store: it keeps ownership of the backend Arc and wires the
    // settings-capable builder (like the SQLite/Postgres backends).
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(backend.clone());

    // MongoDB primary; embedded SQLite sidecar for bulk-export job state.
    let export_bundle = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            build_bulk_export(&config, backend.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };
    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    // Bulk submit needs no sidecar: MongoDB hosts the submission, manifest,
    // lease, and artifact state itself, in the same store the ingestion engine
    // writes resources to.
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, None));
    let submit_bundle = build_bulk_submit(
        &config,
        backend.clone(),
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own).
    serve(
        app,
        &config,
        serve_audit_state,
        Some(backend),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when mongodb feature is not enabled.
#[cfg(not(feature = "mongodb"))]
async fn start_mongodb(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The mongodb backend requires the 'mongodb' feature. \
         Build with: cargo build -p helios-hfs --features mongodb"
    )
}

/// Starts the Axum HTTP server.
async fn serve(
    app: axum::Router,
    config: &ServerConfig,
    audit_state: Option<Arc<AuditMiddlewareState>>,
    ui_tenants: Option<Arc<dyn ResourceStorage>>,
    ui_settings: Option<Arc<dyn SettingsStore>>,
    ui_bulk_provider: Option<Arc<dyn BulkProviderStore>>,
    write_observer: Option<Arc<dyn WriteObserver>>,
) -> anyhow::Result<()> {
    let app = attach_ui(
        app,
        config,
        ui_tenants,
        ui_settings,
        ui_bulk_provider,
        write_observer,
    );

    let addr = config.socket_addr();
    info!(address = %addr, "Server listening");

    // Observability: expose `/metrics` (outside the auth layer, so scrapers
    // need no token) and instrument every request with metrics + a trace span.
    let app = app
        .merge(helios_observability::metrics::router())
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Peer address in request extensions: the natural-language search rate
    // limiter falls back to it when auth is disabled and there is no principal
    // to bill a request to.
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let signal = helios_observability::shutdown::signal().await;
        info!(signal, "Shutdown signal received, draining connections");
        // The bulk workers stop now, in parallel with the HTTP drain: they
        // stop claiming and wind their jobs down to a point where the lease
        // can be handed back (#1531).
        worker_shutdown::pools().begin_shutdown();
    })
    .await;

    // Wait for the workers to release their leases before the final audit
    // and trace flush, so what they log on the way out is flushed too. On a
    // serve error too: the workers are still holding leases then.
    let pools = worker_shutdown::pools();
    let timeout = worker_shutdown::drain_timeout();
    let running = pools.running();
    if pools.drain(timeout).await {
        if running > 0 {
            info!(workers = running, "Bulk workers stopped");
        }
    } else {
        warn!(
            still_running = pools.running(),
            timeout_secs = timeout.as_secs(),
            "Bulk workers did not stop in time (HFS_WORKER_SHUTDOWN_TIMEOUT); their leases \
             will lapse and the jobs resume on another instance after the lease duration"
        );
    }

    // Flush only once the drain is over. axum awaits the shutdown future above
    // before it stops accepting or winds down a single connection, so a flush
    // inside it ran while requests were still in flight and lost every audit
    // event and span they produced. Flush on a serve error too.
    if let Some(state) = audit_state {
        lifecycle::record_shutdown(&*state.sink, &state.config.source_observer).await;
        state.sink.flush().await;
    }
    // Flush any buffered OTLP spans (no-op without the `otel` feature).
    helios_observability::telemetry::shutdown();
    served?;
    Ok(())
}

/// Whether this binary can serve the requested web UI and its dashboard.
fn effective_ui_enabled(requested: bool) -> bool {
    requested && cfg!(feature = "ui")
}

/// Attaches the web UI surface to the FHIR router.
///
/// The UI is served when it is compiled in (the `ui` feature) **and** enabled at
/// runtime (`HFS_UI_ENABLED`, default `true`). Otherwise `/ui` answers `404`
/// with an OperationOutcome rather than falling through to the FHIR router,
/// which reads `ui` as a resource type and replies `200` with an empty
/// searchset — a missing UI must not present as success.
///
/// Headless operation is a *runtime* switch on purpose. It used to be a Cargo
/// feature gated negatively (`not(feature = "headless")`). Because
/// `--all-features` — the selection `ci.yml` uses to build the uploaded release
/// artifacts — turns every feature on, it turned the UI off in every published
/// binary, silently (#975). Negative Cargo features cannot express mutual
/// exclusion; don't reintroduce one here.
fn attach_ui(
    app: axum::Router,
    config: &ServerConfig,
    ui_tenants: Option<Arc<dyn ResourceStorage>>,
    ui_settings: Option<Arc<dyn SettingsStore>>,
    ui_bulk_provider: Option<Arc<dyn BulkProviderStore>>,
    write_observer: Option<Arc<dyn WriteObserver>>,
) -> axum::Router {
    #[cfg(feature = "ui")]
    {
        if config.ui_enabled {
            return mount_ui(
                app,
                config,
                ui_tenants,
                ui_settings,
                ui_bulk_provider,
                write_observer,
            );
        }
        info!("Web UI is DISABLED (HFS_UI_ENABLED=false)");
    }
    #[cfg(not(feature = "ui"))]
    {
        let _ = (
            config,
            &ui_tenants,
            &ui_settings,
            &ui_bulk_provider,
            &write_observer,
        );
    }
    ui_absent_routes(app)
}

/// Answers `/ui` with `404` + OperationOutcome when the UI is not served.
///
/// Without this the path falls through to the FHIR router, which treats `ui` as
/// an unknown resource type and returns `200` with an empty searchset — the
/// reason the #975 regression looked like a healthy server.
fn ui_absent_routes(app: axum::Router) -> axum::Router {
    async fn not_found() -> impl axum::response::IntoResponse {
        (
            axum::http::StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/fhir+json; charset=utf-8",
            )],
            axum::Json(serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "not-found",
                    "diagnostics": "The web UI is not available on this server: it is \
            either not compiled in (the `ui` build feature is off) or disabled at runtime \
            (HFS_UI_ENABLED=false)."
                }]
            })),
        )
    }

    app.route("/ui", axum::routing::any(not_found))
        .route("/ui/", axum::routing::any(not_found))
}

/// Builds the UI router and merges it onto the FHIR app.
#[cfg(feature = "ui")]
fn mount_ui(
    app: axum::Router,
    config: &ServerConfig,
    ui_tenants: Option<Arc<dyn ResourceStorage>>,
    ui_settings: Option<Arc<dyn SettingsStore>>,
    ui_bulk_provider: Option<Arc<dyn BulkProviderStore>>,
    write_observer: Option<Arc<dyn WriteObserver>>,
) -> axum::Router {
    // The UI reads SearchParameter/CompartmentDefinition from the server's
    // own FHIR API over HTTP. It calls itself on the loopback address, with
    // the configured outbound service token (HFS_OUTBOUND_BEARER_TOKEN) when
    // set, or no credentials when auth is disabled.
    //
    // TODO(service-token): when auth is enabled, this relies on an operator
    // provisioning a valid, non-expiring bearer via HFS_OUTBOUND_BEARER_TOKEN;
    // without one the self-call is rejected and the conformance pages degrade
    // to a warning. The follow-up is to mint a short-lived, auto-refreshed
    // `system/SearchParameter.rs system/CompartmentDefinition.rs` token via
    // the planned `JwtAssertionOutboundAuthProvider` (SMART Backend Services
    // client_credentials + private_key_jwt; see crates/auth/src/outbound.rs)
    // configured from HFS_UI_* client credentials. The `$sql-export`
    // self-calls (#833) are the exception: they already carry the
    // browser's own `Authorization` when it sent one (the `Caller` seam
    // in `crates/ui/src/conformance.rs`), falling back to this service
    // token only when the request had none.
    let self_base_url = format!("http://127.0.0.1:{}", config.port);
    let outbound_auth = AuthConfig::from_env().outbound_provider();
    let patient_name_search = patient_name_search_support(
        config
            .storage_backend_mode()
            .expect("storage backend was validated before server startup"),
    );
    // The Home dashboard's refresh cadences are process-wide, like its
    // provider: installed here, right before the UI is mounted (#1078).
    helios_ui::set_dashboard_refresh(helios_ui::DashboardRefresh {
        moving_secs: u32::try_from(config.dashboard_refresh_secs).unwrap_or(u32::MAX),
        settled_secs: u32::try_from(config.dashboard_idle_refresh_secs).unwrap_or(u32::MAX),
    });
    helios_ui::mount_with_body_limit_and_tenant_routing(
        app,
        env!("CARGO_PKG_VERSION"),
        config.data_dir.clone(),
        helios_ui::NlSearch {
            enabled: config.nl_search_enabled,
            configured: config.nl_search_api_key.is_some(),
            model: config.nl_search_model.clone(),
        },
        ui_tenants,
        ui_settings,
        config.default_tenant.clone(),
        self_base_url,
        outbound_auth,
        config.default_fhir_version,
        config.terminology_server.clone(),
        config.base_url.clone(),
        config.max_body_size,
        config.multitenancy.routing_mode.supports_url_path(),
        ui_bulk_provider,
        patient_name_search,
        write_observer,
    )
}

#[cfg(feature = "ui")]
fn patient_name_search_support(mode: StorageBackendMode) -> helios_ui::PatientNameSearchSupport {
    match mode {
        StorageBackendMode::S3 => helios_ui::PatientNameSearchSupport::IdOnly,
        StorageBackendMode::Sqlite
        | StorageBackendMode::SqliteElasticsearch
        | StorageBackendMode::Postgres
        | StorageBackendMode::PostgresElasticsearch
        | StorageBackendMode::MongoDB
        | StorageBackendMode::MongoDBElasticsearch
        | StorageBackendMode::S3Elasticsearch => helios_ui::PatientNameSearchSupport::Enabled,
    }
}

/// Initializes the authentication subsystem from environment configuration.
///
/// When audit is enabled, the auth middleware uses an `AuditBridge` to record
/// authentication events. Otherwise, a no-op sink is used.
///
/// Returns the auth config and optional middleware state. If auth is disabled,
/// returns `(config, None)`.
async fn init_auth_with_audit(
    audit_sink: Arc<dyn AuditSink>,
    tenant_url_routing: bool,
    public_base_url: &str,
) -> anyhow::Result<(AuthConfig, Option<Arc<AuthMiddlewareState>>)> {
    let auth_config = AuthConfig::from_env();

    if !auth_config.enabled {
        info!("Authentication is DISABLED");
        return Ok((auth_config, None));
    }

    // Every invariant of an enabled auth config now lives on the type, so an
    // embedder that builds one directly gets the same guarantees this binary
    // does. Issuer validation in particular is required, both to prevent
    // cross-service token reuse and because `iss` qualifies every per-user
    // identity (see `helios_rest::extractors::UserKey`).
    if let Err(errors) = auth_config.validate() {
        anyhow::bail!("Invalid auth configuration:\n  - {}", errors.join("\n  - "));
    }

    let jwks_url = auth_config
        .jwks_url
        .as_ref()
        .expect("validate() guarantees a JWKS URL when auth is enabled");

    // Audience stays optional so an open demo deployment can accept any token
    // from its issuer, but that also means a token minted for a *different*
    // client of the same issuer is accepted here. Make it noisy rather than silent.
    if auth_config.expected_audience.is_none() {
        warn!(
            "HFS_AUTH_AUDIENCE is not set: every token from this issuer will be accepted, \
             including tokens minted for other clients of the same issuer. \
             Set HFS_AUTH_AUDIENCE to restrict tokens to this server."
        );
    }

    // Create JWKS cache
    let jwks_cache = Arc::new(JwksCache::new(
        jwks_url,
        auth_config.jwks_min_refresh_interval,
    ));
    jwks_cache.initial_fetch().await?;

    // Create auth provider
    let provider = JwksBearerAuthProvider::new(jwks_cache, &auth_config);

    info!(
        jwks_url = %redact_url(jwks_url),
        issuer = ?auth_config.expected_issuer,
        audience = ?auth_config.expected_audience,
        "Authentication ENABLED"
    );

    let audit_config = AuditConfig::from_env();

    // The web UI's browser login (#1449) shares one session store between the
    // REST auth layer (which turns the session cookie into a bearer) and the
    // UI (which establishes sessions). Built here, before either is mounted.
    let sessions = init_login_sessions(&auth_config, public_base_url).await?;
    #[cfg(feature = "ui")]
    if let Some(sessions) = &sessions {
        helios_ui::set_interactive_login(helios_ui::LoginRuntime {
            sessions: Arc::clone(sessions),
        });
    }
    #[cfg(feature = "ui")]
    helios_ui::set_bearer_only_auth(sessions.is_none());

    let auth_state = Arc::new(AuthMiddlewareState {
        provider: Arc::new(provider),
        config: Arc::new(auth_config.clone()),
        audit_sink,
        audit_source_observer: audit_config.source_observer.clone(),
        audit_exclusion_filter: ExclusionFilter::new(audit_config.exclusions.clone()),
        tenant_url_routing,
        sessions,
    });

    Ok((auth_config, Some(auth_state)))
}

/// Builds the interactive-login session store when the web UI's IdP client is
/// configured (`HFS_UI_LOGIN_CLIENT_ID`), resolving the authorize/token/
/// end-session endpoints from explicit `HFS_SMART_*` settings or, failing
/// those, from the issuer's OpenID Connect discovery document. `None` when no
/// client is configured — auth stays bearer-only and the UI has no login.
async fn init_login_sessions(
    auth_config: &AuthConfig,
    public_base_url: &str,
) -> anyhow::Result<Option<Arc<helios_auth::SessionStore>>> {
    let Some(client_id) = auth_config.web_client_id.clone() else {
        return Ok(None);
    };
    let issuer = auth_config
        .expected_issuer
        .as_deref()
        .expect("validate() guarantees an issuer when auth is enabled");
    let (authorization_endpoint, token_endpoint, end_session_endpoint) =
        helios_auth::discover_endpoints(
            issuer,
            auth_config.smart_authorize_endpoint.clone(),
            auth_config.smart_token_endpoint.clone(),
            auth_config.smart_end_session_endpoint.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("interactive login could not be configured: {e}"))?;
    let redirect_uri = auth_config
        .web_redirect_uri
        .clone()
        .unwrap_or_else(|| format!("{}/ui/callback", public_base_url.trim_end_matches('/')));
    info!(
        client_id = %client_id,
        redirect_uri = %redirect_uri,
        authorization_endpoint = %redact_url(&authorization_endpoint),
        end_session = ?end_session_endpoint.as_deref().map(redact_url),
        cookie_secure = auth_config.web_cookie_secure,
        "Interactive browser login ENABLED"
    );
    if !auth_config.web_cookie_secure {
        warn!(
            "HFS_UI_LOGIN_COOKIE_SECURE=false: the session cookie is sent over plain HTTP. \
             Only for local development."
        );
    }
    Ok(Some(Arc::new(helios_auth::SessionStore::new(
        helios_auth::LoginConfig {
            client_id,
            client_secret: auth_config.web_client_secret.clone(),
            redirect_uri,
            scopes: auth_config.web_scopes.clone(),
            authorization_endpoint,
            token_endpoint,
            end_session_endpoint,
            cookie_secure: auth_config.web_cookie_secure,
        },
    ))))
}

/// Gives the web UI's login sessions a home in the primary store (#1481), so
/// a session established on one node resolves on every other and outlives a
/// restart. Called from a backend's `start_*` once its Arc exists: the store
/// itself is built with the auth state, before any backend is. Nothing to
/// attach when interactive login is off.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
fn attach_login_sessions(
    auth_state: Option<&Arc<AuthMiddlewareState>>,
    persistence: Arc<dyn helios_auth::SessionPersistence>,
) {
    if let Some(sessions) = auth_state.and_then(|state| state.sessions.as_ref()) {
        sessions.attach_persistence(persistence);
    }
}

/// A deployment with nowhere tenant-independent to keep them (S3
/// bucket-per-tenant with no system bucket — the same case that leaves
/// `/_user/settings` unwired) keeps login sessions in process: fine on one
/// node, but a cluster needs sticky sessions and a restart signs everyone
/// out. Said once at startup so an operator knows which mode they are in.
/// Reached from this binary's environment by `HFS_S3_TENANCY=bucket-per-tenant`
/// with no `HFS_S3_SYSTEM_BUCKET` (#1598; before that, #1514).
#[cfg(feature = "s3")]
fn warn_login_sessions_in_process(auth_state: Option<&Arc<AuthMiddlewareState>>) {
    if auth_state.is_some_and(|state| state.sessions.is_some()) {
        warn!(
            "web login sessions are held in process on this storage backend: a session is \
             not shared across nodes and does not survive a restart"
        );
    }
}

/// Initializes the audit subsystem from environment configuration.
///
/// Returns the audit sink (for use as auth bridge) and optional middleware state.
/// If audit is disabled, returns `(NullSink, None)`.
async fn init_audit(
    server_config: &ServerConfig,
    backend_mode: StorageBackendMode,
) -> anyhow::Result<(Arc<dyn AuditSink>, Option<Arc<AuditMiddlewareState>>)> {
    let config = AuditConfig::from_env();

    let sink: Arc<dyn AuditSink> = match config.backend {
        AuditBackend::None => {
            info!("Audit logging is DISABLED");
            Arc::new(helios_audit::NullSink)
        }
        AuditBackend::File => {
            let path = config.file_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!("HFS_AUDIT_FILE_PATH is required when HFS_AUDIT_BACKEND=file")
            })?;
            info!(path = %path, "Audit logging to file");
            Arc::new(helios_audit::FileSink::new(path).await?)
        }
        AuditBackend::Database => {
            let storage =
                create_database_audit_storage(server_config, backend_mode, &config).await?;
            let adapter = Arc::new(HfsAuditStorageAdapter::new(storage));
            let retry = helios_audit::sinks::database::RetryPolicy {
                max_retries: config.write_retries,
                ..Default::default()
            };
            Arc::new(
                helios_audit::DatabaseSink::new(adapter, server_config.default_fhir_version)
                    .with_retry(retry),
            )
        }
        #[cfg(feature = "cloudwatch")]
        AuditBackend::CloudWatch => {
            let log_group = config.cloudwatch_log_group.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "HFS_AUDIT_CLOUDWATCH_LOG_GROUP is required when HFS_AUDIT_BACKEND=cloudwatch"
                )
            })?;
            let log_stream = config
                .cloudwatch_log_stream
                .as_deref()
                .unwrap_or("hfs-audit");
            info!(log_group = %log_group, log_stream = %log_stream, "Audit logging to CloudWatch Logs");
            Arc::new(
                helios_audit::CloudWatchLogsSink::new(
                    log_group.to_string(),
                    log_stream.to_string(),
                    config.cloudwatch_region.clone(),
                )
                .await,
            )
        }
        #[cfg(not(feature = "cloudwatch"))]
        AuditBackend::CloudWatch => {
            anyhow::bail!(
                "CloudWatch audit backend requires the `cloudwatch` feature. \
                 Build with: cargo build --features cloudwatch"
            );
        }
    };

    // Every real sink sits behind a bounded queue drained by a fixed pool of
    // writers, so audit writes cannot stampede the store under load (#2234).
    let sink: Arc<dyn AuditSink> = if config.backend == AuditBackend::None {
        sink
    } else {
        info!(
            capacity = config.queue.capacity,
            writers = config.queue.writers,
            overflow = %config.queue.overflow,
            "Audit events are queued for background writers"
        );
        Arc::new(helios_audit::QueuedSink::new(sink, config.queue))
    };

    let audit_state = if config.backend != AuditBackend::None {
        Some(Arc::new(AuditMiddlewareState {
            sink: Arc::clone(&sink),
            config: config.clone(),
            exclusion_filter: ExclusionFilter::default_exclusions(),
        }))
    } else {
        None
    };

    Ok((sink, audit_state))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Keep the skipped sub-configs populated while surfacing invalid CLI and
    // environment values instead of silently replacing the whole config with
    // defaults.
    let (mut config, default_fhir_version_is_implicit) =
        ServerConfig::try_from_env_with_provenance().unwrap_or_else(|error| error.exit());
    // The REST builder uses this same flag to start the dashboard's background
    // work. A binary without the UI must not seed or reconcile unseen figures.
    config.ui_enabled = effective_ui_enabled(config.ui_enabled);
    helios_observability::uptime::init();
    helios_observability::telemetry::init("hfs", &config.log_level);
    helios_observability::metrics::init("hfs");
    // hfs is the one server that mounts the console traffic/tenants endpoints
    // backed by the reqlog ring buffer, so it opts into recording. Servers that
    // don't (hts, sof-server, fhirpath-server) leave it off and skip the cost.
    helios_observability::reqlog::enable();

    if let Some(message) = config.loopback_public_base_warning() {
        warn!(
            public_base_url = %config.base_url,
            bind_address = %config.socket_addr(),
            "{message}. Set HFS_BASE_URL to the public HTTP(S) origin clients can reach"
        );
    }

    // A build without R4 that enables several versions has no default FHIR
    // version to fall back to (#2064): every R5 or R6 build also enables R4B,
    // and serving R4B because nobody chose a version is hard to undo. Refuse to
    // start, in the same form and with the same exit code as the
    // `config.validate()` failure below.
    if let Some(message) =
        config.implicit_default_fhir_version_error(default_fhir_version_is_implicit)
    {
        eprintln!("Configuration error: {}", message);
        std::process::exit(1);
    }

    if let Err(errors) = config.validate() {
        for error in &errors {
            eprintln!("Configuration error: {}", error);
        }
        std::process::exit(1);
    }

    let backend_mode = config
        .storage_backend_mode()
        .map_err(|e| anyhow::anyhow!("Invalid storage backend configuration: {}", e))?;

    // Propagate HFS_TERMINOLOGY_SERVER to FHIRPATH_TERMINOLOGY_SERVER so that any
    // FHIRPath evaluation (CDS Hooks, _filter, etc.) delegates terminology
    // operations (memberOf, subsumes) to the configured HTS instance.
    if let Some(ref ts_url) = config.terminology_server {
        if std::env::var("FHIRPATH_TERMINOLOGY_SERVER").is_err() {
            // Safety: single-threaded at this point (before tokio runtime hands
            // off to worker threads), so set_var is safe here.
            // SAFETY: called before any threads are spawned by the tokio runtime.
            unsafe {
                std::env::set_var("FHIRPATH_TERMINOLOGY_SERVER", ts_url);
            }
            info!(
                url = %redact_url(ts_url),
                "HFS_TERMINOLOGY_SERVER wired to FHIRPath context"
            );
        }
    }

    // Initialize audit subsystem
    let (audit_sink, audit_state) = init_audit(&config, backend_mode).await?;

    // Initialize authentication (with audit bridge if audit is enabled)
    let (auth_config, auth_state) = init_auth_with_audit(
        audit_sink,
        config.multitenancy.routing_mode.supports_url_path(),
        &config.base_url,
    )
    .await?;

    let audit_config = AuditConfig::from_env();

    info!(
        port = config.port,
        host = %config.host,
        bind_address = %config.socket_addr(),
        public_base_url = %config.base_url,
        fhir_version = ?config.default_fhir_version,
        storage_backend = %backend_mode,
        terminology_server = ?config.terminology_server.as_deref().map(redact_url),
        auth_enabled = auth_config.enabled,
        audit_backend = %audit_config.backend,
        "Starting Helios FHIR Server"
    );

    // Record startup audit event with server configuration
    if let Some(ref state) = audit_state {
        lifecycle::record_startup(
            &*state.sink,
            &state.config.source_observer,
            vec![
                ("storage-backend", backend_mode.to_string()),
                ("fhir-version", format!("{:?}", config.default_fhir_version)),
                ("auth-enabled", auth_config.enabled.to_string()),
                ("audit-backend", audit_config.backend.to_string()),
            ],
        )
        .await;
    }

    match backend_mode {
        StorageBackendMode::Sqlite => {
            start_sqlite(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::SqliteElasticsearch => {
            start_sqlite_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::Postgres => {
            start_postgres(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::PostgresElasticsearch => {
            start_postgres_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::MongoDB => {
            start_mongodb(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::MongoDBElasticsearch => {
            start_mongodb_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::S3 => {
            start_s3(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::S3Elasticsearch => {
            start_s3_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
    }

    Ok(())
}

/// Seeds storage with the spec SearchParameters (#235) and CompartmentDefinitions
/// (#237/#238), making primary storage the source of truth the FHIR routes and
/// web UI read. Seeds every provisioned tenant — auto-provisioning the default
/// tenant first — so `GET /SearchParameter` and `GET /CompartmentDefinition` are
/// populated for each valid tenant. A failed seed logs and boots anyway: the
/// in-memory registry still resolves searches; only API discovery is degraded.
///
/// Standalone backends seed themselves; Elasticsearch composites seed through
/// the composite so the writes also reach the search index. Standalone S3 has
/// no search index, but it lists these two types by scan (#1822), so it is
/// seeded like the rest (#1838).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch",
    feature = "s3"
))]
async fn seed_conformance_resources<S>(
    backend: &S,
    config: &ServerConfig,
    observer: Option<&dyn WriteObserver>,
) where
    S: helios_persistence::core::ResourceStorage,
{
    if !config.seed_conformance {
        return;
    }

    let data_dir = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));

    for tenant_id in provisioned_tenants(backend, config).await {
        helios_persistence::search::seed_tenant_conformance(
            backend,
            config.default_fhir_version,
            &data_dir,
            &tenant_id,
            observer,
        )
        .await;
    }
}

/// The set of tenants to seed: the auto-provisioned default tenant plus every
/// registered tenant. Tenants are provisioned-only, so this is the complete set
/// of valid tenants. Falls back to just the default tenant when the backend has
/// no tenant registry (e.g. a minimal deployment).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch",
    feature = "s3"
))]
async fn provisioned_tenants<S>(backend: &S, config: &ServerConfig) -> Vec<String>
where
    S: helios_persistence::core::ResourceStorage,
{
    let default = config.default_tenant.clone();
    if !backend.supports_tenant_registry() {
        return vec![default];
    }

    // Auto-provision the default tenant so it is a valid, enumerable tenant
    // (single-tenant and unauthenticated deployments read it).
    match backend.get_tenant(&default).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = backend.register_tenant(&default, None).await {
                tracing::warn!(tenant = %default, "Auto-provisioning default tenant failed: {e}");
            }
        }
        Err(e) => tracing::warn!(tenant = %default, "Checking default tenant failed: {e}"),
    }

    let mut ids: Vec<String> = match backend.list_tenants().await {
        Ok(records) => records.into_iter().map(|r| r.id).collect(),
        Err(e) => {
            tracing::warn!("Listing tenants for seeding failed: {e}");
            Vec::new()
        }
    };
    if !ids.iter().any(|id| id == &default) {
        ids.push(default);
    }
    ids
}

/// Keeps the SQLite WAL bounded on every write path, not only `$bulk-submit`
/// (#2025). Every 2 s the -wal file's size is read (a `stat`, no pooled
/// connection); past the threshold it is folded back into the database and
/// truncated. Under 20 concurrent transaction Bundles the WAL grows by
/// 200–250 MB/s, so a slower check lets it reach gigabytes between passes. A
/// pass that finds readers or writers in the way gives up after a second and
/// the next one retries.
#[cfg(feature = "sqlite")]
fn spawn_sqlite_wal_maintenance(backend: Arc<SqliteBackend>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) = backend.maintain_wal().await {
                tracing::warn!("SQLite WAL maintenance failed; the next pass retries: {e}");
            }
        }
    });
}

/// Folds the SQLite WAL back and truncates it after the server drained, so a
/// restart does not inherit it (#2025).
#[cfg(feature = "sqlite")]
async fn checkpoint_sqlite_wal_at_shutdown(backend: &SqliteBackend) {
    if let Err(e) = backend.checkpoint_wal_at_shutdown().await {
        warn!("SQLite WAL checkpoint at shutdown failed: {e}");
    }
}

/// Spawns the periodic registry refresh from storage for the SQLite backend
/// (#235). `HFS_SEARCH_PARAM_CACHE_TTL=0` disables it. A failed pass keeps
/// serving the stale cache; the next tick retries.
#[cfg(feature = "sqlite")]
fn spawn_sqlite_search_param_refresh(backend: Arc<SqliteBackend>, config: &ServerConfig) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let refresh = backend.clone();
            match tokio::task::spawn_blocking(move || refresh.refresh_stored_search_parameters())
                .await
            {
                Ok(Ok(stored)) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Ok(Err(e)) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
                Err(e) => tracing::warn!("SearchParameter registry refresh task failed: {e}"),
            }
        }
    });
}

/// Postgres flavor of the periodic registry refresh (#235).
#[cfg(feature = "postgres")]
fn spawn_postgres_search_param_refresh(
    backend: Arc<helios_persistence::backends::postgres::PostgresBackend>,
    config: &ServerConfig,
) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match backend.refresh_stored_search_parameters().await {
                Ok(stored) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Err(e) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
            }
        }
    });
}

/// MongoDB flavor of the periodic registry refresh (#235).
#[cfg(feature = "mongodb")]
fn spawn_mongodb_search_param_refresh(backend: Arc<MongoBackend>, config: &ServerConfig) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match backend.refresh_stored_search_parameters().await {
                Ok(stored) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Err(e) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
            }
        }
    });
}

/// The youngest ledger document a deferred-index repair pass takes (#1975): at
/// least a minute, and at least the Bundle deadline
/// ([`bundle_transaction_deadline`]).
///
/// The request that wrote a ledger document starts no post-commit batch after
/// its Bundle deadline, counted from its call, and the call precedes the
/// document's `not_before`; an attempt still running at the deadline is cut. A
/// document older than the deadline therefore has no live writer, and one
/// instance's repair never duplicates a post-commit step that is still running.
/// (A pass that did race one would only repeat work: the rewrite is gated on a
/// per-resource marker.) The lag after a crash or a deadline cut is this age
/// plus up to one repair interval.
#[cfg(feature = "mongodb")]
fn deferred_index_repair_min_age(request_timeout_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(60).max(bundle_transaction_deadline(request_timeout_secs))
}

/// Ledger documents (up to 5,000 resources each) one repair pass takes, so a
/// pass over a large backlog stays bounded and the rest waits for the next.
#[cfg(feature = "mongodb")]
const DEFERRED_INDEX_REPAIR_CHUNKS_PER_PASS: usize = 20;

/// Finishes the search rows of large transaction Bundles that were committed
/// without them (#1975). Each pass re-runs the post-commit rewrite for ledger
/// documents at least `min_age` ([`deferred_index_repair_min_age`]) old, up to
/// [`DEFERRED_INDEX_REPAIR_CHUNKS_PER_PASS`] of them, so rows left by a crash,
/// a cancelled request or the Bundle deadline are picked up. Runs right away
/// (startup) and then every `HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL`
/// seconds (default 60; `0` disables the task, the ledger is still written).
///
/// Only spawned for the native MongoDB backend: with Elasticsearch serving
/// search nothing is deferred, so there is nothing to repair.
#[cfg(feature = "mongodb")]
fn spawn_mongodb_deferred_index_repair(backend: Arc<MongoBackend>, min_age: std::time::Duration) {
    let interval = env_u64("HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL", 60);
    if interval == 0 {
        info!(
            "HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL=0: repair of deferred MongoDB search rows is off; large transaction Bundles still write their ledger"
        );
        return;
    }
    info!(
        min_age_secs = min_age.as_secs(),
        interval_secs = interval,
        "Repair of deferred MongoDB search rows is on"
    );
    let interval = std::time::Duration::from_secs(interval);
    tokio::spawn(async move {
        let mut first_pass = true;
        loop {
            let started = std::time::Instant::now();
            match backend
                .repair_deferred_search_index(min_age, DEFERRED_INDEX_REPAIR_CHUNKS_PER_PASS)
                .await
            {
                Ok(report) if report.examined > 0 => {
                    let pass_ms = started.elapsed().as_millis() as u64;
                    if report.failed > 0 {
                        warn!(
                            examined = report.examined,
                            skipped = report.skipped,
                            resources = report.resources,
                            reindexed = report.reindexed,
                            rows = report.rows,
                            cleared = report.cleared,
                            failed = report.failed,
                            remaining = report.remaining,
                            pass_ms,
                            "Deferred search index repair pass finished"
                        );
                    } else {
                        info!(
                            examined = report.examined,
                            skipped = report.skipped,
                            resources = report.resources,
                            reindexed = report.reindexed,
                            rows = report.rows,
                            cleared = report.cleared,
                            failed = report.failed,
                            remaining = report.remaining,
                            pass_ms,
                            "Deferred search index repair pass finished"
                        );
                    }
                }
                Ok(report) if first_pass && report.remaining > 0 => info!(
                    remaining = report.remaining,
                    "Deferred search index ledger holds chunks younger than the repair's minimum age; the next pass takes them"
                ),
                Ok(_) => {}
                Err(e) => warn!("Repair of deferred MongoDB search rows could not run: {e}"),
            }
            first_pass = false;
            tokio::time::sleep(interval).await;
        }
    });
}

/// S3 flavor of the periodic registry refresh (#235, #787). Before #787, S3
/// had no such refresh at all — stored SearchParameters never took effect,
/// not even eventually, because the composite handed Elasticsearch a
/// `base_only()` registry whose loader unconditionally returned an empty
/// overlay. S3 now owns a real per-tenant overlay refreshed on every write
/// (immediate effect) plus this TTL sweep (multi-instance drift), matching
/// every other backend.
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
fn spawn_s3_search_param_refresh(
    backend: Arc<helios_persistence::backends::s3::S3Backend>,
    config: &ServerConfig,
) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match backend.refresh_stored_search_parameters().await {
                Ok(stored) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Err(e) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
            }
        }
    });
}

/// Registers the shared base SearchParameters — embedded fallback, the FHIR
/// spec file, and every custom file in `data_dir` (e.g. the SQL-on-FHIR
/// `ViewDefinition` params in `sql-on-fhir-search-parameters.json`) — into
/// `registry`.
///
/// This is the same three-tier load the SQLite, PostgreSQL, and MongoDB
/// backends perform when they build their own registries. A primary with no
/// `data_dir` of its own (S3) needs a starter to do it instead; skipping the
/// custom tier left Elasticsearch unaware of params such as `ViewDefinition`
/// `name`, so `name:contains` was silently dropped from searches (#1070).
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
fn populate_base_search_registry(
    registry: &mut helios_persistence::search::SearchParameterRegistry,
    fhir_version: helios_fhir::FhirVersion,
    data_dir: &std::path::Path,
) {
    use helios_persistence::search::SearchParameterLoader;

    let loader = SearchParameterLoader::new(fhir_version);
    let mut fallback_count = 0;
    let mut spec_count = 0;
    let mut custom_count = 0;
    let mut custom_files: Vec<String> = Vec::new();

    match loader.load_embedded() {
        Ok(params) => {
            for p in params {
                if registry.register(p).is_ok() {
                    fallback_count += 1;
                }
            }
        }
        Err(e) => warn!("Failed to load embedded SearchParameters: {e}"),
    }

    let spec_path = data_dir.join(loader.spec_filename());
    match loader.load_from_spec_file(data_dir) {
        Ok(params) => {
            for p in params {
                if registry.register(p).is_ok() {
                    spec_count += 1;
                }
            }
        }
        Err(e) => warn!(
            "Could not load spec SearchParameters from {}: {e}. Using minimal fallback.",
            spec_path.display()
        ),
    }

    match loader.load_custom_from_directory_with_files(data_dir) {
        Ok((params, files)) => {
            for p in params {
                if registry.register(p).is_ok() {
                    custom_count += 1;
                }
            }
            custom_files = files;
        }
        Err(e) => warn!(
            "Error loading custom SearchParameters from {}: {e}",
            data_dir.display()
        ),
    }

    let custom_info = if custom_files.is_empty() {
        String::new()
    } else {
        format!(" [{}]", custom_files.join(", "))
    };
    info!(
        "SearchParameter registry initialized: {} total ({} spec from {}, {} fallback, {} custom{}) covering {} resource types",
        registry.len(),
        spec_count,
        spec_path.display(),
        fallback_count,
        custom_count,
        custom_info,
        registry.resource_types().len()
    );
}

/// Starts the server with SQLite-only backend.
#[cfg(feature = "sqlite")]
async fn start_sqlite(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let serve_audit_state = audit_state.clone();
    let backend = Arc::new(create_sqlite_backend(&config)?);
    attach_login_sessions(auth_state.as_ref(), backend.clone());
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*backend, &config, Some(observability.observers.as_ref())).await;
    spawn_sqlite_search_param_refresh(backend.clone(), &config);
    spawn_sqlite_wal_maintenance(backend.clone());
    let wal_backend = backend.clone();
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own). Cheap: the SQLite backend
    // shares one connection pool behind the Arc.
    let ui_tenants: Option<Arc<dyn ResourceStorage>> = Some(backend.clone());

    // The SQLite backend also hosts the per-user settings store, so it always
    // keeps ownership of the backend Arc and uses the settings-capable builder.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(backend.clone());
    let export_bundle = build_bulk_export(&config, backend.clone(), backend.clone()).await?;
    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, Some(backend.clone())));
    let submit_bundle = build_bulk_submit(
        &config,
        backend.clone(),
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    let served = serve(
        app,
        &config,
        serve_audit_state,
        ui_tenants,
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await;
    checkpoint_sqlite_wal_at_shutdown(&wal_backend).await;
    served
}

/// Constructs an embedded SQLite job store for backends that can't host job
/// state themselves (MongoDB, S3). Path resolution, in priority order:
///
/// 1. `${HFS_BULK_EXPORT_OUTPUT_DIR}/bulk_export.db` — co-located with local
///    output, so single-instance dev / CI runs keep job state next to data.
/// 2. `${HFS_DATA_DIR}/bulk_export.db` — when explicitly configured.
/// 3. `${TMPDIR}/hfs-bulk-export-{pid}.db` — last-resort fallback that's
///    unique per process so parallel HFS instances (e.g. CI smoke jobs running
///    with `max-parallel`) don't race on a single file. Job state is ephemeral
///    in this mode — fine for tests; production deployments should configure
///    one of the persistent options above.
///
/// Isolated from FHIR resource data either way.
///
/// Only compiled for backends that can't host job state themselves and so need
/// the sidecar (MongoDB, S3); pure-SQLite/Postgres deployments never call it.
#[cfg(all(
    feature = "sqlite",
    any(feature = "mongodb", all(feature = "s3", feature = "elasticsearch"))
))]
fn build_embedded_job_store(config: &ServerConfig) -> anyhow::Result<Arc<dyn BulkExportJobStore>> {
    let job_db = config
        .bulk_export
        .output_dir
        .clone()
        .map(|d| format!("{d}/bulk_export.db"))
        .or_else(|| {
            config
                .data_dir
                .as_ref()
                .map(|d| format!("{}/bulk_export.db", d.display()))
        })
        .unwrap_or_else(|| {
            let mut path = std::env::temp_dir();
            path.push(format!("hfs-bulk-export-{}.db", std::process::id()));
            path.to_string_lossy().into_owned()
        });
    if let Some(parent) = std::path::Path::new(&job_db).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "create bulk export job DB directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let job_backend = SqliteBackend::with_config(
        &job_db,
        SqliteBackendConfig {
            fhir_version: config.default_fhir_version,
            data_dir: config.data_dir.clone(),
            ..Default::default()
        },
    )?;
    job_backend.init_schema()?;
    Ok(Arc::new(job_backend))
}

/// Builds the bulk-export subsystem (output store + file auth + worker pool)
/// from a caller-supplied job store and resource data provider. Returns `None`
/// when bulk export is disabled.
///
/// The job store is supplied by the caller so it reuses the same backend
/// instance (and connection pool) that holds the FHIR resources. This means
/// pure-SQLite deployments share `./data/hfs.db` between resources and job
/// state, and Postgres deployments share the configured `HFS_DATABASE_URL`.
/// Backends that can't host transactional job state (MongoDB, S3) use a
/// sidecar SQLite via [`build_embedded_job_store`].
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn build_bulk_export<Dp>(
    config: &ServerConfig,
    data: Arc<Dp>,
    jobs: Arc<dyn BulkExportJobStore>,
) -> anyhow::Result<Option<helios_rest::BulkExportBundle>>
where
    Dp: helios_persistence::core::ExportResourceProvider + 'static,
{
    let cfg = config.bulk_export.clone();
    info!(
        "Bulk export config: enabled={} output_backend={} requires_access_token={}",
        cfg.enabled, cfg.output_backend, cfg.requires_access_token
    );
    if !cfg.enabled {
        return Ok(None);
    }

    // --- Output store ---------------------------------------------------
    let output: Arc<dyn ExportOutputStore> = match cfg.output_backend.as_str() {
        "local-fs" => {
            let output_dir = cfg
                .output_dir
                .clone()
                .or_else(|| {
                    config
                        .data_dir
                        .as_ref()
                        .map(|d| format!("{}/exports", d.display()))
                })
                .unwrap_or_else(|| "./data/exports".to_string());
            // The served file endpoint enforces a token only when auth is on, so
            // the manifest's requiresAccessToken must follow the auth state under
            // `auto`; an explicit true/false override still wins (#1269). `false`
            // is rejected for local-fs by config validation (no pre-signing).
            let requires_token = match cfg.requires_access_token.as_str() {
                "true" => true,
                "false" => false,
                _ => AuthConfig::from_env().enabled,
            };
            Arc::new(
                LocalFsOutputStore::new(output_dir, config.base_url.clone())
                    .with_access_token_required(requires_token),
            )
        }
        "s3" => {
            #[cfg(feature = "s3")]
            {
                use helios_persistence::backends::s3::{
                    AccessTokenMode, AwsS3Client, AwsS3ClientOptions, S3OutputStore,
                };
                let bucket = cfg.s3_bucket.clone().ok_or_else(|| {
                    anyhow::anyhow!("HFS_BULK_EXPORT_S3_BUCKET is required for OUTPUT_BACKEND=s3")
                })?;
                let region = std::env::var("HFS_BULK_EXPORT_S3_REGION")
                    .ok()
                    .or_else(|| std::env::var("HFS_S3_REGION").ok());
                let sdk_config = AwsS3Client::load_sdk_config(region.as_deref()).await;
                let endpoint_url =
                    non_blank_endpoint(std::env::var("HFS_BULK_EXPORT_S3_ENDPOINT").ok());
                // A custom endpoint forces path-style addressing, as for the
                // S3 storage backend and the `$sql-export` S3 sink.
                let force_path_style = endpoint_url.is_some()
                    || parse_env_bool("HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE", false);
                let client = Arc::new(AwsS3Client::from_sdk_config_with_options(
                    &sdk_config,
                    AwsS3ClientOptions {
                        endpoint_url,
                        force_path_style,
                    },
                ));
                Arc::new(S3OutputStore::new(
                    client,
                    bucket,
                    config.base_url.clone(),
                    AccessTokenMode::parse(&cfg.requires_access_token),
                    std::time::Duration::from_secs(cfg.file_url_ttl_secs),
                ))
            }
            #[cfg(not(feature = "s3"))]
            {
                anyhow::bail!(
                    "HFS_BULK_EXPORT_OUTPUT_BACKEND=s3 requires the 's3' feature. \
                     Build with: cargo build -p helios-hfs --features s3"
                );
            }
        }
        other => anyhow::bail!("invalid HFS_BULK_EXPORT_OUTPUT_BACKEND '{other}'"),
    };

    spawn_export_workers(jobs.clone(), data, output.clone(), &cfg);
    // A tenant purge from the web UI deletes this store's files too (#2279).
    #[cfg(feature = "ui")]
    helios_ui::register_tenant_purge_output(output.clone());

    Ok(Some(helios_rest::BulkExportBundle {
        jobs,
        output,
        file_auth: Arc::new(BearerScopeAuth),
    }))
}

/// Attaches the audit sink to a reindex driver and hands back a shared handle.
/// Used by both the standalone and composite ops bundles; the S3-only build has
/// no reindex target and so never calls it.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn wire_reindex(
    op: ReindexOperation,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
) -> Arc<ReindexOperation> {
    let op = match audit_state {
        Some(state) => op.with_audit(
            Arc::clone(&state.sink),
            state.config.source_observer.clone(),
        ),
        None => op,
    };
    Arc::new(op)
}

/// Builds the deferred bulk-submit hook using the existing submit-worker
/// concurrency as the per-process automatic reindex limit, plus where to clear
/// the persisted "this manifest still owes a rebuild" marker when a generation
/// finishes (#1125). Without a ledger (`None`) nothing is recorded and a
/// restart cannot resume, as before.
///
/// Split into this function and [`automatic_reindex_hook_with_ledger`] so a
/// test can read the concrete `ReindexOnFinish`'s `batch_bytes()` (#1499)
/// without downcasting the trait object every other caller uses.
///
/// Gated exactly like [`wire_reindex`], which produces the `op` every caller
/// passes in: any build with a reindex target. Keep the two in step rather
/// than naming individual backends here — a narrower gate breaks the builds
/// that leave that backend out (#1291).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn build_automatic_reindex_hook(
    op: Arc<ReindexOperation>,
    config: &ServerConfig,
    ledger: Option<Arc<dyn helios_persistence::search::DeferredReindexLedger>>,
) -> helios_persistence::search::ReindexOnFinish {
    let hook = helios_persistence::search::ReindexOnFinish::with_max_concurrency(
        op,
        config.bulk_submit.worker_concurrency as usize,
    )
    .with_batch_size(config.reindex_batch_size)
    .with_batch_bytes(config.reindex_batch_bytes)
    .with_bulk_index_rebuild(config.bulk_submit.bulk_index_rebuild);
    match ledger {
        Some(ledger) => hook.with_ledger(ledger),
        None => hook,
    }
}

/// Builds [`build_automatic_reindex_hook`]'s hook and erases it behind
/// `Arc<dyn DeferredReindexHook>`, the shape every wiring site outside tests
/// needs. See that function's doc comment for what it configures and why the
/// two are split (#1499).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn automatic_reindex_hook_with_ledger(
    op: Arc<ReindexOperation>,
    config: &ServerConfig,
    ledger: Option<Arc<dyn helios_persistence::search::DeferredReindexLedger>>,
) -> Arc<dyn helios_persistence::core::DeferredReindexHook> {
    Arc::new(build_automatic_reindex_hook(op, config, ledger))
}

/// Ops bundle for a backend that indexes itself — the standalone deployments
/// (SQLite, PostgreSQL, MongoDB), where resources and search index share a home.
///
/// `prefetch` is `HFS_REINDEX_PREFETCH`: whether the rebuild fetches the next
/// page while the current one is written, where the source allows it (#1250).
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
fn standalone_ops<B>(
    backend: Arc<B>,
    registries: Arc<helios_persistence::search::TenantSearchRegistries>,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
    observability: helios_rest::WriteObservability,
    prefetch: bool,
) -> OperationsBundle
where
    B: PurgableStorage + helios_persistence::search::ReindexableStorage + 'static,
{
    OperationsBundle {
        purge: Some(backend.clone() as Arc<dyn PurgableStorage>),
        reindex: Some(wire_reindex(
            ReindexOperation::new(backend, registries).with_prefetch(prefetch),
            audit_state,
        )),
        observability: Some(observability),
    }
}

/// Ops bundle for a composite deployment.
///
/// `purge` is the **composite**, not the primary: purging the primary alone
/// would leave the resource in the Elasticsearch index, still searchable and
/// still holding its content.
///
/// `reindex` reads from the primary and writes to every index in `targets` —
/// which must include the Elasticsearch secondary, since that is what actually
/// serves search here. Rebuilding only the primary's index would leave search
/// untouched by `$reindex`.
///
/// Composite deployments always pair a primary with the Elasticsearch
/// secondary, so this is only reachable when `elasticsearch` is enabled.
///
/// `prefetch` is `HFS_REINDEX_PREFETCH`: whether the rebuild fetches the next
/// page from the primary while the current one is sent to Elasticsearch
/// (#1250).
#[cfg(feature = "elasticsearch")]
fn composite_ops(
    composite: Arc<helios_persistence::composite::CompositeStorage>,
    source: Arc<dyn helios_persistence::search::ReindexSource>,
    targets: Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    registries: Arc<helios_persistence::search::TenantSearchRegistries>,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
    observability: helios_rest::WriteObservability,
    prefetch: bool,
) -> OperationsBundle {
    OperationsBundle {
        purge: Some(composite as Arc<dyn PurgableStorage>),
        reindex: Some(wire_reindex(
            ReindexOperation::with_parts(source, targets, registries).with_prefetch(prefetch),
            audit_state,
        )),
        observability: Some(observability),
    }
}

/// Spawns the in-process export worker pool and the periodic cleanup task.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn spawn_export_workers<Dp>(
    jobs: Arc<dyn BulkExportJobStore>,
    data: Arc<Dp>,
    output: Arc<dyn ExportOutputStore>,
    cfg: &helios_rest::config::BulkExportConfig,
) where
    Dp: helios_persistence::core::ExportResourceProvider + 'static,
{
    if cfg.disable_local_worker {
        info!("Bulk export in-process worker pool is disabled");
        return;
    }
    let lease = std::time::Duration::from_secs(cfg.lease_duration_secs);
    let heartbeat = std::time::Duration::from_secs(cfg.heartbeat_interval_secs);
    let max_attempts = cfg.max_attempts;
    let pools = worker_shutdown::pools();
    for i in 0..cfg.worker_concurrency {
        let jobs = jobs.clone();
        let data = data.clone();
        let output = output.clone();
        let worker_id = WorkerId::new(format!("hfs-worker-{i}"));
        let exclude_newly_added = cfg.since_newly_added.eq_ignore_ascii_case("exclude");
        let shutdown = pools.token();
        pools.spawn(async move {
            let worker = DefaultExportWorker::new(jobs.clone(), data, output, worker_id.clone())
                .with_exclude_since_newly_added(exclude_newly_added)
                .with_heartbeat_interval(heartbeat)
                .with_shutdown(shutdown.clone());
            // The claim itself is never raced against shutdown: a claim that
            // commits after its future was dropped would leave a leased job
            // that nobody runs.
            while !shutdown.is_cancelled() {
                let pause = match jobs.claim_next(&worker_id, lease, max_attempts).await {
                    Ok(Some(claimed)) => {
                        if let Err(e) = worker.run_job(claimed).await {
                            tracing::error!("export worker job failed: {e}");
                        }
                        continue;
                    }
                    Ok(None) => std::time::Duration::from_secs(2),
                    Err(e) => {
                        tracing::error!("export worker claim failed: {e}");
                        std::time::Duration::from_secs(5)
                    }
                };
                if worker_shutdown::idle(&shutdown, pause).await {
                    break;
                }
            }
        });
    }

    // Periodic cleanup of expired job output.
    let cleanup_jobs = jobs.clone();
    let cleanup_output = output.clone();
    let interval = std::time::Duration::from_secs(cfg.cleanup_interval_secs);
    let output_ttl = std::time::Duration::from_secs(cfg.output_ttl_secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match cleanup_jobs
                .list_expired_exports(chrono::Utc::now(), output_ttl, 100)
                .await
            {
                Ok(expired) => {
                    for job in expired {
                        let _ = cleanup_output
                            .delete_job_outputs(&job.tenant, &job.job_id)
                            .await;
                        let _ = cleanup_jobs.delete_export(&job.tenant, &job.job_id).await;
                    }
                }
                Err(e) => tracing::error!("export cleanup scan failed: {e}"),
            }
        }
    });
    info!(
        "Bulk export worker pool started ({} workers)",
        cfg.worker_concurrency
    );
}

/// Picks the `$bulk-submit` job store for a primary + Elasticsearch composite.
///
/// The raw primary never feeds Elasticsearch, so by default the store is
/// wrapped in [`CompositeSubmitJobs`], which syncs each finished manifest's
/// resources into the secondary index (#882). Under bulk fast-load the worker
/// fires a per-type reindex after each manifest that rebuilds Elasticsearch
/// too, so the wrapper's per-resource sync would only duplicate that work (#903)
/// and is switched off with `with_ingest_sync(false)`. The wrapper itself stays
/// in the chain: it is what mirrors a rolled-back change onto Elasticsearch
/// (a rolled-back create becomes a delete) and forwards the #1125 rebuild
/// ledger to the primary. Handing the worker the raw primary instead left
/// rolled-back resources searchable in Elasticsearch after they were gone from
/// the primary (#1161). Aborting a submission rolls nothing back on any
/// primary, so abort leaves Elasticsearch alone. Fast-load without a reindex
/// hook keeps the per-resource sync, otherwise the data never reaches
/// Elasticsearch.
///
/// With `HFS_BULK_SUBMIT_DEFER_INDEXING=false` (#1127, #1242) the wrapper is
/// itself wrapped in [`IndexingSubmitJobs`]: every committed batch is handed to
/// an [`IngestIndexSink`] writing into `search_targets` (the Elasticsearch
/// secondary, not the primary's own offloaded index), and the manifest's sync
/// drains that sink instead of re-reading the manifest. The deferred reindex
/// then rebuilds only the types the sink rejected. `source` is the primary the
/// sink reads resources back from for engines that do not hand them over. On
/// this ES-composite path `false` always means the sink; the operator no longer
/// names the mechanism (the old `HFS_BULK_SUBMIT_INDEX_DURING_INGEST` is gone).
///
/// [`CompositeSubmitJobs`]: helios_persistence::composite::CompositeSubmitJobs
/// [`IndexingSubmitJobs`]: helios_persistence::composite::IndexingSubmitJobs
/// [`IngestIndexSink`]: helios_persistence::composite::IngestIndexSink
#[cfg(all(
    feature = "elasticsearch",
    any(
        feature = "sqlite",
        feature = "postgres",
        feature = "mongodb",
        feature = "s3"
    )
))]
fn composite_submit_jobs(
    primary: Arc<dyn BulkSubmitJobStore>,
    source: Arc<dyn ResourceStorage>,
    composite: Arc<helios_persistence::composite::CompositeStorage>,
    search_targets: Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    cfg: &helios_rest::config::BulkSubmitConfig,
    has_reindex_hook: bool,
) -> Arc<dyn BulkSubmitJobStore> {
    use helios_persistence::composite::{
        CompositeSubmitJobs, IndexingSubmitJobs, IngestIndexSink, IngestIndexSinkConfig,
    };

    if !cfg.defer_indexing {
        // Search is offloaded to Elasticsearch here (this fn is the ES-composite
        // path), so "index as the manifest ingests" means the batch-by-batch
        // sink (#1127, #1242).
        let sink_config = IngestIndexSinkConfig {
            queue: cfg.index_queue as usize,
            concurrency: cfg.index_concurrency as usize,
            coalesce: cfg.index_coalesce as usize,
            max_wait: std::time::Duration::from_secs(cfg.index_max_wait_secs),
            page_bytes: usize::try_from(cfg.index_page_bytes).unwrap_or(usize::MAX),
        };
        info!(
            queue = sink_config.queue,
            concurrency = sink_config.concurrency,
            coalesce = sink_config.coalesce,
            max_wait_secs = cfg.index_max_wait_secs,
            page_bytes = sink_config.page_bytes,
            "Bulk submit indexes into Elasticsearch during ingest (DEFER_INDEXING=false); \
             the deferred reindex runs only for types the search index rejected"
        );
        let sink = Arc::new(IngestIndexSink::new(source, search_targets, sink_config));
        let inner: Arc<dyn BulkSubmitJobStore> =
            Arc::new(CompositeSubmitJobs::new(primary, composite));
        // With a hook, what the index rejects is rebuilt by the deferred
        // reindex, so it is reported as a warning rather than a failure (#1666).
        Arc::new(IndexingSubmitJobs::new(inner, sink).with_automatic_reindex(has_reindex_hook))
    } else if has_reindex_hook {
        info!(
            "Bulk submit defers indexing (DEFER_INDEXING=true); Elasticsearch is rebuilt \
             per type after each manifest, and rollbacks are mirrored to it"
        );
        Arc::new(CompositeSubmitJobs::new(primary, composite).with_ingest_sync(false))
    } else {
        info!(
            "Bulk submit syncs each finished manifest into Elasticsearch (no reindex hook); \
             set DEFER_INDEXING=false to index batch by batch instead"
        );
        Arc::new(CompositeSubmitJobs::new(primary, composite))
    }
}

/// Builds the bulk-submit subsystem (input fetcher + output store + file auth +
/// worker pool) from a caller-supplied job store. Returns `None` when bulk submit
/// is disabled. The job store is the same backend instance that holds the FHIR
/// resources (so ingestion writes go to the primary store).
///
/// Unlike bulk *export*, every backend that can run `$bulk-submit` hosts its own
/// job state — MongoDB in its own collections, S3 in the same objects its
/// ingestion engine already writes — so there is no sidecar variant here.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
fn use_independent_submit_files(
    mode: Option<StorageBackendMode>,
    effective_file_concurrency: u32,
) -> bool {
    matches!(mode, Some(StorageBackendMode::Postgres)) && effective_file_concurrency > 1
}

#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
struct SubmitFileScheduling {
    concurrency: u32,
    independent_tasks: bool,
}

#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
async fn build_bulk_submit(
    config: &ServerConfig,
    jobs: Arc<dyn BulkSubmitJobStore>,
    reindex_hook: Option<Arc<dyn helios_persistence::core::DeferredReindexHook>>,
    write_observer: Arc<dyn WriteObserver>,
) -> anyhow::Result<Option<helios_rest::BulkSubmitBundle>> {
    let cfg = config.bulk_submit.clone();
    info!(
        "Bulk submit config: enabled={} output_backend={} requires_access_token={}",
        cfg.enabled, cfg.output_backend, cfg.requires_access_token
    );
    if !cfg.enabled {
        return Ok(None);
    }

    // --- Output store (status-manifest artifacts) -----------------------
    let output: Arc<dyn ExportOutputStore> = match cfg.output_backend.as_str() {
        "local-fs" => {
            let output_dir = cfg
                .output_dir
                .clone()
                .or_else(|| {
                    config
                        .data_dir
                        .as_ref()
                        .map(|d| format!("{}/submit", d.display()))
                })
                .unwrap_or_else(|| "./data/submit".to_string());
            Arc::new(LocalFsOutputStore::new(output_dir, config.base_url.clone()))
        }
        "s3" => {
            #[cfg(feature = "s3")]
            {
                use helios_persistence::backends::s3::{
                    AccessTokenMode, AwsS3Client, AwsS3ClientOptions, S3OutputStore,
                };
                let bucket = cfg.s3_bucket.clone().ok_or_else(|| {
                    anyhow::anyhow!("HFS_BULK_SUBMIT_S3_BUCKET is required for OUTPUT_BACKEND=s3")
                })?;
                let region = std::env::var("HFS_BULK_SUBMIT_S3_REGION")
                    .ok()
                    .or_else(|| std::env::var("HFS_S3_REGION").ok());
                let sdk_config = AwsS3Client::load_sdk_config(region.as_deref()).await;
                let endpoint_url =
                    non_blank_endpoint(std::env::var("HFS_BULK_SUBMIT_S3_ENDPOINT").ok());
                // A custom endpoint forces path-style addressing, as for bulk
                // export, the S3 storage backend and the `$sql-export` S3 sink.
                let force_path_style = endpoint_url.is_some()
                    || parse_env_bool("HFS_BULK_SUBMIT_S3_FORCE_PATH_STYLE", false);
                let client = Arc::new(AwsS3Client::from_sdk_config_with_options(
                    &sdk_config,
                    AwsS3ClientOptions {
                        endpoint_url,
                        force_path_style,
                    },
                ));
                Arc::new(S3OutputStore::new(
                    client,
                    bucket,
                    config.base_url.clone(),
                    AccessTokenMode::parse(&cfg.requires_access_token),
                    std::time::Duration::from_secs(cfg.file_url_ttl_secs),
                ))
            }
            #[cfg(not(feature = "s3"))]
            {
                anyhow::bail!(
                    "HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3 requires the 's3' feature. \
                     Build with: cargo build -p helios-hfs --features s3"
                );
            }
        }
        other => anyhow::bail!("invalid HFS_BULK_SUBMIT_OUTPUT_BACKEND '{other}'"),
    };

    // --- Remote input fetcher (+ optional outbound SMART client) --------
    // When a client_id + private key are configured, protected-file fetches use
    // a read-scoped `client_credentials` token; otherwise they surface a recorded
    // manifest-level error.
    let token_provider: Option<Arc<dyn helios_persistence::core::FileTokenProvider>> =
        match (cfg.client_id.as_deref(), cfg.private_key.as_deref()) {
            (Some(client_id), Some(pem)) => {
                match helios_rest::bulk_submit_oauth::JwtClientCredentialsTokenProvider::new(
                    client_id,
                    pem,
                    &cfg.signing_alg,
                ) {
                    Some(p) => Some(p),
                    None => anyhow::bail!(
                        "HFS_BULK_SUBMIT_PRIVATE_KEY could not be parsed as a {} PEM key",
                        cfg.signing_alg
                    ),
                }
            }
            _ => None,
        };
    // Private keys for JWE `fileEncryptionKey` material addressed to HFS
    // asymmetrically (ECDH-ES*, P-256/P-384). `dir` and the A*KW families use the
    // symmetric key the provider supplies and need no configuration.
    let decryption_keys = match cfg.decryption_key.as_deref() {
        Some(material) => helios_rest::jwe::load_private_keys(material)
            .map_err(|e| anyhow::anyhow!("HFS_BULK_SUBMIT_DECRYPTION_KEY is invalid: {e}"))?,
        None => Vec::new(),
    };
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(
        helios_rest::bulk_submit_fetcher::HttpSubmitInputFetcher::new(
            token_provider,
            cfg.outbound_scope.clone(),
        )
        .with_read_timeout(std::time::Duration::from_secs(cfg.fetch_read_timeout_secs))
        .with_decryption_keys(decryption_keys),
    );

    // SQLite serialises writers, so a fan-out queues batch writes behind one
    // lock until they outlast `busy_timeout` and abort the manifest (#942).
    // Ignore the configured value there, and warn about it, rather than letting
    // it fail the import.
    let backend_mode = config.storage_backend_mode().ok();
    let backend_kind = backend_mode
        .map(StorageBackendMode::primary_backend_kind)
        .unwrap_or(BackendKind::Sqlite);
    let file_concurrency = cfg.effective_file_concurrency(backend_kind);
    let file_scheduling = SubmitFileScheduling {
        concurrency: file_concurrency,
        independent_tasks: use_independent_submit_files(backend_mode, file_concurrency),
    };
    if file_concurrency < cfg.file_concurrency.max(1) {
        warn!(
            configured = cfg.file_concurrency,
            effective = file_concurrency,
            "Bulk submit file fan-out is not supported on SQLite and is \
             running at 1: SQLite serialises writers, so a fan-out queues \
             batch writes past busy_timeout and aborts the import. Use \
             PostgreSQL for a higher file concurrency."
        );
    }

    spawn_submit_workers(
        jobs.clone(),
        fetcher.clone(),
        output.clone(),
        &cfg,
        file_scheduling,
        reindex_hook,
        write_observer,
    );
    // A tenant purge from the web UI deletes this store's files too (#2279).
    #[cfg(feature = "ui")]
    helios_ui::register_tenant_purge_output(output.clone());

    Ok(Some(helios_rest::BulkSubmitBundle {
        jobs,
        fetcher,
        output,
        file_auth: Arc::new(BearerScopeAuth),
    }))
}

/// Spawns the in-process submit worker pool and the periodic cleanup task.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
))]
fn spawn_submit_workers(
    jobs: Arc<dyn BulkSubmitJobStore>,
    fetcher: Arc<dyn SubmitInputFetcher>,
    output: Arc<dyn ExportOutputStore>,
    cfg: &helios_rest::config::BulkSubmitConfig,
    file_scheduling: SubmitFileScheduling,
    reindex_hook: Option<Arc<dyn helios_persistence::core::DeferredReindexHook>>,
    write_observer: Arc<dyn WriteObserver>,
) {
    if cfg.disable_local_worker {
        info!("Bulk submit in-process worker pool is disabled");
        return;
    }
    let lease = std::time::Duration::from_secs(cfg.lease_duration_secs);
    let defer_indexing = cfg.defer_indexing;
    if defer_indexing {
        info!("Bulk submit fast-load: search indexing deferred to post-manifest reindex");
        if !jobs.records_index_pending() {
            warn!(
                "Bulk submit job store cannot record a pending search-index rebuild: if HFS stops before a deferred rebuild finishes, the manifest's resources stay unsearchable until POST /$reindex is run"
            );
        }
    }
    let batch_size = cfg.batch_size;
    let skip_unchanged = cfg.skip_unchanged;
    if skip_unchanged {
        info!("Bulk submit skips resources whose content is identical to the stored version");
    }
    // #1125: manifests whose rebuild was still outstanding when the server
    // stopped. The marker is on the manifest row, so it survived; re-firing
    // the same hook the live path uses keeps one code path.
    if defer_indexing && let Some(hook) = reindex_hook.clone() {
        let jobs = jobs.clone();
        tokio::spawn(async move {
            match jobs.list_manifests_awaiting_reindex(256).await {
                Ok(pending) if !pending.is_empty() => {
                    info!(
                        manifests = pending.len(),
                        "resuming search-index rebuilds left outstanding by an earlier run"
                    );
                    for manifest in pending {
                        if manifest.resource_types.is_empty() {
                            continue;
                        }
                        hook.reindex_types_with_context(
                            &manifest.tenant,
                            manifest.resource_types,
                            helios_persistence::core::DeferredReindexContext {
                                submission_id: Some(manifest.submission.to_string()),
                                manifest_id: Some(manifest.manifest_id),
                            },
                        )
                        .await;
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    error = %e,
                    "could not look for search-index rebuilds left outstanding by an earlier run"
                ),
            }
        });
    }
    let file_concurrency = file_scheduling.concurrency.max(1) as usize;
    let scheduling = if file_scheduling.independent_tasks {
        "independent-tasks"
    } else {
        "inline"
    };
    info!(
        file_concurrency,
        scheduling, "Bulk submit file scheduling selected"
    );
    for i in 0..cfg.worker_concurrency {
        let jobs = jobs.clone();
        let fetcher = fetcher.clone();
        let output = output.clone();
        let reindex_hook = reindex_hook.clone();
        let write_observer = write_observer.clone();
        let worker_id = WorkerId::new(format!("hfs-submit-worker-{i}"));
        let pools = worker_shutdown::pools();
        let shutdown = pools.token();
        pools.spawn(async move {
            let mut worker =
                DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id.clone())
                    .with_deferred_indexing(defer_indexing, reindex_hook.clone())
                    .with_write_observer(Some(write_observer))
                    .with_file_concurrency(file_concurrency)
                    .with_batch_size(batch_size)
                    .with_skip_unchanged(skip_unchanged)
                    .with_shutdown(shutdown.clone());
            if file_scheduling.independent_tasks {
                worker = worker.with_independent_file_tasks();
            }
            // The claim itself is never raced against shutdown: a claim that
            // commits after its future was dropped would leave a leased
            // manifest that nobody runs.
            while !shutdown.is_cancelled() {
                let pause = match jobs.claim_next_manifest(&worker_id, lease).await {
                    Ok(Some(claimed)) => {
                        if let Err(e) = worker.run_job(claimed).await {
                            tracing::error!("submit worker job failed: {e}");
                        }
                        continue;
                    }
                    Ok(None) => std::time::Duration::from_secs(2),
                    Err(e) => {
                        tracing::error!("submit worker claim failed: {e}");
                        std::time::Duration::from_secs(5)
                    }
                };
                if worker_shutdown::idle(&shutdown, pause).await {
                    break;
                }
            }
        });
    }
    // Periodic cleanup of expired submission artifacts.
    let cleanup_jobs = jobs.clone();
    let cleanup_output = output.clone();
    let interval = std::time::Duration::from_secs(cfg.cleanup_interval_secs);
    let output_ttl = std::time::Duration::from_secs(cfg.output_ttl_secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match cleanup_jobs
                .list_expired_submissions(chrono::Utc::now(), output_ttl, 100)
                .await
            {
                Ok(expired) => {
                    for (tenant, sub_id) in expired {
                        let job_id = helios_persistence::core::submission_output_job_id(&sub_id);
                        let _ = cleanup_output.delete_job_outputs(&tenant, &job_id).await;
                        let _ = cleanup_jobs
                            .delete_submission_artifacts(&tenant, &sub_id)
                            .await;
                        let _ = cleanup_jobs.clear_poll_token(&tenant, &sub_id).await;
                    }
                }
                Err(e) => tracing::error!("submit cleanup scan failed: {e}"),
            }
        }
    });

    info!(
        "Bulk submit worker pool started ({} workers)",
        cfg.worker_concurrency
    );
}

/// Fallback when sqlite feature is not enabled.
#[cfg(not(feature = "sqlite"))]
async fn start_sqlite(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite backend requires the 'sqlite' feature. \
         Build with: cargo build -p helios-hfs --features sqlite"
    )
}

/// Starts the server with SQLite + Elasticsearch composite backend.
#[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
async fn start_sqlite_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create SQLite backend with search offloaded to Elasticsearch
    let mut sqlite = create_sqlite_backend(&config)?;
    sqlite.set_search_offloaded(true);
    let sqlite = Arc::new(sqlite);
    attach_login_sessions(auth_state.as_ref(), sqlite.clone());
    info!("SQLite search indexing disabled (offloaded to Elasticsearch)");
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_sqlite_search_param_refresh(sqlite.clone(), &config);
    spawn_sqlite_wal_maintenance(sqlite.clone());
    let wal_backend = sqlite.clone();

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if es_nodes.is_empty() {
        anyhow::bail!(
            "sqlite-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        refresh_interval: config.elasticsearch_refresh_interval.clone(),
        write_refresh: es_write_refresh_from_config(&config)?,
        nested_objects_limit: config.elasticsearch_nested_objects_limit,
        max_terms_count: config.elasticsearch_max_terms_count,
        request_timeout_ms: config.elasticsearch_request_timeout_ms,
        bulk_max_bytes: config.elasticsearch_bulk_max_bytes,
        bulk_concurrency: config.elasticsearch_bulk_concurrency,
        contained_sweep_concurrency: config.elasticsearch_contained_sweep_concurrency,
        reindex_refresh: es_reindex_refresh_from_config(&config)?,
        reindex_prepare_threads: config.elasticsearch_reindex_prepare_threads,
        ..Default::default()
    };

    info!(
        nodes = ?redact_urls(&es_nodes),
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // Create ES backend sharing SQLite's search parameter registry
    let es = Arc::new(
        ElasticsearchBackend::start_with_shared_registry(
            es_config,
            sqlite.tenant_registries().clone(),
        )
        .await?,
    );
    es_startup_refresh_setup(&es, &config).await;

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
        .fhir_version(config.default_fhir_version)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(sqlite.clone())
        // A failed Elasticsearch sync is counted, logged, and recorded in the
        // primary as "needs reindex" (#1334).
        .with_sync_observer(Arc::new(CompositeSyncMetrics))
        .with_sync_failure_ledger(sqlite.clone())
        // `$purge` must reach the Elasticsearch secondary too — purging only
        // the SQLite primary would leave the resource in the search index,
        // still searchable and still holding its content.
        .with_purgable_backends(
            sqlite.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: SQLite (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);
    spawn_secondary_sync_repair(composite.clone(), true);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable
    // (empty /SearchParameter and /CompartmentDefinition, and empty UI viewers).
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*composite, &config, Some(observability.observers.as_ref())).await;

    // The per-user settings store lives on the SQLite primary (Elasticsearch is
    // search-only), so it is wired from the underlying `sqlite` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(sqlite.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(sqlite.clone());

    // The worker searches through `composite` (not the bare `sqlite` primary)
    // so a `_typeFilter` runs against Elasticsearch, the index that actually
    // serves search in this deployment.
    let export_bundle = build_bulk_export(&config, composite.clone(), sqlite.clone()).await?;
    // Reindex reads from the SQLite primary and writes only the indexes a query
    // can read: Elasticsearch, plus SQLite's own index while it is not offloaded.
    let (reindex_targets, reindex_target_names) = sqlite_es_reindex_targets(&sqlite, &es);
    info!(targets = ?reindex_target_names, "Reindex writes to search targets");
    let ops = composite_ops(
        composite.clone(),
        sqlite.clone(),
        reindex_targets,
        sqlite.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, Some(sqlite.clone())));
    // Bulk ingestion runs on the SQLite primary's engine, but wrapped so that
    // finished manifests sync their resources into Elasticsearch — the raw
    // primary skips local indexing when search is offloaded, and without the
    // wrapper bulk-loaded data is invisible to every search (#882). In
    // fast-load mode the post-manifest reindex already rebuilds Elasticsearch,
    // so the wrapper skips its per-resource sync rather than doing it twice
    // (#903), but stays in the chain to mirror rollbacks (#1161).
    let submit_jobs = composite_submit_jobs(
        sqlite.clone(),
        sqlite.clone(),
        composite.clone(),
        vec![es.clone()],
        &config.bulk_submit,
        reindex_hook.is_some(),
    );
    let submit_bundle = build_bulk_submit(
        &config,
        submit_jobs,
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    let served = serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await;
    checkpoint_sqlite_wal_at_shutdown(&wal_backend).await;
    served
}

/// Fallback when elasticsearch feature is not enabled.
#[cfg(not(all(feature = "sqlite", feature = "elasticsearch")))]
async fn start_sqlite_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite-elasticsearch backend requires the 'elasticsearch' feature. \
         Build with: cargo build -p helios-hfs --features sqlite,elasticsearch"
    )
}

/// Selects reindex width only for the exact standalone PostgreSQL mode.
#[cfg(feature = "postgres")]
fn postgres_reindex_selection(
    config: &ServerConfig,
    pool_max_connections: usize,
) -> (Option<StorageBackendMode>, usize, usize) {
    let mode = config.storage_backend_mode().ok();
    let n = config
        .bulk_submit
        .effective_file_concurrency(BackendKind::Postgres) as usize;
    let width = if mode == Some(StorageBackendMode::Postgres) {
        n.min(pool_max_connections.max(1))
    } else {
        1
    };
    (mode, n, width)
}

/// Starts the server with PostgreSQL backend.
#[cfg(feature = "postgres")]
async fn start_postgres(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let backend = create_postgres_backend(&config).await?;
    let (exact_mode, requested, width) =
        postgres_reindex_selection(&config, backend.config().max_connections);
    let backend = backend.with_reindex_concurrency(width);
    info!(
        storage_mode = ?exact_mode,
        file_concurrency = requested,
        reindex_width = width,
        concurrent_reindex = width > 1,
        "Selected PostgreSQL reindex scheduling"
    );

    backend.init_schema().await?;
    let backend = Arc::new(backend);
    register_pg_pool_metrics("primary", backend.clone());
    attach_login_sessions(auth_state.as_ref(), backend.clone());
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*backend, &config, Some(observability.observers.as_ref())).await;
    spawn_postgres_search_param_refresh(backend.clone(), &config);

    let serve_audit_state = audit_state.clone();
    // The PostgreSQL backend also hosts the per-user settings store, so it always
    // keeps ownership of the backend Arc and uses the settings-capable builder.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(backend.clone());
    let export_bundle = build_bulk_export(&config, backend.clone(), backend.clone()).await?;
    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, None));
    let submit_bundle = build_bulk_submit(
        &config,
        backend.clone(),
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own).
    serve(
        app,
        &config,
        serve_audit_state,
        Some(backend),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when postgres feature is not enabled.
#[cfg(not(feature = "postgres"))]
async fn start_postgres(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres backend requires the 'postgres' feature. \
         Build with: cargo build -p helios-hfs --features postgres"
    )
}

/// Starts the server with PostgreSQL + Elasticsearch composite backend.
#[cfg(all(feature = "postgres", feature = "elasticsearch"))]
async fn start_postgres_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create PostgreSQL backend
    let mut backend = create_postgres_backend(&config).await?;

    // Mark search as offloaded before schema initialization. The patient
    // export index is still built: `$export` reads compartments from
    // PostgreSQL even when search is offloaded (#1663).
    backend.set_search_offloaded(true);
    backend.init_schema().await?;

    // Offload search to Elasticsearch
    let pg = Arc::new(backend);
    register_pg_pool_metrics("primary", pg.clone());
    attach_login_sessions(auth_state.as_ref(), pg.clone());
    info!("PostgreSQL search indexing disabled (offloaded to Elasticsearch)");
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_postgres_search_param_refresh(pg.clone(), &config);

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if es_nodes.is_empty() {
        anyhow::bail!(
            "postgres-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        refresh_interval: config.elasticsearch_refresh_interval.clone(),
        write_refresh: es_write_refresh_from_config(&config)?,
        nested_objects_limit: config.elasticsearch_nested_objects_limit,
        max_terms_count: config.elasticsearch_max_terms_count,
        request_timeout_ms: config.elasticsearch_request_timeout_ms,
        bulk_max_bytes: config.elasticsearch_bulk_max_bytes,
        bulk_concurrency: config.elasticsearch_bulk_concurrency,
        contained_sweep_concurrency: config.elasticsearch_contained_sweep_concurrency,
        reindex_refresh: es_reindex_refresh_from_config(&config)?,
        reindex_prepare_threads: config.elasticsearch_reindex_prepare_threads,
        ..Default::default()
    };

    info!(
        nodes = ?redact_urls(&es_nodes),
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // Create ES backend sharing PostgreSQL's search parameter registry
    let es = Arc::new(
        ElasticsearchBackend::start_with_shared_registry(es_config, pg.tenant_registries().clone())
            .await?,
    );
    es_startup_refresh_setup(&es, &config).await;

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("postgres", BackendKind::Postgres)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
        .fhir_version(config.default_fhir_version)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(pg.clone())
        // A failed Elasticsearch sync is counted, logged, and recorded in the
        // primary as "needs reindex" (#1334).
        .with_sync_observer(Arc::new(CompositeSyncMetrics))
        .with_sync_failure_ledger(pg.clone())
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            pg.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: PostgreSQL (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);
    spawn_secondary_sync_repair(composite.clone(), true);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable.
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*composite, &config, Some(observability.observers.as_ref())).await;

    // The per-user settings store lives on the PostgreSQL primary (Elasticsearch
    // is search-only), so it is wired from the underlying `pg` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(pg.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(pg.clone());

    // The worker searches through `composite` (not the bare `pg` primary) so a
    // `_typeFilter` runs against Elasticsearch, the index that actually serves
    // search in this deployment.
    let export_bundle = build_bulk_export(&config, composite.clone(), pg.clone()).await?;
    // Reindex reads from the PostgreSQL primary and writes only the indexes a
    // query can read: Elasticsearch alone while search is offloaded (#939).
    let (reindex_targets, reindex_target_names) = postgres_es_reindex_targets(&pg, &es);
    info!(targets = ?reindex_target_names, "Reindex writes to search targets");
    let ops = composite_ops(
        composite.clone(),
        pg.clone(),
        reindex_targets,
        pg.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, None));
    // Wrapped like sqlite-es: finished manifests sync their ingested
    // resources into Elasticsearch, which the raw primary never does (#882),
    // unless fast-load's post-manifest reindex covers it (#903); rollbacks are
    // mirrored to Elasticsearch either way (#1161).
    let submit_jobs = composite_submit_jobs(
        pg.clone(),
        pg.clone(),
        composite.clone(),
        vec![es.clone()],
        &config.bulk_submit,
        reindex_hook.is_some(),
    );
    let submit_bundle = build_bulk_submit(
        &config,
        submit_jobs,
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when postgres+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "postgres", feature = "elasticsearch")))]
async fn start_postgres_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres-elasticsearch backend requires both 'postgres' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features postgres,elasticsearch"
    )
}

/// Starts the server with MongoDB + Elasticsearch composite backend.
#[cfg(all(feature = "mongodb", feature = "elasticsearch"))]
async fn start_mongodb_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create MongoDB backend
    let backend_config = build_mongodb_config(&config, true)?;
    info!(
        url = %redact_database_url(&backend_config.connection_string, "mongodb"),
        database = %backend_config.database_name,
        "Initializing MongoDB backend"
    );
    let backend = MongoBackend::new(backend_config)?;

    backend.init_schema().await?;

    // Offload search to Elasticsearch
    let mongo = Arc::new(backend);
    attach_login_sessions(auth_state.as_ref(), mongo.clone());
    info!("MongoDB search indexing disabled (offloaded to Elasticsearch)");
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_mongodb_search_param_refresh(mongo.clone(), &config);

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if es_nodes.is_empty() {
        anyhow::bail!(
            "mongodb-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        refresh_interval: config.elasticsearch_refresh_interval.clone(),
        write_refresh: es_write_refresh_from_config(&config)?,
        nested_objects_limit: config.elasticsearch_nested_objects_limit,
        max_terms_count: config.elasticsearch_max_terms_count,
        request_timeout_ms: config.elasticsearch_request_timeout_ms,
        bulk_max_bytes: config.elasticsearch_bulk_max_bytes,
        bulk_concurrency: config.elasticsearch_bulk_concurrency,
        contained_sweep_concurrency: config.elasticsearch_contained_sweep_concurrency,
        reindex_refresh: es_reindex_refresh_from_config(&config)?,
        reindex_prepare_threads: config.elasticsearch_reindex_prepare_threads,
        ..Default::default()
    };

    info!(
        nodes = ?redact_urls(&es_nodes),
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // Create ES backend sharing MongoDB's search parameter registry
    let es = Arc::new(
        ElasticsearchBackend::start_with_shared_registry(
            es_config,
            mongo.tenant_registries().clone(),
        )
        .await?,
    );
    es_startup_refresh_setup(&es, &config).await;

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("mongodb", BackendKind::MongoDB)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
        .fhir_version(config.default_fhir_version)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "mongodb".to_string(),
        mongo.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "mongodb".to_string(),
        mongo.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(mongo.clone())
        // A failed Elasticsearch sync is counted, logged, and recorded in the
        // primary as "needs reindex" (#1334).
        .with_sync_observer(Arc::new(CompositeSyncMetrics))
        .with_sync_failure_ledger(mongo.clone())
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            mongo.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: MongoDB (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);
    spawn_secondary_sync_repair(composite.clone(), true);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable.
    let observability = helios_rest::WriteObservability::new();
    seed_conformance_resources(&*composite, &config, Some(observability.observers.as_ref())).await;

    // The per-user settings store lives on the MongoDB primary (Elasticsearch is
    // search-only), so it is wired from the underlying `mongo` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(mongo.clone());
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(mongo.clone());

    // MongoDB primary; embedded SQLite sidecar for bulk-export job state.
    let export_bundle = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            // The worker searches through `composite` (not the bare `mongo`
            // primary) so a `_typeFilter` runs against Elasticsearch, the
            // index that actually serves search in this deployment.
            build_bulk_export(&config, composite.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };
    let ops = composite_ops(
        composite.clone(),
        mongo.clone(),
        vec![mongo.clone(), es.clone()],
        mongo.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, None));
    // Bulk submit runs against the MongoDB primary, which hosts its own job
    // state, but wrapped like sqlite-es and pg-es so finished manifests sync
    // their resources into Elasticsearch (#882). The comment this replaces said
    // the composite's search half was "fed by the primary's own indexing
    // hooks" — on this backend those hooks are exactly what is turned off
    // (`search_offloaded`, logged above as "MongoDB search indexing disabled"),
    // so nothing indexed the bulk-loaded data anywhere and 99.9 % of an import
    // was readable by id and invisible to every search (#1021). Under fast-load
    // the wrapper skips its per-resource sync but still mirrors rollbacks
    // (#903, #1161).
    let submit_jobs = composite_submit_jobs(
        mongo.clone(),
        mongo.clone(),
        composite.clone(),
        vec![es.clone()],
        &config.bulk_submit,
        reindex_hook.is_some(),
    );
    let submit_bundle = build_bulk_submit(
        &config,
        submit_jobs,
        reindex_hook,
        observability.observers.clone(),
    )
    .await?;
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when mongodb+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "mongodb", feature = "elasticsearch")))]
async fn start_mongodb_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The mongodb-elasticsearch backend requires both 'mongodb' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features mongodb,elasticsearch"
    )
}

/// Starts the server with AWS S3 backend.
#[cfg(feature = "s3")]
async fn start_s3(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use helios_persistence::backends::s3::S3Backend;

    let s3_config = s3_backend_config_from_env(&config.default_tenant)?;
    let tenancy = describe_s3_tenancy(&s3_config.tenancy_mode);

    info!(
        tenancy = %tenancy,
        region = ?s3_config.region,
        prefix = ?s3_config.prefix,
        endpoint_url = ?s3_config.endpoint_url.as_deref().map(redact_url),
        validate_buckets = s3_config.validate_buckets_on_startup,
        keyspace_layout = ?s3_config.default_keyspace_layout,
        scan_concurrency = s3_config.scan_concurrency,
        "Initializing S3 backend"
    );

    let backend = S3Backend::from_env_async(s3_config).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize S3 backend ({}, region={:?}): {}",
            tenancy,
            std::env::var("AWS_REGION").ok(),
            e
        )
    })?;

    let backend = Arc::new(backend.with_fhir_version(config.default_fhir_version));
    if backend.supports_user_settings() {
        attach_login_sessions(auth_state.as_ref(), backend.clone());
    } else {
        warn_login_sessions_in_process(auth_state.as_ref());
    }
    let serve_audit_state = audit_state.clone();
    // Its REST writes, bulk submit, UI purges and the conformance seed below
    // all report to the one write observer (#1078).
    let observability = helios_rest::WriteObservability::new();
    // Bucket-per-tenant: the mapped tenants are the provisioned tenants.
    provision_mapped_tenants(
        &*backend,
        &backend.mapped_tenants(),
        &config,
        Some(observability.observers.as_ref()),
    )
    .await;
    // Standalone S3 lists SearchParameter and CompartmentDefinition by scan
    // (#1822), and the scan only finds what is stored (#1838). The seed is
    // idempotent: a tenant that already holds the set costs one LIST-backed
    // count per type on later starts.
    seed_conformance_resources(&*backend, &config, Some(observability.observers.as_ref())).await;

    // Second handle to the same backend (S3Backend clones share the client)
    // for the web UI's tenant-maintenance read/write path.
    let ui_tenants: Option<Arc<dyn ResourceStorage>> =
        Some(backend.clone() as Arc<dyn ResourceStorage>);

    // The S3 backend also hosts the per-user settings store (a compare-and-swap
    // over conditional PutObject). Bulk *export* is still not wired on a
    // standalone S3 primary, which has no export job store.
    //
    // A bucket-per-tenant configuration with no `default_system_bucket` has
    // nowhere tenant-independent to keep a user-global document, so the store is
    // left unwired and `/_user/settings` reports the explained 501 rather than
    // failing every request.
    let settings_store: Option<Arc<dyn SettingsStore>> = if backend.supports_user_settings() {
        Some(backend.clone())
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             per-user settings (/_user/settings) will report 501 Not Implemented"
        );
        None
    };
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(backend.clone());

    // S3 standalone can purge, but it has NO search index of any kind — its
    // SearchProvider reports search unsupported — so `$reindex` has nothing to
    // rebuild and the handler reports 501 rather than accepting a job that
    // would do nothing. This is the one deployment where a 501 is the honest
    // answer.
    let ops = OperationsBundle {
        purge: Some(backend.clone() as Arc<dyn PurgableStorage>),
        reindex: None,
        observability: Some(observability.clone()),
    };

    // Bulk submit *is* wired: S3 keeps the submission, manifest, lease, and
    // artifact state in the same objects its ingestion engine already writes,
    // compare-and-swapped over conditional PutObject. The one configuration
    // that cannot is bucket-per-tenant with no `default_system_bucket`, which
    // has nowhere to keep the cross-tenant claim queue and poll-token index —
    // there the backend does not declare `BulkSubmitRestWorker` and the worker
    // simply never claims anything.
    let submit_bundle = if backend.supports_bulk_submit_worker() {
        build_bulk_submit(
            &config,
            backend.clone(),
            ops.reindex.clone().map(|op| {
                Arc::new(
                    helios_persistence::search::ReindexOnFinish::new(op)
                        .with_batch_size(config.reindex_batch_size)
                        .with_batch_bytes(config.reindex_batch_bytes)
                        .with_bulk_index_rebuild(config.bulk_submit.bulk_index_rebuild),
                ) as Arc<dyn helios_persistence::core::DeferredReindexHook>
            }),
            observability.observers.clone(),
        )
        .await?
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             $bulk-submit will report 501 Not Implemented"
        );
        None
    };

    let app = create_app_with_auth_bulk_settings_and_ops(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        None,
        submit_bundle,
        settings_store,
        ops,
    );
    serve(
        app,
        &config,
        serve_audit_state,
        ui_tenants,
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when s3 feature is not enabled.
#[cfg(not(feature = "s3"))]
async fn start_s3(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The s3 backend requires the 's3' feature. \
         Build with: cargo build -p helios-hfs --features s3"
    )
}

/// Starts the server with S3 + Elasticsearch composite backend.
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
async fn start_s3_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::backends::s3::S3Backend;
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // --- S3 backend (primary) ---
    let s3_config = s3_backend_config_from_env(&config.default_tenant)?;
    let tenancy = describe_s3_tenancy(&s3_config.tenancy_mode);

    info!(
        tenancy = %tenancy,
        region = ?s3_config.region,
        prefix = ?s3_config.prefix,
        endpoint_url = ?s3_config.endpoint_url.as_deref().map(redact_url),
        validate_buckets = s3_config.validate_buckets_on_startup,
        keyspace_layout = ?s3_config.default_keyspace_layout,
        scan_concurrency = s3_config.scan_concurrency,
        "Initializing S3 backend (primary)"
    );

    let s3 = Arc::new(
        S3Backend::from_env_async(s3_config)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to initialize S3 backend ({}, region={:?}): {}",
                    tenancy,
                    std::env::var("AWS_REGION").ok(),
                    e
                )
            })?
            .with_fhir_version(config.default_fhir_version),
    );
    let mapped_tenants = s3.mapped_tenants();
    if s3.supports_user_settings() {
        attach_login_sessions(auth_state.as_ref(), s3.clone());
    } else {
        warn_login_sessions_in_process(auth_state.as_ref());
    }
    // Refresh reads from the primary; the ES backend shares its registry Arc
    // (wired below, once it's populated). Seeding waits for the composite
    // further down, so the writes also index into ES.
    spawn_s3_search_param_refresh(s3.clone(), &config);

    // --- Elasticsearch backend (search) ---
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if es_nodes.is_empty() {
        anyhow::bail!(
            "s3-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        refresh_interval: config.elasticsearch_refresh_interval.clone(),
        write_refresh: es_write_refresh_from_config(&config)?,
        nested_objects_limit: config.elasticsearch_nested_objects_limit,
        max_terms_count: config.elasticsearch_max_terms_count,
        request_timeout_ms: config.elasticsearch_request_timeout_ms,
        bulk_max_bytes: config.elasticsearch_bulk_max_bytes,
        bulk_concurrency: config.elasticsearch_bulk_concurrency,
        contained_sweep_concurrency: config.elasticsearch_contained_sweep_concurrency,
        reindex_refresh: es_reindex_refresh_from_config(&config)?,
        reindex_prepare_threads: config.elasticsearch_reindex_prepare_threads,
        ..Default::default()
    };

    info!(
        nodes = ?redact_urls(&es_nodes),
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend (search)"
    );

    // Populate S3's own per-tenant registry container with the shared base
    // (embedded + spec + custom, e.g. the SQL-on-FHIR `ViewDefinition` params) —
    // S3 has no `data_dir` and does not load base params during construction,
    // so a composite starter does it once here, the same three tiers the SQLite,
    // PostgreSQL, and MongoDB primaries load into theirs. Omitting the custom
    // tier left ES blind to `ViewDefinition?name:contains=…` (#1070). Unlike
    // before, this container is *S3's real registries* (`s3.tenant_registries()`),
    // not a throwaway one: S3's own create/update/delete hooks now keep each
    // tenant's stored SearchParameter overlay on it current (#787), so sharing
    // it with Elasticsearch below gives ES the same live overlay every other
    // composite's search backend already gets from its primary.
    {
        let data_dir = config
            .data_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("./data"));
        populate_base_search_registry(
            &mut s3.tenant_registries().base().write(),
            config.default_fhir_version,
            &data_dir,
        );
    }
    let es = Arc::new(
        ElasticsearchBackend::start_with_shared_registry(es_config, s3.tenant_registries().clone())
            .await?,
    );
    es_startup_refresh_setup(&es, &config).await;

    // --- Composite wiring ---
    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
        .fhir_version(config.default_fhir_version)
        .build()?;

    let mut backends = HashMap::new();
    backends.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(s3.clone())
        // S3 keeps no needs-reindex ledger: a failed Elasticsearch sync is
        // counted and logged, and repaired by `$reindex` (#1334).
        .with_sync_observer(Arc::new(CompositeSyncMetrics))
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            s3.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: S3 (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);
    spawn_secondary_sync_repair(composite.clone(), false);

    // Seed through the composite so the conformance resources land in the S3
    // primary and get indexed into Elasticsearch — the only search index here.
    let observability = helios_rest::WriteObservability::new();
    provision_mapped_tenants(
        &*composite,
        &mapped_tenants,
        &config,
        Some(observability.observers.as_ref()),
    )
    .await;
    seed_conformance_resources(&*composite, &config, Some(observability.observers.as_ref())).await;

    // The per-user settings store lives on the S3 primary (Elasticsearch is
    // search-only), so it is wired from the underlying `s3` backend even though
    // the app is served over the composite storage. As in `start_s3`, a tenancy
    // mode with no tenant-independent bucket leaves it unwired (explained 501).
    let settings_store: Option<Arc<dyn SettingsStore>> = if s3.supports_user_settings() {
        Some(s3.clone())
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             per-user settings (/_user/settings) will report 501 Not Implemented"
        );
        None
    };
    let ui_settings = settings_store.clone();
    let ui_bulk_provider: Option<Arc<dyn BulkProviderStore>> = Some(s3.clone());

    // Reindex reads from the S3 primary and writes to Elasticsearch, which is
    // the only search index in this deployment — S3 maintains none. The
    // extractor is Elasticsearch's for the same reason.
    let ops = composite_ops(
        composite.clone(),
        s3.clone(),
        vec![es.clone()],
        es.tenant_registries().clone(),
        audit_state.as_ref(),
        observability.clone(),
        config.reindex_prefetch,
    );

    // S3 primary; embedded SQLite sidecar for bulk-export job state. A single
    // builder call covers both the bulk-enabled and bulk-disabled cases, so
    // `ops` and `settings_store` are each consumed exactly once.
    let bulk_export = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            // The worker searches through `composite` (not the bare `s3`
            // primary) so a `_typeFilter` runs against Elasticsearch, the
            // only search index in this deployment.
            build_bulk_export(&config, composite.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };
    // Bulk submit needs no sidecar here either: the S3 primary hosts its own
    // job state. It is wrapped like every other composite (#882): S3 maintains
    // no search index of its own — Elasticsearch is the only one in this
    // deployment, as the `ops` comment above says — so the primary has no
    // indexing hooks for the composite's search half to be "fed by", which is
    // what the comment this replaces claimed. Without the wrapper a completed
    // `$bulk-submit` here leaves its resources searchable nowhere (#1021).
    // Under fast-load the wrapper skips its per-resource sync but still mirrors
    // rollbacks (#903, #1161).
    let reindex_hook = ops
        .reindex
        .clone()
        .map(|op| automatic_reindex_hook_with_ledger(op, &config, None));
    let bulk_submit = if s3.supports_bulk_submit_worker() {
        let submit_jobs = composite_submit_jobs(
            s3.clone(),
            s3.clone(),
            composite.clone(),
            vec![es.clone()],
            &config.bulk_submit,
            reindex_hook.is_some(),
        );
        build_bulk_submit(
            &config,
            submit_jobs,
            reindex_hook,
            observability.observers.clone(),
        )
        .await?
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             $bulk-submit will report 501 Not Implemented"
        );
        None
    };

    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        bulk_export,
        bulk_submit,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the bare
    // primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
        ui_bulk_provider,
        Some(observability.observers.clone() as Arc<dyn WriteObserver>),
    )
    .await
}

/// Fallback when s3+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "s3", feature = "elasticsearch")))]
async fn start_s3_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The s3-elasticsearch backend requires both 's3' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features s3,elasticsearch"
    )
}

#[cfg(not(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
)))]
compile_error!("At least one database backend feature must be enabled");

#[cfg(test)]
mod tests {
    use super::*;
    use helios_audit::AuditConfig;
    use helios_rest::ServerConfig;

    #[test]
    fn redact_urls_drops_credentials_and_keeps_hosts() {
        let nodes = vec![
            "https://elastic:NODE-SECRET@es1.internal:9200".to_string(),
            "http://es2.internal:9200/?token=QUERY-SECRET".to_string(),
            "http://es3.internal:9200".to_string(),
        ];
        let logged = format!("{:?}", redact_urls(&nodes));
        assert!(!logged.contains("SECRET"), "{logged}");
        for host in [
            "es1.internal:9200",
            "es2.internal:9200",
            "es3.internal:9200",
        ] {
            assert!(logged.contains(host), "`{host}` missing from {logged}");
        }
    }

    #[test]
    fn non_blank_endpoint_drops_unset_and_blank_values() {
        assert_eq!(non_blank_endpoint(None), None);
        assert_eq!(non_blank_endpoint(Some(String::new())), None);
        assert_eq!(non_blank_endpoint(Some("   ".to_string())), None);
        assert_eq!(
            non_blank_endpoint(Some(" http://localhost:9000 ".to_string())),
            Some("http://localhost:9000".to_string())
        );
    }

    // ── S3 tenancy from the environment (#1598) ───────────────────

    #[cfg(feature = "s3")]
    mod s3_tenancy {
        use super::*;
        use helios_persistence::backends::s3::S3TenancyMode;

        fn env(
            tenancy: Option<&str>,
            bucket: Option<&str>,
            tenant_buckets: Option<&str>,
            system_bucket: Option<&str>,
        ) -> S3TenancyEnv {
            S3TenancyEnv {
                tenancy: tenancy.map(str::to_string),
                bucket: bucket.map(str::to_string),
                tenant_buckets: tenant_buckets.map(str::to_string),
                system_bucket: system_bucket.map(str::to_string),
            }
        }

        fn err(env: &S3TenancyEnv) -> String {
            env.resolve("default")
                .expect_err("expected a configuration error")
                .to_string()
        }

        #[test]
        fn unset_environment_is_prefix_per_tenant_on_the_hfs_bucket() {
            let mode = env(None, None, None, None).resolve("default").unwrap();
            assert!(matches!(mode, S3TenancyMode::PrefixPerTenant { bucket } if bucket == "hfs"));
        }

        #[test]
        fn prefix_mode_takes_hfs_s3_bucket_and_accepts_spelling_variants() {
            for spelling in [
                "prefix-per-tenant",
                "PREFIX_PER_TENANT",
                " Prefix-Per-Tenant ",
            ] {
                let mode = env(Some(spelling), Some("my-bucket"), None, None)
                    .resolve("default")
                    .unwrap();
                assert!(
                    matches!(mode, S3TenancyMode::PrefixPerTenant { ref bucket } if bucket == "my-bucket"),
                    "{spelling}: {mode:?}"
                );
            }
        }

        #[test]
        fn unknown_tenancy_is_refused() {
            let message = err(&env(Some("bucket"), None, None, None));
            assert!(
                message.contains("HFS_S3_TENANCY 'bucket' invalid"),
                "{message}"
            );
            assert!(message.contains("bucket-per-tenant"), "{message}");
        }

        /// A bucket-per-tenant variable under prefix mode is a misconfiguration
        /// to report, not to ignore: the operator meant something by it.
        #[test]
        fn prefix_mode_refuses_bucket_per_tenant_variables() {
            let message = err(&env(None, None, Some("acme=acme-bucket"), None));
            assert!(
                message.starts_with("HFS_S3_TENANT_BUCKETS applies only"),
                "{message}"
            );

            let message = err(&env(Some("prefix-per-tenant"), None, None, Some("sys")));
            assert!(
                message.starts_with("HFS_S3_SYSTEM_BUCKET applies only"),
                "{message}"
            );
        }

        #[test]
        fn bucket_mode_requires_the_tenant_map_and_refuses_hfs_s3_bucket() {
            let message = err(&env(Some("bucket-per-tenant"), None, None, None));
            assert!(
                message.starts_with("HFS_S3_TENANT_BUCKETS is required"),
                "{message}"
            );

            let message = err(&env(
                Some("bucket-per-tenant"),
                Some("hfs"),
                Some("default=d"),
                None,
            ));
            assert!(
                message.starts_with("HFS_S3_BUCKET='hfs' has no meaning"),
                "{message}"
            );
        }

        #[test]
        fn bucket_mode_builds_the_map_and_optional_system_bucket() {
            let mode = env(
                Some("bucket-per-tenant"),
                None,
                Some(" default = hfs-default , acme=hfs-acme,globex=hfs-globex, "),
                Some("hfs-system"),
            )
            .resolve("default")
            .unwrap();
            let S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            } = mode
            else {
                panic!("expected bucket-per-tenant, got {mode:?}");
            };
            assert_eq!(tenant_bucket_map.len(), 3);
            assert_eq!(tenant_bucket_map["default"], "hfs-default");
            assert_eq!(tenant_bucket_map["acme"], "hfs-acme");
            assert_eq!(tenant_bucket_map["globex"], "hfs-globex");
            assert_eq!(default_system_bucket.as_deref(), Some("hfs-system"));

            // No system bucket is a valid, if reduced, configuration: the
            // library leaves the registry, settings and bulk-submit worker
            // unwired and the binary warns.
            let mode = env(Some("bucket-per-tenant"), None, Some("default=d"), None)
                .resolve("default")
                .unwrap();
            assert!(matches!(
                mode,
                S3TenancyMode::BucketPerTenant {
                    default_system_bucket: None,
                    ..
                }
            ));
        }

        /// An unmapped default tenant would fail every tenant-less request
        /// after a clean start; refuse at boot instead.
        #[test]
        fn bucket_mode_requires_the_default_tenant_to_be_mapped() {
            let message = env(
                Some("bucket-per-tenant"),
                None,
                Some("acme=hfs-acme"),
                Some("hfs-system"),
            )
            .resolve("main")
            .unwrap_err()
            .to_string();
            assert!(
                message.starts_with("HFS_DEFAULT_TENANT 'main' has no bucket"),
                "{message}"
            );
        }

        #[test]
        fn tenant_map_entries_are_validated() {
            let cases = [
                ("acme", "is not of the form tenant=bucket"),
                ("acme=", "names no bucket"),
                ("=hfs-acme", "tenant id '' is not valid"),
                ("acme=a,acme=b", "maps tenant 'acme' more than once"),
                ("__system__=sys", "tenant id '__system__' is not valid"),
                ("a/resources=x", "tenant id 'a/resources' is not valid"),
                (" , ", "names no tenant"),
            ];
            for (spec, expected) in cases {
                let message = parse_s3_tenant_buckets(spec).expect_err(spec).to_string();
                assert!(message.contains(expected), "{spec}: {message}");
            }
        }

        #[test]
        fn describe_is_deterministic_and_names_every_bucket() {
            let mode = env(
                Some("bucket-per-tenant"),
                None,
                Some("zeta=z,default=d,alpha=a"),
                None,
            )
            .resolve("default")
            .unwrap();
            let text = describe_s3_tenancy(&mode);
            assert_eq!(
                text,
                "bucket-per-tenant tenants=[alpha=a,default=d,zeta=z] system_bucket=(none)"
            );
            for _ in 0..8 {
                assert_eq!(describe_s3_tenancy(&mode), text);
            }
            assert_eq!(
                describe_s3_tenancy(&S3TenancyMode::PrefixPerTenant {
                    bucket: "hfs".to_string()
                }),
                "prefix-per-tenant bucket=hfs"
            );
        }
    }

    #[cfg(feature = "s3")]
    mod s3_keyspace_layout {
        use super::*;
        use helios_persistence::backends::s3::KeyspaceLayout;

        #[test]
        fn unset_or_blank_is_the_flat_layout() {
            assert_eq!(parse_s3_keyspace_layout(None).unwrap(), KeyspaceLayout::V1);
            assert_eq!(
                parse_s3_keyspace_layout(Some("  ")).unwrap(),
                KeyspaceLayout::V1
            );
        }

        #[test]
        fn v1_and_v2_parse_case_insensitively() {
            assert_eq!(
                parse_s3_keyspace_layout(Some("v1")).unwrap(),
                KeyspaceLayout::V1
            );
            assert_eq!(
                parse_s3_keyspace_layout(Some(" V2 ")).unwrap(),
                KeyspaceLayout::V2
            );
        }

        #[test]
        fn anything_else_is_refused_with_the_expected_set() {
            for bad in ["v3", "2", "sharded", "v1,v2"] {
                let err = parse_s3_keyspace_layout(Some(bad)).unwrap_err().to_string();
                assert!(err.contains("HFS_S3_KEYSPACE_LAYOUT"), "{err}");
                assert!(err.contains("expected v1|v2"), "{err}");
            }
        }
    }

    #[cfg(feature = "s3")]
    mod s3_scan_concurrency {
        use super::*;

        #[test]
        fn unset_or_blank_is_the_default() {
            assert_eq!(parse_s3_scan_concurrency(None).unwrap(), 32);
            assert_eq!(parse_s3_scan_concurrency(Some("  ")).unwrap(), 32);
        }

        #[test]
        fn whole_numbers_of_at_least_one_parse() {
            assert_eq!(parse_s3_scan_concurrency(Some("1")).unwrap(), 1);
            assert_eq!(parse_s3_scan_concurrency(Some(" 8 ")).unwrap(), 8);
        }

        #[test]
        fn zero_negative_and_junk_are_refused() {
            for bad in ["0", "-1", "abc", "1.5", "32x"] {
                let err = parse_s3_scan_concurrency(Some(bad))
                    .unwrap_err()
                    .to_string();
                assert!(err.contains("HFS_S3_SCAN_CONCURRENCY"), "{err}");
                assert!(err.contains("at least 1"), "{err}");
            }
        }
    }

    // ── PostgreSQL pool gauges (#1907) ────────────────────────────

    /// Each pool field lands in its own gauge; distinct values catch a swap.
    #[cfg(feature = "postgres")]
    #[test]
    fn pg_pool_gauges_maps_each_status_field() {
        use helios_observability::pool_metrics::PoolGauges;
        use helios_persistence::backends::postgres::PoolStatus;

        let gauges = pg_pool_gauges(PoolStatus {
            max_size: 4,
            size: 3,
            available: 2,
            waiting: 1,
        });
        assert_eq!(
            gauges,
            PoolGauges {
                max_size: 4,
                size: 3,
                available: 2,
                waiting: 1,
            }
        );
    }

    // ── Elasticsearch rebuild wiring (#1125) ──────────────────────

    #[cfg(feature = "elasticsearch")]
    #[test]
    fn test_es_refresh_wait_warning() {
        use helios_persistence::backends::elasticsearch::WriteRefreshPolicy::{False, WaitFor};
        use helios_persistence::composite::SyncMode::{Asynchronous, Synchronous};

        // Defaults, and anything that is not wait_for, are quiet.
        assert_eq!(
            es_refresh_wait_warning(WaitFor, "1s", 30_000, 5_000, Synchronous),
            None
        );
        assert_eq!(
            es_refresh_wait_warning(False, "-1", 30_000, 5_000, Synchronous),
            None
        );
        assert_eq!(
            es_refresh_wait_warning(WaitFor, "bogus", 30_000, 5_000, Synchronous),
            None
        );

        let disabled = es_refresh_wait_warning(WaitFor, "-1", 30_000, 5_000, Synchronous)
            .expect("a disabled interval stalls wait_for writes");
        assert!(
            disabled.contains("HFS_ELASTICSEARCH_REFRESH_INTERVAL"),
            "{disabled}"
        );

        let timeout = es_refresh_wait_warning(WaitFor, "30s", 30_000, 5_000, Asynchronous)
            .expect("an interval at the request timeout");
        assert!(
            timeout.contains("HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS"),
            "{timeout}"
        );

        let status = es_refresh_wait_warning(WaitFor, "10s", 30_000, 5_000, Synchronous)
            .expect("an interval above the subscription status budget");
        assert!(
            status.contains("HFS_SUBSCRIPTION_STATUS_WRITE_TIMEOUT_MS"),
            "{status}"
        );
        // Asynchronous writes do not hold up status transitions.
        assert_eq!(
            es_refresh_wait_warning(WaitFor, "10s", 30_000, 5_000, Asynchronous),
            None
        );
    }

    #[cfg(feature = "elasticsearch")]
    #[test]
    fn test_es_reindex_refresh_follows_write_refresh_when_unset() {
        use helios_persistence::backends::elasticsearch::WriteRefreshPolicy;

        for unset in [None, Some(String::new()), Some("  ".to_string())] {
            let config = ServerConfig {
                elasticsearch_reindex_refresh: unset,
                ..Default::default()
            };
            assert_eq!(es_reindex_refresh_from_config(&config).unwrap(), None);
        }
        let config = ServerConfig {
            elasticsearch_write_refresh: "wait_for".to_string(),
            elasticsearch_reindex_refresh: Some("false".to_string()),
            ..Default::default()
        };
        assert_eq!(
            es_write_refresh_from_config(&config).unwrap(),
            WriteRefreshPolicy::WaitFor
        );
        assert_eq!(
            es_reindex_refresh_from_config(&config).unwrap(),
            Some(WriteRefreshPolicy::False)
        );
        let config = ServerConfig {
            elasticsearch_reindex_refresh: Some("sometimes".to_string()),
            ..Default::default()
        };
        let error = es_reindex_refresh_from_config(&config).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("HFS_ELASTICSEARCH_REINDEX_REFRESH"),
            "{error}"
        );
    }

    #[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
    #[test]
    fn test_sqlite_es_reindex_skips_an_offloaded_primary() {
        use helios_persistence::backends::elasticsearch::{
            ElasticsearchBackend, ElasticsearchConfig,
        };

        // Never contacted: building the client does not connect.
        let es = Arc::new(
            ElasticsearchBackend::new(ElasticsearchConfig {
                nodes: vec!["http://127.0.0.1:1".to_string()],
                ..Default::default()
            })
            .expect("ES backend builds without a cluster"),
        );
        let config = ServerConfig {
            database_url: Some(":memory:".to_string()),
            ..Default::default()
        };

        let mut offloaded = create_sqlite_backend(&config).unwrap();
        offloaded.set_search_offloaded(true);
        let (targets, names) = sqlite_es_reindex_targets(&Arc::new(offloaded), &es);
        assert_eq!(names, &["elasticsearch"]);
        assert_eq!(targets.len(), 1);

        let local = Arc::new(create_sqlite_backend(&config).unwrap());
        let (targets, names) = sqlite_es_reindex_targets(&local, &es);
        assert_eq!(names, &["sqlite", "elasticsearch"]);
        assert_eq!(targets.len(), 2);
    }

    /// The rule pg-es shares with sqlite-es (#939 C3): an offloaded primary is
    /// not a reindex target. `PostgresBackend` cannot be built without a
    /// database, so the shared rule is what gets exercised here; pg-es feeds it
    /// `is_search_offloaded()`, which its startup sets to `true`.
    #[cfg(all(
        feature = "elasticsearch",
        any(feature = "sqlite", feature = "postgres")
    ))]
    #[test]
    fn test_es_only_when_offloaded_drops_the_primary() {
        use helios_persistence::backends::elasticsearch::{
            ElasticsearchBackend, ElasticsearchConfig,
        };
        use helios_persistence::search::ReindexTarget;

        // Any two targets do; building the ES client does not connect.
        let target = |port: u16| -> Arc<dyn ReindexTarget> {
            Arc::new(
                ElasticsearchBackend::new(ElasticsearchConfig {
                    nodes: vec![format!("http://127.0.0.1:{port}")],
                    ..Default::default()
                })
                .expect("ES backend builds without a cluster"),
            )
        };
        let both: &'static [&'static str] = &["postgres", "elasticsearch"];

        let (targets, names) = es_only_when_offloaded(true, target(1), both, target(2));
        assert_eq!(names, &["elasticsearch"]);
        assert_eq!(targets.len(), 1);

        let (targets, names) = es_only_when_offloaded(false, target(1), both, target(2));
        assert_eq!(names, both);
        assert_eq!(targets.len(), 2);
    }

    // ── Automatic reindex hook wiring (#1499) ──────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_automatic_reindex_hook_gets_the_server_default_batch_bytes_when_unset() {
        use clap::Parser;

        let config = ServerConfig::try_parse_from(["rest-server"]).unwrap();
        assert_eq!(
            config.reindex_batch_bytes,
            32 * 1024 * 1024,
            "HFS_REINDEX_BATCH_BYTES server default (#1499)"
        );

        let backend = Arc::new(
            create_sqlite_backend(&ServerConfig {
                database_url: Some(":memory:".to_string()),
                ..Default::default()
            })
            .unwrap(),
        );
        let registries = backend.tenant_registries().clone();
        let op = Arc::new(ReindexOperation::new(backend, registries));

        let hook = build_automatic_reindex_hook(op, &config, None);
        assert_eq!(hook.batch_bytes(), 32 * 1024 * 1024);
    }

    /// `HFS_REINDEX_PREFETCH` reaches the driver through the ops bundle, for
    /// standalone and composite deployments alike (#1250).
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_ops_bundle_passes_the_reindex_prefetch_setting_to_the_operation() {
        let config = ServerConfig {
            database_url: Some(":memory:".to_string()),
            ..Default::default()
        };
        assert!(
            config.reindex_prefetch,
            "HFS_REINDEX_PREFETCH defaults to on"
        );
        for prefetch in [true, false] {
            let backend = Arc::new(create_sqlite_backend(&config).unwrap());
            let registries = backend.tenant_registries().clone();
            let ops = standalone_ops(
                backend,
                registries,
                None,
                helios_rest::WriteObservability::new(),
                prefetch,
            );
            let op = ops
                .reindex
                .expect("standalone SQLite rebuilds its own index");
            assert_eq!(op.prefetch(), prefetch);
        }
    }

    // ── create_sqlite_backend() ───────────────────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_memory() {
        let config = ServerConfig {
            database_url: Some(":memory:".to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        assert!(result.is_ok(), "in-memory SQLite backend should succeed");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_temp_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "hfs_test_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let config = ServerConfig {
            database_url: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        // Clean up the file even if the assertion fails.
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok(), "file-based SQLite backend should succeed");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_default_url() {
        // When database_url is None, defaults to "fhir.db" in the current dir.
        // We redirect to a temp path to avoid creating persistent side-effects.
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "hfs_test_default_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        // Override the default path via the env var that ServerConfig reads.
        let config = ServerConfig {
            database_url: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok());
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_overlays_env_with_mongo_database_url() {
        use helios_fhir::FhirVersion;

        let data_dir = std::path::PathBuf::from("/tmp/hfs-data");
        let config = ServerConfig {
            database_url: Some("mongodb://mongo.example:27017/?replicaSet=rs0".to_string()),
            default_fhir_version: FhirVersion::R4,
            data_dir: Some(data_dir.clone()),
            ..Default::default()
        };

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_DATABASE" => Some("inferno_suite".to_string()),
            "HFS_MONGODB_MAX_CONNECTIONS" => Some("24".to_string()),
            "HFS_MONGODB_CONNECT_TIMEOUT_MS" => Some("7500".to_string()),
            "HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS" => Some("2500".to_string()),
            _ => None,
        })
        .expect("valid config");

        assert_eq!(
            mongo_config.connection_string,
            "mongodb://mongo.example:27017/?replicaSet=rs0"
        );
        assert_eq!(mongo_config.database_name, "inferno_suite");
        assert_eq!(mongo_config.max_connections, 24);
        assert_eq!(mongo_config.connect_timeout_ms, 7500);
        assert_eq!(mongo_config.server_selection_timeout_ms, 2500);
        assert_eq!(mongo_config.fhir_version, FhirVersion::R4);
        assert_eq!(mongo_config.data_dir, Some(data_dir));
        assert!(!mongo_config.search_offloaded);
        assert_eq!(mongo_config.index_build, IndexBuildMode::Background);
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_ignores_non_mongo_database_url() {
        let config = ServerConfig {
            database_url: Some("postgres://localhost/hfs".to_string()),
            ..Default::default()
        };

        let mongo_config = build_mongodb_config_with_env(&config, true, |name| match name {
            "HFS_DATABASE_URL" => Some("postgres://localhost/hfs".to_string()),
            "HFS_MONGODB_DATABASE" => Some("mongo_db".to_string()),
            _ => None,
        })
        .expect("valid config");

        assert_eq!(mongo_config.connection_string, "mongodb://localhost:27017");
        assert_eq!(mongo_config.database_name, "mongo_db");
        assert!(mongo_config.search_offloaded);
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_index_build_and_rejects_invalid_values() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_INDEX_BUILD" => Some("off".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert_eq!(mongo_config.index_build, IndexBuildMode::Off);

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_INDEX_BUILD" => Some("inline".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert_eq!(mongo_config.index_build, IndexBuildMode::Inline);

        let err = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_INDEX_BUILD" => Some("nonsense".to_string()),
            _ => None,
        })
        .expect_err("invalid mode must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_INDEX_BUILD"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_reindex_pipeline_knobs_and_rejects_invalid_values() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_REINDEX_OVERLAP" => Some("false".to_string()),
            "HFS_MONGODB_REINDEX_PREPARE_THREADS" => Some("2".to_string()),
            "HFS_MONGODB_REINDEX_PREFETCH" => Some("off".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert!(!mongo_config.reindex_overlap);
        assert_eq!(mongo_config.reindex_prepare_threads, 2);
        assert!(!mongo_config.reindex_prefetch);

        let default_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert!(default_config.reindex_overlap);
        assert_eq!(default_config.reindex_prepare_threads, 0);
        assert!(default_config.reindex_prefetch);

        let err = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_REINDEX_OVERLAP" => Some("sideways".to_string()),
            _ => None,
        })
        .expect_err("invalid value must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_REINDEX_OVERLAP"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_transaction_bundle_limit_and_rejects_invalid_values() {
        let config = ServerConfig::default();
        let limit = |value: Option<&'static str>| {
            build_mongodb_config_with_env(&config, false, |name| match name {
                "HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES" => value.map(str::to_string),
                _ => None,
            })
        };

        assert_eq!(limit(None).unwrap().max_concurrent_transaction_bundles, 4);
        assert_eq!(
            limit(Some("2")).unwrap().max_concurrent_transaction_bundles,
            2
        );
        assert_eq!(
            limit(Some("0")).unwrap().max_concurrent_transaction_bundles,
            0
        );
        let err = limit(Some("many")).expect_err("invalid value must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_MAX_CONCURRENT_TRANSACTION_BUNDLES"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_broad_search_concurrency_and_rejects_invalid_values() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_BROAD_SEARCH_CONCURRENCY" => Some(" 3 ".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert_eq!(mongo_config.broad_search_concurrency, Some(3));

        let default_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert_eq!(default_config.broad_search_concurrency, None);

        for invalid in ["0", "-1", "two"] {
            let err = build_mongodb_config_with_env(&config, false, |name| match name {
                "HFS_MONGODB_BROAD_SEARCH_CONCURRENCY" => Some(invalid.to_string()),
                _ => None,
            })
            .expect_err("invalid value must fail startup");
            assert!(
                format!("{err}").contains("HFS_MONGODB_BROAD_SEARCH_CONCURRENCY"),
                "{invalid}: {err}"
            );
        }
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_count_by_tenant_max_time_and_rejects_invalid_values() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS" => Some(" 120000 ".to_string()),
            _ => None,
        })
        .expect("valid config");
        assert_eq!(mongo_config.count_by_tenant_max_time_ms, 120_000);

        let default_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert_eq!(default_config.count_by_tenant_max_time_ms, 25_000);

        for invalid in ["0", "-1", "soon", "2147483648"] {
            let err = build_mongodb_config_with_env(&config, false, |name| match name {
                "HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS" => Some(invalid.to_string()),
                _ => None,
            })
            .expect_err("invalid value must fail startup");
            assert!(
                format!("{err}").contains("HFS_MONGODB_COUNT_BY_TENANT_MAX_TIME_MS"),
                "{invalid}: {err}"
            );
        }
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_transaction_bundle_weight_and_rejects_invalid_values() {
        let config = ServerConfig::default();
        let weight = |value: Option<&'static str>| {
            build_mongodb_config_with_env(&config, false, |name| match name {
                "HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES" => value.map(str::to_string),
                _ => None,
            })
        };

        assert_eq!(
            weight(None).unwrap().transaction_bundle_weight_entries,
            1000
        );
        assert_eq!(
            weight(Some("250"))
                .unwrap()
                .transaction_bundle_weight_entries,
            250
        );
        assert_eq!(
            weight(Some("0")).unwrap().transaction_bundle_weight_entries,
            0
        );
        let err = weight(Some("lots")).expect_err("invalid value must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_TRANSACTION_BUNDLE_WEIGHT_ENTRIES"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_reads_deferred_index_entries_and_rejects_invalid_values() {
        let config = ServerConfig::default();
        let entries = |value: Option<&'static str>| {
            build_mongodb_config_with_env(&config, false, |name| match name {
                "HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES" => {
                    value.map(str::to_string)
                }
                _ => None,
            })
        };

        assert_eq!(
            entries(None)
                .unwrap()
                .transaction_bundle_deferred_index_entries,
            None
        );
        assert_eq!(
            entries(Some("off"))
                .unwrap()
                .transaction_bundle_deferred_index_entries,
            Some(0)
        );
        assert_eq!(
            entries(Some("0"))
                .unwrap()
                .transaction_bundle_deferred_index_entries,
            Some(0)
        );
        assert_eq!(
            entries(Some("auto"))
                .unwrap()
                .transaction_bundle_deferred_index_entries,
            None
        );
        assert_eq!(
            entries(Some("6000"))
                .unwrap()
                .transaction_bundle_deferred_index_entries,
            Some(6000)
        );
        let err = entries(Some("lots")).expect_err("invalid value must fail startup");
        assert!(format!("{err}").contains("HFS_MONGODB_TRANSACTION_BUNDLE_DEFERRED_INDEX_ENTRIES"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_deferred_index_repair_min_age_covers_the_bundle_deadline() {
        use std::time::Duration;
        // A short timeout leaves the one-minute floor.
        assert_eq!(deferred_index_repair_min_age(30), Duration::from_secs(60));
        assert_eq!(deferred_index_repair_min_age(0), Duration::from_secs(60));
        // A long one is the Bundle deadline, `HFS_REQUEST_TIMEOUT` - 2 s.
        assert_eq!(deferred_index_repair_min_age(600), Duration::from_secs(598));
        assert_eq!(deferred_index_repair_min_age(900), Duration::from_secs(898));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_bundle_transaction_deadline_tracks_the_request_timeout_uncapped() {
        use std::time::Duration;

        assert_eq!(bundle_transaction_deadline(30), Duration::from_secs(28));
        // Unlike the replay budget, not capped at 120 s.
        assert_eq!(bundle_transaction_deadline(600), Duration::from_secs(598));
        assert_eq!(bundle_transaction_deadline(2), Duration::ZERO);
        assert_eq!(bundle_transaction_deadline(1), Duration::ZERO);
        assert_eq!(bundle_transaction_deadline(0), Duration::ZERO);
        assert_eq!(
            bundle_transaction_deadline(u64::MAX),
            Duration::from_secs(u64::MAX) - Duration::from_secs(2)
        );
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_bundle_transaction_budget_tracks_the_request_timeout() {
        use std::time::Duration;

        // The default HFS_REQUEST_TIMEOUT: a replay must finish, and the 503
        // be written, before the 30 s timeout layer answers 408.
        assert_eq!(bundle_transaction_budget(30), Duration::from_secs(28));
        assert_eq!(bundle_transaction_budget(5), Duration::from_secs(3));
        // No room for a margin: no budget, so no replay.
        assert_eq!(bundle_transaction_budget(2), Duration::ZERO);
        assert_eq!(bundle_transaction_budget(1), Duration::ZERO);
        assert_eq!(bundle_transaction_budget(0), Duration::ZERO);
        // Capped at the backend's own default, however long the request may run.
        let default = MongoBackendConfig::default().bundle_transaction_budget;
        assert_eq!(default, Duration::from_secs(120));
        assert_eq!(bundle_transaction_budget(122), default);
        assert_eq!(bundle_transaction_budget(3600), default);
        assert_eq!(bundle_transaction_budget(u64::MAX), default);
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_derives_the_bundle_budget_from_request_timeout() {
        let config = ServerConfig {
            request_timeout: 30,
            ..Default::default()
        };
        let mongo_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert_eq!(
            mongo_config.bundle_transaction_budget,
            std::time::Duration::from_secs(28)
        );
        assert_eq!(
            mongo_config.bundle_transaction_deadline,
            Some(std::time::Duration::from_secs(28))
        );

        let config = ServerConfig {
            request_timeout: 600,
            ..Default::default()
        };
        let mongo_config =
            build_mongodb_config_with_env(&config, false, |_| None).expect("valid config");
        assert_eq!(
            mongo_config.bundle_transaction_budget,
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            mongo_config.bundle_transaction_deadline,
            Some(std::time::Duration::from_secs(598))
        );
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_uses_mongo_specific_url_before_database_url_fallback() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_URI" => Some("mongodb://mongo-specific:27017".to_string()),
            "HFS_DATABASE_URL" => Some("mongodb://database-url:27017".to_string()),
            _ => None,
        })
        .expect("valid config");

        assert_eq!(
            mongo_config.connection_string,
            "mongodb://mongo-specific:27017"
        );
    }

    // ── ServerConfig validation is exercised at startup ──────────

    #[test]
    fn test_server_config_default_is_valid() {
        let config = ServerConfig::default();
        assert!(
            config.validate().is_ok(),
            "default ServerConfig should be valid"
        );
    }

    #[test]
    fn test_server_config_validation() {
        let mut config = ServerConfig {
            base_url: "https://fhir.example.test/fhir/".to_string(),
            ..Default::default()
        };
        config.normalize_public_base_url().unwrap();
        assert_eq!(config.base_url, "https://fhir.example.test/fhir");
        assert!(config.validate().is_ok());

        config.base_url = "javascript:alert(1)".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_storage_backend_mode_primary_backend_kind_mapping() {
        assert_eq!(
            StorageBackendMode::Sqlite.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::SqliteElasticsearch.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::Postgres.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::PostgresElasticsearch.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::MongoDB.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::MongoDBElasticsearch.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::S3.primary_backend_kind(),
            BackendKind::S3
        );
        assert_eq!(
            StorageBackendMode::S3Elasticsearch.primary_backend_kind(),
            BackendKind::S3
        );
    }

    #[test]
    fn bulk_submit_file_scheduling_selection() {
        let modes = [
            Some(StorageBackendMode::Sqlite),
            Some(StorageBackendMode::SqliteElasticsearch),
            Some(StorageBackendMode::Postgres),
            Some(StorageBackendMode::PostgresElasticsearch),
            Some(StorageBackendMode::MongoDB),
            Some(StorageBackendMode::MongoDBElasticsearch),
            Some(StorageBackendMode::S3),
            Some(StorageBackendMode::S3Elasticsearch),
            None,
        ];

        for mode in modes {
            for effective in [0, 1, 2, 8] {
                let expected = matches!(mode, Some(StorageBackendMode::Postgres)) && effective > 1;
                assert_eq!(
                    use_independent_submit_files(mode, effective),
                    expected,
                    "mode={mode:?} effective={effective}"
                );
            }

            for configured in [0, 1, 2, 8] {
                let cfg = helios_rest::config::BulkSubmitConfig {
                    file_concurrency: configured,
                    ..Default::default()
                };
                let backend = mode
                    .map(StorageBackendMode::primary_backend_kind)
                    .unwrap_or(BackendKind::Sqlite);
                let effective = cfg.effective_file_concurrency(backend);
                let expected = matches!(mode, Some(StorageBackendMode::Postgres)) && effective > 1;
                assert_eq!(
                    use_independent_submit_files(mode, effective),
                    expected,
                    "mode={mode:?} configured={configured} effective={effective}"
                );
            }
        }

        assert!(!use_independent_submit_files(None, 8));
        assert!(!use_independent_submit_files(
            Some(StorageBackendMode::PostgresElasticsearch),
            8,
        ));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn test_postgres_count_budget_defaults_below_the_request_timeout() {
        use helios_persistence::backends::postgres::PostgresConfig;

        let server = ServerConfig::default();
        let defaults = PostgresConfig::default();
        assert_eq!(
            defaults.effective_count_by_tenant_statement_timeout_ms(),
            25_000
        );
        assert_eq!(server.request_timeout, 30);
        assert_eq!(
            count_budget_race_warning(&defaults, server.request_timeout),
            None,
            "at defaults a slow tenant count answers 504, not 408"
        );

        // Equal to, or above, the request timeout: the 408 race returns.
        for count_ms in [30_000, 45_000] {
            let config = PostgresConfig {
                count_by_tenant_statement_timeout_ms: count_ms,
                statement_timeout_ms: 0,
                ..Default::default()
            };
            let warning = count_budget_race_warning(&config, server.request_timeout)
                .expect("budget at or above the request timeout warns");
            assert!(
                warning.contains("HFS_PG_COUNT_BY_TENANT_STATEMENT_TIMEOUT_MS")
                    && warning.contains("HFS_REQUEST_TIMEOUT"),
                "{warning}"
            );
        }
        // A general statement timeout below the request timeout caps the count.
        let capped = PostgresConfig {
            count_by_tenant_statement_timeout_ms: 45_000,
            statement_timeout_ms: 20_000,
            ..Default::default()
        };
        assert_eq!(count_budget_race_warning(&capped, 30), None);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn test_postgres_reindex_groups_selection() {
        let mut config = ServerConfig::default();
        config.bulk_submit.file_concurrency = 4;
        for mode in [
            StorageBackendMode::Sqlite,
            StorageBackendMode::SqliteElasticsearch,
            StorageBackendMode::Postgres,
            StorageBackendMode::PostgresElasticsearch,
            StorageBackendMode::MongoDB,
            StorageBackendMode::MongoDBElasticsearch,
            StorageBackendMode::S3,
            StorageBackendMode::S3Elasticsearch,
        ] {
            config.storage_backend = mode.to_string();
            let (_, n, width) = postgres_reindex_selection(&config, 8);
            assert_eq!(n, 4);
            assert_eq!(
                width,
                if mode == StorageBackendMode::Postgres {
                    4
                } else {
                    1
                }
            );
        }
        config.storage_backend = "postgres".to_string();
        assert_eq!(postgres_reindex_selection(&config, 1).2, 1);
        assert_eq!(postgres_reindex_selection(&config, 2).2, 2);
        config.bulk_submit.file_concurrency = 0;
        assert_eq!(postgres_reindex_selection(&config, 8).2, 1);
        config.storage_backend.clear();
        assert_eq!(postgres_reindex_selection(&config, 8).2, 1);
        config.storage_backend = "invalid".to_string();
        assert_eq!(postgres_reindex_selection(&config, 8).2, 1);
    }

    #[cfg(feature = "ui")]
    #[test]
    fn test_patient_name_search_support_matches_storage_capability() {
        for (mode, expected) in [
            (
                StorageBackendMode::Sqlite,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::SqliteElasticsearch,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::Postgres,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::PostgresElasticsearch,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::MongoDB,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::MongoDBElasticsearch,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
            (
                StorageBackendMode::S3,
                helios_ui::PatientNameSearchSupport::IdOnly,
            ),
            (
                StorageBackendMode::S3Elasticsearch,
                helios_ui::PatientNameSearchSupport::Enabled,
            ),
        ] {
            assert_eq!(patient_name_search_support(mode), expected, "{mode:?}");
        }
    }

    #[test]
    fn test_is_database_audit_dedicated_detection() {
        let mut config = AuditConfig::default();
        assert!(!is_database_audit_dedicated(&config, BackendKind::Sqlite));
        assert!(!is_database_audit_dedicated(&config, BackendKind::Postgres));
        assert!(!is_database_audit_dedicated(&config, BackendKind::MongoDB));
        assert!(!is_database_audit_dedicated(&config, BackendKind::S3));

        config.database_url = Some("sqlite:///tmp/audit.db".to_string());
        assert!(is_database_audit_dedicated(&config, BackendKind::Sqlite));
        assert!(is_database_audit_dedicated(&config, BackendKind::Postgres));
        assert!(is_database_audit_dedicated(&config, BackendKind::MongoDB));

        let mut mongo_only = AuditConfig::default();
        mongo_only.mongodb_database = Some("helios_audit".to_string());
        assert!(is_database_audit_dedicated(
            &mongo_only,
            BackendKind::MongoDB
        ));
        assert!(!is_database_audit_dedicated(
            &mongo_only,
            BackendKind::Postgres
        ));

        let mut s3_only = AuditConfig::default();
        s3_only.s3_bucket = Some("audit-bucket".to_string());
        assert!(is_database_audit_dedicated(&s3_only, BackendKind::S3));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_validate_shared_sqlite_audit_path_guard() {
        let shared = validate_shared_sqlite_audit_path(":memory:", false);
        assert!(shared.is_err(), "shared SQLite + :memory: must be rejected");

        let dedicated = validate_shared_sqlite_audit_path(":memory:", true);
        assert!(dedicated.is_ok(), "dedicated SQLite may use :memory:");

        let shared_file = validate_shared_sqlite_audit_path("fhir.db", false);
        assert!(
            shared_file.is_ok(),
            "shared SQLite file path must be accepted"
        );
    }

    #[test]
    fn dashboard_ui_disabled_request_stays_disabled() {
        assert!(!effective_ui_enabled(false));
    }

    #[cfg(feature = "ui")]
    #[test]
    fn dashboard_ui_enabled_request_stays_enabled_when_compiled_in() {
        assert!(effective_ui_enabled(true));
    }

    #[cfg(not(feature = "ui"))]
    #[test]
    fn dashboard_ui_enabled_request_is_disabled_when_compiled_out() {
        // The shared REST configuration keeps its default; only the binary
        // accounts for whether it can mount the UI.
        let config = ServerConfig::default();
        assert!(config.ui_enabled);
        assert!(!effective_ui_enabled(config.ui_enabled));
    }

    /// The regression guard for #975: whatever feature selection this test
    /// binary was built with — `--all-features` in `ci.yml`'s `test-rust` job,
    /// the same selection the release job uses for the uploaded artifacts —
    /// the assembled router must actually serve the UI at `/ui`.
    ///
    /// Asserting on the served response rather than on a `cfg!` is the point:
    /// the old bug was a *negative* feature (`not(feature = "headless")`) that
    /// `--all-features` tripped, and no `cfg` assertion downstream of the mount
    /// would have caught it.
    #[cfg(feature = "ui")]
    #[tokio::test]
    async fn ui_is_served_under_this_builds_feature_selection() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let config = ServerConfig::default();
        assert!(config.ui_enabled, "the UI must default to on");

        let response = attach_ui(axum::Router::new(), &config, None, None, None, None)
            .oneshot(
                axum::http::Request::get("/ui")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "GET /ui must be served by the UI router"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("<html") || html.contains("<!DOCTYPE"),
            "GET /ui must return HTML, got: {}",
            &html[..html.len().min(200)]
        );
    }

    /// Headless deployments turn the UI off at runtime, and `/ui` must then say
    /// so with a 404 + OperationOutcome. Without the explicit stub the path
    /// falls through to the FHIR router, which reads `ui` as a resource type
    /// and answers `200` with an empty searchset (#975).
    #[tokio::test]
    async fn ui_disabled_at_runtime_answers_404_not_an_empty_searchset() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let mut config = ServerConfig::default();
        config.ui_enabled = false;

        let response = attach_ui(axum::Router::new(), &config, None, None, None, None)
            .oneshot(
                axum::http::Request::get("/ui")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let outcome: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"][0]["code"], "not-found");
    }

    // ── Deferred bulk-submit job store (#1161) ──────────────────

    #[cfg(all(feature = "elasticsearch", feature = "sqlite"))]
    mod composite_submit_jobs_1161 {
        use super::*;
        use helios_fhir::FhirVersion;
        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, DynStorage, SyncMode,
        };
        use helios_persistence::core::{SubmissionChange, SubmissionId};
        use helios_persistence::{StorageResult, StoredResource, TenantId, TenantPermissions};
        use parking_lot::Mutex;
        use serde_json::Value;

        /// A search secondary that only records deletes; nothing else is
        /// reached by a rollback.
        struct DeleteSpy {
            deletes: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait::async_trait]
        impl ResourceStorage for DeleteSpy {
            fn backend_name(&self) -> &'static str {
                "delete-spy"
            }

            async fn create(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _resource: Value,
                _fhir_version: FhirVersion,
            ) -> StorageResult<StoredResource> {
                unimplemented!("a rollback never creates on the secondary")
            }

            async fn create_or_update(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _id: &str,
                _resource: Value,
                _fhir_version: FhirVersion,
            ) -> StorageResult<(StoredResource, bool)> {
                unimplemented!("a rollback of a create never upserts")
            }

            async fn read(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _id: &str,
            ) -> StorageResult<Option<StoredResource>> {
                Ok(None)
            }

            async fn update(
                &self,
                _tenant: &TenantContext,
                _current: &StoredResource,
                _resource: Value,
            ) -> StorageResult<StoredResource> {
                unimplemented!("a rollback of a create never updates")
            }

            async fn delete(
                &self,
                _tenant: &TenantContext,
                resource_type: &str,
                id: &str,
            ) -> StorageResult<()> {
                self.deletes
                    .lock()
                    .push(format!("delete {resource_type}/{id}"));
                Ok(())
            }

            async fn count(
                &self,
                _tenant: &TenantContext,
                _resource_type: Option<&str>,
            ) -> StorageResult<u64> {
                Ok(0)
            }
        }

        /// #1161: the job store `composite_submit_jobs` hands the worker under
        /// deferred indexing must mirror a rolled-back create to the search
        /// backend. Returning the raw primary there left the document in
        /// Elasticsearch after the primary had dropped it.
        #[tokio::test]
        async fn deferred_submit_jobs_mirror_rollbacks_to_the_search_backend() {
            let sqlite = Arc::new(SqliteBackend::in_memory().unwrap());
            sqlite.init_schema().unwrap();
            let deletes = Arc::new(Mutex::new(Vec::new()));

            let config = CompositeConfig::builder()
                .primary("sqlite", BackendKind::Sqlite)
                .search_backend("es", BackendKind::Elasticsearch)
                // Synchronous, so the delete lands before the assertion.
                .sync_mode(SyncMode::Synchronous)
                .build()
                .unwrap();
            let mut backends = std::collections::HashMap::new();
            backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
            backends.insert(
                "es".to_string(),
                Arc::new(DeleteSpy {
                    deletes: deletes.clone(),
                }) as DynStorage,
            );
            let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());

            let cfg = helios_rest::config::BulkSubmitConfig {
                defer_indexing: true,
                ..Default::default()
            };
            let jobs = composite_submit_jobs(
                sqlite.clone(),
                sqlite.clone(),
                composite,
                Vec::new(),
                &cfg,
                true,
            );

            // No ingest needed: SQLite treats a create that is already gone
            // as reverted, which is all the wrapper needs to mirror it.
            let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
            let change = SubmissionChange::create("m1", "Patient", "p-1161", "1");
            let sub = SubmissionId::generate("t1161");
            assert!(jobs.rollback_change(&tenant, &sub, &change).await.unwrap());

            assert_eq!(
                deletes.lock().clone(),
                vec!["delete Patient/p-1161".to_string()],
                "the deferred-path job store must mirror rollbacks to the search backend \
                 (#1161); nothing recorded means composite_submit_jobs handed the worker \
                 the raw primary"
            );
        }
    }
}
