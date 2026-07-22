//! Per-user UI settings storage.
//!
//! This module defines [`SettingsStore`], a small storage abstraction for an
//! opaque, per-user JSON settings document. It is deliberately *separate* from
//! the FHIR [`ResourceStorage`](crate::core::ResourceStorage) hierarchy: UI
//! preferences such as theme, default tenant, active FHIR version, or recent
//! queries are private user state, not FHIR resources, and should not surface
//! in `CapabilityStatement`, `_history`, `_search`, or `$export`.
//!
//! # Design
//!
//! - **One document per user.** Each user owns a single JSON *object* keyed by a
//!   caller-supplied `user_key` (e.g. `"u2:{issuer_len}:{issuer}:{subject}"`
//!   from an authenticated principal, or a fixed local key when auth is
//!   disabled). The key is opaque to this layer; deriving it injectively is the
//!   caller's job — see `helios_rest::extractors::UserKey`.
//! - **Opaque and extensible.** The document is stored as an arbitrary
//!   [`serde_json::Value`] object, so new settings keys require no schema or
//!   code changes — the frontend owns the document shape.
//! - **User-global.** Settings are keyed by user only, not by tenant: a
//!   preference like "default tenant" is inherently cross-tenant. Settings that
//!   genuinely need per-tenant scoping should nest inside the document (e.g.
//!   `{"perTenant": {"<tenant>": {...}}}`) rather than changing the key.
//! - **Optimistically lockable.** Each document carries a monotonic `version`
//!   that increments on every write and is surfaced to clients as a weak ETag
//!   (`W/"{version}"`). Callers may pass `if_match_version` to make a write
//!   conditional and avoid lost updates.
//!
//! # Document conventions
//!
//! The document is schema-less, but keys shared between clients follow agreed
//! conventions. The established ones:
//!
//! - `theme` — `"light"` / `"dark"` (the web UI theme toggle).
//! - `savedQueries` — per-user saved FHIR queries, grouped by resource type
//!   and **keyed by query id** so a JSON merge patch can create, update, or
//!   delete a single entry without clobbering its siblings (RFC 7386 replaces
//!   arrays wholesale, hence objects, not arrays):
//!
//!   ```json
//!   {
//!     "savedQueries": {
//!       "Patient": {
//!         "01J8ZQ3F9V": {
//!           "name": "Smiths in Boston",
//!           "query": "name=smith&address-city=Boston",
//!           "createdAt": "2026-07-01T12:00:00Z",
//!           "lastAccessedAt": "2026-07-09T09:14:22Z",
//!           "accessCount": 12
//!         }
//!       }
//!     }
//!   }
//!   ```
//!
//!   Clients bump `lastAccessedAt` / `accessCount` with a merge patch when the
//!   query is run, and sort by `lastAccessedAt` descending, falling back to
//!   `createdAt` for never-run entries. The REST layer enforces structural
//!   bounds on this key (entries-per-type and whole-document size caps); see
//!   `helios-rest`'s user-settings handlers.
//!
//! - `recentSearches` — the search-builder's run history, newest first,
//!   deduped by query and capped by the client (currently 10):
//!
//!   ```json
//!   {
//!     "recentSearches": [
//!       { "query": "/Patient?name=smith", "at": "2026-07-11T09:14:22Z" }
//!     ]
//!   }
//!   ```
//!
//!   Unlike `savedQueries`, this is an array on purpose: it is a small
//!   bounded cache rewritten wholesale on every run, not sibling-keyed
//!   state, so RFC 7386's replace-the-array semantics are exactly right.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::StorageResult;

/// A stored per-user settings document together with its optimistic-lock
/// version and last-modified timestamp.
#[derive(Debug, Clone)]
pub struct StoredUserSettings {
    /// The opaque key identifying the owning user.
    pub user_key: String,
    /// The settings document. Always a JSON object.
    pub document: Value,
    /// Monotonic version, bumped on every write. Surfaced as the `W/"{version}"`
    /// ETag so clients can perform conditional (`If-Match`) updates.
    pub version: i64,
    /// Timestamp of the most recent write.
    pub updated_at: DateTime<Utc>,
}

/// Storage abstraction for opaque, per-user JSON settings documents.
///
/// Implemented by every standalone primary backend: SQLite, PostgreSQL, and
/// MongoDB, and S3 (where the read-modify-write is a compare-and-swap over
/// conditional `PutObject` rather than a transaction). The trait is intentionally
/// minimal — get the whole document, replace it, or merge-patch it — because the
/// document body is opaque to the server.
#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// Returns the user's settings document, or `None` if the user has never
    /// stored any settings.
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>>;

    /// Replaces the user's entire settings document.
    ///
    /// `document` must be a JSON object; callers are expected to validate this
    /// before invoking. When `if_match_version` is `Some`, the write only
    /// succeeds if it matches the currently stored version, otherwise a
    /// [`ConcurrencyError::OptimisticLockFailure`](crate::error::ConcurrencyError::OptimisticLockFailure)
    /// is returned. A `Some(0)` precondition asserts the document does not yet
    /// exist.
    async fn put_settings(
        &self,
        user_key: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings>;

    /// Applies an [RFC 7386](https://www.rfc-editor.org/rfc/rfc7386) JSON Merge
    /// Patch to the user's settings document, creating an empty document first
    /// if none exists.
    ///
    /// The read-modify-write is performed atomically by the backend (under a row
    /// lock / transaction). `if_match_version` enforces optimistic locking as in
    /// [`put_settings`](Self::put_settings).
    async fn patch_settings(
        &self,
        user_key: &str,
        merge_patch: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings>;

    /// Deletes the user's settings document, returning whether one existed.
    ///
    /// Deleting an absent document is not an error — the method is idempotent,
    /// so a caller that races another deleter still sees success.
    ///
    /// This exists to complete the lifecycle the other three methods imply: a
    /// document that can be created must be removable, both to migrate a
    /// document written under a superseded key encoding (see issue #270) and so
    /// that a user's stored preferences — which may include recent FHIR search
    /// strings — are erasable at all.
    async fn delete_settings(&self, user_key: &str) -> StorageResult<bool>;
}

/// Applies an [RFC 7386](https://www.rfc-editor.org/rfc/rfc7386) JSON Merge
/// Patch to `target`, returning the merged result.
///
/// Per the specification: a non-object patch replaces the target outright; an
/// object patch is applied member-wise, where a `null` member deletes the
/// corresponding key and any other value is merged recursively.
pub fn apply_merge_patch(target: Value, patch: &Value) -> Value {
    match patch {
        Value::Object(patch_members) => {
            // A non-object target is discarded in favor of an empty object, per
            // the spec ("if Target is not an Object, set it to an empty Object").
            let mut merged = match target {
                Value::Object(map) => map,
                _ => serde_json::Map::new(),
            };
            for (key, patch_value) in patch_members {
                if patch_value.is_null() {
                    merged.remove(key);
                } else {
                    let existing = merged.remove(key).unwrap_or(Value::Null);
                    merged.insert(key.clone(), apply_merge_patch(existing, patch_value));
                }
            }
            Value::Object(merged)
        }
        _ => patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_patch_sets_and_overwrites_keys() {
        let target = json!({"theme": "dark", "defaultTenant": "acme"});
        let patch = json!({"theme": "light"});
        assert_eq!(
            apply_merge_patch(target, &patch),
            json!({"theme": "light", "defaultTenant": "acme"})
        );
    }

    #[test]
    fn merge_patch_null_deletes_key() {
        let target = json!({"theme": "dark", "defaultTenant": "acme"});
        let patch = json!({"defaultTenant": null});
        assert_eq!(apply_merge_patch(target, &patch), json!({"theme": "dark"}));
    }

    #[test]
    fn merge_patch_merges_nested_objects() {
        let target = json!({"recentQueries": {"Patient": ["name=smith"]}});
        let patch = json!({"recentQueries": {"Observation": ["code=1234"]}});
        assert_eq!(
            apply_merge_patch(target, &patch),
            json!({"recentQueries": {"Patient": ["name=smith"], "Observation": ["code=1234"]}})
        );
    }

    #[test]
    fn merge_patch_non_object_replaces_target() {
        let target = json!({"theme": "dark"});
        let patch = json!("scalar");
        assert_eq!(apply_merge_patch(target, &patch), json!("scalar"));
    }

    #[test]
    fn merge_patch_replaces_non_object_target_with_object() {
        let target = json!("not-an-object");
        let patch = json!({"theme": "dark"});
        assert_eq!(apply_merge_patch(target, &patch), json!({"theme": "dark"}));
    }
}
