//! Settings failures and CAS races against a real SQLite settings document.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use helios_persistence::{
    StorageResult,
    backends::sqlite::SqliteBackend,
    core::{SettingsStore, StoredUserSettings},
};
use serde_json::Value;

pub struct ExportSettings {
    pub backend: Arc<SqliteBackend>,
    pub fail_read: usize,
    pub race_patch: Mutex<Option<Value>>,
    pub fail_patches: AtomicBool,
    reads: AtomicUsize,
}

impl ExportSettings {
    pub fn new(backend: Arc<SqliteBackend>, fail_read: usize, race_patch: Option<Value>) -> Self {
        Self {
            backend,
            fail_read,
            race_patch: Mutex::new(race_patch),
            fail_patches: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl SettingsStore for ExportSettings {
    async fn get_settings(&self, user_key: &str) -> StorageResult<Option<StoredUserSettings>> {
        if self.reads.fetch_add(1, Ordering::SeqCst) + 1 == self.fail_read {
            return Err(helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::Internal {
                    backend_name: "test".to_string(),
                    message: "simulated settings read failure".to_string(),
                    source: None,
                },
            ));
        }
        self.backend.get_settings(user_key).await
    }

    async fn put_settings(
        &self,
        user_key: &str,
        document: Value,
        version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        self.backend.put_settings(user_key, document, version).await
    }

    async fn patch_settings(
        &self,
        user_key: &str,
        patch: Value,
        version: Option<i64>,
    ) -> StorageResult<StoredUserSettings> {
        if self.fail_patches.load(Ordering::SeqCst) {
            return Err(helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::Internal {
                    backend_name: "test".to_string(),
                    message: "simulated settings write failure".to_string(),
                    source: None,
                },
            ));
        }
        // A concurrent actor changes the actual member immediately before
        // the handler's CAS. The stale patch must lose, not overwrite it.
        let race = self.race_patch.lock().unwrap().take();
        if let Some(race) = race {
            self.backend.patch_settings(user_key, race, None).await?;
        }
        self.backend.patch_settings(user_key, patch, version).await
    }

    async fn delete_settings(&self, user_key: &str) -> StorageResult<bool> {
        self.backend.delete_settings(user_key).await
    }

    async fn purge_tenant_settings(&self, tenant: &str) -> StorageResult<u64> {
        self.backend.purge_tenant_settings(tenant).await
    }
}
