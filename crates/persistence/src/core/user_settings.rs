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
//! - **Keyed by user, scoped by tenant *inside* the document.** The document is
//!   keyed by user only — a preference like "default tenant" is inherently
//!   cross-tenant, and the key must stay tenant-free for it to roam. But most of
//!   what a client stores is *derived from one tenant's data*: a saved FHIR query
//!   such as `Patient?name=smith&birthdate=1970-01-01` is a PHI-bearing string
//!   belonging to the tenant it was written against. Those keys therefore live
//!   under a reserved [`BY_TENANT_KEY`] map so a tenant purge can reach them.
//!   See [the scoping section](#tenant-scoping-within-the-document).
//! - **Optimistically lockable.** Each document carries a monotonic `version`
//!   that increments on every write and is surfaced to clients as a weak ETag
//!   (`W/"{version}"`). Callers may pass `if_match_version` to make a write
//!   conditional and avoid lost updates.
//!
//! # Tenant scoping within the document
//!
//! The **wire format is flat** and unchanged: a client `GET`s, `PUT`s and
//! `PATCH`es a plain object with `theme`, `savedQueries`, … at the top level. The
//! REST layer projects and scopes on the way through, so no client — including
//! ones this project does not ship — has to know about the layout below.
//!
//! The **stored format** separates the two lifetimes:
//!
//! ```json
//! {
//!   "theme": "dark",              // user-global: roams across tenants
//!   "fhirVersion": "R4",          // user-global
//!   "byTenant": {
//!     "acme": { "savedQueries": { "Patient": { … } }, "recentSearches": [ … ] },
//!     "beta": { "savedQueries": { … } }
//!   }
//! }
//! ```
//!
//! [`GLOBAL_SETTINGS_KEYS`] is the *complete* list of top-level keys that stay
//! user-global — deliberately a denylist rather than an allowlist of scoped keys.
//! A client key the server has never heard of is scoped by default, so a future
//! key holding PHI-derived content is covered without anyone remembering to add
//! it here. The cost of getting it wrong in this direction is that a preference
//! resets once per tenant; the cost in the other direction is unreachable PHI,
//! which is the whole point of this design (issue #313).
//!
//! ## Documents written before scoping existed
//!
//! A pre-#313 document has its scoped keys at the top level with no
//! [`BY_TENANT_KEY`] map. Such a document is *attributed* to its `tenantId` key
//! (the tenant the UI was pointed at, and therefore the tenant those queries were
//! written against) when that names a tenant, and is otherwise **unattributed**.
//!
//! - [`project_for_tenant`] shows a legacy document's scoped keys to its
//!   attributed tenant, or — when unattributed — to every tenant, exactly
//!   reproducing the pre-#313 user-global behaviour until the document is
//!   normalized. Reads never rewrite: `GET` stays side-effect-free.
//! - [`normalize_legacy`] runs on the first **write**, moving those keys under
//!   `byTenant.{attribution}`.
//! - [`purge_tenant_subtree`] removes an *unattributed* legacy document's scoped
//!   keys on **any** tenant purge. That is deliberate: unattributed PHI cannot be
//!   proven to belong to another tenant, and leaving it behind is the exact gap
//!   this design closes. A saved-query list is convenience state that a user can
//!   recreate; unreachable PHI is not. After the first purge the store is fully
//!   attributed and this branch stops firing.
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
use serde_json::{Map, Value};

use crate::error::StorageResult;

/// The reserved top-level key holding the per-tenant subtrees of a stored
/// settings document.
///
/// This key is **never** accepted from a client: the REST layer rejects a
/// request body carrying it. If it were writable, a caller scoped to one tenant
/// could plant (or read back) content under another tenant's subtree, turning
/// this scoping into the cross-user leak that issue #270 closed.
pub const BY_TENANT_KEY: &str = "byTenant";

/// The complete set of top-level keys that stay **user-global**. Every other
/// top-level key is tenant-scoped.
///
/// These are exactly the keys the server itself reads or writes, and each is a
/// preference that must roam across tenants for the UI to behave:
///
/// | Key | Written by | Why it is global |
/// |-----|------------|------------------|
/// | `theme` | `crates/ui/assets/theme.js` | a user's light/dark choice is not tenant state |
/// | `nav` | `crates/ui/assets/nav.js` | sidebar collapsed/expanded, ditto |
/// | `fhirVersion` | `helios_ui::set_version` | the version selector, read by `resolve_prefs` |
/// | `tenantId` | `helios_ui::set_tenant` | **the tenant selector itself** — scoping this per tenant would make it unreadable before a tenant is known |
///
/// Adding a key here removes it from a tenant purge's reach, so it must be a
/// preference that provably cannot carry data derived from a tenant's records.
pub const GLOBAL_SETTINGS_KEYS: &[&str] = &["theme", "nav", "fhirVersion", "tenantId"];

/// The `tenantId` key, read to attribute a pre-#313 document to a tenant.
const TENANT_CHOICE_KEY: &str = "tenantId";

/// Whether a top-level settings key is user-global (as opposed to tenant-scoped).
///
/// [`BY_TENANT_KEY`] itself is structural rather than global — it is neither
/// projected to a client nor swept by a purge of some other tenant — so it is
/// handled separately by every caller and answers `false` here.
fn is_global_key(key: &str) -> bool {
    GLOBAL_SETTINGS_KEYS.contains(&key)
}

/// Borrows a document as an object, or an empty one for any non-object value.
///
/// A stored document is always an object (the REST layer rejects anything else),
/// but the store is schema-less and this module must not panic on a hand-edited
/// or corrupt row.
fn as_object(value: &Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

/// The tenant a pre-#313 (un-normalized) document's top-level scoped keys belong
/// to, taken from its `tenantId` preference.
///
/// `None` means *unattributed*: the user never used the tenant selector, so the
/// server has no evidence of which tenant those keys came from. See the module
/// docs for how each operation treats that case.
fn legacy_attribution(document: &Value) -> Option<&str> {
    document
        .get(TENANT_CHOICE_KEY)
        .and_then(Value::as_str)
        .filter(|choice| !choice.is_empty())
}

/// Whether `document` still carries tenant-scoped keys at the top level, i.e.
/// was written before this scoping existed.
fn has_legacy_scoped_keys(document: &Value) -> bool {
    as_object(document)
        .keys()
        .any(|key| key != BY_TENANT_KEY && !is_global_key(key))
}

/// Projects a stored document into the flat, client-facing view for `tenant`.
///
/// The result is the shape every client has always seen: user-global keys and
/// this tenant's scoped keys, side by side at the top level, with no
/// [`BY_TENANT_KEY`] map. **Exactly one** tenant subtree is ever projected, so a
/// caller scoped to one tenant can never observe another's.
///
/// A pre-#313 document's top-level scoped keys are projected when the document is
/// attributed to `tenant`, or when it is unattributed (preserving the pre-#313
/// user-global behaviour until a write normalizes it). This is a pure function:
/// reads never rewrite the store.
pub fn project_for_tenant(document: &Value, tenant: &str) -> Value {
    let stored = as_object(document);
    let mut out = Map::new();

    let show_legacy = legacy_attribution(document).is_none_or(|owner| owner == tenant);
    for (key, value) in &stored {
        if key == BY_TENANT_KEY {
            continue;
        }
        if is_global_key(key) || show_legacy {
            out.insert(key.clone(), value.clone());
        }
    }

    // This tenant's subtree wins over any legacy key of the same name: it is the
    // normalized, authoritative copy.
    if let Some(subtree) = stored
        .get(BY_TENANT_KEY)
        .and_then(Value::as_object)
        .and_then(|by_tenant| by_tenant.get(tenant))
        .and_then(Value::as_object)
    {
        for (key, value) in subtree {
            out.insert(key.clone(), value.clone());
        }
    }

    Value::Object(out)
}

/// Moves a pre-#313 document's top-level scoped keys under
/// `byTenant.{attribution}`, where `attribution` is the document's own
/// `tenantId` when it has one and `writing_tenant` otherwise.
///
/// Returns `true` when the document was changed. Idempotent: a document that has
/// already been normalized has no top-level scoped keys left to move.
///
/// Called on the **write** path only. Attribution prefers the document's own
/// `tenantId` so a user who has switched tenants does not have queries written
/// against tenant A silently re-filed under tenant B.
pub fn normalize_legacy(document: &mut Value, writing_tenant: &str) -> bool {
    if !has_legacy_scoped_keys(document) {
        return false;
    }
    let owner = legacy_attribution(document)
        .unwrap_or(writing_tenant)
        .to_string();

    let stored = as_object(document);
    let mut globals = Map::new();
    let mut moved = Map::new();
    let mut by_tenant = stored
        .get(BY_TENANT_KEY)
        .map(|value| as_object(value))
        .unwrap_or_default();

    for (key, value) in stored {
        if key == BY_TENANT_KEY {
            continue;
        }
        if is_global_key(&key) {
            globals.insert(key, value);
        } else {
            moved.insert(key, value);
        }
    }

    // An existing subtree wins: it is the normalized copy, and the legacy key is
    // the stale one it superseded.
    let subtree = by_tenant
        .entry(owner)
        .or_insert_with(|| Value::Object(Map::new()));
    let mut merged = as_object(subtree);
    for (key, value) in moved {
        merged.entry(key).or_insert(value);
    }
    *subtree = Value::Object(merged);

    globals.insert(BY_TENANT_KEY.to_string(), Value::Object(by_tenant));
    *document = Value::Object(globals);
    true
}

/// Builds the stored document for a `PUT` of the flat `incoming` view by `tenant`.
///
/// `PUT` replaces the document the client can *see*, which is exactly
/// [`project_for_tenant`]'s output — so it replaces the user-global keys and this
/// tenant's subtree, and **leaves every other tenant's subtree untouched**. A
/// naive whole-document replace would destroy them.
///
/// `current` is normalized first, so a legacy document's scoped keys are filed
/// under their attributed tenant rather than silently dropped by the replace.
pub fn stored_for_put(current: &Value, incoming: Value, tenant: &str) -> Value {
    let mut base = current.clone();
    normalize_legacy(&mut base, tenant);

    let mut by_tenant = as_object(&base)
        .get(BY_TENANT_KEY)
        .map(|value| as_object(value))
        .unwrap_or_default();

    let mut globals = Map::new();
    let mut scoped = Map::new();
    for (key, value) in as_object(&incoming) {
        // A client body carrying `byTenant` is rejected upstream; drop it here
        // too so this function is safe for any caller.
        if key == BY_TENANT_KEY {
            continue;
        }
        if is_global_key(&key) {
            globals.insert(key, value);
        } else {
            scoped.insert(key, value);
        }
    }

    if scoped.is_empty() {
        by_tenant.remove(tenant);
    } else {
        by_tenant.insert(tenant.to_string(), Value::Object(scoped));
    }
    if !by_tenant.is_empty() {
        globals.insert(BY_TENANT_KEY.to_string(), Value::Object(by_tenant));
    }
    Value::Object(globals)
}

/// Rewrites a flat [RFC 7386](https://www.rfc-editor.org/rfc/rfc7386) merge-patch
/// so its tenant-scoped members apply to `tenant`'s subtree.
///
/// Wrapping — rather than rewriting member by member — preserves merge-patch
/// semantics exactly, including `null` deletion: a patch of
/// `{"savedQueries": {"Patient": {"id": null}}}` becomes
/// `{"byTenant": {"acme": {"savedQueries": {"Patient": {"id": null}}}}}`, which
/// deletes the same single entry and nothing else.
pub fn scope_merge_patch(patch: Value, tenant: &str) -> Value {
    let mut globals = Map::new();
    let mut scoped = Map::new();
    for (key, value) in as_object(&patch) {
        if key == BY_TENANT_KEY {
            continue;
        }
        if is_global_key(&key) {
            globals.insert(key, value);
        } else {
            scoped.insert(key, value);
        }
    }
    if !scoped.is_empty() {
        let mut by_tenant = Map::new();
        by_tenant.insert(tenant.to_string(), Value::Object(scoped));
        globals.insert(BY_TENANT_KEY.to_string(), Value::Object(by_tenant));
    }
    Value::Object(globals)
}

/// Removes everything belonging to `tenant` from a stored settings document,
/// in place. Returns `true` when the document was changed.
///
/// Removes two things:
///
/// 1. `byTenant.{tenant}` — the normalized subtree. Always, and only, that one
///    key: other tenants' subtrees and every user-global key survive.
/// 2. A pre-#313 document's top-level scoped keys, when the document is
///    attributed to `tenant` **or is unattributed**.
///
/// The unattributed case is the deliberate choice recorded in the module docs:
/// such content cannot be proven to belong to another tenant, and leaving it is
/// the erasure gap this exists to close. It is bounded — only tenant-scoped keys,
/// never a user-global preference, never another tenant's subtree — and
/// self-limiting, since the first purge leaves the store fully attributed.
pub fn purge_tenant_subtree(document: &mut Value, tenant: &str) -> bool {
    let mut stored = as_object(document);
    let mut changed = false;

    // Attribution is read up front, as an owned value: it comes from `tenantId`,
    // a user-global key that neither removal below touches, so the order is
    // immaterial — but taking it first keeps the two edits independent.
    let owner = legacy_attribution(document).map(str::to_string);

    // 1. The normalized subtree. Exactly this one key: `Map::remove` is an exact
    //    match, so a sibling whose id merely starts with `tenant` is untouched.
    let mut drop_empty_map = false;
    if let Some(Value::Object(by_tenant)) = stored.get_mut(BY_TENANT_KEY) {
        if by_tenant.remove(tenant).is_some() {
            changed = true;
            // Only tidy away a map this call emptied. An already-empty one is
            // left alone so a purge that found nothing stays a true no-op and
            // does not burn a version (and a client's ETag) for cosmetics.
            drop_empty_map = by_tenant.is_empty();
        }
    }
    if drop_empty_map {
        stored.remove(BY_TENANT_KEY);
    }

    // 2. A pre-#313 document's top-level scoped keys, when this tenant owns them
    //    or nobody does.
    if owner.as_deref().is_none_or(|owner| owner == tenant) {
        let doomed: Vec<String> = stored
            .keys()
            .filter(|key| {
                let key = key.as_str();
                key != BY_TENANT_KEY && !is_global_key(key)
            })
            .cloned()
            .collect();
        for key in doomed {
            stored.remove(&key);
            changed = true;
        }
    }

    if changed {
        *document = Value::Object(stored);
    }
    changed
}

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

    /// Removes everything belonging to `tenant_id` from **every** stored settings
    /// document, returning how many documents were changed.
    ///
    /// This is the tenant-offboarding half of the erasure story (issue #313).
    /// Settings are keyed by user, not tenant, so
    /// [`purge_tenant_data`](crate::core::ResourceStorage::purge_tenant_data)
    /// could not otherwise reach the PHI-derived query strings a client stores —
    /// a saved `Patient?name=…&birthdate=…` is derived from one tenant's records
    /// even though the document holding it is user-global.
    ///
    /// Every implementation applies [`purge_tenant_subtree`] to each document, so
    /// all four backends erase byte-identically; a backend's only freedom is how
    /// it enumerates and writes back. Each changed document's `version` is bumped
    /// so a client holding a stale `ETag` is forced to re-read rather than being
    /// able to `PUT` the purged content straight back.
    ///
    /// This is deliberately a purpose-built sweep rather than a general
    /// "enumerate every user" primitive: nothing else needs to list users, and
    /// adding that surface would invite exactly the cross-user access this store
    /// spent issue #270 closing.
    ///
    /// Callers do not invoke this directly — every backend's `purge_tenant_data`
    /// runs it as part of the purge, so there is one choke point rather than one
    /// per call site.
    async fn purge_tenant_settings(&self, tenant_id: &str) -> StorageResult<u64>;
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

    // ── Tenant scoping within the document (issue #313) ─────────────────────

    /// A normalized document projects globals plus exactly one tenant's subtree.
    #[test]
    fn projection_shows_one_tenant_and_hides_the_reserved_key() {
        let stored = json!({
            "theme": "dark",
            "byTenant": {
                "acme": {"savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}},
                "beta": {"savedQueries": {"Patient": {"q2": {"query": "name=jones"}}}}
            }
        });
        assert_eq!(
            project_for_tenant(&stored, "acme"),
            json!({"theme": "dark", "savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}})
        );
    }

    /// The reserved key is never visible on the wire, and another tenant's
    /// content is never projected — the leak the scoping exists to prevent.
    #[test]
    fn projection_never_leaks_another_tenants_subtree() {
        let stored = json!({
            "byTenant": {"acme": {"savedQueries": {"Patient": {"secret": {"query": "name=smith"}}}}}
        });
        let seen = project_for_tenant(&stored, "beta");
        assert_eq!(seen, json!({}));
        assert!(seen.get(BY_TENANT_KEY).is_none());
        assert!(!serde_json::to_string(&seen).unwrap().contains("smith"));
    }

    /// A tenant with no subtree sees only the globals.
    #[test]
    fn projection_for_unknown_tenant_returns_globals_only() {
        let stored = json!({"theme": "dark", "byTenant": {"acme": {"savedQueries": {}}}});
        assert_eq!(
            project_for_tenant(&stored, "beta"),
            json!({"theme": "dark"})
        );
    }

    /// A legacy document attributed by `tenantId` shows its scoped keys to that
    /// tenant only.
    #[test]
    fn projection_of_attributed_legacy_document_is_scoped_to_its_owner() {
        let legacy = json!({
            "theme": "dark",
            "tenantId": "acme",
            "savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}
        });
        let owner = project_for_tenant(&legacy, "acme");
        assert_eq!(
            owner["savedQueries"]["Patient"]["q1"]["query"],
            "name=smith"
        );

        let other = project_for_tenant(&legacy, "beta");
        assert!(other.get("savedQueries").is_none());
        assert_eq!(other["tenantId"], "acme", "globals still roam");
    }

    /// An unattributed legacy document keeps its pre-#313 user-global behaviour
    /// until a write normalizes it — nobody's settings vanish on upgrade.
    #[test]
    fn projection_of_unattributed_legacy_document_is_visible_everywhere() {
        let legacy = json!({"savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}});
        for tenant in ["acme", "beta"] {
            assert_eq!(
                project_for_tenant(&legacy, tenant)["savedQueries"]["Patient"]["q1"]["query"],
                "name=smith"
            );
        }
    }

    /// A normalized subtree supersedes a same-named legacy key.
    #[test]
    fn projection_prefers_the_normalized_subtree_over_a_legacy_key() {
        let stored = json!({
            "savedQueries": {"Patient": {"old": {}}},
            "byTenant": {"acme": {"savedQueries": {"Patient": {"new": {}}}}}
        });
        assert_eq!(
            project_for_tenant(&stored, "acme")["savedQueries"],
            json!({"Patient": {"new": {}}})
        );
    }

    /// Normalization files legacy keys under the document's own `tenantId`, not
    /// under whichever tenant happens to be writing.
    #[test]
    fn normalize_prefers_the_documents_own_attribution() {
        let mut doc = json!({
            "theme": "dark",
            "tenantId": "acme",
            "savedQueries": {"Patient": {"q1": {}}}
        });
        assert!(normalize_legacy(&mut doc, "beta"));
        assert_eq!(
            doc,
            json!({
                "theme": "dark",
                "tenantId": "acme",
                "byTenant": {"acme": {"savedQueries": {"Patient": {"q1": {}}}}}
            })
        );
    }

    /// With no attribution, the writing tenant is the best available evidence.
    #[test]
    fn normalize_falls_back_to_the_writing_tenant() {
        let mut doc = json!({"savedQueries": {"Patient": {"q1": {}}}});
        assert!(normalize_legacy(&mut doc, "beta"));
        assert_eq!(
            doc,
            json!({"byTenant": {"beta": {"savedQueries": {"Patient": {"q1": {}}}}}})
        );
    }

    /// Normalization is idempotent and reports "no change" on an already-scoped
    /// document, so it can run on every write without churning versions.
    #[test]
    fn normalize_is_idempotent() {
        let mut doc = json!({"theme": "dark", "byTenant": {"acme": {"savedQueries": {}}}});
        let before = doc.clone();
        assert!(!normalize_legacy(&mut doc, "acme"));
        assert_eq!(doc, before);
    }

    /// An unknown client key is scoped by default — the denylist property that
    /// keeps a future PHI-bearing key from silently reintroducing issue #313.
    #[test]
    fn unknown_keys_are_tenant_scoped_by_default() {
        let mut doc = json!({"theme": "dark", "somethingNew": {"mrn": "12345"}});
        assert!(normalize_legacy(&mut doc, "acme"));
        assert_eq!(doc["theme"], "dark");
        assert_eq!(doc["byTenant"]["acme"]["somethingNew"]["mrn"], "12345");
    }

    /// Every documented global key stays at the top level.
    #[test]
    fn every_global_key_stays_global() {
        let mut doc = json!({
            "theme": "dark", "nav": "collapsed", "fhirVersion": "R4", "tenantId": "acme",
            "savedQueries": {}
        });
        normalize_legacy(&mut doc, "acme");
        for key in GLOBAL_SETTINGS_KEYS {
            assert!(doc.get(*key).is_some(), "{key} must stay user-global");
        }
    }

    /// A `PUT` replaces this tenant's subtree and the globals, and must leave
    /// every other tenant's subtree intact.
    #[test]
    fn put_replaces_only_the_writing_tenants_subtree() {
        let current = json!({
            "theme": "dark",
            "byTenant": {
                "acme": {"savedQueries": {"Patient": {"a": {}}}},
                "beta": {"savedQueries": {"Patient": {"b": {}}}}
            }
        });
        let stored = stored_for_put(
            &current,
            json!({"theme": "light", "savedQueries": {"Patient": {"a2": {}}}}),
            "acme",
        );
        assert_eq!(stored["theme"], "light");
        assert_eq!(
            stored["byTenant"]["acme"]["savedQueries"],
            json!({"Patient": {"a2": {}}})
        );
        assert_eq!(
            stored["byTenant"]["beta"]["savedQueries"],
            json!({"Patient": {"b": {}}}),
            "a PUT by acme must not touch beta"
        );
    }

    /// A `PUT` that drops every scoped key removes this tenant's subtree without
    /// disturbing the others.
    #[test]
    fn put_with_no_scoped_keys_clears_only_this_tenants_subtree() {
        let current = json!({
            "byTenant": {"acme": {"savedQueries": {}}, "beta": {"savedQueries": {"P": {}}}}
        });
        let stored = stored_for_put(&current, json!({"theme": "dark"}), "acme");
        assert!(stored["byTenant"].get("acme").is_none());
        assert!(stored["byTenant"].get("beta").is_some());
    }

    /// A `PUT` normalizes first, so a legacy document attributed elsewhere is
    /// filed rather than dropped by the replace.
    #[test]
    fn put_preserves_legacy_content_attributed_to_another_tenant() {
        let current = json!({
            "tenantId": "acme",
            "savedQueries": {"Patient": {"acme-query": {}}}
        });
        let stored = stored_for_put(
            &current,
            json!({"savedQueries": {"Patient": {"beta-q": {}}}}),
            "beta",
        );
        assert_eq!(
            stored["byTenant"]["acme"]["savedQueries"],
            json!({"Patient": {"acme-query": {}}})
        );
        assert_eq!(
            stored["byTenant"]["beta"]["savedQueries"],
            json!({"Patient": {"beta-q": {}}})
        );
    }

    /// A client cannot smuggle content into another tenant by naming the
    /// reserved key.
    #[test]
    fn put_ignores_a_client_supplied_reserved_key() {
        let stored = stored_for_put(
            &json!({}),
            json!({"byTenant": {"victim": {"savedQueries": {"P": {"x": {}}}}}}),
            "attacker",
        );
        assert!(stored.get("byTenant").is_none());
    }

    /// Wrapping preserves merge-patch semantics, including `null` deletion of a
    /// single sibling entry.
    #[test]
    fn scoped_merge_patch_deletes_one_entry_and_nothing_else() {
        let stored = json!({
            "theme": "dark",
            "byTenant": {"acme": {"savedQueries": {"Patient": {"keep": {}, "drop": {}}}}}
        });
        let patch = scope_merge_patch(json!({"savedQueries": {"Patient": {"drop": null}}}), "acme");
        let merged = apply_merge_patch(stored, &patch);
        assert_eq!(
            merged["byTenant"]["acme"]["savedQueries"]["Patient"],
            json!({"keep": {}})
        );
        assert_eq!(merged["theme"], "dark");
    }

    /// Global members of a patch stay at the top level; scoped ones are wrapped.
    #[test]
    fn scoped_merge_patch_splits_globals_from_scoped_members() {
        assert_eq!(
            scope_merge_patch(json!({"theme": "light", "savedQueries": {"P": {}}}), "acme"),
            json!({"theme": "light", "byTenant": {"acme": {"savedQueries": {"P": {}}}}})
        );
        // A purely-global patch produces no subtree at all.
        assert_eq!(
            scope_merge_patch(json!({"theme": null}), "acme"),
            json!({"theme": null})
        );
    }

    /// A purge removes exactly one subtree and leaves globals and siblings alone.
    #[test]
    fn purge_removes_only_the_named_tenants_subtree() {
        let mut doc = json!({
            "theme": "dark",
            "byTenant": {"acme": {"savedQueries": {"P": {}}}, "beta": {"savedQueries": {"Q": {}}}}
        });
        assert!(purge_tenant_subtree(&mut doc, "acme"));
        assert_eq!(
            doc,
            json!({"theme": "dark", "byTenant": {"beta": {"savedQueries": {"Q": {}}}}})
        );
    }

    /// Emptying the last subtree drops the now-pointless reserved key.
    #[test]
    fn purge_drops_an_emptied_by_tenant_map() {
        let mut doc = json!({"theme": "dark", "byTenant": {"acme": {"savedQueries": {}}}});
        assert!(purge_tenant_subtree(&mut doc, "acme"));
        assert_eq!(doc, json!({"theme": "dark"}));
    }

    /// Purging a tenant with nothing stored changes nothing, so no version is
    /// burned and no client ETag is needlessly invalidated.
    #[test]
    fn purge_of_an_absent_tenant_is_a_no_op() {
        let mut doc = json!({"theme": "dark", "byTenant": {"acme": {"savedQueries": {}}}});
        let before = doc.clone();
        assert!(!purge_tenant_subtree(&mut doc, "beta"));
        assert_eq!(doc, before);
    }

    /// An attributed legacy document is swept only by *its own* tenant's purge.
    #[test]
    fn purge_sweeps_attributed_legacy_content_only_for_its_owner() {
        let legacy = json!({
            "theme": "dark",
            "tenantId": "acme",
            "savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}
        });

        let mut other = legacy.clone();
        assert!(!purge_tenant_subtree(&mut other, "beta"));
        assert!(
            other.get("savedQueries").is_some(),
            "beta must not erase acme's content"
        );

        let mut owner = legacy;
        assert!(purge_tenant_subtree(&mut owner, "acme"));
        assert_eq!(owner, json!({"theme": "dark", "tenantId": "acme"}));
    }

    /// The decision recorded in the module docs: unattributed legacy PHI is swept
    /// by any tenant purge. Leaving it is the erasure gap #313 exists to close.
    #[test]
    fn purge_sweeps_unattributed_legacy_content() {
        let mut doc = json!({
            "theme": "dark",
            "savedQueries": {"Patient": {"q1": {"query": "name=smith&birthdate=1970-01-01"}}},
            "recentSearches": [{"query": "/Patient?name=smith"}]
        });
        assert!(purge_tenant_subtree(&mut doc, "any-tenant"));
        assert_eq!(doc, json!({"theme": "dark"}));
        assert!(!serde_json::to_string(&doc).unwrap().contains("smith"));
    }

    /// A purge never removes a user-global preference, however aggressive the
    /// legacy sweep is.
    #[test]
    fn purge_never_removes_a_global_preference() {
        let mut doc = json!({
            "theme": "dark", "nav": "collapsed", "fhirVersion": "R4", "tenantId": "acme",
            "savedQueries": {"P": {}}
        });
        purge_tenant_subtree(&mut doc, "acme");
        assert_eq!(
            doc,
            json!({"theme": "dark", "nav": "collapsed", "fhirVersion": "R4", "tenantId": "acme"})
        );
    }

    /// Tenant ids may contain `.` and `/` (`admin_tenants::validate_tenant_id`
    /// permits both), which is why every backend edits a parsed `Value` rather
    /// than a dotted JSON path — a `$unset: {"byTenant.a.b": 1}` would target the
    /// wrong node for a tenant literally named `a.b`.
    #[test]
    fn tenant_ids_containing_dots_and_slashes_round_trip() {
        for tenant in ["a.b", "org/unit", "a.b/c"] {
            let mut doc = stored_for_put(&json!({}), json!({"savedQueries": {"P": {}}}), tenant);
            assert_eq!(
                project_for_tenant(&doc, tenant)["savedQueries"],
                json!({"P": {}})
            );
            assert!(
                purge_tenant_subtree(&mut doc, tenant),
                "{tenant} must be purgeable"
            );
            assert_eq!(doc, json!({}));
        }
    }

    /// A sibling whose id is a prefix of the purged one is untouched — the case a
    /// naive string-prefix implementation would get wrong.
    #[test]
    fn purge_does_not_match_a_prefix_sibling() {
        let mut doc =
            json!({"byTenant": {"acme": {"savedQueries": {}}, "acme-2": {"savedQueries": {}}}});
        assert!(purge_tenant_subtree(&mut doc, "acme"));
        assert!(doc["byTenant"].get("acme-2").is_some());
    }

    /// The transforms must not panic on a corrupt or hand-edited row.
    #[test]
    fn transforms_tolerate_a_non_object_document() {
        assert_eq!(project_for_tenant(&json!("corrupt"), "acme"), json!({}));
        let mut doc = json!(["corrupt"]);
        assert!(!purge_tenant_subtree(&mut doc, "acme"));
        assert!(!normalize_legacy(&mut json!(null), "acme"));
    }

    /// A `byTenant` value of the wrong shape is inert rather than fatal.
    #[test]
    fn transforms_tolerate_a_malformed_by_tenant_value() {
        let stored = json!({"theme": "dark", "byTenant": "not-a-map"});
        assert_eq!(
            project_for_tenant(&stored, "acme"),
            json!({"theme": "dark"})
        );
        let mut doc = stored;
        assert!(!purge_tenant_subtree(&mut doc, "acme"));
    }

    /// End to end: write under two tenants, then offboard one.
    #[test]
    fn offboarding_one_tenant_leaves_the_other_intact() {
        let mut stored = json!({"theme": "dark"});
        stored = stored_for_put(
            &stored,
            json!({"theme": "dark", "savedQueries": {"P": {"a": {}}}}),
            "acme",
        );
        stored = stored_for_put(
            &stored,
            json!({"theme": "dark", "savedQueries": {"P": {"b": {}}}}),
            "beta",
        );

        assert!(purge_tenant_subtree(&mut stored, "acme"));
        assert_eq!(
            project_for_tenant(&stored, "acme"),
            json!({"theme": "dark"})
        );
        assert_eq!(
            project_for_tenant(&stored, "beta"),
            json!({"theme": "dark", "savedQueries": {"P": {"b": {}}}})
        );
    }
}
