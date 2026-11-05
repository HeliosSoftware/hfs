//! S3 implementation of the provider-side [`BulkProviderStore`].
//!
//! One JSON object per `(tenant, id)` under the tenant's
//! `bulk/provider-submissions/` prefix, holding the opaque submission document
//! plus a monotonic `version`. Writes are compare-and-swapped against the S3
//! ETag exactly as the per-user settings store does (`If-Match` on update,
//! `If-None-Match: *` on create) — never an unconditional `PutObject` — with
//! the client-facing optimistic token being the `version` in the body (#772).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::core::bulk_provider::{BulkProviderStore, StoredProviderSubmission};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::backend::{S3Backend, TenantLocation};
use super::client::S3ClientError;

/// Attempts at a compare-and-swap that lost its race (counts total attempts).
/// Contention on one submission is the 5s status poller racing a user action,
/// so the bound is small; exhausting it surfaces as a concurrency error.
const CAS_ATTEMPTS: usize = 8;

/// The stored form: the opaque document plus the metadata to reconstruct a
/// [`StoredProviderSubmission`]. The id is persisted verbatim as a tripwire
/// against key-derivation regressions, mirroring the settings store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderObject {
    id: String,
    document: Value,
    version: i64,
    updated_at: DateTime<Utc>,
}

/// Derives the object name for a submission id: lowercase hex SHA-256, for the
/// same reasons the settings store hashes its user keys — a user-pinned id is
/// unconstrained input, and hashing keeps it injective, path-safe, and
/// fixed-length.
fn provider_object_id(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "s3".to_string(),
        message,
        source: None,
    })
}

fn lock_failure(id: &str, expected: i64, actual: Option<i64>) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "BulkProviderSubmission".to_string(),
        id: id.to_string(),
        expected_etag: format!("W/\"{expected}\""),
        actual_etag: actual.map(|v| format!("W/\"{v}\"")),
    })
}

impl S3Backend {
    fn provider_key(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<(TenantLocation, String)> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.bulk_provider_key(&provider_object_id(id));
        Ok((location, key))
    }

    async fn load_provider_object(
        &self,
        bucket: &str,
        key: &str,
        id: &str,
    ) -> StorageResult<Option<(ProviderObject, Option<String>)>> {
        let Some((stored, metadata)) = self
            .get_json_object::<ProviderObject>(bucket, key)
            .await
            .map_err(|e| backend_err(format!("read provider submission: {e:?}")))?
        else {
            return Ok(None);
        };
        if stored.id != id {
            return Err(backend_err(format!(
                "provider submission object {key} belongs to a different submission than requested"
            )));
        }
        Ok(Some((stored, metadata.etag)))
    }
}

#[async_trait]
impl BulkProviderStore for S3Backend {
    async fn list_provider_submissions(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<StoredProviderSubmission>> {
        let location = self.tenant_location(tenant)?;
        let prefix = location.keyspace.bulk_provider_prefix();
        let mut out = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let page = self
                .client
                .list_objects(&location.bucket, &prefix, continuation.as_deref(), None)
                .await
                .map_err(|e| backend_err(format!("list provider submissions: {e:?}")))?;
            for item in &page.items {
                if let Some((stored, _etag)) = self
                    .get_json_object::<ProviderObject>(&location.bucket, &item.key)
                    .await
                    .map_err(|e| backend_err(format!("read provider submission: {e:?}")))?
                {
                    out.push(StoredProviderSubmission {
                        id: stored.id,
                        document: stored.document,
                        version: stored.version,
                        updated_at: stored.updated_at,
                    });
                }
            }
            match page.next_continuation_token {
                Some(token) => continuation = Some(token),
                None => break,
            }
        }
        Ok(out)
    }

    async fn get_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<Option<StoredProviderSubmission>> {
        let (location, key) = self.provider_key(tenant, id)?;
        Ok(self
            .load_provider_object(&location.bucket, &key, id)
            .await?
            .map(|(stored, _)| StoredProviderSubmission {
                id: stored.id,
                document: stored.document,
                version: stored.version,
                updated_at: stored.updated_at,
            }))
    }

    async fn put_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredProviderSubmission> {
        let (location, key) = self.provider_key(tenant, id)?;
        for _attempt in 0..CAS_ATTEMPTS {
            let existing = self
                .load_provider_object(&location.bucket, &key, id)
                .await?;
            let current_version = existing.as_ref().map(|(s, _)| s.version).unwrap_or(0);
            if let Some(expected) = if_match_version
                && expected != current_version
            {
                return Err(lock_failure(id, expected, Some(current_version)));
            }

            let now = Utc::now();
            let new_object = ProviderObject {
                id: id.to_string(),
                document: document.clone(),
                version: current_version + 1,
                updated_at: now,
            };
            let payload = self.serialize_json(&new_object)?;
            let etag = existing.as_ref().and_then(|(_, etag)| etag.clone());
            let (if_match, if_none_match) = match etag.as_deref() {
                Some(etag) => (Some(etag), None),
                None => (None, Some("*")),
            };

            let result = self
                .client
                .put_object(
                    &location.bucket,
                    &key,
                    payload,
                    Some("application/json"),
                    if_match,
                    if_none_match,
                )
                .await;
            let err = match result {
                Ok(_) => {
                    return Ok(StoredProviderSubmission {
                        id: id.to_string(),
                        document: new_object.document,
                        version: new_object.version,
                        updated_at: now,
                    });
                }
                Err(err) => err,
            };
            let lost_race = matches!(err, S3ClientError::PreconditionFailed)
                || (matches!(err, S3ClientError::NotFound) && if_match.is_some());
            if !lost_race {
                return Err(backend_err(format!("write provider submission: {err:?}")));
            }
            // A version-conditioned write must not retry: the precondition the
            // caller asked for no longer holds.
            if let Some(expected) = if_match_version {
                let actual = self
                    .load_provider_object(&location.bucket, &key, id)
                    .await?
                    .map(|(s, _)| s.version);
                return Err(lock_failure(id, expected, actual));
            }
        }
        Err(lock_failure(id, 0, None))
    }

    async fn delete_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<bool> {
        let (location, key) = self.provider_key(tenant, id)?;
        let existed = self
            .load_provider_object(&location.bucket, &key, id)
            .await?
            .is_some();
        self.client
            .delete_object(&location.bucket, &key)
            .await
            .map_err(|e| backend_err(format!("delete provider submission: {e:?}")))?;
        Ok(existed)
    }
}
