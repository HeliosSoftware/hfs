//! Subscription operation handlers ($status, $events).
//!
//! Implements custom FHIR operations for subscription management:
//! - `GET /Subscription/{id}/$status` — returns the current `SubscriptionStatus`
//! - `GET /Subscription/{id}/$events` — returns recent events (R5/R6 only)

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use serde_json::json;

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Handler for the `$status` operation on Subscription resources.
///
/// Returns a `SubscriptionStatus` (R5/R6) or `Parameters` (R4/R4B backport)
/// resource reflecting the subscription's current runtime state.
///
/// # HTTP Request
///
/// `GET [base]/Subscription/{id}/$status`
pub async fn subscription_status_handler<S>(
    State(state): State<AppState<S>>,
    Path((_resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let engine = state
        .subscription_engine()
        .ok_or(RestError::NotImplemented {
            feature: "Subscriptions".to_string(),
        })?;

    let sub = engine
        .manager()
        .get_subscription(tenant.tenant_id(), &id)
        .ok_or(RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: id.clone(),
        })?;

    let status_resource = build_subscription_status(&sub, &id, state.base_url());

    Ok((StatusCode::OK, Json(status_resource)).into_response())
}

/// Handler for the `$events` operation on Subscription resources.
///
/// Returns recent events for the subscription. This is a simplified
/// implementation that returns the current event count.
///
/// # HTTP Request
///
/// `GET [base]/Subscription/{id}/$events`
pub async fn subscription_events_handler<S>(
    State(state): State<AppState<S>>,
    Path((_resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let engine = state
        .subscription_engine()
        .ok_or(RestError::NotImplemented {
            feature: "Subscriptions".to_string(),
        })?;

    let sub = engine
        .manager()
        .get_subscription(tenant.tenant_id(), &id)
        .ok_or(RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: id.clone(),
        })?;

    // Return a Bundle with a SubscriptionStatus indicating query-status
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "history",
        "entry": [{
            "resource": {
                "resourceType": "SubscriptionStatus",
                "status": sub.status.as_fhir_str(),
                "type": "query-status",
                "eventsSinceSubscriptionStart": sub.events_since_start.to_string(),
                "subscription": {
                    "reference": format!("Subscription/{}", id)
                },
                "topic": sub.topic_url
            }
        }]
    });

    Ok((StatusCode::OK, Json(bundle)).into_response())
}

/// Handler for the `$get-ws-binding-token` operation on Subscription resources.
///
/// Returns a `Parameters` resource containing a short-lived token, expiration,
/// and the WebSocket URL for binding.
///
/// # HTTP Request
///
/// `GET [base]/Subscription/{id}/$get-ws-binding-token`
pub async fn get_ws_binding_token_handler<S>(
    State(state): State<AppState<S>>,
    Path((_resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let engine = state
        .subscription_engine()
        .ok_or(RestError::NotImplemented {
            feature: "Subscriptions".to_string(),
        })?;

    // Verify subscription exists.
    let sub = engine
        .manager()
        .get_subscription(tenant.tenant_id(), &id)
        .ok_or(RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: id.clone(),
        })?;

    // Verify it's a websocket subscription.
    if sub.channel.channel_type != helios_subscriptions::manager::ChannelType::Websocket {
        return Err(RestError::BadRequest {
            message: format!(
                "$get-ws-binding-token is only valid for websocket subscriptions, \
                 but this subscription uses channel type '{}'",
                sub.channel.channel_type.as_fhir_str()
            ),
        });
    }

    // Generate binding token.
    let (token, expiration) = engine
        .ws_token_manager()
        .generate_token(tenant.tenant_id(), &id);

    // Build WebSocket URL from the base URL.
    let ws_base = state
        .base_url()
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    let ws_url = format!("{}/ws/subscriptions/bind", ws_base);

    // Return Parameters resource per FHIR spec.
    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "token",
                "valueString": token
            },
            {
                "name": "expiration",
                "valueDateTime": expiration.to_rfc3339()
            },
            {
                "name": "websocket-url",
                "valueUrl": ws_url
            }
        ]
    });

    Ok((StatusCode::OK, Json(parameters)).into_response())
}

/// Builds a SubscriptionStatus resource for the $status response.
fn build_subscription_status(
    sub: &helios_subscriptions::manager::ActiveSubscription,
    id: &str,
    base_url: &str,
) -> serde_json::Value {
    if uses_backport_ig(sub.fhir_version) {
        // R4/R4B backport: return Parameters resource
        json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "subscription",
                    "valueReference": {
                        "reference": format!("{}/Subscription/{}", base_url, id)
                    }
                },
                {
                    "name": "topic",
                    "valueCanonical": sub.topic_url
                },
                {
                    "name": "status",
                    "valueCode": sub.status.as_fhir_str()
                },
                {
                    "name": "type",
                    "valueCode": "query-status"
                },
                {
                    "name": "events-since-subscription-start",
                    "valueString": sub.events_since_start.to_string()
                }
            ]
        })
    } else {
        // R5/R6 native: return SubscriptionStatus resource
        json!({
            "resourceType": "SubscriptionStatus",
            "status": sub.status.as_fhir_str(),
            "type": "query-status",
            "eventsSinceSubscriptionStart": sub.events_since_start.to_string(),
            "subscription": {
                "reference": format!("Subscription/{}", id)
            },
            "topic": sub.topic_url
        })
    }
}

/// Returns true for FHIR versions that use the Subscriptions R5 Backport IG
/// (R4 and R4B), false for versions with native subscription support (R5, R6).
fn uses_backport_ig(version: helios_fhir::FhirVersion) -> bool {
    matches!(version.as_str(), "R4" | "R4B")
}
