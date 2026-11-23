//! `/ws/subscriptions/bind` reads one short `bind-with-token <token>` message.
//! A message larger than the route's limit is refused by the WebSocket layer
//! before the server reads it as a bind message, rather than buffered up to
//! the library's 64 MiB default.
#![cfg(feature = "subscriptions")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AppState, ServerConfig};
use helios_subscriptions::{SubscriptionConfig, SubscriptionEngine};
use tokio_tungstenite::tungstenite::Message;

/// Serves the FHIR routes with a subscription engine on a loopback port.
async fn serve() -> std::net::SocketAddr {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    let engine = Arc::new(SubscriptionEngine::new(
        SubscriptionConfig::default(),
        "http://localhost".to_string(),
    ));
    let state =
        AppState::new(backend, ServerConfig::for_testing()).with_subscription_engine(engine);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

/// Sends `message` as the bind message and returns the reason of the close
/// frame the server answers with, if it sends one before the connection ends.
async fn close_reason_after(addr: std::net::SocketAddr, message: String) -> Option<String> {
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws/subscriptions/bind"))
            .await
            .expect("upgrade");
    let _ = socket.send(Message::Text(message.into())).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Close(Some(close))) => return Some(close.reason.to_string()),
                Ok(Message::Close(None)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    })
    .await
    .expect("the server ends the connection")
}

#[tokio::test]
async fn an_oversized_bind_message_is_not_read_as_a_bind_message() {
    let addr = serve().await;

    let small = close_reason_after(addr, "bind-with-token not-a-token".to_string()).await;
    assert_eq!(
        small.as_deref(),
        Some("invalid-or-expired-token"),
        "a bind message within the limit is read and its token judged"
    );

    let oversized = format!("bind-with-token {}", "a".repeat(8 * 1024));
    let large = close_reason_after(addr, oversized).await;
    assert_ne!(
        large.as_deref(),
        Some("invalid-or-expired-token"),
        "an 8 KiB message must be refused before it is read as a bind message"
    );
}
