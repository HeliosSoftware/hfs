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
use helios_persistence::core::{
    ResourceStorage, SettingsStore, StoredUserSettings, apply_merge_patch,
};
use helios_persistence::error::{ConcurrencyError, StorageError};
use serde_json::Value;

use crate::error::{RestError, RestResult};
use crate::extractors::UserKey;
use crate::middleware::conditional::ConditionalHeaders;
use crate::state::AppState;

/// Upper bound on the serialized settings document, applied to every write.
/// The document lives in a single row/document on every backend, so an
/// unbounded blob degrades reads of *all* of a user's settings at once.
///
/// The only other limit on this endpoint is the server-wide request body limit,
/// which is sized for FHIR Bundles (10 MB by default, and routinely raised) — far
/// too generous for a document that holds a theme name and a few recent queries.
/// A settings document is read back on *every* page load and, on backends whose
/// write is a read-modify-write, re-read and re-written on every retry, so an
/// oversized one is paid for repeatedly. 256 KiB is orders of magnitude more than
/// any legitimate document needs.
const MAX_SETTINGS_DOCUMENT_BYTES: usize = 256 * 1024;

/// Upper bound on saved queries per resource type under the `savedQueries`
/// convention (see `helios_persistence::core::user_settings` module docs).
const MAX_SAVED_QUERIES_PER_TYPE: usize = 100;

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
    validate_settings_document(&document)?;
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
    let caller_if_match = parse_if_match_version(&conditional);

    // The bounds below apply to the *post-merge* document, which only the
    // backend sees. Compute the merge here against the version we read, pin the
    // write to that version so what was validated is exactly what lands, and —
    // only when the caller sent no precondition of their own — absorb a benign
    // concurrent-writer race with a couple of retries.
    let mut attempts = 0;
    loop {
        let (current, version) = match store.get_settings(user.as_str()).await? {
            Some(stored) => (stored.document, stored.version),
            None => (Value::Object(Default::default()), 0),
        };
        if let Some(expected) = caller_if_match
            && expected != version
        {
            return Err(RestError::PreconditionFailed {
                message: format!("Expected settings version {expected}, but found {version}"),
            });
        }
        let merged = apply_merge_patch(current, &merge_patch);
        validate_settings_document(&merged)?;

        match store
            .patch_settings(user.as_str(), merge_patch.clone(), Some(version))
            .await
        {
            Ok(stored) => return Ok(settings_response(stored)),
            Err(e) if is_lock_failure(&e) && caller_if_match.is_none() && attempts < 2 => {
                attempts += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Whether a storage error is an optimistic-lock (version) conflict.
fn is_lock_failure(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
            | StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })
    )
}

/// Enforces the structural bounds the server guarantees on the otherwise
/// opaque settings document: a whole-document size cap, and — when the
/// [`savedQueries` convention](helios_persistence::core::user_settings) is
/// present — that it is an object keyed by resource type, each holding an
/// object of at most [`MAX_SAVED_QUERIES_PER_TYPE`] entries keyed by query id.
fn validate_settings_document(document: &Value) -> RestResult<()> {
    let size = serde_json::to_vec(document).map(|v| v.len()).unwrap_or(0);
    if size > MAX_SETTINGS_DOCUMENT_BYTES {
        return Err(RestError::PayloadTooLarge {
            message: format!(
                "Settings document is {size} bytes; the limit is {MAX_SETTINGS_DOCUMENT_BYTES}"
            ),
        });
    }

    let Some(saved_queries) = document.get("savedQueries") else {
        return Ok(());
    };
    let Some(by_type) = saved_queries.as_object() else {
        return Err(RestError::UnprocessableEntity {
            message: "savedQueries must be an object keyed by FHIR resource type".to_string(),
        });
    };
    for (resource_type, entries) in by_type {
        let Some(entries) = entries.as_object() else {
            return Err(RestError::UnprocessableEntity {
                message: format!(
                    "savedQueries.{resource_type} must be an object keyed by query id"
                ),
            });
        };
        if entries.len() > MAX_SAVED_QUERIES_PER_TYPE {
            return Err(RestError::UnprocessableEntity {
                message: format!(
                    "savedQueries.{resource_type} has {} entries; the limit is {}",
                    entries.len(),
                    MAX_SAVED_QUERIES_PER_TYPE
                ),
            });
        }
    }
    Ok(())
}

/// Returns the configured settings store, or a `501 Not Implemented` error when
/// the active backend does not provide one.
///
/// The per-user settings store is implemented by every standalone *primary*
/// backend: SQLite, PostgreSQL, and MongoDB provide the read-modify-write +
/// monotonic-version primitives directly (a transaction or a version-conditioned
/// update), and S3 reaches the same semantics with a compare-and-swap over
/// conditional `PutObject`. It is intentionally unavailable only on
/// Elasticsearch, which is search-only and never a standalone primary. The
/// message names the supported backends so an operator on an unsupported one gets
/// an explained `501` rather than a bare one.
fn settings_store<S>(state: &AppState<S>) -> RestResult<&Arc<dyn SettingsStore>>
where
    S: ResourceStorage + Send + Sync,
{
    state
        .settings_store()
        .ok_or_else(|| RestError::NotImplemented {
            feature: "per-user settings (supported on the SQLite, PostgreSQL, MongoDB, and S3 \
                      backends; not available on Elasticsearch, which is search-only and never \
                      a standalone primary store)"
                .to_string(),
        })
}

/// Parses and validates a request body as a JSON object of a sane size.
fn parse_object_body(body: &Bytes) -> RestResult<Value> {
    if body.is_empty() {
        return Err(RestError::BadRequest {
            message: "Request body must be a JSON object".to_string(),
        });
    }
    // A cheap guard on the wire size so an oversized blob is refused before it
    // is parsed. `validate_settings_document` re-checks the *post-merge*
    // document, which a small PATCH can still push over the cap.
    if body.len() > MAX_SETTINGS_DOCUMENT_BYTES {
        return Err(RestError::PayloadTooLarge {
            message: format!(
                "Settings document is {} bytes; the limit is {MAX_SETTINGS_DOCUMENT_BYTES}",
                body.len()
            ),
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
    fn parse_object_body_rejects_oversized_document() {
        let bloated = format!(
            r#"{{"junk":"{}"}}"#,
            "x".repeat(MAX_SETTINGS_DOCUMENT_BYTES)
        );
        let err = parse_object_body(&Bytes::from(bloated)).unwrap_err();
        match err {
            RestError::PayloadTooLarge { message } => assert!(
                message.contains("the limit is"),
                "unexpected message: {message}"
            ),
            other => panic!("expected a payload-too-large error, got {other:?}"),
        }
    }

    #[test]
    fn parse_object_body_accepts_document_at_the_limit() {
        // Exactly at the limit is fine; only *over* it is rejected.
        let padding = MAX_SETTINGS_DOCUMENT_BYTES - r#"{"k":""}"#.len();
        let doc = format!(r#"{{"k":"{}"}}"#, "x".repeat(padding));
        assert_eq!(doc.len(), MAX_SETTINGS_DOCUMENT_BYTES);
        assert!(parse_object_body(&Bytes::from(doc)).unwrap().is_object());
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
