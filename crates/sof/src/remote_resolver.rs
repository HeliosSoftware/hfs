//! Remote reference resolution — configuration and the trusted-server gate.
//!
//! This module holds the configuration and the allowlist + SSRF guard for remote
//! `resolve()`. See the user-facing docs in `book/src/ch06-sql-on-fhir.md`
//! ("Remote resolution against trusted servers") and the storage-backed follow-up
//! tracked in issue [#167](https://github.com/HeliosSoftware/hfs/issues/167).
//!
//! It contains **no networking**. It only decides *whether* a given reference URL
//! is eligible to be fetched from a trusted server, under a default-deny policy.
//! The actual prefetch/fetch stage (Phase 4) consumes [`RemoteResolveConfig`] and
//! reuses [`is_disallowed_ip`] for the post-DNS rebinding re-check.
//!
//! ## Security posture (default-deny, strict allowlist)
//!
//! A reference is fetchable only if **all** of the following hold:
//! 1. Remote resolution is enabled and the allowlist is non-empty.
//! 2. The reference is an absolute `http`/`https` URL.
//! 3. It matches an allowlist entry on **scheme + host + port + path-prefix**
//!    (parsed-URL comparison, never substring — so
//!    `https://evil/?u=https://trusted.org` does not match `https://trusted.org`).
//!
//! Because matching requires *scheme equality*, an `http://` reference can only
//! match an `http://` allowlist entry — i.e. plaintext is allowed only where an
//! operator has explicitly opted in (typically a trusted internal/test server).
//! Likewise, a reference to a private/loopback IP literal can only be fetched if
//! that exact IP host was explicitly allowlisted; otherwise it fails to match.
//!
//! The [`is_blocked_address`] guard runs at the fetch stage on DNS-resolved
//! addresses (DNS-rebinding defense). By default an allowlisted *hostname* that
//! resolves to a private/internal address is refused; setting
//! `SOF_RESOLVE_ALLOW_PRIVATE_ADDRESSES=true`
//! ([`RemoteResolveConfig::allow_private_addresses`]) permits RFC1918 / IPv6-ULA
//! targets so references can point at an internal load balancer or reverse proxy
//! (e.g. Traefik) by hostname. The most dangerous ranges — loopback and
//! link-local (incl. the cloud-metadata endpoint) — stay blocked either way.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use url::Url;

/// Environment variable names (documented in `book/src/ch06-sql-on-fhir.md`).
pub mod env_keys {
    pub const ENABLED: &str = "SOF_RESOLVE_REMOTE";
    pub const ALLOWED_BASE_URLS: &str = "SOF_RESOLVE_ALLOWED_BASE_URLS";
    pub const TIMEOUT_MS: &str = "SOF_RESOLVE_TIMEOUT_MS";
    pub const MAX_FETCHES: &str = "SOF_RESOLVE_MAX_FETCHES";
    pub const MAX_DEPTH: &str = "SOF_RESOLVE_MAX_DEPTH";
    pub const MAX_RESPONSE_BYTES: &str = "SOF_RESOLVE_MAX_RESPONSE_BYTES";
    pub const CONCURRENCY: &str = "SOF_RESOLVE_CONCURRENCY";
    /// Per-host bearer tokens, formatted `host=token,host2=token2`.
    pub const AUTH: &str = "SOF_RESOLVE_AUTH";
    /// Allow allowlisted hosts to resolve to private/internal addresses.
    pub const ALLOW_PRIVATE_ADDRESSES: &str = "SOF_RESOLVE_ALLOW_PRIVATE_ADDRESSES";
    /// Max entries in the cross-chunk fetched-resource cache (streaming path).
    pub const CACHE_MAX_ENTRIES: &str = "SOF_RESOLVE_CACHE_MAX_ENTRIES";
}

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_MAX_FETCHES: usize = 256;
const DEFAULT_MAX_DEPTH: usize = 1;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 5_000_000;
const DEFAULT_CONCURRENCY: usize = 8;
const DEFAULT_CACHE_MAX_ENTRIES: usize = 10_000;

/// Configuration for remote (trusted-server) `resolve()`.
///
/// Defaults are **off**: an empty/default config never permits a fetch. Build one
/// from the process environment with [`RemoteResolveConfig::from_env`].
#[derive(Debug, Clone)]
pub struct RemoteResolveConfig {
    /// Master switch (`SOF_RESOLVE_REMOTE`). When `false`, nothing is ever fetched.
    pub enabled: bool,
    /// Trusted base URLs that references must match to be fetchable.
    pub allowed_base_urls: Vec<AllowedBaseUrl>,
    /// Per-request timeout (`SOF_RESOLVE_TIMEOUT_MS`).
    pub timeout: Duration,
    /// Hard cap on total fetches per run (`SOF_RESOLVE_MAX_FETCHES`).
    pub max_fetches: usize,
    /// Bounded prefetch rounds for chained references (`SOF_RESOLVE_MAX_DEPTH`).
    pub max_depth: usize,
    /// Maximum accepted response size in bytes (`SOF_RESOLVE_MAX_RESPONSE_BYTES`).
    pub max_response_bytes: usize,
    /// Maximum concurrent fetches (`SOF_RESOLVE_CONCURRENCY`).
    pub concurrency: usize,
    /// Optional per-host bearer tokens (`SOF_RESOLVE_AUTH`), keyed by lowercased
    /// host. Sent only on requests to that exact (already-allowlisted) host.
    pub bearer_tokens: std::collections::HashMap<String, String>,
    /// Allow allowlisted *hostnames* to resolve to private/internal addresses —
    /// RFC1918 (`10/8`, `172.16/12`, `192.168/16`) and IPv6 ULA (`fc00::/7`)
    /// (`SOF_RESOLVE_ALLOW_PRIVATE_ADDRESSES`, default `false`).
    ///
    /// Needed for internal deployments where references point at an internal
    /// load balancer / reverse proxy (e.g. Traefik) by hostname. Even when `true`,
    /// the always-blocked ranges (loopback, link-local incl. cloud metadata,
    /// multicast, broadcast, unspecified, CGNAT, reserved) remain refused — see
    /// [`is_blocked_address`].
    pub allow_private_addresses: bool,
    /// Maximum entries in the cross-chunk fetched-resource cache used by the
    /// streaming path (`SOF_RESOLVE_CACHE_MAX_ENTRIES`). The cache (LRU, with
    /// negative caching of misses) lets a reference recurring across chunks be
    /// fetched once; bounding it keeps streaming memory bounded — evicted-then-
    /// reused references are re-fetched. Ignored by the single-Bundle path, which
    /// holds all references in one pass.
    pub cache_max_entries: usize,
}

impl Default for RemoteResolveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_base_urls: Vec::new(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            max_fetches: DEFAULT_MAX_FETCHES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            concurrency: DEFAULT_CONCURRENCY,
            bearer_tokens: std::collections::HashMap::new(),
            allow_private_addresses: false,
            cache_max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        }
    }
}

impl RemoteResolveConfig {
    /// Builds the configuration from the process environment.
    ///
    /// Malformed numeric values fall back to their defaults (with a `tracing`
    /// warning); malformed allowlist entries are skipped individually.
    pub fn from_env() -> Self {
        Self::from_env_with(|key| std::env::var(key).ok())
    }

    /// Builds the configuration from an arbitrary environment lookup.
    ///
    /// Exposed so the parsing logic can be unit-tested without touching the
    /// process-global environment.
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Self {
        let enabled = get(env_keys::ENABLED)
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let allowed_base_urls = get(env_keys::ALLOWED_BASE_URLS)
            .map(|v| parse_allowlist(&v))
            .unwrap_or_default();

        let timeout = Duration::from_millis(parse_or_default(
            get(env_keys::TIMEOUT_MS).as_deref(),
            env_keys::TIMEOUT_MS,
            DEFAULT_TIMEOUT_MS,
        ));
        let max_fetches = parse_or_default(
            get(env_keys::MAX_FETCHES).as_deref(),
            env_keys::MAX_FETCHES,
            DEFAULT_MAX_FETCHES,
        );
        let max_depth = parse_or_default(
            get(env_keys::MAX_DEPTH).as_deref(),
            env_keys::MAX_DEPTH,
            DEFAULT_MAX_DEPTH,
        );
        let max_response_bytes = parse_or_default(
            get(env_keys::MAX_RESPONSE_BYTES).as_deref(),
            env_keys::MAX_RESPONSE_BYTES,
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let concurrency = parse_or_default(
            get(env_keys::CONCURRENCY).as_deref(),
            env_keys::CONCURRENCY,
            DEFAULT_CONCURRENCY,
        )
        .max(1);

        let bearer_tokens = get(env_keys::AUTH)
            .map(|v| parse_bearer_tokens(&v))
            .unwrap_or_default();

        let allow_private_addresses = get(env_keys::ALLOW_PRIVATE_ADDRESSES)
            .map(|v| parse_bool(&v))
            .unwrap_or(false);

        let cache_max_entries = parse_or_default(
            get(env_keys::CACHE_MAX_ENTRIES).as_deref(),
            env_keys::CACHE_MAX_ENTRIES,
            DEFAULT_CACHE_MAX_ENTRIES,
        )
        .max(1);

        Self {
            enabled,
            allowed_base_urls,
            timeout,
            max_fetches,
            max_depth,
            max_response_bytes,
            concurrency,
            bearer_tokens,
            allow_private_addresses,
            cache_max_entries,
        }
    }

    /// Returns the bearer token configured for `host`, if any (case-insensitive).
    pub fn bearer_for_host(&self, host: &str) -> Option<&str> {
        self.bearer_tokens
            .get(&host.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Whether remote resolution can fetch anything at all (enabled *and* the
    /// allowlist is non-empty). When `false`, [`Self::fetch_decision`] always denies.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.allowed_base_urls.is_empty()
    }

    /// Decides whether `reference` may be fetched from a trusted server.
    ///
    /// This is the single gate enforcing the default-deny policy. It performs no
    /// I/O and no DNS resolution.
    pub fn fetch_decision(&self, reference: &str) -> FetchDecision {
        if !self.enabled {
            return FetchDecision::Deny(DenyReason::Disabled);
        }

        let url = match Url::parse(reference) {
            Ok(u) => u,
            Err(_) => return FetchDecision::Deny(DenyReason::NotAbsoluteUrl),
        };

        // `Url::parse` accepts relative-looking inputs as some schemes; require a
        // real network scheme with a host.
        match url.scheme() {
            "http" | "https" => {}
            _ => return FetchDecision::Deny(DenyReason::UnsupportedScheme),
        }
        if url.host_str().is_none() {
            return FetchDecision::Deny(DenyReason::NotAbsoluteUrl);
        }

        if self.allowed_base_urls.iter().any(|base| base.matches(&url)) {
            FetchDecision::Allow
        } else {
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        }
    }
}

/// A parsed, normalised trusted base URL.
///
/// Matching compares scheme, lowercased host, effective port (scheme default when
/// absent), and a path prefix anchored on segment boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedBaseUrl {
    scheme: String,
    host: String,
    port: u16,
    /// Path prefix without a trailing slash; empty means "any path on this host".
    path_prefix: String,
}

impl AllowedBaseUrl {
    /// Parses a single allowlist entry such as `https://fhir.example.org/r4`.
    pub fn parse(raw: &str) -> Result<Self, AllowlistParseError> {
        let url = Url::parse(raw.trim()).map_err(|_| AllowlistParseError::InvalidUrl)?;

        let scheme = url.scheme().to_string();
        if scheme != "http" && scheme != "https" {
            return Err(AllowlistParseError::UnsupportedScheme);
        }

        let host = url
            .host_str()
            .ok_or(AllowlistParseError::MissingHost)?
            .to_ascii_lowercase();

        let port = url
            .port_or_known_default()
            .ok_or(AllowlistParseError::MissingPort)?;

        let path_prefix = url.path().trim_end_matches('/').to_string();

        Ok(Self {
            scheme,
            host,
            port,
            path_prefix,
        })
    }

    /// Returns `true` if `url` falls within this trusted base.
    fn matches(&self, url: &Url) -> bool {
        url.scheme() == self.scheme
            && url
                .host_str()
                .map(|h| h.eq_ignore_ascii_case(&self.host))
                .unwrap_or(false)
            && url.port_or_known_default() == Some(self.port)
            && path_prefix_matches(&self.path_prefix, url.path())
    }
}

/// Errors from parsing a single allowlist entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AllowlistParseError {
    #[error("not a valid absolute URL")]
    InvalidUrl,
    #[error("unsupported scheme (only http/https are allowed)")]
    UnsupportedScheme,
    #[error("URL has no host")]
    MissingHost,
    #[error("URL has no resolvable port")]
    MissingPort,
}

/// Outcome of [`RemoteResolveConfig::fetch_decision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchDecision {
    /// The reference matches a trusted base and may be fetched.
    Allow,
    /// The reference must not be fetched, with the reason.
    Deny(DenyReason),
}

impl FetchDecision {
    /// Convenience: whether the decision is [`FetchDecision::Allow`].
    pub fn is_allowed(&self) -> bool {
        matches!(self, FetchDecision::Allow)
    }
}

/// Why a reference was denied remote resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Remote resolution is disabled (`SOF_RESOLVE_REMOTE` is not set/true).
    Disabled,
    /// The reference is not an absolute URL (e.g. `Patient/1`, `#frag`).
    NotAbsoluteUrl,
    /// The URL scheme is not `http`/`https`.
    UnsupportedScheme,
    /// The URL did not match any trusted base in the allowlist.
    NotAllowlisted,
}

/// Strict SSRF guard: `true` if `ip` must never be the target of a fetch under
/// the default (most restrictive) policy.
///
/// Equivalent to [`is_blocked_address(ip, false)`](is_blocked_address): the union
/// of the always-blocked ranges and the private/internal ranges. Retained as the
/// canonical predicate used where private addresses are never acceptable.
pub fn is_disallowed_ip(ip: IpAddr) -> bool {
    is_blocked_address(ip, false)
}

/// SSRF guard parameterised by whether private/internal addresses are permitted.
///
/// Applied by the fetch stage to addresses obtained from DNS resolution (to defeat
/// DNS rebinding): an allowlisted *hostname* that resolves to a blocked address is
/// refused.
///
/// - The **always-blocked** ranges are refused regardless of `allow_private`:
///   loopback, link-local (incl. the `169.254.169.254` cloud-metadata endpoint),
///   unspecified, broadcast, multicast, carrier-grade NAT, documentation/test, and
///   reserved (`0.0.0.0/8`, `240.0.0.0/4`).
/// - The **private/internal** ranges — RFC1918 (`10/8`, `172.16/12`, `192.168/16`)
///   and IPv6 ULA (`fc00::/7`) — are refused only when `allow_private` is `false`.
///   Setting it `true` (via `SOF_RESOLVE_ALLOW_PRIVATE_ADDRESSES`) lets an
///   allowlisted hostname point at an internal load balancer / reverse proxy.
pub fn is_blocked_address(ip: IpAddr, allow_private: bool) -> bool {
    let (always_blocked, private) = match ip {
        IpAddr::V4(v4) => (is_always_blocked_ipv4(v4), v4.is_private()),
        IpAddr::V6(v6) => {
            // IPv4-mapped (`::ffff:a.b.c.d`) addresses are reachable as IPv4.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                (is_always_blocked_ipv4(mapped), mapped.is_private())
            } else {
                (is_always_blocked_ipv6(v6), is_unique_local_ipv6(v6))
            }
        }
    };
    always_blocked || (!allow_private && private)
}

fn is_always_blocked_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        // "this network" 0.0.0.0/8
        || a == 0
        // carrier-grade NAT 100.64.0.0/10
        || (a == 100 && (64..=127).contains(&b))
        // reserved for future use 240.0.0.0/4 (and 255.* broadcast)
        || a >= 240
}

fn is_always_blocked_ipv6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // link-local unicast fe80::/10
        || (first & 0xffc0) == 0xfe80
}

/// IPv6 unique-local addresses `fc00::/7` (the ULA "private" range).
fn is_unique_local_ipv6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// Anchored path-prefix match: `path` must equal `prefix` or continue past it at
/// a `/` boundary. An empty `prefix` matches any path.
fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let path = path.trim_end_matches('/');
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Parses a comma-separated allowlist into trusted base URLs, skipping (and
/// warning about) malformed entries. Exposed for CLI/server wiring that builds a
/// [`RemoteResolveConfig`] from flags rather than the environment.
pub fn parse_allowed_base_urls(csv: &str) -> Vec<AllowedBaseUrl> {
    parse_allowlist(csv)
}

/// Parses `host=token,host2=token2` into a per-host bearer map (hosts lowercased).
fn parse_bearer_tokens(csv: &str) -> std::collections::HashMap<String, String> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|pair| {
            let (host, token) = pair.split_once('=')?;
            let host = host.trim();
            let token = token.trim();
            if host.is_empty() || token.is_empty() {
                tracing::warn!(pair, "ignoring malformed {} entry", env_keys::AUTH);
                return None;
            }
            Some((host.to_ascii_lowercase(), token.to_string()))
        })
        .collect()
}

/// Parses a comma-separated allowlist, skipping (and warning about) bad entries.
fn parse_allowlist(csv: &str) -> Vec<AllowedBaseUrl> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|entry| match AllowedBaseUrl::parse(entry) {
            Ok(base) => Some(base),
            Err(err) => {
                tracing::warn!(
                    entry,
                    error = %err,
                    "ignoring invalid {} entry",
                    env_keys::ALLOWED_BASE_URLS
                );
                None
            }
        })
        .collect()
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_or_default<T: std::str::FromStr>(value: Option<&str>, key: &str, default: T) -> T {
    match value {
        None => default,
        Some(raw) => match raw.trim().parse() {
            Ok(parsed) => parsed,
            Err(_) => {
                tracing::warn!(key, value = raw, "invalid value; using default");
                default
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(enabled: bool, allow: &[&str]) -> RemoteResolveConfig {
        RemoteResolveConfig {
            enabled,
            allowed_base_urls: allow
                .iter()
                .map(|s| AllowedBaseUrl::parse(s).expect("valid test base"))
                .collect(),
            ..Default::default()
        }
    }

    // ---- Phase 1: configuration parsing -------------------------------------

    #[test]
    fn default_config_is_off() {
        let c = RemoteResolveConfig::default();
        assert!(!c.enabled);
        assert!(c.allowed_base_urls.is_empty());
        assert!(!c.is_active());
        assert_eq!(c.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert_eq!(c.max_depth, DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn from_env_parses_values() {
        let env: HashMap<&str, &str> = HashMap::from([
            (env_keys::ENABLED, "true"),
            (
                env_keys::ALLOWED_BASE_URLS,
                "https://fhir.example.org/r4, https://hapi.example.com/baseR4",
            ),
            (env_keys::TIMEOUT_MS, "1500"),
            (env_keys::MAX_FETCHES, "10"),
            (env_keys::MAX_DEPTH, "3"),
            (env_keys::CONCURRENCY, "4"),
        ]);
        let c = RemoteResolveConfig::from_env_with(|k| env.get(k).map(|s| s.to_string()));

        assert!(c.enabled);
        assert!(c.is_active());
        assert_eq!(c.allowed_base_urls.len(), 2);
        assert_eq!(c.timeout, Duration::from_millis(1500));
        assert_eq!(c.max_fetches, 10);
        assert_eq!(c.max_depth, 3);
        assert_eq!(c.concurrency, 4);
    }

    #[test]
    fn from_env_defaults_off_when_unset() {
        let c = RemoteResolveConfig::from_env_with(|_| None);
        assert!(!c.enabled);
        assert!(c.allowed_base_urls.is_empty());
    }

    #[test]
    fn from_env_skips_invalid_allowlist_entries() {
        let env: HashMap<&str, &str> = HashMap::from([
            (env_keys::ENABLED, "1"),
            (
                env_keys::ALLOWED_BASE_URLS,
                "https://ok.example.org/fhir, not-a-url, ftp://nope.example.org, ",
            ),
        ]);
        let c = RemoteResolveConfig::from_env_with(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(c.allowed_base_urls.len(), 1);
    }

    #[test]
    fn from_env_bad_numbers_fall_back_to_default() {
        let env: HashMap<&str, &str> =
            HashMap::from([(env_keys::TIMEOUT_MS, "abc"), (env_keys::MAX_FETCHES, "")]);
        let c = RemoteResolveConfig::from_env_with(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(c.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert_eq!(c.max_fetches, DEFAULT_MAX_FETCHES);
    }

    #[test]
    fn bool_parsing_is_lenient() {
        for v in ["1", "true", "TRUE", "Yes", "on"] {
            assert!(parse_bool(v), "{v} should be true");
        }
        for v in ["0", "false", "no", "", "off", "maybe"] {
            assert!(!parse_bool(v), "{v} should be false");
        }
    }

    // ---- Phase 2: allowlist matching ----------------------------------------

    #[test]
    fn allows_reference_under_trusted_base() {
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        assert!(
            c.fetch_decision("https://fhir.example.org/r4/Patient/123")
                .is_allowed()
        );
        // The base path itself matches.
        assert!(c.fetch_decision("https://fhir.example.org/r4").is_allowed());
        // Query strings are irrelevant to the host/path decision.
        assert!(
            c.fetch_decision("https://fhir.example.org/r4/Patient/1?_format=json")
                .is_allowed()
        );
    }

    #[test]
    fn path_prefix_is_anchored_on_segment_boundary() {
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        // `/r4extra` must NOT be treated as under `/r4`.
        assert_eq!(
            c.fetch_decision("https://fhir.example.org/r4extra/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
        assert_eq!(
            c.fetch_decision("https://fhir.example.org/other"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
    }

    #[test]
    fn host_scheme_and_port_must_match() {
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        // Wrong host.
        assert_eq!(
            c.fetch_decision("https://evil.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
        // Wrong scheme (only the https base is trusted).
        assert_eq!(
            c.fetch_decision("http://fhir.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
        // Wrong (explicit) port.
        assert_eq!(
            c.fetch_decision("https://fhir.example.org:8443/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
    }

    #[test]
    fn substring_smuggling_does_not_match() {
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        // Host is evil.com; the trusted URL only appears in the query.
        assert_eq!(
            c.fetch_decision("https://evil.com/?u=https://fhir.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
        // Userinfo trick: real host is still evil.com.
        assert_eq!(
            c.fetch_decision("https://fhir.example.org@evil.com/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
    }

    #[test]
    fn explicit_http_base_allows_plaintext() {
        // Operators may opt into a trusted internal/test http server.
        let c = cfg(true, &["http://localhost:8080/baseR4"]);
        assert!(
            c.fetch_decision("http://localhost:8080/baseR4/Patient/1")
                .is_allowed()
        );
        // https to the same host/port is a different base and is not trusted.
        assert_eq!(
            c.fetch_decision("https://localhost:8080/baseR4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
    }

    #[test]
    fn literal_private_ip_requires_explicit_allowlisting() {
        // Not allowlisted -> denied (this is the SSRF-by-literal-IP case).
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        assert_eq!(
            c.fetch_decision("https://10.0.0.5/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
        // Explicitly allowlisted private host -> the operator owns that decision.
        let c2 = cfg(true, &["https://10.0.0.5/r4"]);
        assert!(
            c2.fetch_decision("https://10.0.0.5/r4/Patient/1")
                .is_allowed()
        );
    }

    #[test]
    fn non_absolute_and_unsupported_schemes_are_denied() {
        let c = cfg(true, &["https://fhir.example.org/r4"]);
        assert_eq!(
            c.fetch_decision("Patient/123"),
            FetchDecision::Deny(DenyReason::NotAbsoluteUrl)
        );
        assert_eq!(
            c.fetch_decision("#contained-1"),
            FetchDecision::Deny(DenyReason::NotAbsoluteUrl)
        );
        assert_eq!(
            c.fetch_decision("ftp://fhir.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::UnsupportedScheme)
        );
        assert_eq!(
            c.fetch_decision("file:///etc/passwd"),
            FetchDecision::Deny(DenyReason::UnsupportedScheme)
        );
    }

    #[test]
    fn disabled_or_empty_allowlist_denies_everything() {
        let disabled = cfg(false, &["https://fhir.example.org/r4"]);
        assert_eq!(
            disabled.fetch_decision("https://fhir.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::Disabled)
        );

        let empty = cfg(true, &[]);
        assert!(!empty.is_active());
        assert_eq!(
            empty.fetch_decision("https://fhir.example.org/r4/Patient/1"),
            FetchDecision::Deny(DenyReason::NotAllowlisted)
        );
    }

    #[test]
    fn root_base_matches_any_path_on_host() {
        let c = cfg(true, &["https://fhir.example.org"]);
        assert!(
            c.fetch_decision("https://fhir.example.org/anything/here")
                .is_allowed()
        );
        assert!(c.fetch_decision("https://fhir.example.org/").is_allowed());
    }

    // ---- Phase 2: SSRF IP guard ---------------------------------------------

    #[test]
    fn disallowed_ipv4_ranges() {
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "192.0.2.1",       // documentation
            "255.255.255.255",
            "224.0.0.1", // multicast
            "240.0.0.1", // reserved
        ] {
            assert!(
                is_disallowed_ip(ip.parse().unwrap()),
                "{ip} should be disallowed"
            );
        }
    }

    #[test]
    fn allowed_public_ipv4() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(
                !is_disallowed_ip(ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
    }

    #[test]
    fn disallowed_ipv6_ranges() {
        for ip in [
            "::1",                    // loopback
            "fe80::1",                // link-local
            "fc00::1",                // unique local
            "fd12:3456::1",           // unique local
            "ff02::1",                // multicast
            "::",                     // unspecified
            "::ffff:127.0.0.1",       // v4-mapped loopback
            "::ffff:169.254.169.254", // v4-mapped metadata
        ] {
            assert!(
                is_disallowed_ip(ip.parse().unwrap()),
                "{ip} should be disallowed"
            );
        }
    }

    #[test]
    fn allowed_public_ipv6() {
        for ip in [
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(
                !is_disallowed_ip(ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
    }

    // ---- Phase 5: private-address opt-in (internal load balancers) ----------

    #[test]
    fn allow_private_permits_rfc1918_and_ula() {
        // With opt-in, internal LB ranges (RFC1918 + IPv6 ULA) are reachable...
        for ip in [
            "10.0.0.5",
            "172.16.4.4",
            "192.168.1.10",
            "fc00::1",
            "fd12:3456::1",
        ] {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(
                is_blocked_address(addr, false),
                "{ip} must be blocked by default"
            );
            assert!(
                !is_blocked_address(addr, true),
                "{ip} must be permitted when allow_private is set"
            );
        }
    }

    #[test]
    fn always_blocked_ranges_ignore_allow_private() {
        // ...but loopback, link-local/metadata, and friends stay blocked even then.
        for ip in [
            "127.0.0.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "100.64.0.1", // CGNAT
            "224.0.0.1",  // multicast
            "240.0.0.1",  // reserved
            "::1",
            "fe80::1",          // link-local
            "::ffff:127.0.0.1", // v4-mapped loopback
        ] {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(
                is_blocked_address(addr, true),
                "{ip} must stay blocked even with allow_private"
            );
        }
    }

    #[test]
    fn public_addresses_allowed_regardless_of_flag() {
        for ip in ["8.8.8.8", "2606:4700:4700::1111"] {
            let addr: IpAddr = ip.parse().unwrap();
            assert!(!is_blocked_address(addr, false));
            assert!(!is_blocked_address(addr, true));
        }
    }

    #[test]
    fn from_env_parses_allow_private() {
        let env: HashMap<&str, &str> = HashMap::from([(env_keys::ALLOW_PRIVATE_ADDRESSES, "true")]);
        let c = RemoteResolveConfig::from_env_with(|k| env.get(k).map(|s| s.to_string()));
        assert!(c.allow_private_addresses);
        // Default (unset) stays false.
        let d = RemoteResolveConfig::from_env_with(|_| None);
        assert!(!d.allow_private_addresses);
    }
}
