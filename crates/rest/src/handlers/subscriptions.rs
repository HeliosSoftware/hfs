//! Subscription operation handlers ($status, $events).
//!
//! Implements custom FHIR operations for subscription management:
//! - `GET /Subscription/{id}/$status` — returns the current `SubscriptionStatus`
//! - `GET /Subscription/{id}/$events` — returns the status and the events in a number range

use axum::{
    Json,
    extract::{Path, Query, State},
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

    let snapshot = resolve_snapshot(&state, engine, &tenant, &id).await?;

    let subscription_url = state.public_url_for_request(&tenant, ["Subscription", id.as_str()]);
    let status_resource = build_subscription_status(&snapshot, &subscription_url);

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
///
/// The storage read uses the request's own resolved [`TenantExtractor`] context,
/// never a fabricated full-access one: this is a request path, so the tenant and
/// its permissions must be the ones the request actually resolved to.
async fn resolve_snapshot<S>(
    state: &AppState<S>,
    engine: &std::sync::Arc<helios_subscriptions::SubscriptionEngine>,
    tenant: &TenantExtractor,
    id: &str,
) -> RestResult<StatusSnapshot>
where
    S: ResourceStorage + Send + Sync,
{
    if let Some(sub) = engine.manager().get_subscription(tenant.tenant_id(), id) {
        return Ok(StatusSnapshot {
            status: sub.status,
            topic_url: sub.topic_url,
            events_since_start: sub.events_since_start,
            fhir_version: sub.fhir_version,
        });
    }

    let stored = state
        .storage()
        .read(tenant.context(), "Subscription", id)
        .await?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "Subscription".to_string(),
            id: id.to_string(),
        })?;

    Ok(snapshot_from_stored(
        stored.content(),
        stored.fhir_version(),
    ))
}

/// Derives a [`StatusSnapshot`] from a stored `Subscription`'s content.
///
/// Split out from [`resolve_snapshot`] so the parsing — the only part with real
/// branching — is unit-testable without standing up an app and an engine.
///
/// `events_since_start` is reported as `0` rather than guessed: this process has
/// genuinely delivered nothing for a subscription it is not holding, and the
/// counter is not persisted (see the PR for #357).
fn snapshot_from_stored(
    content: &serde_json::Value,
    fhir_version: helios_fhir::FhirVersion,
) -> StatusSnapshot {
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

    StatusSnapshot {
        status,
        topic_url,
        events_since_start: 0,
        fhir_version,
    }
}

/// Query parameters of the `$events` operation.
///
/// Both numbers are received as strings (the Subscriptions Backport IG types
/// them as `string`; R6 as `integer64`) and parsed by hand so a malformed value
/// answers with a precise `OperationOutcome` instead of an extractor rejection.
#[derive(Debug, Default, serde::Deserialize)]
pub struct EventsParams {
    #[serde(rename = "eventsSinceNumber")]
    events_since_number: Option<String>,
    #[serde(rename = "eventsUntilNumber")]
    events_until_number: Option<String>,
}

fn parse_event_number(param: &str, raw: Option<&str>) -> RestResult<Option<u64>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            value
                .trim()
                .parse::<u64>()
                .map(Some)
                .map_err(|_| RestError::InvalidParameter {
                    param: param.to_string(),
                    message: format!("'{value}' is not a positive integer"),
                })
        }
    }
}

/// Handler for the `$events` operation on Subscription resources.
///
/// Returns a `history` Bundle (`subscription-notification` on R5/R6) whose
/// first entry is the subscription status with `type = query-event`: a
/// `Parameters` resource on R4 (Subscriptions Backport) or a
/// `SubscriptionStatus` with `notificationEvent` on R4B and later. It is
/// followed by one id-only entry per event in the requested range.
///
/// `eventsSinceNumber` defaults to the oldest retained event and
/// `eventsUntilNumber` to the latest one. Invalid ranges (non-numeric, below 1,
/// since after until, or entirely older than the retained history) answer `400`
/// with an `OperationOutcome`. A `eventsSinceNumber` beyond the latest event
/// (a client polling for the next one) returns the status with no events. A
/// range that only partly overlaps the retained history returns the retained
/// subset.
///
/// The event history is bounded (`HFS_SUBSCRIPTION_EVENT_HISTORY_SIZE`) and held
/// in memory only, like the event counter: it is empty after a restart. A
/// subscription the engine does not hold is answered from storage with no
/// events and a counter of `0`; any explicit range is then a `400`.
///
/// # HTTP Request
///
/// `GET [base]/Subscription/{id}/$events?eventsSinceNumber=1&eventsUntilNumber=3`
pub async fn subscription_events_handler<S>(
    State(state): State<AppState<S>>,
    Path((_resource_type, id)): Path<(String, String)>,
    Query(params): Query<EventsParams>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    use helios_subscriptions::manager::{EventQuery, RangeError};

    let engine = state
        .subscription_engine()
        .ok_or(RestError::NotImplemented {
            feature: "Subscriptions".to_string(),
        })?;

    let since = parse_event_number("eventsSinceNumber", params.events_since_number.as_deref())?;
    let until = parse_event_number("eventsUntilNumber", params.events_until_number.as_deref())?;

    let query = match engine
        .manager()
        .events_in_range(tenant.tenant_id(), &id, since, until)
    {
        Some(result) => result,
        None => {
            // Same storage fallback as `$status`: the engine does not hold the
            // subscription, so there is no history to serve.
            let snapshot = resolve_snapshot(&state, engine, &tenant, &id).await?;
            if since.is_some() || until.is_some() {
                Err(RangeError::NoLongerRetained { oldest: None })
            } else {
                Ok(EventQuery {
                    status: snapshot.status,
                    topic_url: snapshot.topic_url,
                    fhir_version: snapshot.fhir_version,
                    events_since_start: snapshot.events_since_start,
                    oldest_retained: None,
                    events: Vec::new(),
                })
            }
        }
    }
    .map_err(|e| RestError::InvalidParameter {
        param: e.param().to_string(),
        message: e.to_string(),
    })?;

    let subscription_url = state.public_url_for_request(&tenant, ["Subscription", id.as_str()]);
    let base_url = state.public_url_for_request(&tenant, []);
    let bundle = helios_subscriptions::notification::build_query_events_bundle(
        &query,
        &id,
        &subscription_url,
        &base_url,
    )
    .map_err(|e| RestError::InternalError {
        message: format!("Failed to build $events response: {e}"),
    })?;

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
    let ws_http_url = state.public_url_for_request(&tenant, ["ws", "subscriptions", "bind"]);
    let ws_url = websocket_url(&ws_http_url)?;

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

fn websocket_url(http_url: &str) -> RestResult<String> {
    let mut ws_url = url::Url::parse(http_url).map_err(|error| RestError::InternalError {
        message: format!("failed to build WebSocket URL: {error}"),
    })?;
    let ws_scheme = if ws_url.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    ws_url
        .set_scheme(ws_scheme)
        .map_err(|_| RestError::InternalError {
            message: "failed to set WebSocket URL scheme".to_string(),
        })?;

    Ok(ws_url.to_string())
}

/// Builds a SubscriptionStatus resource for the $status response.
fn build_subscription_status(sub: &StatusSnapshot, subscription_url: &str) -> serde_json::Value {
    if uses_backport_ig(sub.fhir_version) {
        // R4 backport: return Parameters resource
        json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "subscription",
                    "valueReference": {
                        "reference": subscription_url
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
                "reference": subscription_url
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

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhir::FhirVersion;
    use helios_subscriptions::manager::SubscriptionStatusCode;
    use serde_json::json;

    /// The `$status` / `$events` storage fallback (#357): when the engine does
    /// not hold a subscription, its stored status is reported rather than `404`.
    #[test]
    fn snapshot_reads_status_and_native_topic() {
        let snapshot = snapshot_from_stored(
            &json!({
                "resourceType": "Subscription",
                "status": "off",
                "topic": "http://example.org/topic/x",
                "channelType": { "code": "rest-hook" }
            }),
            FhirVersion::default(),
        );
        assert_eq!(snapshot.status, SubscriptionStatusCode::Off);
        assert_eq!(snapshot.topic_url, "http://example.org/topic/x");
        assert_eq!(snapshot.events_since_start, 0);
    }

    /// R4 backport carries the topic in `criteria`, not `topic`.
    #[test]
    fn snapshot_reads_r4_backport_criteria_topic() {
        let snapshot = snapshot_from_stored(
            &json!({
                "resourceType": "Subscription",
                "status": "error",
                "criteria": "http://example.org/topic/y",
                "channel": { "type": "rest-hook" }
            }),
            FhirVersion::default(),
        );
        assert_eq!(snapshot.status, SubscriptionStatusCode::Error);
        assert_eq!(snapshot.topic_url, "http://example.org/topic/y");
    }

    /// `entered-in-error` must survive the fallback as itself, not decay to
    /// `requested` — the same trap the manager had before #357.
    #[test]
    fn snapshot_preserves_entered_in_error() {
        let snapshot = snapshot_from_stored(
            &json!({ "resourceType": "Subscription", "status": "entered-in-error" }),
            FhirVersion::default(),
        );
        assert_eq!(snapshot.status, SubscriptionStatusCode::EnteredInError);
    }

    /// An absent or unrecognised status falls back to `requested`, and a missing
    /// topic yields an empty string rather than panicking.
    #[test]
    fn snapshot_tolerates_missing_and_unknown_fields() {
        let snapshot = snapshot_from_stored(
            &json!({ "resourceType": "Subscription", "status": "not-a-code" }),
            FhirVersion::default(),
        );
        assert_eq!(snapshot.status, SubscriptionStatusCode::Requested);
        assert_eq!(snapshot.topic_url, "");

        let bare = snapshot_from_stored(
            &json!({ "resourceType": "Subscription" }),
            FhirVersion::default(),
        );
        assert_eq!(bare.status, SubscriptionStatusCode::Requested);
        assert_eq!(bare.topic_url, "");
    }

    #[test]
    fn public_subscription_urls_keep_prefix_and_tenant() {
        let snapshot = StatusSnapshot {
            status: SubscriptionStatusCode::Active,
            topic_url: "https://example.test/topic".to_string(),
            events_since_start: 1,
            fhir_version: FhirVersion::default(),
        };
        let subscription_url = "https://public.example/fhir/acme/Subscription/sub-1";
        let status = build_subscription_status(&snapshot, subscription_url);
        assert_eq!(
            status["parameter"][0]["valueReference"]["reference"],
            subscription_url
        );
        assert_eq!(
            websocket_url("https://public.example/fhir/acme/ws/subscriptions/bind").unwrap(),
            "wss://public.example/fhir/acme/ws/subscriptions/bind"
        );
    }
}
