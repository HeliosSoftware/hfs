//! S3 implementation of the [`SofTerminologyStore`] (#2185).
//!
//! S3 has no transactions, so this is the generation-and-registry algorithm
//! of [`sof_terminology_cas`](crate::backends::sof_terminology_cas) over
//! objects in the tenant's keyspace:
//!
//! - `bulk/sof-terminology/registry.json`: the tenant's registry, swapped
//!   with a conditional `PutObject` pinned to the ETag that was read
//!   (`If-Match`, or `If-None-Match: *` for the first write) — the same
//!   compare-and-swap as `user_settings`. The body carries a monotonic
//!   `version`, so no two successive registries are byte-identical and the
//!   ETag cannot repeat (no ABA).
//! - `bulk/sof-terminology/generations/<generation>.json`: one object per
//!   generation holding its rows. A single `PutObject` is atomic, so a
//!   generation is never visible partially written.
//!
//! [`SofTerminologyStore`]: crate::core::sof_terminology::SofTerminologyStore

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::backend::S3Backend;
use super::client::S3ClientError;
use crate::backends::sof_terminology_cas::{
    GenerationStore, TerminologyRegistry, impl_sof_terminology_store,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// The stored registry object.
#[derive(Debug, Serialize, Deserialize)]
struct RegistryObject {
    version: u64,
    registry: TerminologyRegistry,
}

/// The stored generation object.
#[derive(Debug, Serialize, Deserialize)]
struct GenerationObject {
    rows: Vec<Value>,
}

/// The registry's swap token: the ETag and body version read, or `None` when
/// no registry object exists yet.
pub(crate) type RegistryToken = Option<(String, u64)>;

impl S3Backend {
    /// Deletes every terminology object of the tenant (purge). The registry
    /// goes first, so a failure part way leaves only unreachable generations.
    pub(super) async fn purge_tenant_sof_terminology(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;
        let registry = location.keyspace.sof_terminology_registry_key();
        self.client
            .delete_object(&location.bucket, &registry)
            .await
            .map_err(|e| self.map_client_error(e))?;
        let prefix = location.keyspace.sof_terminology_prefix();
        for item in self.list_objects_all(&location.bucket, &prefix).await? {
            self.client
                .delete_object(&location.bucket, &item.key)
                .await
                .map_err(|e| self.map_client_error(e))?;
        }
        Ok(())
    }
}

#[async_trait]
impl GenerationStore for S3Backend {
    type Token = RegistryToken;

    async fn read_registry(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<(TerminologyRegistry, RegistryToken)> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.sof_terminology_registry_key();
        let Some((stored, metadata)) = self
            .get_json_object::<RegistryObject>(&location.bucket, &key)
            .await?
        else {
            return Ok((TerminologyRegistry::default(), None));
        };
        // A swap must be pinned to the generation read; never fall back to an
        // unconditional write.
        let etag = metadata.etag.ok_or_else(|| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "s3".to_string(),
                message: format!("S3 returned no ETag for terminology registry {key}"),
                source: None,
            })
        })?;
        Ok((stored.registry, Some((etag, stored.version))))
    }

    async fn swap_registry(
        &self,
        tenant: &TenantContext,
        expected: &RegistryToken,
        registry: &TerminologyRegistry,
    ) -> StorageResult<bool> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.sof_terminology_registry_key();
        let version = expected.as_ref().map_or(0, |(_, v)| *v) + 1;
        let payload = self.serialize_json(&RegistryObject {
            version,
            registry: registry.clone(),
        })?;
        let (if_match, if_none_match) = match expected {
            Some((etag, _)) => (Some(etag.as_str()), None),
            None => (None, Some("*")),
        };
        // The client directly, not `put_json_object`: that folds
        // `PreconditionFailed` into a query error, and the lost race must stay
        // distinguishable (see `user_settings::write_settings`).
        match self
            .client
            .put_object(
                &location.bucket,
                &key,
                payload,
                Some("application/json"),
                if_match,
                if_none_match,
            )
            .await
        {
            Ok(_) => Ok(true),
            // Lost races, none of which wrote anything: a stale ETag (412), a
            // conditional write in flight on the same key (409, #2047), and
            // some S3-compatible stores' 404 for `If-Match` against a deleted
            // object. A 404 on a create is a missing bucket, a genuine error.
            Err(S3ClientError::PreconditionFailed | S3ClientError::ConditionalConflict(_)) => {
                Ok(false)
            }
            Err(S3ClientError::NotFound) if if_match.is_some() => Ok(false),
            Err(e) => Err(self.map_client_error(e)),
        }
    }

    async fn write_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
        rows: &[Value],
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.sof_terminology_generation_key(generation);
        let payload = self.serialize_json(&GenerationObject {
            rows: rows.to_vec(),
        })?;
        self.put_json_object(&location.bucket, &key, &payload, None, None)
            .await?;
        Ok(())
    }

    async fn read_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
    ) -> StorageResult<Option<Vec<Value>>> {
        let location = self.tenant_location(tenant)?;
        let key = location.keyspace.sof_terminology_generation_key(generation);
        Ok(self
            .get_json_object::<GenerationObject>(&location.bucket, &key)
            .await?
            .map(|(object, _)| object.rows))
    }

    async fn delete_generations(
        &self,
        tenant: &TenantContext,
        generations: &[String],
    ) -> StorageResult<()> {
        let location = self.tenant_location(tenant)?;
        for generation in generations {
            let key = location.keyspace.sof_terminology_generation_key(generation);
            self.client
                .delete_object(&location.bucket, &key)
                .await
                .map_err(|e| self.map_client_error(e))?;
        }
        Ok(())
    }
}

impl_sof_terminology_store!(S3Backend);
