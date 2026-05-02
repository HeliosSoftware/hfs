//! WebSocket subscription binding handler.
//!
//! Implements the server-side WebSocket endpoint that clients connect to
//! after obtaining a binding token via `$get-ws-binding-token`.
//!
//! The protocol is unidirectional (server → client): the server streams
//! notification bundles as JSON text frames after the client sends a
//! `bind-with-token <token>` message.

use std::sync::Arc;

use axum::{
    extract::ws::{CloseFrame, Message, WebSocket},
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use helios_persistence::core::ResourceStorage;
use tracing::{debug, info, warn};

use crate::error::{RestError, RestResult};
use crate::state::AppState;

/// WebSocket binding handler.
///
/// The client connects to `/ws/subscriptions/bind` and then sends
/// `bind-with-token <binding-token>` as the first message.
pub async fn ws_bind_handler<S>(
    State(state): State<AppState<S>>,
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
    let base_url = state.base_url().to_string();

    // Upgrade the HTTP connection to WebSocket.
    Ok(ws.on_upgrade(move |socket| handle_ws_connection(socket, engine, base_url)))
}

/// Handles the WebSocket connection lifecycle after upgrade.
///
/// Waits for a `bind-with-token` message, sends a handshake notification,
/// then streams events from the subscription engine until disconnect.
async fn handle_ws_connection(
    mut socket: WebSocket,
    engine: Arc<helios_subscriptions::SubscriptionEngine>,
    base_url: String,
) {
    let token = match receive_bind_token(&mut socket).await {
        Some(token) => token,
        None => return,
    };

    // Validate and consume the binding token (single-use).
    let (tenant_id, subscription_id) = match engine.ws_token_manager().validate_and_consume(&token)
    {
        Some(binding) => binding,
        None => {
            warn!("Invalid or expired WebSocket binding token");
            close_with_policy_violation(&mut socket, "invalid-or-expired-token").await;
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
    let (client_id, mut rx) = engine
        .ws_manager()
        .register_client(&tenant_id, &subscription_id);
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
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    warn!(
                        client_id = %client_id,
                        "Failed to send handshake, client disconnected"
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
                    Some(bundle) => {
                        match serde_json::to_string(&bundle) {
                            Ok(msg) => {
                                if socket.send(Message::Text(msg.into())).await.is_err() {
                                    debug!(
                                        client_id = %client_id,
                                        "Client disconnected during send"
                                    );
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!(
                                    client_id = %client_id,
                                    error = %e,
                                    "Failed to serialize notification bundle"
                                );
                            }
                        }
                    }
                    None => {
                        // Channel closed (subscription deregistered).
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

async fn receive_bind_token(socket: &mut WebSocket) -> Option<String> {
    match socket.recv().await {
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

#[cfg(test)]
mod tests {
    use super::parse_bind_with_token_message;

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
}
