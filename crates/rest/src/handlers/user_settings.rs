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
/// the active backend does not provide one.
///
/// The per-user settings store is implemented by every standalone *primary*
/// backend that offers the required read-modify-write + monotonic-version
/// primitives: SQLite, PostgreSQL, and MongoDB. It is intentionally unavailable
/// on backends that are not a transactional primary FHIR store — notably the
/// S3 object store (whose recommended role is archival and whose concurrency /
/// version story differs; tracked in issue #199) and Elasticsearch (search-only,
/// never a standalone primary). The message names the supported backends so an
/// operator on an unsupported one gets an explained `501` rather than a bare one.
fn settings_store<S>(state: &AppState<S>) -> RestResult<&Arc<dyn SettingsStore>>
where
    S: ResourceStorage + Send + Sync,
{
    state
        .settings_store()
        .ok_or_else(|| RestError::NotImplemented {
            feature: "per-user settings (supported on the SQLite, PostgreSQL, and MongoDB \
                      backends; not available on the S3 or Elasticsearch backends)"
                .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn parse_object_body_rejects_empty() {
        let err = parse_object_body(&Bytes::new()).unwrap_err();
        assert!(matches!(err, RestError::BadRequest { .. }));
    }

    #[test]
    fn parse_object_body_rejects_invalid_json() {
        let err = parse_object_body(&Bytes::from_static(b"{ not json")).unwrap_err();
        assert!(matches!(err, RestError::BadRequest { .. }));
    }

    #[test]
    fn parse_object_body_rejects_non_object() {
        let err = parse_object_body(&Bytes::from_static(b"[1, 2, 3]")).unwrap_err();
        assert!(matches!(err, RestError::BadRequest { .. }));
    }

    #[test]
    fn parse_object_body_accepts_object() {
        let value = parse_object_body(&Bytes::from_static(b"{\"theme\":\"dark\"}")).unwrap();
        assert!(value.is_object());
    }

    #[test]
    fn parse_if_match_version_variants() {
        let with_if_match = |raw: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_MATCH, raw.parse().unwrap());
            ConditionalHeaders::from_headers(&headers)
        };
        assert_eq!(parse_if_match_version(&with_if_match("W/\"5\"")), Some(5));
        assert_eq!(parse_if_match_version(&with_if_match("\"7\"")), Some(7));
        assert_eq!(parse_if_match_version(&with_if_match("9")), Some(9));
        // A wildcard means "no version precondition".
        assert_eq!(parse_if_match_version(&with_if_match("*")), None);
        // An unparseable value is ignored rather than rejected.
        assert_eq!(parse_if_match_version(&with_if_match("garbage")), None);
        // An absent header yields no precondition.
        assert_eq!(
            parse_if_match_version(&ConditionalHeaders::from_headers(&HeaderMap::new())),
            None
        );
    }

    /// Backends without a settings store surface `501 Not Implemented`.
    #[cfg(feature = "sqlite")]
    #[test]
    fn settings_store_absent_returns_not_implemented() {
        use crate::config::ServerConfig;
        use helios_persistence::backends::sqlite::SqliteBackend;

        let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
        backend.init_schema().expect("init schema");
        // `AppState::new` leaves the settings store unset. The `Ok` variant
        // (`&Arc<dyn SettingsStore>`) is not `Debug`, so match instead of unwrap.
        let state = AppState::new(Arc::new(backend), ServerConfig::default());
        let result = settings_store(&state);
        assert!(matches!(result, Err(RestError::NotImplemented { .. })));
    }
}
