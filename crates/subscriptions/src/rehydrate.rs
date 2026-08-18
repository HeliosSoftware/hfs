//! Startup rehydration of engine state from stored resources (issue #305).
//!
//! The engine's registries are in-memory (`DashMap`-backed) and were populated
//! **only** by the REST write handlers, which call
//! [`SubscriptionEngine::on_resource_event`] after a successful write. Nothing
//! read stored `Subscription` / `SubscriptionTopic` resources back at startup,
//! so a restart, redeploy, or crash left every previously-registered
//! subscription inert: the resources still read back fine over the REST API,
//! but the engine had no knowledge of them, so nothing matched, evaluated, or
//! delivered. This module is the missing mirror of the write path.
//!
//! # Why `ExportDataProvider`
//!
//! Rehydration needs to page through every resource of a type, for every
//! tenant. `ExportDataProvider::fetch_export_batch` is the one enumeration
//! primitive that **every** primary backend implements — SQLite, PostgreSQL,
//! MongoDB, S3, and `CompositeStorage` — so a single implementation here covers
//! all of them with no schema change, no migration, and no per-backend query to
//! drift out of sync.
//!
//! It also already does two things this code would otherwise have to repeat: it
//! filters out deleted resources, and it is cursor-paged, so peak memory stays
//! bounded no matter how many subscriptions a tenant holds.
//!
//! The richer alternative, `SearchProvider::search`, yields a `StoredResource`
//! that carries its own `fhir_version` — but it is *not* universally available:
//! a standalone S3 deployment reports search unsupported, and would therefore
//! silently rehydrate nothing. See [`infer_fhir_version`] for how the missing
//! version is instead recovered exactly.
//!
//! # Ordering
//!
//! [`crate::manager::SubscriptionManager::register`] rejects a subscription
//! whose topic is not already in the registry (`TopicNotFound`). Rehydration
//! order is therefore a correctness constraint, not a preference:
//!
//! 1. `SubscriptionTopic` — native topics (R4B / R5 / R6)
//! 2. `Basic` — R4 backport topics, which are stored as `Basic` resources
//! 3. `Subscription` — now that every topic they may reference is registered
//!
//! # Status policy
//!
//! | Stored status | Action | Why |
//! |---|---|---|
//! | `requested` | registered **and activated** | The spec state for "server has not yet activated"; runs the same activation path the write handler runs. |
//! | `active` | registered, no handshake | The handshake completed in a previous process lifetime; re-sending it is noise. |
//! | `error` | registered **and activated** | Nothing in the engine performs `error` → `active` while a subscription is dormant, so re-running activation here is its only recovery path. |
//! | `off` | registered, dormant | Terminal. Registered so `$status` keeps answering; never handshaken, never matched. |
//! | `entered-in-error` | registered, dormant | Client-retracted. Same treatment as `off`. |
//!
//! Two of those rows changed with #357, and both changes exist to stop a
//! *persisted* status from being worse than the volatile one it replaced.
//!
//! `off` used to be skipped outright on the reasoning that a terminal status
//! could never become useful again. That was defensible only while the status
//! was volatile. Now that the delivery circuit breaker's decision survives a
//! restart, skipping would mean an `off` subscription is absent from the engine
//! forever — and `$status` answers `404` for a resource that reads back `200`
//! from `GET /Subscription/{id}`. Registering it dormant costs one map entry and
//! keeps the two endpoints telling the same story. It cannot leak delivery:
//! [`crate::manager::SubscriptionManager::active_subscriptions_for_topic`]
//! selects only `active`.
//!
//! `error` is now re-activated rather than left dormant, for the mirror reason.
//! A dormant `error` subscription is never dispatched to, so it never succeeds,
//! so nothing ever resets it — before #357 the *loss* of the status on restart
//! was what quietly recovered it. Persisting `error` without re-running
//! activation would have converted a transient subscriber outage into permanent
//! silent death. Re-running it preserves the recovery a restart always gave,
//! now explicitly rather than by accident.
//!
//! The cost is that a restart re-handshakes those subscriptions. That is bounded
//! by [`RehydrationConfig::max_concurrent_handshakes`], and can be turned off
//! with [`RehydrationConfig::handshake_requested`] by operators who would rather
//! bring subscriptions back with no outbound traffic. The population is much
//! smaller than it used to be: with status write-back enabled, a subscription
//! the server activated now reads back `active` and is not handshaken again.

use std::collections::BTreeSet;
use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::engine::SubscriptionEngine;
use crate::manager::{ActiveSubscription, SubscriptionStatusCode};
use crate::topics::InMemoryTopicRegistry;

/// Resource type holding native `SubscriptionTopic` definitions (R4B / R5 / R6).
const TYPE_SUBSCRIPTION_TOPIC: &str = "SubscriptionTopic";
/// Resource type holding R4 backport topic definitions.
const TYPE_BASIC: &str = "Basic";
/// Resource type holding `Subscription` resources.
const TYPE_SUBSCRIPTION: &str = "Subscription";

/// Backstop on pages fetched per resource type, per tenant.
///
/// Every in-tree backend terminates correctly — each returns `next_cursor: None`
/// together with `is_last: true` once exhausted — and the paging loops only
/// continue when a backend explicitly reports both "more data" and a cursor. This
/// bound exists solely so that a backend which ever regressed to a non-advancing
/// cursor would degrade to a logged warning instead of spinning a startup task
/// forever. At the default batch size it allows millions of resources per type,
/// far beyond any realistic subscription count.
const MAX_PAGES_PER_TYPE: usize = 10_000;

/// Tuning for a rehydration pass.
#[derive(Debug, Clone)]
pub struct RehydrationConfig {
    /// Master switch. When `false`, [`SubscriptionEngine::rehydrate`] returns an
    /// empty report without touching storage.
    pub enabled: bool,

    /// Resources fetched per storage round-trip. Bounds peak memory: only one
    /// batch is held at a time regardless of how many resources exist.
    pub batch_size: u32,

    /// Whether stored `requested` and `error` subscriptions are activated
    /// (handshaked) during rehydration.
    ///
    /// Defaults to `true`. With status write-back enabled (#357) this is no
    /// longer the bulk-reactivation path it once was — a subscription the server
    /// activated reads back `active` and is skipped — but it remains the only
    /// recovery an `error` subscription has, and the only way a subscription
    /// that died mid-handshake ever leaves `requested`. Set to `false` to
    /// rehydrate with no outbound traffic at all, accepting that those two
    /// populations stay inert until the resource is re-written.
    pub handshake_requested: bool,

    /// Maximum handshakes in flight at once, across all tenants.
    ///
    /// A restart would otherwise fire one outbound request per `requested`
    /// subscription simultaneously — a thundering herd against subscriber
    /// endpoints. Treated as at least 1.
    pub max_concurrent_handshakes: usize,
}

impl Default for RehydrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batch_size: 500,
            handshake_requested: true,
            max_concurrent_handshakes: 8,
        }
    }
}

/// Outcome of a rehydration pass.
///
/// Returned rather than only logged so the behaviour is unit-testable without
/// standing up a server, and so the operator gets one structured summary line
/// instead of one line per resource.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RehydrationReport {
    /// Tenants scanned.
    pub tenants: usize,
    /// Topics registered (native `SubscriptionTopic` + R4 backport `Basic`).
    pub topics_registered: usize,
    /// Topic resources that could not be parsed.
    pub topics_failed: usize,
    /// Subscriptions registered with the manager.
    pub subscriptions_registered: usize,
    /// Subscriptions that failed to register (unknown topic, bad channel, …).
    pub subscriptions_failed: usize,
    /// Subscriptions registered in a terminal status (`off`, `entered-in-error`).
    ///
    /// They are counted in [`subscriptions_registered`](Self::subscriptions_registered)
    /// too: they *are* registered, so `$status` answers for them, but they are
    /// never handshaken and never matched against an event.
    ///
    /// Before #357 an `off` subscription was skipped outright and never entered
    /// the engine at all, which made `$status` answer `404` for it after every
    /// restart — a worse answer than the stale one it replaced.
    pub subscriptions_dormant: usize,
    /// Subscriptions handed to the activation (handshake) path.
    pub handshakes_started: usize,
    /// Storage errors encountered while paging. A non-zero value means the pass
    /// is incomplete: some subscriptions may still be inert.
    pub storage_errors: usize,
}

impl RehydrationReport {
    /// Whether the pass completed without any storage-level failure.
    ///
    /// Per-resource parse failures are counted separately and do **not** make a
    /// pass incomplete — one malformed resource must not stop the scan.
    pub fn is_complete(&self) -> bool {
        self.storage_errors == 0
    }

    fn merge(&mut self, other: RehydrationReport) {
        self.tenants += other.tenants;
        self.topics_registered += other.topics_registered;
        self.topics_failed += other.topics_failed;
        self.subscriptions_registered += other.subscriptions_registered;
        self.subscriptions_failed += other.subscriptions_failed;
        self.subscriptions_dormant += other.subscriptions_dormant;
        self.handshakes_started += other.handshakes_started;
        self.storage_errors += other.storage_errors;
    }
}

/// Whether `version` is R4 (the backport shape), in a build that may not have
/// R4 compiled in at all.
fn is_r4(version: FhirVersion) -> bool {
    #[cfg(feature = "R4")]
    {
        version == FhirVersion::R4
    }
    #[cfg(not(feature = "R4"))]
    {
        let _ = version;
        false
    }
}

/// The lowest enabled native (R4B+) version, or `None` when only R4 is compiled
/// in and there is therefore no native version to choose.
///
/// Which native version is picked only affects the version string in emitted
/// notification bundles: R4B, R5, and R6 all take the same `build_native` code
/// path, and the same `channelType`/`topic` parsing path.
fn lowest_native_version() -> Option<FhirVersion> {
    #[cfg(feature = "R4B")]
    {
        Some(FhirVersion::R4B)
    }
    #[cfg(all(not(feature = "R4B"), feature = "R5"))]
    {
        Some(FhirVersion::R5)
    }
    #[cfg(all(not(feature = "R4B"), not(feature = "R5"), feature = "R6"))]
    {
        Some(FhirVersion::R6)
    }
    #[cfg(all(not(feature = "R4B"), not(feature = "R5"), not(feature = "R6")))]
    {
        None
    }
}

/// R4 when it is compiled in, otherwise the supplied fallback.
fn r4_or(fallback: FhirVersion) -> FhirVersion {
    #[cfg(feature = "R4")]
    {
        let _ = fallback;
        FhirVersion::R4
    }
    #[cfg(not(feature = "R4"))]
    {
        fallback
    }
}

/// Infers the FHIR version of a stored `Subscription` resource from its shape.
///
/// `fetch_export_batch` yields raw resource JSON, not the `StoredResource`
/// envelope, so the persisted `fhir_version` is unavailable. It does not need to
/// be: **every** version-dependent branch in this crate is a binary
/// R4-vs-native split, and both sides of that split are directly observable in
/// the resource:
///
/// | | R4 (backport) | R4B / R5 / R6 (native) |
/// |---|---|---|
/// | topic | `criteria`, or the `backport-topic-canonical` extension | `topic` |
/// | channel | `channel` object | `channelType` coding |
///
/// (See `extract_topic_url` and `extract_channel_config` in [`crate::manager`],
/// and `build_r4` vs `build_native` in [`crate::notification`].)
///
/// Native markers win, because `topic`/`channelType` cannot appear in a genuine
/// R4 backport resource, whereas an R4 resource may carry neither marker. A
/// resource with no usable marker falls back to `default_version`, matching what
/// the write path stores for a request that did not specify `fhirVersion`.
pub fn infer_fhir_version(resource: &Value, default_version: FhirVersion) -> FhirVersion {
    let has_native_marker =
        resource.get("topic").is_some() || resource.get("channelType").is_some();
    if has_native_marker {
        return if is_r4(default_version) {
            // Server default is R4 but the resource is native-shaped.
            lowest_native_version().unwrap_or(default_version)
        } else {
            default_version
        };
    }

    let has_r4_marker =
        resource.get("criteria").is_some() || resource.get("channel").is_some_and(Value::is_object);
    if has_r4_marker {
        return r4_or(default_version);
    }

    default_version
}

/// Enumerates the tenants to rehydrate.
///
/// Mirrors the conformance-seeding logic in `crates/hfs/src/main.rs`: every
/// registered tenant plus the configured default (which is auto-provisioned and
/// so may not be listed yet). Backends without a tenant registry — mock stores,
/// minimal deployments — yield just the default tenant, so the common
/// single-tenant configuration works with no registry at all.
async fn tenants_to_scan<S>(storage: &S, default_tenant: &str) -> Vec<String>
where
    S: ResourceStorage,
{
    let mut ids: BTreeSet<String> = BTreeSet::new();
    ids.insert(default_tenant.to_string());

    if storage.supports_tenant_registry() {
        match storage.list_tenants().await {
            Ok(records) => ids.extend(records.into_iter().map(|r| r.id)),
            Err(e) => warn!(
                error = %e,
                "Listing tenants for subscription rehydration failed; \
                 rehydrating the default tenant only"
            ),
        }
    }

    ids.into_iter().collect()
}

impl SubscriptionEngine {
    /// Repopulates the engine from stored resources, for every tenant.
    ///
    /// This is the startup mirror of the write-handler path: for each tenant it
    /// registers stored topics and then stored subscriptions, so subscriptions
    /// that were active before a restart resume matching and delivering.
    ///
    /// Errors are absorbed, never propagated: a storage failure on one tenant
    /// must not prevent the others from rehydrating, and one malformed resource
    /// must not stop the scan. Both are counted in the returned
    /// [`RehydrationReport`]; check [`RehydrationReport::is_complete`] to tell a
    /// clean pass from a partial one.
    pub async fn rehydrate<S>(
        &self,
        storage: &S,
        default_tenant: &str,
        default_version: FhirVersion,
        config: &RehydrationConfig,
    ) -> RehydrationReport
    where
        S: ResourceStorage + ExportDataProvider,
    {
        let mut report = RehydrationReport::default();
        if !config.enabled {
            debug!("Subscription rehydration is disabled");
            return report;
        }

        let tenants = tenants_to_scan(storage, default_tenant).await;
        info!(
            tenants = tenants.len(),
            "Rehydrating subscription engine from storage"
        );

        // Activation is deferred so registration — the fast, DB-bound part that
        // actually restores delivery for `active` subscriptions — completes for
        // every tenant before any outbound handshake is attempted.
        let mut pending: Vec<ActiveSubscription> = Vec::new();

        for tenant_id in &tenants {
            let tenant_report = self
                .rehydrate_tenant(storage, tenant_id, default_version, config, &mut pending)
                .await;
            report.merge(tenant_report);
        }
        report.tenants = tenants.len();
        report.handshakes_started = pending.len();

        self.activate_pending(pending, config).await;

        if report.is_complete() {
            info!(
                tenants = report.tenants,
                topics = report.topics_registered,
                subscriptions = report.subscriptions_registered,
                dormant = report.subscriptions_dormant,
                failed = report.subscriptions_failed,
                handshakes = report.handshakes_started,
                "Subscription engine rehydrated"
            );
        } else {
            warn!(
                tenants = report.tenants,
                topics = report.topics_registered,
                subscriptions = report.subscriptions_registered,
                storage_errors = report.storage_errors,
                "Subscription engine rehydration incomplete; some subscriptions \
                 may not deliver until they are re-written"
            );
        }

        report
    }

    /// Rehydrates a single tenant: topics → R4 backport topics → subscriptions.
    async fn rehydrate_tenant<S>(
        &self,
        storage: &S,
        tenant_id: &str,
        default_version: FhirVersion,
        config: &RehydrationConfig,
        pending: &mut Vec<ActiveSubscription>,
    ) -> RehydrationReport
    where
        S: ResourceStorage + ExportDataProvider,
    {
        let mut report = RehydrationReport::default();
        let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());

        // 1. Native SubscriptionTopic resources.
        self.rehydrate_topics(
            storage,
            &tenant,
            TYPE_SUBSCRIPTION_TOPIC,
            config,
            &mut report,
        )
        .await;

        // 2. R4 backport topics, stored as `Basic`. Skipped when R4 is not
        //    compiled in, since nothing can then produce or consume them.
        if cfg!(feature = "R4") {
            self.rehydrate_topics(storage, &tenant, TYPE_BASIC, config, &mut report)
                .await;
        }

        // 3. Subscriptions — every topic they can reference is now registered.
        self.rehydrate_subscriptions(
            storage,
            &tenant,
            tenant_id,
            default_version,
            config,
            pending,
            &mut report,
        )
        .await;

        report
    }

    /// Pages through one topic-bearing resource type and registers what parses.
    ///
    /// `resource_type` is either `SubscriptionTopic` (native) or `Basic` (R4
    /// backport). A `Basic` resource that is not a topic is not an error — the
    /// backport parser returns `Ok(None)` for it and it is silently ignored,
    /// which matters because `Basic` is a general-purpose resource type a tenant
    /// may well use for unrelated data.
    async fn rehydrate_topics<S>(
        &self,
        storage: &S,
        tenant: &TenantContext,
        resource_type: &str,
        config: &RehydrationConfig,
        report: &mut RehydrationReport,
    ) where
        S: ExportDataProvider,
    {
        let mut cursor: Option<String> = None;
        for page in 0.. {
            if page >= MAX_PAGES_PER_TYPE {
                warn!(
                    tenant = %tenant.tenant_id().as_str(),
                    resource_type,
                    pages = page,
                    "Stopping topic rehydration at the page backstop; \
                     the backend cursor is not advancing"
                );
                report.storage_errors += 1;
                return;
            }
            let batch = match fetch_batch(storage, tenant, resource_type, cursor.as_deref(), config)
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        tenant = %tenant.tenant_id().as_str(),
                        resource_type,
                        error = %e,
                        "Failed to page stored topics during rehydration"
                    );
                    report.storage_errors += 1;
                    return;
                }
            };
            let ResourceBatch {
                resources,
                next_cursor,
                is_last,
            } = batch;

            for resource in &resources {
                let parsed = if resource_type == TYPE_BASIC {
                    match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
                        // Not a topic at all — ordinary `Basic` data.
                        Ok(None) => continue,
                        Ok(Some(topic)) => Ok(topic),
                        Err(e) => Err(e),
                    }
                } else {
                    InMemoryTopicRegistry::parse_topic_resource(resource)
                };

                match parsed {
                    Ok(topic) => {
                        debug!(
                            tenant = %tenant.tenant_id().as_str(),
                            topic_url = %topic.canonical_url,
                            "Rehydrated SubscriptionTopic"
                        );
                        self.topic_registry()
                            .add_topic(tenant.tenant_id().as_str(), topic);
                        report.topics_registered += 1;
                    }
                    Err(e) => {
                        warn!(
                            tenant = %tenant.tenant_id().as_str(),
                            resource_type,
                            error = %e,
                            "Skipping unparseable topic during rehydration"
                        );
                        report.topics_failed += 1;
                    }
                }
            }

            match next_cursor {
                Some(next) if !is_last => cursor = Some(next),
                _ => return,
            }
        }
    }

    /// Pages through stored `Subscription` resources and registers them.
    #[allow(clippy::too_many_arguments)]
    async fn rehydrate_subscriptions<S>(
        &self,
        storage: &S,
        tenant: &TenantContext,
        tenant_id: &str,
        default_version: FhirVersion,
        config: &RehydrationConfig,
        pending: &mut Vec<ActiveSubscription>,
        report: &mut RehydrationReport,
    ) where
        S: ExportDataProvider,
    {
        let mut cursor: Option<String> = None;
        for page in 0.. {
            if page >= MAX_PAGES_PER_TYPE {
                warn!(
                    tenant = tenant_id,
                    pages = page,
                    "Stopping subscription rehydration at the page backstop; \
                     the backend cursor is not advancing"
                );
                report.storage_errors += 1;
                return;
            }
            let batch = match fetch_batch(
                storage,
                tenant,
                TYPE_SUBSCRIPTION,
                cursor.as_deref(),
                config,
            )
            .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        tenant = tenant_id,
                        error = %e,
                        "Failed to page stored subscriptions during rehydration"
                    );
                    report.storage_errors += 1;
                    return;
                }
            };
            let ResourceBatch {
                resources,
                next_cursor,
                is_last,
            } = batch;

            for resource in &resources {
                let Some(id) = resource.get("id").and_then(|v| v.as_str()) else {
                    warn!(
                        tenant = tenant_id,
                        "Skipping stored Subscription with no id during rehydration"
                    );
                    report.subscriptions_failed += 1;
                    continue;
                };

                let fhir_version = infer_fhir_version(resource, default_version);
                match self
                    .manager()
                    .register(tenant_id, id, resource, fhir_version)
                {
                    Ok(sub) => {
                        info!(
                            tenant = tenant_id,
                            subscription_id = %sub.id,
                            topic_url = %sub.topic_url,
                            status = %sub.status,
                            channel_type = %sub.channel.channel_type.as_fhir_str(),
                            fhir_version = %fhir_version,
                            "Rehydrated subscription"
                        );
                        report.subscriptions_registered += 1;

                        // `off` and `entered-in-error` are terminal: registered
                        // so `$status` keeps answering for them, never
                        // handshaken, and never matched (the evaluator only
                        // selects `active`). Counted separately because a
                        // deployment wants to see how many of its subscriptions
                        // are parked rather than serving.
                        if sub.status.is_terminal() {
                            debug!(
                                tenant = tenant_id,
                                subscription_id = %sub.id,
                                status = %sub.status,
                                "Registered terminal subscription as dormant"
                            );
                            report.subscriptions_dormant += 1;
                            continue;
                        }

                        // `requested` has never been activated. `error` is a
                        // subscription the delivery circuit breaker gave up on
                        // — and, because nothing in the engine performs an
                        // `error` → `active` transition while it is dormant, a
                        // restart is the *only* recovery it has. Re-running
                        // activation for both is what stops a persisted `error`
                        // from becoming a permanent one.
                        if matches!(
                            sub.status,
                            SubscriptionStatusCode::Requested | SubscriptionStatusCode::Error
                        ) && config.handshake_requested
                        {
                            pending.push(sub);
                        }
                    }
                    Err(e) => {
                        warn!(
                            tenant = tenant_id,
                            subscription_id = id,
                            error = %e,
                            "Failed to rehydrate subscription; it will not deliver \
                             until the resource is re-written"
                        );
                        report.subscriptions_failed += 1;
                    }
                }
            }

            match next_cursor {
                Some(next) if !is_last => cursor = Some(next),
                _ => return,
            }
        }
    }

    /// Runs the deferred handshakes, at most
    /// [`RehydrationConfig::max_concurrent_handshakes`] at a time.
    ///
    /// Without the cap, a restart would fire one outbound request per
    /// `requested` subscription at once — and because status transitions are not
    /// persisted, that is potentially *every* subscription in the deployment.
    async fn activate_pending(&self, pending: Vec<ActiveSubscription>, config: &RehydrationConfig) {
        if pending.is_empty() {
            return;
        }

        let permits = config.max_concurrent_handshakes.max(1);
        info!(
            subscriptions = pending.len(),
            max_concurrent = permits,
            "Re-activating `requested` subscriptions after rehydration"
        );

        let semaphore = Arc::new(tokio::sync::Semaphore::new(permits));
        let tasks = pending.into_iter().map(|subscription| {
            let semaphore = Arc::clone(&semaphore);
            async move {
                // `acquire_owned` only errors once the semaphore is closed,
                // which never happens here — it is dropped with these futures.
                let _permit = semaphore.acquire_owned().await;
                self.activate_subscription(&subscription).await;
            }
        });

        futures::future::join_all(tasks).await;
    }
}

/// One page of stored resources, parsed from the backend's NDJSON batch.
struct ResourceBatch {
    resources: Vec<Value>,
    next_cursor: Option<String>,
    is_last: bool,
}

/// Fetches and parses one page of resources of `resource_type`.
///
/// A line that does not parse as JSON is dropped with a warning rather than
/// failing the page: one corrupt row must not stop a tenant from rehydrating.
async fn fetch_batch<S>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    cursor: Option<&str>,
    config: &RehydrationConfig,
) -> Result<ResourceBatch, String>
where
    S: ExportDataProvider,
{
    let mut request = ExportRequest::system();
    request.resource_types = vec![resource_type.to_string()];
    request.batch_size = config.batch_size;

    let batch = storage
        .fetch_export_batch(tenant, &request, resource_type, cursor, config.batch_size)
        .await
        .map_err(|e| e.to_string())?;

    let mut resources = Vec::with_capacity(batch.lines.len());
    for line in &batch.lines {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => resources.push(value),
            Err(e) => warn!(
                tenant = %tenant.tenant_id().as_str(),
                resource_type,
                error = %e,
                "Skipping unparseable stored resource during rehydration"
            ),
        }
    }

    Ok(ResourceBatch {
        resources,
        next_cursor: batch.next_cursor,
        is_last: batch.is_last,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_merges_counters() {
        let mut a = RehydrationReport {
            topics_registered: 1,
            subscriptions_registered: 2,
            ..Default::default()
        };
        a.merge(RehydrationReport {
            topics_registered: 3,
            subscriptions_failed: 1,
            storage_errors: 1,
            ..Default::default()
        });
        assert_eq!(a.topics_registered, 4);
        assert_eq!(a.subscriptions_registered, 2);
        assert_eq!(a.subscriptions_failed, 1);
        assert_eq!(a.storage_errors, 1);
    }

    #[test]
    fn report_is_complete_only_without_storage_errors() {
        // Per-resource parse failures do NOT make a pass incomplete — one bad
        // resource must not be reported as "some subscriptions may be inert".
        let parse_failures = RehydrationReport {
            topics_failed: 3,
            subscriptions_failed: 2,
            ..Default::default()
        };
        assert!(parse_failures.is_complete());

        let storage_failure = RehydrationReport {
            storage_errors: 1,
            ..Default::default()
        };
        assert!(!storage_failure.is_complete());
    }

    #[test]
    fn default_config_rehydrates_and_handshakes() {
        let config = RehydrationConfig::default();
        assert!(config.enabled);
        assert!(
            config.handshake_requested,
            "status transitions are not persisted, so `requested` must be \
             activated by default or the fix restores nothing"
        );
        assert!(config.max_concurrent_handshakes >= 1);
    }

    // ── FHIR version inference ───────────────────────────────────────────

    #[cfg(feature = "R4")]
    #[test]
    fn infers_r4_from_backport_shape() {
        // R4 backport: topic in `criteria`, channel as a nested object.
        let resource = json!({
            "resourceType": "Subscription",
            "id": "sub-1",
            "status": "active",
            "criteria": "http://example.org/topic",
            "channel": { "type": "rest-hook", "endpoint": "https://example.org/hook" }
        });
        assert_eq!(
            infer_fhir_version(&resource, FhirVersion::default_enabled()),
            FhirVersion::R4
        );
    }

    #[cfg(feature = "R4")]
    #[test]
    fn infers_r4_from_channel_object_alone() {
        // A `channel` object with no `criteria` is still unambiguously R4:
        // native versions use `channelType`, never `channel`.
        let resource = json!({
            "resourceType": "Subscription",
            "id": "sub-2",
            "status": "requested",
            "channel": { "type": "rest-hook" }
        });
        assert_eq!(
            infer_fhir_version(&resource, FhirVersion::default_enabled()),
            FhirVersion::R4
        );
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn infers_native_from_topic_field() {
        // `topic` exists only in R4B+, so this must not resolve to R4 even when
        // R4 is the server default.
        let resource = json!({
            "resourceType": "Subscription",
            "id": "sub-3",
            "status": "active",
            "topic": "http://example.org/topic",
            "channelType": { "code": "rest-hook" }
        });
        assert!(
            !is_r4(infer_fhir_version(
                &resource,
                FhirVersion::default_enabled()
            )),
            "a native-shaped resource must not be parsed as R4 backport"
        );
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn native_marker_wins_over_r4_marker() {
        // A resource carrying both shapes is native: `topic` cannot appear in a
        // genuine R4 backport resource, whereas `criteria` may linger.
        let resource = json!({
            "resourceType": "Subscription",
            "topic": "http://example.org/topic",
            "criteria": "Patient?active=true"
        });
        assert!(!is_r4(infer_fhir_version(
            &resource,
            FhirVersion::default_enabled()
        )));
    }

    #[test]
    fn falls_back_to_default_when_shape_is_ambiguous() {
        // Neither marker present — mirror what the write path stores for a
        // request that did not specify `fhirVersion`.
        let resource = json!({ "resourceType": "Subscription", "id": "sub-4" });
        let default = FhirVersion::default_enabled();
        assert_eq!(infer_fhir_version(&resource, default), default);
    }

    #[test]
    fn r4_only_build_has_no_native_version_to_pick() {
        // Guards the inference fallback: with no native version compiled in, a
        // native-shaped resource must still resolve to *something* usable rather
        // than panicking or yielding a version the build cannot parse.
        let resource = json!({ "resourceType": "Subscription", "topic": "http://x/y" });
        let default = FhirVersion::default_enabled();
        let inferred = infer_fhir_version(&resource, default);
        if lowest_native_version().is_none() {
            assert_eq!(inferred, default);
        }
    }
}
