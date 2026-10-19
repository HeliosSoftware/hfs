//! Raw resources and terminology rows share one persistence transaction.

use async_trait::async_trait;
#[cfg(feature = "sqlite")]
use helios_persistence::core::TransactionProvider;
use helios_persistence::core::{Transaction, TransactionOptions};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::{SYSTEM_TENANT, TenantContext};
use serde_json::Value;

use crate::backends::resource_identity::ResourceKind;
use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::TerminologyBackend;

#[cfg(feature = "postgres")]
use helios_persistence::backends::postgres::PostgresTransaction;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteTransaction;

/// Only HTS-owned tables extend the persistence transaction; resource versions,
/// history, deletion and search indexing remain persistence's responsibility.
#[async_trait]
pub(super) trait CrudTransaction: Transaction {
    /// Coordinate the old and requested normalized identities before any raw
    /// resource read or write. The lock set remains fixed through commit.
    async fn prepare(
        &mut self,
        kind: ResourceKind,
        id: &str,
        replacement: Option<&Value>,
    ) -> Result<(), HtsError>;
    async fn normalized_resource(
        &mut self,
        kind: ResourceKind,
        id: &str,
    ) -> Result<Option<Value>, HtsError>;
    async fn normalize(
        &mut self,
        kind: ResourceKind,
        content: Value,
        replace: bool,
    ) -> Result<(), HtsError>;
    async fn remove_normalized(&mut self, kind: ResourceKind, id: &str) -> Result<(), HtsError>;
}

pub(super) fn storage_error(error: helios_persistence::StorageError) -> HtsError {
    match error {
        StorageError::Resource(ResourceError::AlreadyExists { .. }) => {
            HtsError::InvalidRequest(error.to_string())
        }
        StorageError::Resource(ResourceError::NotFound { .. } | ResourceError::Gone { .. }) => {
            HtsError::NotFound(error.to_string())
        }
        _ => HtsError::StorageError(error.to_string()),
    }
}

pub(super) async fn begin<B: TerminologyBackend>(
    state: &AppState<B>,
    kind: ResourceKind,
    id: &str,
) -> Result<Box<dyn CrudTransaction>, HtsError> {
    let tenant = TenantContext::system();
    #[cfg(feature = "sqlite")]
    if let Some(store) = &state.resource_store {
        return store
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .map(|tx| Box::new(tx) as Box<dyn CrudTransaction>)
            .map_err(storage_error);
    }
    #[cfg(feature = "postgres")]
    if let Some(store) = &state.resource_store_pg {
        return store
            .begin_planned_transaction(
                &tenant,
                TransactionOptions::default(),
                vec![(kind.resource_type().to_owned(), id.to_owned())],
            )
            .await
            .map(|tx| Box::new(tx) as Box<dyn CrudTransaction>)
            .map_err(storage_error);
    }
    #[cfg(not(feature = "postgres"))]
    let _ = (kind, id);
    Err(HtsError::Internal("Resource store not initialized".into()))
}

#[cfg(feature = "sqlite")]
#[async_trait]
impl CrudTransaction for SqliteTransaction {
    async fn prepare(
        &mut self,
        _kind: ResourceKind,
        _id: &str,
        _replacement: Option<&Value>,
    ) -> Result<(), HtsError> {
        // BEGIN IMMEDIATE already serializes imports and resource mutations.
        Ok(())
    }

    async fn normalized_resource(
        &mut self,
        kind: ResourceKind,
        id: &str,
    ) -> Result<Option<Value>, HtsError> {
        let id = id.to_owned();
        self.run_on_connection(move |conn| {
            let deleted = conn
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM resources
                         WHERE tenant_id = ?1 AND resource_type = ?2
                           AND id = ?3 AND is_deleted = 1
                     )",
                    rusqlite::params![SYSTEM_TENANT, kind.resource_type(), id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| HtsError::StorageError(error.to_string()));
            Ok(match deleted {
                Ok(true) => Ok(None),
                Ok(false) => crate::backends::sqlite::resource_identity::read(conn, kind, &id),
                Err(error) => Err(error),
            })
        })
        .await
        .map_err(storage_error)?
    }

    async fn normalize(
        &mut self,
        kind: ResourceKind,
        content: Value,
        replace: bool,
    ) -> Result<(), HtsError> {
        self.run_on_connection(move |conn| {
            let id = if replace {
                content.get("id").and_then(Value::as_str)
            } else {
                None
            };
            Ok(crate::import::fhir_bundle::write_resource_in_transaction(
                conn,
                kind.resource_type(),
                &content,
                id,
            ))
        })
        .await
        .map_err(storage_error)?
    }

    async fn remove_normalized(&mut self, kind: ResourceKind, id: &str) -> Result<(), HtsError> {
        let id = id.to_owned();
        self.run_on_connection(move |conn| {
            Ok(crate::import::fhir_bundle::delete_resource_in_transaction(
                conn, kind, &id,
            ))
        })
        .await
        .map_err(storage_error)?
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl CrudTransaction for PostgresTransaction {
    async fn prepare(
        &mut self,
        kind: ResourceKind,
        id: &str,
        replacement: Option<&Value>,
    ) -> Result<(), HtsError> {
        let client = self.transaction_client().await.map_err(storage_error)?;
        crate::backends::postgres::resource_identity::lock_mutation(client, kind, id, replacement)
            .await
    }

    async fn normalized_resource(
        &mut self,
        kind: ResourceKind,
        id: &str,
    ) -> Result<Option<Value>, HtsError> {
        let client = self.transaction_client().await.map_err(storage_error)?;
        let deleted: bool = client
            .query_one(
                "SELECT EXISTS(
                     SELECT 1 FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2
                       AND id = $3 AND is_deleted
                 )",
                &[&SYSTEM_TENANT, &kind.resource_type(), &id],
            )
            .await
            .map_err(|error| HtsError::StorageError(error.to_string()))?
            .get(0);
        if deleted {
            return Ok(None);
        }
        crate::backends::postgres::resource_identity::read(client, kind, id).await
    }

    async fn normalize(
        &mut self,
        kind: ResourceKind,
        content: Value,
        replace: bool,
    ) -> Result<(), HtsError> {
        let client = self.transaction_client().await.map_err(storage_error)?;
        let id = if replace {
            content.get("id").and_then(Value::as_str)
        } else {
            None
        };
        crate::backends::postgres::write_resource_in_transaction(
            client,
            kind.resource_type(),
            &content,
            id,
        )
        .await
    }

    async fn remove_normalized(&mut self, kind: ResourceKind, id: &str) -> Result<(), HtsError> {
        let client = self.transaction_client().await.map_err(storage_error)?;
        crate::backends::postgres::delete_resource_in_transaction(client, kind, id).await
    }
}
