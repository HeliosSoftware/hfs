//! Per-user UI settings handlers.
//!
//! Implements a small `application/json` API for an opaque, per-user settings
//! document (theme, default tenant, active FHIR version, recent queries, …):
//!
//! - `GET    /_user/settings` — fetch the document (defaults to `{}`)
//! - `PUT    /_user/settings` — replace the whole document
//! - `PATCH  /_user/settings` — [RFC 7386] JSON merge-patch a subset of keys
//!
//! The endpoints live under a leading-underscore path so they are authenticated
//! (a [`Principal`](helios_auth::Principal) is injected when auth is enabled) but
//! exempt from FHIR scope checks, and invisible to FHIR machinery
//! (`CapabilityStatement`, search, history, export).
//!
//! Each response carries a weak `ETag` (`W/"{version}"`). Clients may send
//! `If-Match` on `PUT`/`PATCH` for optimistic concurrency, or `If-None-Match`
//! on `GET` for conditional fetches.
//!
//! [RFC 7386]: https://www.rfc-editor.org/rfc/rfc7386

use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ResourceStorage, SettingsStore, StoredUserSettings};
use serde_json::Value;

use crate::error::{RestError, RestResult};
use crate::extractors::UserKey;
use crate::middleware::conditional::ConditionalHeaders;
use crate::state::AppState;

/// Handler for `GET /_user/settings`.
///
/// Returns the caller's settings document, or an empty object (`{}`, version 0)
/// when none has been stored yet, so the UI always receives a usable document.
pub async fn get_user_settings<S>(
    State(state): State<AppState<S>>,
    user: UserKey,
    conditional: ConditionalHeaders,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let store = settings_store(&state)?;
    let (document, version) = match store.get_settings(user.as_str()).await? {
        Some(stored) => (stored.document, stored.version),
        None => (Value::Object(Default::default()), 0),
    };
    let etag = weak_etag(version);

    // Honor If-None-Match only when a document actually exists; an empty default
    // document (version 0) must never be reported as "not modified".
    if version > 0
        && let Some(inm) = conditional.if_none_match()
        && (inm == etag || inm == "*")
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }

    Ok(([(header::ETAG, etag)], Json(document)).into_response())
}

/// Handler for `PUT /_user/settings`.
///
/// Replaces the caller's entire settings document with the request body, which
/// must be a JSON object. An optional `If-Match` header makes the write
/// conditional on the current version.
pub async fn put_user_settings<S>(
    State(state): State<AppState<S>>,
    user: UserKey,
    conditional: ConditionalHeaders,
    body: Bytes,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let store = settings_store(&state)?;
    let document = parse_object_body(&body)?;
    let if_match = parse_if_match_version(&conditional);
    let stored = store
        .put_settings(user.as_str(), document, if_match)
        .await?;
    Ok(settings_response(stored))
}

/// Handler for `PATCH /_user/settings`.
///
/// Applies an [RFC 7386] JSON merge-patch (request body, a JSON object) to the
/// caller's settings document — ideal for toggling a single key such as the
/// theme. An optional `If-Match` header makes the write conditional.
///
/// [RFC 7386]: https://www.rfc-editor.org/rfc/rfc7386
pub async fn patch_user_settings<S>(
    State(state): State<AppState<S>>,
    user: UserKey,
    conditional: ConditionalHeaders,
    body: Bytes,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    let store = settings_store(&state)?;
    let merge_patch = parse_object_body(&body)?;
    let if_match = parse_if_match_version(&conditional);
    let stored = store
        .patch_settings(user.as_str(), merge_patch, if_match)
        .await?;
    Ok(settings_response(stored))
}

/// Returns the configured settings store, or a `501 Not Implemented` error when
/// the active backend does not provide one (e.g. MongoDB, S3, Elasticsearch).
fn settings_store<S>(state: &AppState<S>) -> RestResult<&Arc<dyn SettingsStore>>
where
    S: ResourceStorage + Send + Sync,
{
    state
        .settings_store()
        .ok_or_else(|| RestError::NotImplemented {
            feature: "per-user settings (requires the SQLite or PostgreSQL backend)".to_string(),
        })
}

/// Parses and validates a request body as a JSON object.
fn parse_object_body(body: &Bytes) -> RestResult<Value> {
    if body.is_empty() {
        return Err(RestError::BadRequest {
            message: "Request body must be a JSON object".to_string(),
        });
    }
    let value: Value = serde_json::from_slice(body).map_err(|e| RestError::BadRequest {
        message: format!("Invalid JSON: {e}"),
    })?;
    if !value.is_object() {
        return Err(RestError::BadRequest {
            message: "Settings document must be a JSON object".to_string(),
        });
    }
    Ok(value)
}

/// Extracts the version number from an `If-Match` weak ETag (`W/"{n}"`, `"{n}"`,
/// or bare `{n}`). A wildcard (`*`) or absent/unparseable header yields `None`,
/// meaning "no version precondition".
fn parse_if_match_version(conditional: &ConditionalHeaders) -> Option<i64> {
    let raw = conditional.if_match()?.trim();
    if raw == "*" {
        return None;
    }
    raw.trim_start_matches("W/").trim_matches('"').parse().ok()
}

/// Builds the success response for a write: the stored document plus its ETag.
fn settings_response(stored: StoredUserSettings) -> Response {
    (
        [(header::ETAG, weak_etag(stored.version))],
        Json(stored.document),
    )
        .into_response()
}

/// Formats a version number as a weak ETag.
fn weak_etag(version: i64) -> String {
    format!("W/\"{version}\"")
}
