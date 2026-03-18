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
        let mut results: Vec<Option<BundleEntryResult>> = vec![None; entries.len()];
        let mut compensations: Vec<(usize, CompensationAction)> = Vec::new();
        let mut reference_map: HashMap<String, String> = HashMap::new();
        let mut entries = entries;

        // Phase 1: Process POST entries sequentially to build the reference map.
        // Later entries may use urn:uuid: references to resources created here.
        let mut concurrent_indices = Vec::new();

        for (idx, entry) in entries.iter_mut().enumerate() {
            if entry.method != BundleMethod::Post {
                concurrent_indices.push(idx);
                continue;
            }

            if let Some(resource) = entry.resource.as_mut() {
                resolve_bundle_references(resource, &reference_map);
            }

            let (result, compensation) = match self.execute_bundle_entry(tenant, entry).await {
                Ok(v) => v,
                Err(err) => {
                    let base = format!("entry failed: {err}");
                    let comps = compensations.into_iter().map(|(_, c)| c).collect();
                    let message = self
                        .rollback_compensations(tenant, comps)
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
            };

            if result.status >= 400 {
                let base = format!("entry failed with status {}", result.status);
                let comps = compensations.into_iter().map(|(_, c)| c).collect();
                let message = self
                    .rollback_compensations(tenant, comps)
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

            if let (Some(full_url), Some(location)) = (&entry.full_url, &result.location) {
                let resolved = location
                    .split("/_history")
                    .next()
                    .unwrap_or(location)
                    .to_string();
                reference_map.insert(full_url.clone(), resolved);
            }

            if let Some(c) = compensation {
                compensations.push((idx, c));
            }
            results[idx] = Some(result);
        }

        // Phase 2: Resolve urn:uuid references in remaining entries.
        if !reference_map.is_empty() {
            for &idx in &concurrent_indices {
                if let Some(resource) = entries[idx].resource.as_mut() {
                    resolve_bundle_references(resource, &reference_map);
                }
            }
        }

        // Phase 3: Process non-POST entries concurrently.
        if !concurrent_indices.is_empty() {
            let futs: Vec<_> = concurrent_indices
                .iter()
                .map(|&idx| self.execute_bundle_entry(tenant, &entries[idx]))
                .collect();

            let outcomes = futures::future::join_all(futs).await;

            for (&idx, outcome) in concurrent_indices.iter().zip(outcomes) {
                match outcome {
                    Ok((result, compensation)) => {
                        if result.status >= 400 {
                            let base = format!("entry failed with status {}", result.status);
                            let comps = compensations.into_iter().map(|(_, c)| c).collect();
                            let message = self
                                .rollback_compensations(tenant, comps)
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
                            compensations.push((idx, c));
                        }
                        results[idx] = Some(result);
                    }
                    Err(err) => {
                        let base = format!("entry failed: {err}");
                        let comps = compensations.into_iter().map(|(_, c)| c).collect();
                        let message = self
                            .rollback_compensations(tenant, comps)
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
        }

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results.into_iter().map(|r| r.unwrap()).collect(),
        })
    }

    async fn process_batch(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> crate::error::StorageResult<BundleResult> {
        let mut results = Vec::with_capacity(entries.len());

        for entry in &entries {
            results.push(self.process_batch_entry(tenant, entry).await);
        }

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
