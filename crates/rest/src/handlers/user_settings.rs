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
//! Each response carries a strong `ETag` (`"{version}"`) — strong because
//! `If-Match` is defined in terms of the strong comparison function, which a
//! `W/` validator can never satisfy. Clients may send `If-Match` on
//! `PUT`/`PATCH` for optimistic concurrency, or `If-None-Match` on `GET` for
//! conditional fetches. The weak form is still *accepted* on `If-Match` for
//! clients built against earlier releases.
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
    EntityTagPrecondition, ResourceStorage, SettingsStore, StoredUserSettings, apply_merge_patch,
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
    let (document, version) = match load_settings(store, &user).await? {
        Some(stored) => (stored.document, stored.version),
        None => (Value::Object(Default::default()), 0),
    };
    let current_etag = etag(version);

    // Honor If-None-Match only when a document actually exists; an empty default
    // document (version 0) must never be reported as "not modified".
    //
    // List-aware and weak-comparison (RFC 9110 §13.1.2): 304 when ANY supplied
    // tag matches. A malformed value cannot be satisfied, so it is treated as
    // "no match" and the full document is returned — safe for a read.
    if version > 0
        && let Ok(inm) = conditional.if_none_match_tags()
        && !inm.if_none_match_satisfied(Some(&version.to_string()))
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, current_etag)]).into_response());
    }

    Ok(([(header::ETAG, current_etag)], Json(document)).into_response())
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
    // Parse before touching storage so a malformed precondition fails fast.
    let precondition = parse_if_match(&conditional)?;

    // Adopt any pre-#270 document first, so a wholesale replace does not leave
    // the legacy copy stranded and unreachable.
    let current_version = load_settings(store, &user)
        .await?
        .map(|stored| stored.version)
        .unwrap_or(0);

    // Evaluate the precondition against the version we just read, then pin the
    // write to that same version. The store cannot express `*` (or a multi-tag
    // list) as a single version, so resolving it here and pinning to the
    // observed version keeps the write conditional — the backend still rejects
    // it atomically if another writer lands in between.
    check_if_match(&precondition, current_version)?;
    let if_match = precondition.is_present().then_some(current_version);

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
    let precondition = parse_if_match(&conditional)?;

    // The bounds below apply to the *post-merge* document, which only the
    // backend sees. Compute the merge here against the version we read, pin the
    // write to that version so what was validated is exactly what lands, and —
    // only when the caller sent no precondition of their own — absorb a benign
    // concurrent-writer race with a couple of retries.
    let mut attempts = 0;
    loop {
        let (current, version) = match load_settings(store, &user).await? {
            Some(stored) => (stored.document, stored.version),
            None => (Value::Object(Default::default()), 0),
        };
        check_if_match(&precondition, version)?;
        let merged = apply_merge_patch(current, &merge_patch);
        validate_settings_document(&merged)?;

        match store
            .patch_settings(user.as_str(), merge_patch.clone(), Some(version))
            .await
        {
            Ok(stored) => return Ok(settings_response(stored)),
            // Only a caller who sent *no* precondition gets the conflict
            // absorbed. A caller who sent one — including `*` — asked to be
            // told about the race, so the failure must surface.
            Err(e) if is_lock_failure(&e) && !precondition.is_present() && attempts < 2 => {
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

/// Parses `If-Match` for `/_user/settings`.
///
/// Delegates to the shared, list-aware parser in
/// [`helios_persistence::core::preconditions`] — this endpoint used to carry its
/// own private tri-state enum, which handled only a *single* entity-tag and so
/// rejected the comma-separated list the header is actually defined as
/// (issue #311). The shared type subsumes it.
///
/// Per [RFC 9110 §13.1.1] a precondition that cannot be satisfied means the
/// method MUST NOT be performed. A syntactically invalid field value can never
/// be satisfied, so it is a failed precondition rather than a missing one —
/// `412`, not "carry on". (`400` would be equally conformant for malformed
/// syntax; `412` is kept for consistency with every other precondition path in
/// this crate, and because #270 already settled the question here.)
///
/// Both the strong form `"{n}"` and the legacy weak form `W/"{n}"` are accepted;
/// the shared comparison ignores the weakness flag. This endpoint emits strong
/// ETags (see [`etag`]), but a client deployed against an earlier build may
/// still echo back a weak one, so rejecting `W/` outright would break it for no
/// benefit.
///
/// [RFC 9110 §13.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.1
fn parse_if_match(conditional: &ConditionalHeaders) -> RestResult<EntityTagPrecondition> {
    conditional
        .if_match_tags()
        .cloned()
        .map_err(|e| RestError::PreconditionFailed {
            message: format!(
                "Malformed If-Match value: {e}; expected an entity-tag such as \"3\", or *"
            ),
        })
}

/// Checks a parsed precondition against the currently stored `version`, where
/// version `0` means "no document has been written yet".
///
/// The GET handler serves that state as a *real representation* — `{}` with
/// `ETag: "0"` — so the validator it hands out must round-trip: a concrete
/// `If-Match: "0"` matches version 0, or a fresh user's read-then-conditional-
/// write could never succeed (#442). `If-Match: *` keeps requiring an actually
/// stored document, per its "unless it does not exist" semantics.
fn check_if_match(precondition: &EntityTagPrecondition, version: i64) -> RestResult<()> {
    let satisfied = match precondition {
        EntityTagPrecondition::Tags(_) => {
            precondition.if_match_satisfied(Some(&version.to_string()))
        }
        _ => precondition.if_match_satisfied((version > 0).then(|| version.to_string()).as_deref()),
    };
    if satisfied {
        return Ok(());
    }

    let message = if version > 0 {
        format!("If-Match precondition failed: current settings version is {version}")
    } else {
        "If-Match precondition failed: no settings document exists yet".to_string()
    };
    Err(RestError::PreconditionFailed { message })
}

/// Reads the caller's settings document, adopting a pre-#270 one on a miss.
///
/// The hit path is a single read: the legacy key is only consulted when the
/// current key holds nothing, which after the first access is never again true
/// for a given user. That matters because this runs on every page load.
async fn load_settings(
    store: &Arc<dyn SettingsStore>,
    user: &UserKey,
) -> RestResult<Option<StoredUserSettings>> {
    if let Some(stored) = store.get_settings(user.as_str()).await? {
        return Ok(Some(stored));
    }
    adopt_legacy_document(store, user).await
}

/// Moves a settings document written under the pre-#270 key encoding to the
/// current one, once, on first access.
///
/// A batch migration is not possible: computing the new key needs the issuer and
/// subject *separately*, but the legacy key `"{iss}|{sub}"` is exactly the
/// ambiguous string that cannot be split back apart. At request time we hold the
/// real `Principal`, so both keys are derivable — which is why this runs lazily
/// on the request path instead.
///
/// Backend-agnostic by construction, so it covers SQLite, PostgreSQL, MongoDB
/// and S3 identically and needs no schema change on any of them: `user_key` is
/// an opaque primary key everywhere.
///
/// Copy-then-delete, never the reverse: an interruption in between leaves the
/// legacy document in place and the next request simply retries. The write
/// asserts the current key is still empty (`Some(0)`), so a concurrent request
/// that adopted it first is not clobbered.
async fn adopt_legacy_document(
    store: &Arc<dyn SettingsStore>,
    user: &UserKey,
) -> RestResult<Option<StoredUserSettings>> {
    let Some(legacy_key) = user.legacy_key() else {
        return Ok(None);
    };
    let Some(legacy) = store.get_settings(legacy_key).await? else {
        return Ok(None);
    };

    let adopted = match store
        .put_settings(user.as_str(), legacy.document, Some(0))
        .await
    {
        Ok(stored) => stored,
        // Another request adopted it first; its document is equally valid, so
        // take whatever is now there rather than racing to overwrite it.
        Err(e) if is_lock_failure(&e) => return Ok(store.get_settings(user.as_str()).await?),
        Err(e) => return Err(e.into()),
    };

    store.delete_settings(legacy_key).await?;
    Ok(Some(adopted))
}

/// Builds the success response for a write: the stored document plus its ETag.
fn settings_response(stored: StoredUserSettings) -> Response {
    (
        [(header::ETAG, etag(stored.version))],
        Json(stored.document),
    )
        .into_response()
}

/// Formats a version number as a *strong* ETag.
///
/// `If-Match` requires the strong comparison function ([RFC 9110 §13.1.1],
/// defined in §8.8.3.2), under which a weak validator can never match. Emitting
/// `W/"{n}"` here — as this endpoint previously did — meant a conformant client
/// echoing the server's own ETag back on `If-Match` could never satisfy the
/// precondition, making the advertised optimistic-concurrency contract
/// unimplementable.
///
/// A strong validator is factually justified: the settings document is a single
/// JSON blob keyed by a monotonic integer version, so equal versions are
/// byte-equivalent. The strong form also remains valid for `If-None-Match` on
/// `GET`, which uses weak comparison and accepts strong tags.
///
/// This deliberately differs from the FHIR handlers (`update.rs`, `patch.rs`),
/// which must keep `W/"{versionId}"`: FHIR mandates weak ETags, a deliberate
/// deviation from HTTP. `/_user/settings` is not a FHIR route and should not
/// inherit it.
///
/// [RFC 9110 §13.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.1
fn etag(version: i64) -> String {
    format!("\"{version}\"")
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

    fn with_if_match(raw: &str) -> ConditionalHeaders {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, raw.parse().unwrap());
        ConditionalHeaders::from_headers(&headers)
    }

    #[test]
    fn parse_if_match_accepts_strong_and_weak_tags() {
        // Strong is the form this endpoint emits (and the form the web UI echoes
        // back, since it copies the ETag response header verbatim).
        let strong = parse_if_match(&with_if_match("\"7\"")).unwrap();
        assert!(check_if_match(&strong, 7).is_ok());
        assert!(check_if_match(&strong, 8).is_err());

        // Weak is still accepted, for clients built against earlier releases.
        let weak = parse_if_match(&with_if_match("W/\"5\"")).unwrap();
        assert!(check_if_match(&weak, 5).is_ok());

        assert_eq!(
            parse_if_match(&with_if_match("*")).unwrap(),
            EntityTagPrecondition::Any
        );
        assert_eq!(
            parse_if_match(&ConditionalHeaders::from_headers(&HeaderMap::new())).unwrap(),
            EntityTagPrecondition::Absent
        );
    }

    /// A bare, unquoted value is not a valid `entity-tag` (RFC 9110 §8.8.3
    /// requires the opaque tag to be quoted). It used to be accepted here only
    /// as an artifact of `trim_matches('"')`; nothing sends it — the web UI
    /// echoes the quoted ETag verbatim — so it now fails the precondition
    /// rather than being silently reinterpreted.
    #[test]
    fn parse_if_match_rejects_bare_unquoted_tag() {
        assert!(parse_if_match(&with_if_match("9")).is_err());
    }

    /// Issue #311: the header is a list, and this endpoint's old private parser
    /// could only ever see one tag.
    #[test]
    fn parse_if_match_accepts_a_list_and_matches_any_member() {
        let p = parse_if_match(&with_if_match("\"3\", \"4\"")).unwrap();
        assert!(check_if_match(&p, 3).is_ok());
        assert!(check_if_match(&p, 4).is_ok());
        assert!(check_if_match(&p, 5).is_err());
    }

    /// A malformed `If-Match` must fail the precondition, not be discarded.
    ///
    /// Previously this returned `None` — "no precondition" — silently turning a
    /// request that asked to be conditional into an unconditional
    /// last-writer-wins overwrite (issue #270).
    ///
    /// Note `"1", "2"` is deliberately NOT in this list any more: it is a
    /// well-formed two-element list, and rejecting it was issue #311.
    #[test]
    fn parse_if_match_rejects_a_malformed_value() {
        for raw in ["garbage", "1.5", "-", "3", "W/3", "\"unterminated"] {
            let err = parse_if_match(&with_if_match(raw)).unwrap_err();
            assert!(
                matches!(err, RestError::PreconditionFailed { .. }),
                "expected a precondition failure for {raw:?}, got {err:?}"
            );
        }
    }

    /// An empty opaque tag (`""`) is syntactically valid but can never equal a
    /// settings version, so it parses cleanly and then fails the *check* — still
    /// a 412, just at the correct stage.
    #[test]
    fn empty_opaque_tag_parses_but_never_matches() {
        let p = parse_if_match(&with_if_match("\"\"")).unwrap();
        assert!(check_if_match(&p, 1).is_err());
        assert!(check_if_match(&p, 0).is_err());
    }

    /// `If-Match: *` means "the document must already exist" (RFC 9110
    /// §13.1.1), *not* "no precondition" — so it must fail against version 0.
    #[test]
    fn wildcard_requires_an_existing_document() {
        assert!(check_if_match(&EntityTagPrecondition::Any, 0).is_err());
        assert!(check_if_match(&EntityTagPrecondition::Any, 1).is_ok());
    }

    #[test]
    fn version_precondition_matches_exactly() {
        let v3 = parse_if_match(&with_if_match("\"3\"")).unwrap();
        assert!(check_if_match(&v3, 3).is_ok());
        assert!(check_if_match(&v3, 4).is_err());
        // Version 0 = no document; a caller asserting v3 must be told.
        assert!(check_if_match(&v3, 0).is_err());
    }

    #[test]
    fn absent_precondition_never_fails() {
        assert!(check_if_match(&EntityTagPrecondition::Absent, 0).is_ok());
        assert!(check_if_match(&EntityTagPrecondition::Absent, 42).is_ok());
    }

    /// `If-Match` requires strong comparison, so this endpoint emits a strong
    /// validator (unlike the FHIR routes, where weak is mandated).
    #[test]
    fn etag_is_strong() {
        assert_eq!(etag(5), "\"5\"");
        assert!(!etag(5).starts_with("W/"));
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
