//! Peer-address extractor, for attributing a request that has no principal.
//!
//! [`PeerIp`] reads the [`ConnectInfo`] the binary attaches when it serves the
//! app, and yields `None` when there is none — a router driven directly in a
//! test, or a deployment that never asked for it. It is infallible by design:
//! no request should be rejected for lacking an address, only attributed
//! differently.

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

/// The address the request came from, when the server is wired to record it.
///
/// Not an identity: behind a proxy every request shares one. It is the last
/// resort for rate limiting an unauthenticated deployment, where the
/// alternative is one bucket for the whole world.
#[derive(Debug, Clone, Copy)]
pub struct PeerIp(pub Option<IpAddr>);

impl<S> FromRequestParts<S> for PeerIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(PeerIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip()),
        ))
    }
}

/// The address a per-peer rate limit bills `ip` to: an IPv4 address itself,
/// and for an IPv6 address the network of its first `ipv6_prefix` bits
/// (`HFS_RATE_LIMIT_IPV6_PREFIX`, default 64).
///
/// An IPv6 client is normally given a /64 at least and can send each request
/// from a new address in it, so billing the full address would hand out a
/// fresh budget per request. An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`)
/// is billed as its IPv4 address, not by a prefix it shares with every other
/// mapped host.
pub(crate) fn rate_limit_peer(ip: IpAddr, ipv6_prefix: u8) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let host_bits = 128 - u32::from(ipv6_prefix.min(128));
            let mask = u128::MAX.checked_shl(host_bits).unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    async fn extract(req: Request<()>) -> PeerIp {
        let (mut parts, _) = req.into_parts();
        PeerIp::from_request_parts(&mut parts, &())
            .await
            .expect("infallible")
    }

    #[tokio::test]
    async fn yields_none_when_the_server_records_no_peer() {
        assert!(
            extract(Request::builder().body(()).unwrap())
                .await
                .0
                .is_none()
        );
    }

    #[tokio::test]
    async fn reads_the_address_from_connect_info() {
        let addr: SocketAddr = "203.0.113.7:54321".parse().unwrap();
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));

        assert_eq!(extract(req).await.0, Some(addr.ip()));
    }

    fn billed(ip: &str, prefix: u8) -> IpAddr {
        rate_limit_peer(ip.parse().unwrap(), prefix)
    }

    #[test]
    fn ipv6_peers_in_one_prefix_share_a_bucket() {
        assert_eq!(
            billed("2001:db8:1:2::1", 64),
            billed("2001:db8:1:2:ffff::9", 64)
        );
        assert_eq!(
            billed("2001:db8:1:2:aaaa:bbbb:cccc:dddd", 64),
            "2001:db8:1:2::".parse::<IpAddr>().unwrap()
        );
        assert_ne!(billed("2001:db8:1:2::1", 64), billed("2001:db8:1:3::1", 64));
    }

    #[test]
    fn the_ipv6_prefix_sets_how_wide_a_bucket_is() {
        assert_eq!(
            billed("2001:db8:1:2::1", 48),
            billed("2001:db8:1:ff::1", 48)
        );
        assert_ne!(billed("2001:db8:1:2::1", 48), billed("2001:db8:2:2::1", 48));
        assert_ne!(billed("2001:db8::1", 128), billed("2001:db8::2", 128));
        assert_eq!(billed("2001:db8::1", 0), billed("fe80::1", 0));
    }

    #[test]
    fn ipv4_peers_and_mapped_ipv4_peers_are_billed_per_address() {
        let v4: IpAddr = "203.0.113.7".parse().unwrap();
        assert_eq!(billed("203.0.113.7", 64), v4);
        assert_eq!(billed("::ffff:203.0.113.7", 64), v4);
        assert_ne!(
            billed("::ffff:203.0.113.7", 64),
            billed("::ffff:203.0.113.8", 64)
        );
    }
}
