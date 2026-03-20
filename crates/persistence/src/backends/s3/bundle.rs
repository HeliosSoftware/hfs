//! Bundle processing (batch and transaction) for the S3 backend.
//!
//! Transactions are implemented with a best-effort compensation log: each
//! successful operation records a [`CompensationAction`] that is applied in
//! reverse if a later operation fails. S3 does not provide atomic multi-object
//! operations, so the rollback is advisory rather than strictly atomic.

use std::collections::HashMap;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::core::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
    ResourceStorage, VersionedStorage,
};
use crate::error::{BackendError, ResourceError, StorageError, TransactionError, ValidationError};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::backend::S3Backend;

/// An undo operation recorded for each successful step in a transaction.
///
/// Applied in reverse order if a later step fails, approximating an atomic
/// transaction rollback against an eventually-consistent object store.
#[derive(Debug, Clone)]
enum CompensationAction {
    /// Delete a newly-created resource to undo a POST entry.
    Delete { resource_type: String, id: String },
    /// Overwrite the current version with a captured snapshot to undo a PUT
    /// or DELETE entry.
    Restore { snapshot: StoredResource },
}

#[async_trait]
impl BundleProvider for S3Backend {
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> Result<BundleResult, TransactionError> {
        let mut entries = entries;
        let mut reference_map: HashMap<String, String> = HashMap::new();

        // Phase 1: Pre-assign IDs to POST entries and build the complete
        // reference map up-front so that *all* entries can be executed
        // concurrently. Without this, POST entries would need to run
        // sequentially because later entries may contain urn:uuid references
        // to resources created by earlier ones.
        for entry in entries.iter_mut() {
            if entry.method != BundleMethod::Post {
                continue;
            }
            if let Some(resource) = entry.resource.as_mut() {
                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                // Generate an ID if the resource doesn't already have one.
                let id = match resource.get("id").and_then(|v| v.as_str()) {
                    Some(existing) => existing.to_string(),
                    None => {
                        let generated = Uuid::new_v4().to_string();
                        if let Some(obj) = resource.as_object_mut() {
                            obj.insert("id".to_string(), Value::String(generated.clone()));
                        }
                        generated
                    }
                };

                if let Some(full_url) = &entry.full_url {
                    reference_map.insert(full_url.clone(), format!("{resource_type}/{id}"));
                }
            }
        }

        // Phase 2: Resolve all urn:uuid references across every entry.
        if !reference_map.is_empty() {
            for entry in entries.iter_mut() {
                if let Some(resource) = entry.resource.as_mut() {
                    resolve_bundle_references(resource, &reference_map);
                }
            }
        }

        // Phase 3: Execute ALL entries concurrently.
        let futs: Vec<_> = entries
            .iter()
            .map(|entry| self.execute_bundle_entry(tenant, entry))
            .collect();

        let outcomes = futures::future::join_all(futs).await;

        let mut results = Vec::with_capacity(entries.len());
        let mut compensations: Vec<CompensationAction> = Vec::new();

        for (idx, outcome) in outcomes.into_iter().enumerate() {
            match outcome {
                Ok((result, compensation)) => {
                    if result.status >= 400 {
                        // Also collect compensations from remaining successful
                        // outcomes that were already awaited by join_all.
                        let base = format!("entry failed with status {}", result.status);
                        let message = self
                            .rollback_compensations(tenant, compensations)
                            .await
                            .map(|_| base.clone())
                            .unwrap_or_else(|rollback_err| {
                                format!("{base}; rollback failed: {rollback_err}")
                            });
                        return Err(TransactionError::BundleError {
                            index: idx,
                            message,
                        });
                    }
                    if let Some(c) = compensation {
                        compensations.push(c);
                    }
                    results.push(result);
                }
                Err(err) => {
                    let base = format!("entry failed: {err}");
                    let message = self
                        .rollback_compensations(tenant, compensations)
                        .await
                        .map(|_| base.clone())
                        .unwrap_or_else(|rollback_err| {
                            format!("{base}; rollback failed: {rollback_err}")
                        });
                    return Err(TransactionError::BundleError {
                        index: idx,
                        message,
                    });
                }
            }
        }

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }

    async fn process_batch(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> crate::error::StorageResult<BundleResult> {
        let futs: Vec<_> = entries
            .iter()
            .map(|entry| self.process_batch_entry(tenant, entry))
            .collect();

        let results = futures::future::join_all(futs).await;

        Ok(BundleResult {
            bundle_type: BundleType::Batch,
            entries: results,
        })
    }
}

impl S3Backend {
    /// Executes a single batch entry and converts any error into a 5xx
    /// `BundleEntryResult` rather than propagating it, preserving best-effort
    /// batch semantics.
    async fn process_batch_entry(
        &self,
        tenant: &TenantContext,
        entry: &BundleEntry,
    ) -> BundleEntryResult {
        match self.execute_bundle_entry(tenant, entry).await {
            Ok((result, _)) => result,
            Err(err) => Self::bundle_error_result(&err),
        }
    }

    /// Executes a single bundle entry and returns the result together with an
    /// optional compensation action for rollback.
    async fn execute_bundle_entry(
        &self,
        tenant: &TenantContext,
        entry: &BundleEntry,
    ) -> crate::error::StorageResult<(BundleEntryResult, Option<CompensationAction>)> {
        match entry.method {
            BundleMethod::Get => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
                match self.read(tenant, &resource_type, &id).await {
                    Ok(Some(resource)) => Ok((BundleEntryResult::ok(resource), None)),
                    Ok(None) => Ok((
                        BundleEntryResult::error(
                            404,
                            json!({
                                "resourceType": "OperationOutcome",
                                "issue": [{"severity": "error", "code": "not-found"}]
                            }),
                        ),
                        None,
                    )),
                    Err(StorageError::Resource(ResourceError::Gone { .. })) => Ok((
                        BundleEntryResult::error(
                            410,
                            json!({
                                "resourceType": "OperationOutcome",
                                "issue": [{"severity": "error", "code": "deleted"}]
                            }),
                        ),
                        None,
                    )),
                    Err(err) => Err(err),
                }
            }
            BundleMethod::Post => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        StorageError::Validation(ValidationError::MissingRequiredField {
                            field: "resourceType".to_string(),
                        })
                    })?
                    .to_string();

                let created = self
                    .create(tenant, &resource_type, resource, FhirVersion::default())
                    .await?;

                Ok((
                    BundleEntryResult::created(created.clone()),
                    Some(CompensationAction::Delete {
                        resource_type: created.resource_type().to_string(),
                        id: created.id().to_string(),
                    }),
                ))
            }
            BundleMethod::Put => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

                let current = match self.read(tenant, &resource_type, &id).await {
                    Ok(value) => value,
                    Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
                    Err(err) => return Err(err),
                };

                if let Some(existing) = current {
                    let updated = if let Some(if_match) = entry.if_match.as_deref() {
                        self.update_with_match(tenant, &resource_type, &id, if_match, resource)
                            .await?
                    } else {
                        self.update(tenant, &existing, resource).await?
                    };

                    Ok((
                        BundleEntryResult::ok(updated),
                        Some(CompensationAction::Restore { snapshot: existing }),
                    ))
                } else {
                    let (stored, created) = self
                        .create_or_update(
                            tenant,
                            &resource_type,
                            &id,
                            resource,
                            FhirVersion::default(),
                        )
                        .await?;

                    let result = if created {
                        BundleEntryResult::created(stored.clone())
                    } else {
                        BundleEntryResult::ok(stored.clone())
                    };

                    let compensation = if created {
                        Some(CompensationAction::Delete {
                            resource_type: stored.resource_type().to_string(),
                            id: stored.id().to_string(),
                        })
                    } else {
                        None
                    };

                    Ok((result, compensation))
                }
            }
            BundleMethod::Delete => {
                let (resource_type, id) = self.parse_url(&entry.url)?;

                let snapshot = self.read(tenant, &resource_type, &id).await.ok().flatten();

                let delete_result = if let Some(if_match) = entry.if_match.as_deref() {
                    self.delete_with_match(tenant, &resource_type, &id, if_match)
                        .await
                } else {
                    self.delete(tenant, &resource_type, &id).await
                };

                match delete_result {
                    Ok(()) => Ok((
                        BundleEntryResult::deleted(),
                        snapshot.map(|s| CompensationAction::Restore { snapshot: s }),
                    )),
                    Err(StorageError::Resource(ResourceError::NotFound { .. }))
                    | Err(StorageError::Resource(ResourceError::Gone { .. })) => {
                        Ok((BundleEntryResult::deleted(), None))
                    }
                    Err(err) => Err(err),
                }
            }
            BundleMethod::Patch => Ok((
                BundleEntryResult::error(
                    501,
                    json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "not-supported",
                            "diagnostics": "PATCH is not supported by the S3 bundle backend"
                        }]
                    }),
                ),
                None,
            )),
        }
    }

    /// Applies compensation actions in reverse order to undo completed steps.
    ///
    /// Individual rollback failures are collected and returned as a joined
    /// error string rather than stopping the rollback mid-way.
    async fn rollback_compensations(
        &self,
        tenant: &TenantContext,
        compensations: Vec<CompensationAction>,
    ) -> Result<(), String> {
        let mut failures = Vec::new();

        for compensation in compensations.into_iter().rev() {
            if let Err(err) = self.apply_compensation(tenant, compensation).await {
                failures.push(err.to_string());
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    /// Applies a single compensation action.
    ///
    /// `NotFound` and `Gone` errors on delete compensations are treated as
    /// success since the intended post-rollback state is already achieved.
    async fn apply_compensation(
        &self,
        tenant: &TenantContext,
        compensation: CompensationAction,
    ) -> crate::error::StorageResult<()> {
        match compensation {
            CompensationAction::Delete { resource_type, id } => {
                match self.delete(tenant, &resource_type, &id).await {
                    Ok(())
                    | Err(StorageError::Resource(ResourceError::NotFound { .. }))
                    | Err(StorageError::Resource(ResourceError::Gone { .. })) => Ok(()),
                    Err(err) => Err(err),
                }
            }
            CompensationAction::Restore { snapshot } => {
                self.restore_resource_from_snapshot(tenant, &snapshot)
                    .await?;
                Ok(())
            }
        }
    }

    /// Converts a storage error into a bundle entry result with an appropriate
    /// HTTP status and a minimal OperationOutcome body.
    fn bundle_error_result(err: &StorageError) -> BundleEntryResult {
        BundleEntryResult::error(
            Self::storage_error_status(err),
            Self::operation_outcome(err),
        )
    }

    /// Maps a `StorageError` to an HTTP status code suitable for a bundle entry.
    fn storage_error_status(err: &StorageError) -> u16 {
        match err {
            StorageError::Validation(_) | StorageError::Search(_) => 400,
            StorageError::Tenant(_) => 403,
            StorageError::Resource(ResourceError::NotFound { .. }) => 404,
            StorageError::Resource(ResourceError::VersionNotFound { .. }) => 404,
            StorageError::Resource(ResourceError::Gone { .. }) => 410,
            StorageError::Resource(ResourceError::AlreadyExists { .. }) => 409,
            StorageError::Concurrency(_) => 409,
            StorageError::Backend(BackendError::UnsupportedCapability { .. }) => 501,
            StorageError::BulkExport(_) | StorageError::BulkSubmit(_) => 500,
            StorageError::Transaction(_) => 409,
            StorageError::Backend(_) => 500,
        }
    }

    /// Builds a minimal OperationOutcome `Value` from a `StorageError`.
    fn operation_outcome(err: &StorageError) -> Value {
        let code = match err {
            StorageError::Validation(_) => "invalid",
            StorageError::Tenant(_) => "forbidden",
            StorageError::Resource(ResourceError::NotFound { .. }) => "not-found",
            StorageError::Resource(ResourceError::VersionNotFound { .. }) => "not-found",
            StorageError::Resource(ResourceError::Gone { .. }) => "deleted",
            StorageError::Resource(ResourceError::AlreadyExists { .. }) => "conflict",
            StorageError::Concurrency(_) => "conflict",
            StorageError::Backend(BackendError::UnsupportedCapability { .. }) => "not-supported",
            _ => "exception",
        };

        json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": code,
                "diagnostics": err.to_string()
            }]
        })
    }

    /// Parses a bundle entry URL into `(resource_type, id)`.
    ///
    /// Both absolute URLs (`https://base/Patient/123`) and relative paths
    /// (`Patient/123`) are accepted. Returns a validation error if the URL
    /// does not contain at least two path segments.
    fn parse_url(&self, url: &str) -> crate::error::StorageResult<(String, String)> {
        let path = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .map(|s| s.find('/').map(|idx| &s[idx..]).unwrap_or(s))
            .unwrap_or(url);

        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.len() >= 2 {
            let len = parts.len();
            Ok((parts[len - 2].to_string(), parts[len - 1].to_string()))
        } else {
            Err(StorageError::Validation(
                ValidationError::InvalidReference {
                    reference: url.to_string(),
                    message: "URL must be in format ResourceType/id".to_string(),
                },
            ))
        }
    }
}

/// Recursively rewrites `urn:uuid:…` references in a resource JSON value
/// using the full URL map built from earlier POST entries in the bundle.
fn resolve_bundle_references(value: &mut Value, reference_map: &HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("reference") {
                if reference.starts_with("urn:uuid:") {
                    if let Some(resolved) = reference_map.get(reference) {
                        map.insert("reference".to_string(), Value::String(resolved.clone()));
                    }
                }
            }
            for value in map.values_mut() {
                resolve_bundle_references(value, reference_map);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_bundle_references(item, reference_map);
            }
        }
        _ => {}
    }
}
