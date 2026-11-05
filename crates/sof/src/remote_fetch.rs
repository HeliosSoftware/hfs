//! Remote reference prefetch (Phases 4–5, plus streaming support).
//!
//! This is the only part of remote `resolve()` that performs I/O. It runs at the
//! async edge (CLI / server), *before* the synchronous, rayon-parallel row
//! generation, and returns the fetched resources so they can be folded into the
//! in-memory resolution pool (see `build_resolution_scope` in `lib.rs`). The
//! evaluation core therefore stays pure and the run remains reproducible from
//! `(bundle + fetched snapshot)`.
//!
//! [`RemoteResolver`] holds the config plus a bounded, cross-call cache and a
//! fetch counter, so it serves a whole *run*:
//! - **single Bundle** — one [`RemoteResolver::resolve`] call over all bundle
//!   references ([`prefetch_external_resources`]);
//! - **streaming / NDJSON** — one resolver shared across every chunk, so a
//!   reference recurring across chunks is fetched once and `max_fetches` is a
//!   per-stream cap. The cache is a bounded LRU (with negative caching) so
//!   streaming memory stays bounded.
//!
//! Per resolve call:
//! 1. Keep only references the allowlist permits and that do not already resolve
//!    locally — [`RemoteResolveConfig::fetch_decision`] / `resolves_in_bundle`.
//! 2. Serve cache hits; for misses, validate each host (literal IPs were
//!    explicitly allowlisted; hostnames are resolved via DNS and **pinned** to
//!    addresses that pass [`is_blocked_address`] — enforcing the SSRF guard and
//!    defeating DNS rebinding), then fetch concurrently under the per-run cap,
//!    timeout, size cap, and optional per-host bearer auth.
//! 3. Cache results (success and miss), follow chained references up to
//!    `max_depth`, and parse the collected JSON into [`FhirResource`]s of the
//!    bundle's version (skipping anything that fails to parse — non-fatal).

use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::stream::{self, StreamExt};
use lru::LruCache;
use serde_json::Value;
use tokio::net::lookup_host;
use url::{Host, Url};

use helios_fhir::{FhirResource, FhirVersion};

use crate::reference_collector::{
    collect_reference_strings, collect_references_from_json, collect_resource_keys,
    resolves_in_bundle,
};
use crate::remote_resolver::{RemoteResolveConfig, is_blocked_address};
use crate::{SofBundle, parse_json_to_fhir_resource_pub};

/// A reusable remote-resolution engine for one run (a Bundle or an NDJSON stream).
///
/// Holds the [`RemoteResolveConfig`], a bounded LRU cache of fetched resource JSON
/// (with negative caching of misses), and a fetch counter. Sharing one instance
/// across a stream's chunks means a reference seen in many chunks is fetched once,
/// and `max_fetches` is enforced across the whole run.
pub struct RemoteResolver {
    config: RemoteResolveConfig,
    /// Reference string → fetched resource JSON, or `None` for a known miss.
    cache: Mutex<LruCache<String, Option<Value>>>,
    /// Total fetches performed this run (capped at `config.max_fetches`).
    fetched: AtomicUsize,
}

impl RemoteResolver {
    /// Creates a resolver with an empty cache sized to `config.cache_max_entries`.
    pub fn new(config: RemoteResolveConfig) -> Self {
        let cap = NonZeroUsize::new(config.cache_max_entries.max(1)).expect("cap >= 1");
        Self {
            config,
            cache: Mutex::new(LruCache::new(cap)),
            fetched: AtomicUsize::new(0),
        }
    }

    /// Resolves `seed_refs` — and, within `max_depth`, references discovered inside
    /// fetched resources — returning the parsed external resources to fold into a
    /// resolution pool. References already resolvable in `local_keys` are skipped.
    pub async fn resolve(
        &self,
        seed_refs: Vec<String>,
        local_keys: &BTreeSet<String>,
        version: FhirVersion,
    ) -> Vec<FhirResource> {
        if !self.config.is_active() {
            return Vec::new();
        }

        let mut resolved_json: Vec<Value> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut frontier: Vec<String> = seed_refs
            .into_iter()
            .filter(|r| self.config.fetch_decision(r).is_allowed())
            .filter(|r| !resolves_in_bundle(r, local_keys))
            .filter(|r| seen.insert(r.clone()))
            .collect();

        let mut depth = 0;
        while depth < self.config.max_depth && !frontier.is_empty() {
            depth += 1;

            // Partition this round into cache hits and misses.
            let mut round_json: Vec<Value> = Vec::new();
            let mut misses: Vec<String> = Vec::new();
            {
                let mut cache = self.cache.lock().unwrap();
                for reference in frontier.drain(..) {
                    match cache.get(&reference) {
                        Some(Some(json)) => round_json.push(json.clone()),
                        Some(None) => {} // known miss — skip
                        None => misses.push(reference),
                    }
                }
            }

            // Fetch the misses, then record results (success or miss) in the cache.
            if !misses.is_empty() {
                let fetched = self.fetch_batch(misses).await;
                let mut cache = self.cache.lock().unwrap();
                for (reference, result) in fetched {
                    if let Some(json) = &result {
                        round_json.push(json.clone());
                    }
                    cache.put(reference, result);
                }
            }

            // Queue chained references discovered in fetched resources.
            if depth < self.config.max_depth {
                for json in &round_json {
                    for reference in collect_references_from_json(json) {
                        if self.config.fetch_decision(&reference).is_allowed()
                            && !resolves_in_bundle(&reference, local_keys)
                            && seen.insert(reference.clone())
                        {
                            frontier.push(reference);
                        }
                    }
                }
            }

            resolved_json.extend(round_json);
        }

        resolved_json
            .into_iter()
            .filter_map(
                |json| match parse_json_to_fhir_resource_pub(json, version) {
                    Ok(resource) => Some(resource),
                    Err(err) => {
                        tracing::warn!(error = %err, "remote resolve: skipping unparseable resource");
                        None
                    }
                },
            )
            .collect()
    }

    /// Fetches a batch of (uncached) references concurrently, honouring the per-run
    /// fetch cap, host validation, timeout and size limits. Returns
    /// `(reference, Some(json))` on success and `(reference, None)` on any failure
    /// or cap exhaustion, so the caller can (negative-)cache every outcome.
    async fn fetch_batch(&self, references: Vec<String>) -> Vec<(String, Option<Value>)> {
        let already = self.fetched.load(Ordering::Relaxed);
        let remaining = self.config.max_fetches.saturating_sub(already);
        if remaining == 0 {
            tracing::warn!(
                max_fetches = self.config.max_fetches,
                "remote resolve: fetch cap reached; skipping further fetches"
            );
            return references.into_iter().map(|r| (r, None)).collect();
        }

        let (to_fetch, skipped): (Vec<String>, Vec<String>) = if references.len() > remaining {
            tracing::warn!(
                skipped = references.len() - remaining,
                max_fetches = self.config.max_fetches,
                "remote resolve: fetch cap reached; some references not fetched"
            );
            let mut iter = references.into_iter();
            let take: Vec<String> = iter.by_ref().take(remaining).collect();
            (take, iter.collect())
        } else {
            (references, Vec::new())
        };
        self.fetched.fetch_add(to_fetch.len(), Ordering::Relaxed);

        let (client, allowed_hosts) = build_validated_client(&to_fetch, &self.config).await;
        let config = &self.config;
        let allowed = &allowed_hosts;

        let mut results: Vec<(String, Option<Value>)> = stream::iter(to_fetch)
            .map(|reference| {
                let client = client.clone();
                async move {
                    let json = if host_is_allowed(&reference, allowed) {
                        fetch_one(&client, &reference, config).await
                    } else {
                        None
                    };
                    (reference, json)
                }
            })
            .buffer_unordered(config.concurrency)
            .collect()
            .await;

        // Negative-cache the cap-skipped references too (not retried this run).
        results.extend(skipped.into_iter().map(|r| (r, None)));
        results
    }
}

/// Fetches the allowlisted external references reachable from `bundle` and returns
/// them as parsed resources to merge into the resolution pool (single-Bundle path).
///
/// Returns an empty vector when remote resolution is inactive. Never errors: every
/// failure mode (disallowed host, timeout, non-2xx, oversize, unparseable) is
/// logged and skipped, leaving the reference to fall back to the in-scope
/// typed-stub/empty behaviour.
pub async fn prefetch_external_resources(
    bundle: &SofBundle,
    config: &RemoteResolveConfig,
) -> Vec<FhirResource> {
    if !config.is_active() {
        return Vec::new();
    }
    let resolver = RemoteResolver::new(config.clone());
    let refs = collect_reference_strings(bundle);
    let keys = collect_resource_keys(bundle);
    let fetched = resolver.resolve(refs, &keys, bundle.version()).await;
    if !fetched.is_empty() {
        tracing::info!(
            count = fetched.len(),
            "remote resolve: prefetched resources"
        );
    }
    fetched
}

/// Builds an HTTP client whose DNS is pinned to SSRF-validated addresses, and
/// returns the set of (lowercased) hosts that are cleared to fetch.
///
/// Literal-IP hosts are cleared as-is (they could only reach here by being
/// explicitly allowlisted). Hostnames are resolved once; if every resolved
/// address passes [`is_blocked_address`] the host is pinned to those addresses,
/// otherwise it is dropped.
async fn build_validated_client(
    batch: &[String],
    config: &RemoteResolveConfig,
) -> (reqwest::Client, HashSet<String>) {
    let mut builder = reqwest::Client::builder()
        .timeout(config.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("helios-sof/", env!("CARGO_PKG_VERSION")));

    let mut allowed_hosts: HashSet<String> = HashSet::new();
    let mut processed: HashSet<String> = HashSet::new();

    for reference in batch {
        let Ok(url) = Url::parse(reference) else {
            continue;
        };
        let (Some(host), Some(host_str)) = (url.host(), url.host_str()) else {
            continue;
        };
        let host_key = host_str.to_ascii_lowercase();
        if !processed.insert(host_key.clone()) {
            continue;
        }
        let port = url.port_or_known_default().unwrap_or(443);

        match host {
            // Explicitly-allowlisted literal IPs are the operator's own decision.
            Host::Ipv4(_) | Host::Ipv6(_) => {
                allowed_hosts.insert(host_key);
            }
            Host::Domain(name) => match lookup_host((name, port)).await {
                Ok(addrs) => {
                    let addrs: Vec<std::net::SocketAddr> = addrs.collect();
                    if addrs.is_empty() {
                        tracing::warn!(host = name, "remote resolve: host did not resolve");
                    } else if addrs
                        .iter()
                        .any(|addr| is_blocked_address(addr.ip(), config.allow_private_addresses))
                    {
                        tracing::warn!(
                            host = name,
                            "remote resolve: host resolves to a disallowed address; blocked"
                        );
                    } else {
                        builder = builder.resolve_to_addrs(name, &addrs);
                        allowed_hosts.insert(host_key);
                    }
                }
                Err(err) => {
                    tracing::warn!(host = name, error = %err, "remote resolve: DNS lookup failed")
                }
            },
        }
    }

    let client = builder.build().unwrap_or_else(|err| {
        tracing::warn!(error = %err, "remote resolve: client build failed; using default client");
        reqwest::Client::new()
    });
    (client, allowed_hosts)
}

fn host_is_allowed(reference: &str, allowed_hosts: &HashSet<String>) -> bool {
    Url::parse(reference)
        .ok()
        .and_then(|url| url.host_str().map(|h| h.to_ascii_lowercase()))
        .map(|host| allowed_hosts.contains(&host))
        .unwrap_or(false)
}

/// Fetches a single reference, enforcing auth, status, and size limits.
async fn fetch_one(
    client: &reqwest::Client,
    reference: &str,
    config: &RemoteResolveConfig,
) -> Option<Value> {
    let url = Url::parse(reference).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();

    let mut request = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/fhir+json");
    if let Some(token) = config.bearer_for_host(&host) {
        request = request.bearer_auth(token);
    }

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::debug!(reference, error = %err, "remote resolve: request failed");
            return None;
        }
    };

    if !response.status().is_success() {
        tracing::debug!(reference, status = %response.status(), "remote resolve: non-success");
        return None;
    }

    if response
        .content_length()
        .is_some_and(|len| len as usize > config.max_response_bytes)
    {
        tracing::warn!(
            reference,
            "remote resolve: response exceeds size cap (Content-Length)"
        );
        return None;
    }

    let bytes = response.bytes().await.ok()?;
    if bytes.len() > config.max_response_bytes {
        tracing::warn!(
            reference,
            len = bytes.len(),
            "remote resolve: response exceeds size cap"
        );
        return None;
    }

    serde_json::from_slice(&bytes).ok()
}
