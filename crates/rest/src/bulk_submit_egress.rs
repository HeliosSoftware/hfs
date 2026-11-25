//! Where `$bulk-submit` may connect when the operator has not listed its Data
//! Providers in `HFS_BULK_SUBMIT_TRUSTED_PROVIDERS`.
//!
//! The manifest URL, the OAuth metadata URL, the token endpoint that document
//! names and every file URL come from the submitter, so each request is one
//! the server makes on the submitter's behalf. Unless
//! `HFS_BULK_SUBMIT_ALLOW_PRIVATE_NETWORKS=true`, none of them may reach a
//! loopback, private, link-local, shared (CGNAT) or unspecified address:
//!
//! 1. A URL whose host is such an address literally is refused before any
//!    request is sent ([`url_is_blocked`]), on the first request and on
//!    every redirect hop.
//! 2. A DNS name is resolved through [`PublicOnlyResolver`], which drops every
//!    such address, so the connection can only go to an address that passed
//!    the check: there is no second lookup for a rebinding name to win.
//!
//! A listed provider is the operator's own choice, and the list already
//! confines every request to it, so with a list configured these rules do not
//! apply.

use std::error::Error as StdError;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// Why a URL was refused, after the URL itself.
pub(crate) const REFUSED_DESTINATION: &str = "is not a permitted destination: loopback, private and link-local addresses are refused unless HFS_BULK_SUBMIT_ALLOW_PRIVATE_NETWORKS=true or the origin is listed in HFS_BULK_SUBMIT_TRUSTED_PROVIDERS";

/// Whether `ip` is an address a submitter-chosen request must not reach:
/// loopback, private, link-local, shared (CGNAT), `0.0.0.0/8` or unspecified,
/// including the IPv4-mapped IPv6 form of each.
pub(crate) fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_blocked_v4(v4),
            None => v6.is_loopback() || v6.is_unspecified() || is_v6_private_or_link(&v6),
        },
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let [first, second, ..] = v4.octets();
    // 100.64.0.0/10, shared address space (RFC 6598): carrier NAT, and the
    // range some clouds put internal services in.
    let shared = first == 100 && (second & 0xc0) == 64;
    // 0.0.0.0/8, "this network": Linux connects 0.0.0.0 to the local host.
    let this_network = first == 0;
    v4.is_loopback() || v4.is_private() || v4.is_link_local() || shared || this_network
}

fn is_v6_private_or_link(addr: &Ipv6Addr) -> bool {
    let first = addr.segments()[0];
    // fc00::/7, unique local addresses (RFC 4193).
    let unique_local = (first & 0xfe00) == 0xfc00;
    // fe80::/10, link-local addresses.
    let link_local = (first & 0xffc0) == 0xfe80;
    unique_local || link_local
}

/// Whether the host of `url` is a literal blocked address. A DNS name passes
/// here and is checked when [`PublicOnlyResolver`] resolves it. The URL
/// parser has already normalised an IPv4 host spelled `0x7f.1` or
/// `2130706433` to dotted form.
pub(crate) fn url_is_blocked(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(v4)) => is_blocked_ip(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => is_blocked_ip(IpAddr::V6(v6)),
        Some(Host::Domain(_)) | None => false,
    }
}

/// A name resolved, but only to addresses a request may not reach.
#[derive(Debug)]
pub(crate) struct BlockedResolution {
    host: String,
}

impl fmt::Display for BlockedResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {REFUSED_DESTINATION}", self.host)
    }
}

impl StdError for BlockedResolution {}

/// Whether `error` failed because its host resolved only to blocked
/// addresses. Such a failure is permanent: a retry resolves the same name.
pub(crate) fn is_blocked_resolution(error: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn StdError + 'static)> = error.source();
    while let Some(e) = source {
        if e.downcast_ref::<BlockedResolution>().is_some() {
            return true;
        }
        source = e.source();
    }
    false
}

/// Resolves through the system resolver and drops every blocked address.
pub(crate) struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let resolved: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let allowed: Vec<SocketAddr> = resolved
                .iter()
                .copied()
                .filter(|addr| !is_blocked_ip(addr.ip()))
                .collect();
            if allowed.is_empty() && !resolved.is_empty() {
                return Err(Box::new(BlockedResolution { host }) as Box<dyn StdError + Send + Sync>);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_addresses_cover_every_inward_range() {
        for blocked in [
            "127.0.0.1",
            "127.255.255.254",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            assert!(is_blocked_ip(blocked.parse().unwrap()), "{blocked}");
        }
        for public in [
            "93.184.216.34",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1",
            "::ffff:93.184.216.34",
        ] {
            assert!(!is_blocked_ip(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn literal_hosts_are_checked_in_every_spelling() {
        for url in [
            "http://127.0.0.1:8080/x",
            "http://[::1]:8080/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://2130706433/x",
            "http://0x7f.1/x",
            "http://0/x",
            "http://169.254.169.254/latest/meta-data",
            "https://[fd00::1]/manifest.json",
        ] {
            assert!(url_is_blocked(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "https://provider.example.org/x",
            "https://93.184.216.34/x",
            "http://localhost:8080/x",
        ] {
            assert!(!url_is_blocked(&Url::parse(url).unwrap()), "{url}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_inward_is_refused_at_connect_time() {
        let server = wiremock::MockServer::start().await;
        let url = format!("http://localhost:{}/x", server.address().port());
        let client = reqwest::Client::builder()
            .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
            .build()
            .unwrap();

        let error = client
            .get(&url)
            .send()
            .await
            .expect_err("localhost must not be reachable");
        assert!(is_blocked_resolution(&error), "{error:?}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
