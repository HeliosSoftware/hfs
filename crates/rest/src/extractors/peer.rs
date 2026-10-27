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
use std::net::{IpAddr, SocketAddr};

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
}
