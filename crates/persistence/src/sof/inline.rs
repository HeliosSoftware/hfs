//! Ephemeral SQLite backend for inline `resource:` parameters on
//! `$viewdefinition-run`.
//!
//! When a caller passes `resource:` values inline in the request body, those
//! resources are not in the persistent backing store. Rather than maintain a
//! separate in-memory FHIRPath evaluator just to handle them, we materialise
//! them into a fresh `:memory:` SQLite database, register the same in-DB SOF
//! runner against it, and route the request through the standard pipeline.
//!
//! The ephemeral backend is owned by an `Arc` that the runner clones into
//! the streaming task; once the response stream finishes, both drop and the
//! database is freed.

#![cfg(feature = "sqlite")]

use std::sync::Arc;

use serde_json::Value;

use crate::backends::sqlite::SqliteBackend;
use crate::core::ResourceStorage;
use crate::core::sof_runner::{SofError, SofRunner};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

/// Tenant id used for the ephemeral inline backend. Real tenant isolation
/// is irrelevant here — the database lives only for the duration of the
/// request — but a non-empty id keeps schemas / queries uniform.
const INLINE_TENANT_ID: &str = "inline";

/// Materialises `resources` into a fresh in-memory SQLite backend and returns
/// the in-DB SOF runner pointed at it. The backend's lifetime is tied to the
/// runner via an internal `Arc`, so dropping the returned runner (and any
/// in-flight streams it produced) is what frees the database.
///
/// The returned `TenantContext` must be used for the subsequent `run_view`
/// call so it scopes against the same tenant id the resources were inserted
/// under.
pub async fn build_inline_runner(
    resources: Vec<Value>,
    fhir_version: helios_fhir::FhirVersion,
) -> Result<(Arc<dyn SofRunner>, TenantContext), SofError> {
    let backend = SqliteBackend::in_memory()
        .map_err(|e| SofError::Storage(format!("failed to create inline SQLite backend: {e}")))?;
    backend
        .init_schema()
        .map_err(|e| SofError::Storage(format!("failed to init inline SQLite schema: {e}")))?;

    let tenant = TenantContext::new(
        TenantId::new(INLINE_TENANT_ID),
        TenantPermissions::full_access(),
    );

    for resource in resources {
        let resource_type = resource
            .get("resourceType")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SofError::InvalidViewDefinition(
                    "inline resource is missing 'resourceType'".to_string(),
                )
            })?
            .to_string();
        backend
            .create(&tenant, &resource_type, resource, fhir_version)
            .await
            .map_err(|e| SofError::Storage(format!("failed to seed inline resource: {e}")))?;
    }

    let runner = backend.sof_runner().ok_or_else(|| {
        SofError::Backend("inline SQLite backend did not return a SOF runner".to_string())
    })?;
    Ok((runner, tenant))
}
