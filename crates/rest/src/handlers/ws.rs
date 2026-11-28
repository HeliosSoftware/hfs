//! WebSocket subscription binding handler.
//!
//! Implements the server-side WebSocket endpoint that clients connect to
//! after obtaining a binding token via `$get-ws-binding-token`.
//!
//! The protocol is unidirectional (server → client): the server streams
//! notification bundles as JSON text frames after the client sends a
//! `bind-with-token <token>` message.
//!
//! A connection that does not bind within [`BIND_TIMEOUT`], or does not
//! accept a frame within [`SEND_TIMEOUT`], is closed: an upgraded socket is
//! outside the HTTP request timeout, and a peer that stops reading would
//! otherwise park the handler in `send` for as long as it keeps the socket
//! open while its notification queue fills.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket},
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use helios_persistence::core::ResourceStorage;
use tracing::{debug, info, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// How long a connection may take to send its `bind-with-token` message.
const BIND_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a bound client may take to accept one frame.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The largest message or frame the route reads from a client. The protocol
/// reads one `bind-with-token <token>` message of a few dozen bytes and
/// ignores everything after it, so the 64 MiB message and 16 MiB frame the
/// WebSocket library allows by default would only let a peer make the server
/// buffer that much per connection.
const MAX_INCOMING_MESSAGE_BYTES: usize = 4 * 1024;

/// How long the close frame for an oversized message may take to send. The
/// peer may be bound and not reading, so the send must not wait on it.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// WebSocket binding handler.
///
/// The client connects to `/ws/subscriptions/bind` and then sends
/// `bind-with-token <binding-token>` as the first message.
pub async fn ws_bind_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    ws: WebSocketUpgrade,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let engine = state
        .subscription_engine()
        .ok_or(RestError::NotImplemented {
            feature: "Subscriptions".to_string(),
        })?;

    let engine = Arc::clone(engine);
    let base_url = state.public_base_url_for_request(&tenant);
    let request_tenant_id = tenant.tenant_id().to_string();

    // Upgrade the HTTP connection to WebSocket.
    Ok(ws
        .max_message_size(MAX_INCOMING_MESSAGE_BYTES)
        .max_frame_size(MAX_INCOMING_MESSAGE_BYTES)
        // The library reads into a 128 KiB buffer per connection by default,
        // which no message this route accepts can fill.
        .read_buffer_size(MAX_INCOMING_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            handle_ws_connection(socket, engine, request_tenant_id, base_url)
        }))
}

#[derive(Debug, PartialEq, Eq)]
enum BindingTokenRejection {
    InvalidOrExpired,
    TenantMismatch { token_tenant_id: String },
}

fn validate_binding_token_for_request(
    manager: &helios_subscriptions::WsBindingTokenManager,
    token: &str,
    request_tenant_id: &str,
) -> Result<(String, String), BindingTokenRejection> {
    let (token_tenant_id, subscription_id) = manager
        .validate_and_consume(token)
        .ok_or(BindingTokenRejection::InvalidOrExpired)?;
    if token_tenant_id != request_tenant_id {
        return Err(BindingTokenRejection::TenantMismatch { token_tenant_id });
    }
    Ok((token_tenant_id, subscription_id))
}

/// Handles the WebSocket connection lifecycle after upgrade.
///
/// Waits for a `bind-with-token` message, sends a handshake notification,
/// then streams events from the subscription engine until disconnect.
async fn handle_ws_connection(
    mut socket: WebSocket,
    engine: Arc<helios_subscriptions::SubscriptionEngine>,
    request_tenant_id: String,
    base_url: String,
) {
    let token = match receive_bind_token(&mut socket).await {
        Some(token) => token,
        None => return,
    };

    // Validate and consume the binding token (single-use).
    let (tenant_id, subscription_id) = match validate_binding_token_for_request(
        engine.ws_token_manager(),
        &token,
        &request_tenant_id,
    ) {
        Ok(binding) => binding,
        Err(BindingTokenRejection::InvalidOrExpired) => {
            warn!("Invalid or expired WebSocket binding token");
            close_with_policy_violation(&mut socket, "invalid-or-expired-token").await;
            return;
        }
        Err(BindingTokenRejection::TenantMismatch { token_tenant_id }) => {
            warn!(
                request_tenant_id = %request_tenant_id,
                token_tenant_id = %token_tenant_id,
                "WebSocket binding token tenant does not match request tenant"
            );
            close_with_policy_violation(&mut socket, "tenant-mismatch").await;
            return;
        }
    };

    // Verify subscription still exists.
    let sub = match engine
        .manager()
        .get_subscription(&tenant_id, &subscription_id)
    {
        Some(sub) => sub,
        None => {
            warn!(subscription_id = %subscription_id, "Subscription not found for binding token");
            close_with_policy_violation(&mut socket, "subscription-not-found").await;
            return;
        }
    };

    // Register this client with the WebSocket manager.
    let Some((client_id, mut rx)) = engine
        .ws_manager()
        .register_client(&tenant_id, &subscription_id)
    else {
        close_with_policy_violation(&mut socket, "too-many-clients").await;
        return;
    };
    let ws_manager = Arc::clone(engine.ws_manager());

    info!(
        tenant_id = %tenant_id,
        subscription_id = %subscription_id,
        client_id = %client_id,
        "WebSocket client bound with token"
    );

    // Build handshake notification after successful bind.
    let handshake_bundle = helios_subscriptions::notification::build_handshake(&sub, &base_url);

    // Send handshake notification as the first message.
    if let Ok(bundle) = handshake_bundle {
        match serde_json::to_string(&bundle) {
            Ok(msg) => {
                if send_within_deadline(&mut socket, msg.into()).await.is_err() {
                    warn!(
                        client_id = %client_id,
                        "Failed to send handshake, client disconnected or not reading"
                    );
                    ws_manager.remove_client(&tenant_id, &subscription_id, &client_id);
                    return;
                }
            }
            Err(e) => {
                warn!(
                    client_id = %client_id,
                    error = %e,
                    "Failed to serialize handshake bundle"
                );
            }
        }
    }

    // Stream notifications. WebSocket is unidirectional (server → client),
    // but we monitor the socket for close frames / disconnects.
    loop {
        tokio::select! {
            // Notification from the subscription engine.
            notification = rx.recv() => {
                match notification {
                    // The manager released this client: it let its queue
                    // fill and was dropped, or the subscription went away.
                    // The backlog is not delivered, so a client that reads
                    // just inside the send deadline cannot keep its queued
                    // bundles in memory for hours after being dropped.
                    Some(_) if rx.is_closed() => {
                        warn!(
                            client_id = %client_id,
                            "WebSocket client was released by the subscription engine; closing without delivering its backlog"
                        );
                        break;
                    }
                    Some(payload) => {
                        match Utf8Bytes::try_from(payload) {
                            Ok(msg) => match send_within_deadline(&mut socket, msg).await {
                                Ok(()) => {}
                                Err(SendFailure::Disconnected) => {
                                    debug!(
                                        client_id = %client_id,
                                        "Client disconnected during send"
                                    );
                                    break;
                                }
                                Err(SendFailure::Stalled) => {
                                    warn!(
                                        client_id = %client_id,
                                        deadline_secs = SEND_TIMEOUT.as_secs(),
                                        "Client did not accept a notification within the deadline; closing"
                                    );
                                    break;
                                }
                            },
                            Err(e) => {
                                warn!(
                                    client_id = %client_id,
                                    error = %e,
                                    "Notification payload is not UTF-8"
                                );
                            }
                        }
                    }
                    None => {
                        // Channel closed: the subscription was deregistered,
                        // or the manager dropped this client for not
                        // draining its queue.
                        debug!(
                            client_id = %client_id,
                            "Notification channel closed"
                        );
                        break;
                    }
                }
            }
            // Monitor for client close / disconnect.
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        debug!(
                            client_id = %client_id,
                            "Client sent close frame or disconnected"
                        );
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if parse_bind_with_token_message(text.as_ref()).is_some() {
                            warn!(
                                client_id = %client_id,
                                "Received additional bind-with-token message after initial bind"
                            );
                            close_with_policy_violation(&mut socket, "single-bind-only").await;
                            break;
                        }
                        // Ignore non-bind client messages.
                    }
                    Some(Ok(_)) => {
                        // Ignore other client messages (ping/pong/binary).
                    }
                    Some(Err(e)) => {
                        warn!(
                            client_id = %client_id,
                            error = %e,
                            "WebSocket error"
                        );
                        if is_message_too_big(&e) {
                            close_with_message_too_big(&mut socket).await;
                        }
                        break;
                    }
                }
            }
        }
    }

    // Cleanup: remove this client from the manager.
    ws_manager.remove_client(&tenant_id, &subscription_id, &client_id);
    info!(
        client_id = %client_id,
        subscription_id = %subscription_id,
        "WebSocket client disconnected"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum SendFailure {
    Disconnected,
    Stalled,
}

/// Sends one text frame, giving up after [`SEND_TIMEOUT`]: a peer that stops
/// reading leaves `send` pending once the socket buffers are full.
async fn send_within_deadline(socket: &mut WebSocket, msg: Utf8Bytes) -> Result<(), SendFailure> {
    match tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(msg))).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(SendFailure::Disconnected),
        Err(_) => Err(SendFailure::Stalled),
    }
}

async fn receive_bind_token(socket: &mut WebSocket) -> Option<String> {
    let first = match tokio::time::timeout(BIND_TIMEOUT, socket.recv()).await {
        Ok(first) => first,
        Err(_) => {
            warn!(
                deadline_secs = BIND_TIMEOUT.as_secs(),
                "WebSocket connection sent no bind message within the deadline"
            );
            close_with_policy_violation(socket, "bind-timeout").await;
            return None;
        }
    };
    match first {
        Some(Ok(Message::Text(text))) => {
            if let Some(token) = parse_bind_with_token_message(text.as_ref()) {
                Some(token.to_string())
            } else {
                warn!("Invalid websocket bind message");
                close_with_policy_violation(socket, "expected-bind-with-token").await;
                None
            }
        }
        Some(Ok(Message::Close(_))) | None => {
            debug!("WebSocket disconnected before bind");
            None
        }
        Some(Ok(_)) => {
            warn!("WebSocket bind expects first message to be text");
            close_with_policy_violation(socket, "expected-text-bind-message").await;
            None
        }
        Some(Err(e)) => {
            warn!(error = %e, "WebSocket error before bind");
            if is_message_too_big(&e) {
                close_with_message_too_big(socket).await;
            }
            None
        }
    }
}

fn parse_bind_with_token_message(message: &str) -> Option<&str> {
    let trimmed = message.trim();
    let rest = trimmed.strip_prefix("bind-with-token")?;
    let token = rest.trim_start_matches(':').trim();
    if token.is_empty() { None } else { Some(token) }
}

async fn close_with_policy_violation(socket: &mut WebSocket, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: 1008,
            reason: reason.to_string().into(),
        })))
        .await;
}

/// Whether `error` is the WebSocket layer refusing a message or frame larger
/// than [`MAX_INCOMING_MESSAGE_BYTES`].
fn is_message_too_big(error: &axum::Error) -> bool {
    std::error::Error::source(error)
        .and_then(|source| source.downcast_ref::<tungstenite::Error>())
        .is_some_and(|source| matches!(source, tungstenite::Error::Capacity(_)))
}

/// Tells the peer its message was refused for its size (close code 1009)
/// rather than leaving it with a dropped connection. The oversized payload is
/// left unread, so a peer still sending it may see a reset instead.
async fn close_with_message_too_big(socket: &mut WebSocket) {
    let close = Message::Close(Some(CloseFrame {
        code: 1009,
        reason: "message-too-big".into(),
    }));
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, socket.send(close)).await;
}

#[cfg(test)]
mod tests {
    use super::{
        BindingTokenRejection, parse_bind_with_token_message, validate_binding_token_for_request,
    };

    #[test]
    fn test_parse_bind_with_token_message() {
        assert_eq!(
            parse_bind_with_token_message("bind-with-token abc123"),
            Some("abc123")
        );
        assert_eq!(
            parse_bind_with_token_message("bind-with-token: abc123"),
            Some("abc123")
        );
        assert_eq!(
            parse_bind_with_token_message(" bind-with-token   abc123 "),
            Some("abc123")
        );
        assert_eq!(parse_bind_with_token_message("bind-with-token"), None);
        assert_eq!(parse_bind_with_token_message("hello"), None);
    }

    #[test]
    fn binding_token_tenant_must_match_request_tenant() {
        let manager = helios_subscriptions::WsBindingTokenManager::new(30);
        let (token, _) = manager.generate_token("acme", "subscription-1").unwrap();

        assert_eq!(
            validate_binding_token_for_request(&manager, &token, "other"),
            Err(BindingTokenRejection::TenantMismatch {
                token_tenant_id: "acme".to_string(),
            })
        );
        assert!(manager.validate_and_consume(&token).is_none());
    }

    #[test]
    fn binding_token_matching_request_tenant_is_accepted() {
        let manager = helios_subscriptions::WsBindingTokenManager::new(30);
        let (token, _) = manager.generate_token("acme", "subscription-1").unwrap();

        assert_eq!(
            validate_binding_token_for_request(&manager, &token, "acme"),
            Ok(("acme".to_string(), "subscription-1".to_string()))
        );
    }
}
