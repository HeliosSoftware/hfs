//! Bulk export stub implementations for the MongoDB backend.
//!
//! MongoDB does not yet support bulk export as a primary resource store.
//! These stub impls satisfy the [`ExportDataProvider`] /
//! [`PatientExportProvider`] / [`GroupExportProvider`] trait hierarchy so a
//! MongoDB-primary deployment compiles; every method returns
//! `UnsupportedCapability`.

use async_trait::async_trait;

use crate::core::bulk_export::{
    ExportDataProvider, ExportRequest, GroupExportProvider, NdjsonBatch, PatientExportProvider,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::MongoBackend;

fn mongo_export_unsupported() -> StorageError {
    StorageError::Backend(BackendError::UnsupportedCapability {
        backend_name: "mongodb".to_string(),
        capability: "bulk-export".to_string(),
    })
}

#[async_trait]
impl ExportDataProvider for MongoBackend {
    async fn list_export_types(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        Err(mongo_export_unsupported())
    }

    async fn count_export_resources(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _resource_type: &str,
    ) -> StorageResult<u64> {
        Err(mongo_export_unsupported())
    }

    async fn fetch_export_batch(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _resource_type: &str,
        _cursor: Option<&str>,
        _batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        Err(mongo_export_unsupported())
    }
}

#[async_trait]
impl PatientExportProvider for MongoBackend {
    async fn list_patient_ids(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _cursor: Option<&str>,
        _batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        Err(mongo_export_unsupported())
    }

    async fn fetch_patient_compartment_batch(
        &self,
        _tenant: &TenantContext,
        _request: &ExportRequest,
        _resource_type: &str,
        _patient_ids: &[String],
        _cursor: Option<&str>,
        _batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        Err(mongo_export_unsupported())
    }
}

#[async_trait]
impl GroupExportProvider for MongoBackend {
    async fn get_group_members(
        &self,
        _tenant: &TenantContext,
        _group_id: &str,
    ) -> StorageResult<Vec<String>> {
        Err(mongo_export_unsupported())
    }

    async fn resolve_group_patient_ids(
        &self,
        _tenant: &TenantContext,
        _group_id: &str,
    ) -> StorageResult<Vec<String>> {
        Err(mongo_export_unsupported())
    }
}
