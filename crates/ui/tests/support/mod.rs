//! Shared test doubles for `helios-ui`'s Rust integration tests.
//!
//! Each file directly under `tests/` is auto-discovered by Cargo as its own
//! integration-test binary/crate; a `mod.rs` inside a subdirectory is the
//! standard way to share code between them without Cargo mistaking it for a
//! test target of its own (it has to be pulled in explicitly, with
//! `mod support;`, by whichever test files use it).
//!
//! This module has nothing in it yet beyond [`InMemorySettingsStore`]; add
//! further shared fixtures here rather than starting a second ad hoc support
//! module.

use async_trait::async_trait;
use chrono::Utc;
use helios_persistence::{
    StorageResult,
    core::{SettingsStore, StoredUserSettings, apply_merge_patch, purge_tenant_subtree},
    error::{ConcurrencyError, StorageError},
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// An in-memory [`SettingsStore`] double: a `Mutex<HashMap<user_key,
/// StoredUserSettings>>`, with `patch_settings` applying
/// [`apply_merge_patch`] under the lock — the same read-modify-write every
/// real backend performs atomically, just without a database underneath.
///
/// Unlike `router_http.rs`'s `NoSettingsAccess` (which panics on every call,
/// for asserting a route never touches settings at all), this double actually
/// works, so a test can mount the UI router with `Some(Arc::new(store))`,
/// exercise real page/handler behavior that reads or writes
/// `/_user/settings`-shaped state, and assert on what landed.
#[derive(Default)]
pub struct InMemorySettingsStore {
    documents: Mutex<HashMap<String, StoredUserSettings>>,
    /// Counts `get_settings` calls, so a test can assert the "one settings
    /// read per request" cost model `resolve_prefs` promises.
    reads: AtomicUsize,
}

impl InMemorySettingsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of `get_settings` calls observed so far.
    pub fn get_settings_calls(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    /// Direct peek at a stored document, bypassing ETag/version ceremony —
    /// for asserting the raw stored shape (e.g. under `byTenant.<tenant>`).
    /// Returns `None` when the user has never stored anything.
    pub fn peek(&self, user_key: &str) -> Option<Value> {
        self.documents
            .lock()
            .unwrap()
            .get(user_key)
            .map(|stored| stored.document.clone())
    }
}

#[async_trait]
impl SettingsStore for InMemorySettingsStore {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.documents.lock().unwrap().get(user_key).cloned())
    }

    async fn put_settings(
        &self,
        user_key: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        let mut documents = self.documents.lock().unwrap();
        let current_version = documents.get(user_key).map(|s| s.version).unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(user_key, expected, current_version));
        }
        let stored = StoredUserSettings {
            user_key: user_key.to_string(),
            document,
            version: current_version + 1,
            updated_at: Utc::now(),
        };
        documents.insert(user_key.to_string(), stored.clone());
        Ok(stored)
    }

    async fn patch_settings(
        &self,
        user_key: &str,
        merge_patch: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        let mut documents = self.documents.lock().unwrap();
        let current = documents.get(user_key).cloned();
        let current_version = current.as_ref().map(|s| s.version).unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(user_key, expected, current_version));
        }
        let base = current
            .map(|s| s.document)
            .unwrap_or_else(|| Value::Object(Default::default()));
        let stored = StoredUserSettings {
            user_key: user_key.to_string(),
            document: apply_merge_patch(base, &merge_patch),
            version: current_version + 1,
            updated_at: Utc::now(),
        };
        documents.insert(user_key.to_string(), stored.clone());
        Ok(stored)
    }

    async fn delete_settings(&self, user_key: &str) -> StorageResult<bool> {
        Ok(self.documents.lock().unwrap().remove(user_key).is_some())
    }

    async fn purge_tenant_settings(&self, tenant_id: &str) -> StorageResult<u64> {
        let mut documents = self.documents.lock().unwrap();
        let mut changed = 0u64;
        for stored in documents.values_mut() {
            if purge_tenant_subtree(&mut stored.document, tenant_id) {
                stored.version += 1;
                changed += 1;
            }
        }
        Ok(changed)
    }
}

fn lock_failure(user_key: &str, expected: i64, actual: i64) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "UserSettings".to_string(),
        id: user_key.to_string(),
        expected_etag: format!("\"{expected}\""),
        actual_etag: Some(format!("\"{actual}\"")),
    })
}
