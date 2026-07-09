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
//!   caller-supplied `user_key` (e.g. `"{issuer}|{subject}"` from an
//!   authenticated principal, or a fixed local key when auth is disabled).
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
/// Implemented by the SQLite and PostgreSQL backends. The trait is intentionally
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
