//! WebSocket subscription binding handler.
//!
//! Implements the server-side WebSocket endpoint that clients connect to
//! after obtaining a binding token via `$get-ws-binding-token`.
//!
//! The protocol is unidirectional (server → client): the server streams
//! notification bundles as JSON text frames. Client messages are ignored
//! (pings/pongs are handled automatically by axum).

use std::sync::Arc;

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{Query, State, WebSocketUpgrade},
    response::Response,
};
use helios_persistence::core::ResourceStorage;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::error::{RestError, RestResult};
use crate::state::AppState;

/// Query parameters for the WebSocket bind endpoint.
#[derive(Deserialize)]
pub struct WsBindQuery {
    /// The binding token obtained from `$get-ws-binding-token`.
    token: String,
}

/// WebSocket binding handler.
///
/// The client connects to `/ws/subscriptions/bind?token=<binding-token>`.
/// The server validates the token, upgrades the connection, sends a handshake
/// notification, and then streams notifications as they arrive.
pub async fn ws_bind_handler<S>(
    State(state): State<AppState<S>>,
    Query(query): Query<WsBindQuery>,
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

    // Validate and consume the binding token (single-use).
    let (tenant_id, subscription_id) = engine
        .ws_token_manager()
        .validate_and_consume(&query.token)
        .ok_or(RestError::Unauthorized {
            message: "Invalid or expired WebSocket binding token".to_string(),
        })?;

    // Verify subscription still exists.
    let sub = engine
        .manager()
        .get_subscription(&tenant_id, &subscription_id)
        .ok_or(RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: subscription_id.clone(),
        })?;

    // Register this client with the WebSocket manager.
    let (client_id, rx) = engine
        .ws_manager()
        .register_client(&tenant_id, &subscription_id);

    // Build handshake notification to send immediately after upgrade.
    let handshake_bundle =
        helios_subscriptions::notification::build_handshake(&sub, state.base_url());

    let ws_manager = Arc::clone(engine.ws_manager());
    let tenant_id_owned = tenant_id.clone();
    let sub_id_owned = subscription_id.clone();
    let client_id_owned = client_id.clone();

    info!(
        tenant_id = %tenant_id,
        subscription_id = %subscription_id,
        client_id = %client_id,
        "WebSocket client binding"
    );

    // Upgrade the HTTP connection to WebSocket.
    Ok(ws.on_upgrade(move |socket| {
        handle_ws_connection(
            socket,
            rx,
            ws_manager,
            tenant_id_owned,
            sub_id_owned,
            client_id_owned,
            handshake_bundle,
        )
    }))
}

/// Handles the WebSocket connection lifecycle after upgrade.
///
/// Sends a handshake notification first, then streams events from the
/// subscription engine until the client disconnects or the subscription
/// is deregistered.
async fn handle_ws_connection(
    mut socket: WebSocket,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>,
    ws_manager: Arc<helios_subscriptions::WebSocketManager>,
    tenant_id: String,
    subscription_id: String,
    client_id: String,
    handshake_bundle: Result<serde_json::Value, helios_subscriptions::SubscriptionError>,
) {
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
                    Some(Ok(_)) => {
                        // Ignore other client messages.
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
