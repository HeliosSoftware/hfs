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
/// Returns a `SubscriptionStatus` (R4B/R5/R6) or `Parameters` (R4 backport)
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

    let snapshot = resolve_snapshot(&state, engine, tenant.tenant_id(), &id).await?;

    let status_resource = build_subscription_status(&snapshot, &id, state.base_url());

    Ok((StatusCode::OK, Json(status_resource)).into_response())
}

/// The runtime facts `$status` / `$events` report, from whichever source can
/// supply them.
struct StatusSnapshot {
    status: helios_subscriptions::manager::SubscriptionStatusCode,
    topic_url: String,
    events_since_start: u64,
    fhir_version: helios_fhir::FhirVersion,
}

/// Resolves a subscription's runtime state, falling back to storage when the
/// engine does not hold it.
///
/// The engine is authoritative when it has the subscription: that entry is what
/// delivery decisions are actually made from, so reporting anything else would
/// be a worse lie than a lag.
///
/// The fallback exists because the engine legitimately does not hold every
/// stored subscription. Rehydration can fail for one tenant, an operator can
/// disable it (`HFS_SUBSCRIPTION_REHYDRATE=false`), or a topic can fail to
/// parse. Answering `404` in those cases while `GET /Subscription/{id}` answers
/// `200` is a self-contradiction the client cannot act on — and the R5 Backport
/// IG asks a server to keep responding to `$status` regardless. Falling back to
/// the stored resource turns "I have lost track of this" into the honest
/// "here is what the record says", with the event counter reported as `0`
/// because this process genuinely has not delivered anything for it.
async fn resolve_snapshot<S>(
    state: &AppState<S>,
    engine: &std::sync::Arc<helios_subscriptions::SubscriptionEngine>,
    tenant_id: &str,
    id: &str,
) -> RestResult<StatusSnapshot>
where
    S: ResourceStorage + Send + Sync,
{
    if let Some(sub) = engine.manager().get_subscription(tenant_id, id) {
        return Ok(StatusSnapshot {
            status: sub.status,
            topic_url: sub.topic_url,
            events_since_start: sub.events_since_start,
            fhir_version: sub.fhir_version,
        });
    }

    let tenant_ctx = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new(tenant_id),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    let stored = state
        .storage()
        .read(&tenant_ctx, "Subscription", id)
        .await?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: id.to_string(),
        })?;

    let content = stored.content();
    let status = content
        .get("status")
        .and_then(|v| v.as_str())
        .and_then(helios_subscriptions::manager::SubscriptionStatusCode::from_fhir_str)
        .unwrap_or(helios_subscriptions::manager::SubscriptionStatusCode::Requested);

    // `topic` (R4B+) or `criteria` (R4 backport); mirrors the manager's own
    // version-aware extraction closely enough for a status report.
    let topic_url = content
        .get("topic")
        .and_then(|v| v.as_str())
        .or_else(|| content.get("criteria").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();

    Ok(StatusSnapshot {
        status,
        topic_url,
        events_since_start: 0,
        fhir_version: stored.fhir_version(),
    })
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

    // Same storage fallback as `$status`: both handlers had the identical miss,
    // and fixing only one would look complete in review.
    let snapshot = resolve_snapshot(&state, engine, tenant.tenant_id(), &id).await?;

    // Return a Bundle with a SubscriptionStatus indicating query-status
    let bundle_type = snapshot.fhir_version.notification_bundle_type();

    let bundle = json!({
        "resourceType": "Bundle",
        "type": bundle_type,
        "entry": [{
            "resource": {
                "resourceType": "SubscriptionStatus",
                "status": snapshot.status.as_fhir_str(),
                "type": "query-status",
                "eventsSinceSubscriptionStart": snapshot.events_since_start,
                "subscription": {
                    "reference": format!("Subscription/{}", id)
                },
                "topic": snapshot.topic_url
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
fn build_subscription_status(sub: &StatusSnapshot, id: &str, base_url: &str) -> serde_json::Value {
    if uses_backport_ig(sub.fhir_version) {
        // R4 backport: return Parameters resource
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
            "eventsSinceSubscriptionStart": sub.events_since_start,
            "subscription": {
                "reference": format!("Subscription/{}", id)
            },
            "topic": sub.topic_url
        })
    }
}

/// Returns true for FHIR versions that use the Subscriptions R5 Backport IG
/// (R4), false for versions with native subscription support (R4B, R5, R6).
fn uses_backport_ig(version: helios_fhir::FhirVersion) -> bool {
    version.as_str() == "R4"
}
