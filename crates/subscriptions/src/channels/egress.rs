//! Egress rules shared by the HTTP channels (rest-hook and FHIR Messaging).
//!
//! The endpoint of a Subscription is written by whoever can write the
//! resource, so a delivery is a request the server makes on that principal's
//! behalf. Unless private endpoints are allowed
//! (`HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true`, local development and
//! CI), three rules keep it away from the server's own network:
//!
//! 1. An endpoint whose host is a literal loopback / private / link-local /
//!    shared / unspecified address is refused before any request is made
//!    ([`check_endpoint_host`]).
//! 2. A DNS name is checked at connect time: the client resolves through
//!    [`PublicOnlyResolver`], which drops every such address, so a name that
//!    points inward (`localhost`, an internal record, a rebinding name) has
//!    nothing left to connect to.
//! 3. Redirects are never followed, whatever the setting: a redirect would
//!    move the request, and the Subscription's headers with it, to a host
//!    nobody checked.

use std::error::Error as StdError;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use helios_persistence::core::bulk_submit_input::redact_url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{Client, Url};

/// Build the HTTP client a channel delivers with. Redirects are never
/// followed; unless `private_endpoints_allowed`, names are resolved through
/// [`PublicOnlyResolver`].
pub(crate) fn build_client(private_endpoints_allowed: bool) -> Client {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());
    if !private_endpoints_allowed {
        builder = builder.dns_resolver(Arc::new(PublicOnlyResolver));
    }
    builder.build().expect("failed to build HTTP client")
}

/// Whether `ip` is an address a delivery must not reach by default:
/// loopback, private, link-local, shared (CGNAT) or unspecified, including
/// the IPv4-mapped IPv6 form of each.
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
    // 100.64.0.0/10 — shared address space (RFC 6598), used for carrier NAT
    // and by cloud providers for internal services.
    let shared = v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64;
    v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || shared
}

fn is_v6_private_or_link(addr: &Ipv6Addr) -> bool {
    let segments = addr.segments();
    // fc00::/7 — Unique Local Addresses (RFC 4193)
    let unique_local = (segments[0] & 0xfe00) == 0xfc00;
    // fe80::/10 — Link-Local Addresses
    let link_local = (segments[0] & 0xffc0) == 0xfe80;
    unique_local || link_local
}

/// Whether a Subscription's `channel.header` entry is skipped on delivery.
/// `Host` decides which backend answers on an origin that routes by it, so
/// the endpoint check above, and the credential check that picks which
/// receivers get the server's bearer token, would judge one host while
/// another answered. The rest would reframe the request (`Content-Length`,
/// `Transfer-Encoding`) or are hop-by-hop. Credentials (`Authorization`,
/// API keys) stay allowed: carrying them is what the element is for.
pub(crate) fn is_refused_delivery_header(name: &str) -> bool {
    let name = name.trim();
    [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "keep-alive",
        "te",
        "trailer",
        "upgrade",
    ]
    .iter()
    .any(|refused| name.eq_ignore_ascii_case(refused))
        || name.len() >= 6 && name[..6].eq_ignore_ascii_case("proxy-")
}

/// Inspect the receiver URL and return a rejection reason if it does not
/// parse or its host is a literal blocked address. DNS names pass here and
/// are checked when the client resolves them.
pub(crate) fn check_endpoint_host(endpoint: &str) -> Option<String> {
    let url = match Url::parse(endpoint) {
        Ok(u) => u,
        Err(_) => {
            return Some(format!("invalid endpoint URL: {}", redact_url(endpoint)));
        }
    };

    // The parser has already normalised an IPv4 host (`0x7f.1`, `2130706433`)
    // to dotted form; an IPv6 host comes back in brackets.
    let host = url.host_str()?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let ip: IpAddr = host.parse().ok()?; // a DNS name: checked at connect time

    is_blocked_ip(ip).then(|| {
        format!(
            "endpoint host {ip} is a private/loopback IP; set HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true to allow"
        )
    })
}

/// The reason a request failed because its endpoint's name resolved only to
/// blocked addresses, if that is why it failed. Such a failure is permanent:
/// retrying resolves the same name.
pub(crate) fn blocked_resolution(error: &reqwest::Error) -> Option<String> {
    let mut source: Option<&(dyn StdError + 'static)> = error.source();
    while let Some(e) = source {
        if let Some(blocked) = e.downcast_ref::<BlockedResolution>() {
            return Some(blocked.to_string());
        }
        source = e.source();
    }
    None
}

/// A name resolved, but to nothing a delivery may reach.
#[derive(Debug)]
struct BlockedResolution {
    host: String,
}

impl fmt::Display for BlockedResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "endpoint host {} resolves to a private/loopback IP; set HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true to allow",
            self.host
        )
    }
}

impl StdError for BlockedResolution {}

/// Resolves through the system resolver and drops every blocked address, so
/// the connection can only go to an address that passed the check: there is
/// no second lookup between check and connect for a rebinding name to win.
struct PublicOnlyResolver;

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
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            assert!(is_blocked_ip(blocked.parse().unwrap()), "{blocked}");
        }
        for public in [
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
            "2606:4700::1",
        ] {
            assert!(!is_blocked_ip(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn literal_hosts_are_checked_in_every_spelling() {
        for endpoint in [
            "http://127.0.0.1:8080/x",
            "http://[::1]:8080/x",
            "http://[::ffff:127.0.0.1]/x",
            // The URL parser normalises these to 127.0.0.1.
            "http://2130706433/x",
            "http://0x7f.1/x",
            "http://169.254.169.254/latest/meta-data",
        ] {
            let reason = check_endpoint_host(endpoint);
            assert!(
                reason
                    .as_deref()
                    .is_some_and(|r| r.contains("private/loopback")),
                "{endpoint}: {reason:?}"
            );
        }
        assert_eq!(check_endpoint_host("https://receiver.example.org/x"), None);
        assert_eq!(check_endpoint_host("https://93.184.216.34/x"), None);
    }

    #[test]
    fn routing_and_framing_headers_are_refused_whatever_their_case() {
        for name in [
            "Host",
            " host ",
            "Content-Length",
            "Transfer-Encoding",
            "Connection",
            "Keep-Alive",
            "TE",
            "Trailer",
            "Upgrade",
            "Proxy-Authorization",
        ] {
            assert!(is_refused_delivery_header(name), "{name}");
        }
        for name in [
            "Authorization",
            "X-Api-Key",
            "X-Proxy-Hint",
            "Accept",
            "Hostname",
        ] {
            assert!(!is_refused_delivery_header(name), "{name}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_inward_is_refused_at_connect_time() {
        let server = wiremock::MockServer::start().await;
        let endpoint = format!("http://localhost:{}/x", server.address().port());

        let error = build_client(false)
            .post(&endpoint)
            .send()
            .await
            .expect_err("localhost must not be reachable");
        let reason = blocked_resolution(&error).expect("blocked resolution");
        assert!(reason.contains("localhost"), "{reason}");
        assert!(server.received_requests().await.unwrap().is_empty());

        // The same name is reachable once private endpoints are allowed.
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let response = build_client(true).post(&endpoint).send().await.unwrap();
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let target = wiremock::MockServer::start().await;
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/elsewhere", target.uri())),
            )
            .mount(&server)
            .await;

        let response = build_client(true)
            .post(format!("{}/x", server.uri()))
            .header("X-Api-Key", "k")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        assert!(target.received_requests().await.unwrap().is_empty());
    }
}
