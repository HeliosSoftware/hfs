//! The `patient`/`group` reference rule shared by every SQL-on-FHIR path
//! (#1864).
//!
//! The SQL-on-FHIR v2 spec answers a reference per operation, and every HFS
//! path checks it the same way, in the same order, before any runner starts:
//!
//! 1. **Bound.** At most [`MAX_PATIENT_GROUP_VALUES`] values
//!    ([`check_patient_group_values`]), so nothing below reads an unbounded
//!    list.
//! 2. **Shape.** Every value must be a relative `{Type}/{id}` of its
//!    parameter's type, or a bare id ([`helios_sof::params::compartment_ref_id`],
//!    the rule sof-server applies too). Anything else (an absolute URL,
//!    `urn:uuid:`, the other type, a versioned or contained reference, an empty
//!    id) is a 400 naming the parameter. Shapes are checked for every value
//!    before existence is checked for any.
//! 3. **Existence.** Stored-data runs ask storage with
//!    [`ResourceStorage::exists`], which treats a deleted resource as absent.
//!    The missing references are answered per operation: 400 with code
//!    `not-found` on the `$sql-run` paths ([`check_stored_targets`]), 404 with
//!    code `not-found` at the `$sql-export` kick-off
//!    ([`missing_stored_targets`], rendered by the kick-off). Inline-resource
//!    runs look the target up among the supplied resources instead (the
//!    in-process evaluator's absent-target error, 400 `not-found`).
//!
//! An existence check that fails for another reason is not a missing target:
//! it goes through [`RestError::from`]`(StorageError)`, so an unavailable
//! backend is a 503 with `Retry-After` and a storage timeout a 504, never a
//! blanket 500.
//!
//! **Time of check and time of use.** The check reads storage before the run
//! starts, and nothing locks the targets in between. A Patient or Group that is
//! deleted after the check and before the runner reads it gives the runner's
//! own answer: the SQL runners select nothing for it (an empty result), and the
//! in-process runner reports a typed not-found (an export job then fails with
//! 400 `not-found`).
//!
//! [`MAX_PATIENT_GROUP_VALUES`]: super::input_limits::MAX_PATIENT_GROUP_VALUES

use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::TenantContext;
use helios_sof::params::{ReferenceParameterError, check_patient_group_shapes, compartment_ref_id};

use super::input_limits::check_patient_group_values;
use crate::error::RestError;

/// Existence reads one request may have in flight at once.
const EXISTS_CONCURRENCY: usize = 8;

/// The 400 for a value no runner can act on, naming its parameter.
pub(crate) fn unusable_reference_error(error: ReferenceParameterError) -> RestError {
    RestError::InvalidParameter {
        param: error.param,
        message: error.message,
    }
}

/// Steps 1 and 2 of the rule: the bound, then the shape of every value.
/// Used by every path, the inline-resource one included.
pub(crate) fn check_values(patient: &[String], group: &[String]) -> Result<(), RestError> {
    check_patient_group_values(patient.len() + group.len())?;
    check_patient_group_shapes(patient, group).map_err(unusable_reference_error)
}

/// Step 3 for a stored-data run: the references, as the caller wrote them, that
/// name no live resource in `tenant` (absent, or deleted). Each distinct
/// `(type, id)` is read once. The values must have passed [`check_values`].
///
/// An existence read that fails is the error, mapped through
/// `RestError::from(StorageError)`.
pub(crate) async fn missing_stored_targets<S>(
    storage: &S,
    tenant: &TenantContext,
    patient: &[String],
    group: &[String],
) -> Result<Vec<String>, RestError>
where
    S: ResourceStorage + ?Sized,
{
    let mut seen = std::collections::HashSet::new();
    let mut targets: Vec<Target> = Vec::new();
    for (resource_type, refs) in [("Patient", patient), ("Group", group)] {
        for reference in refs {
            // `None` is unreachable after `check_values`; a value without a
            // usable shape names nothing that could exist.
            let id = compartment_ref_id(reference, resource_type).map(str::to_string);
            if id
                .as_ref()
                .is_none_or(|id| seen.insert((resource_type, id.clone())))
            {
                targets.push(Target {
                    resource_type,
                    id,
                    reference: reference.clone(),
                });
            }
        }
    }

    let mut missing = Vec::new();
    for batch in targets.chunks(EXISTS_CONCURRENCY) {
        let reads = batch
            .iter()
            .map(|target| missing_target(storage, tenant, target));
        missing.extend(
            futures::future::try_join_all(reads)
                .await?
                .into_iter()
                .flatten(),
        );
    }
    Ok(missing)
}

/// One distinct `patient`/`group` target of a request.
struct Target {
    resource_type: &'static str,
    /// `None` for a value without a usable shape.
    id: Option<String>,
    /// The value as the caller wrote it.
    reference: String,
}

/// `Some(reference)` when `target` names no live resource.
async fn missing_target<S>(
    storage: &S,
    tenant: &TenantContext,
    target: &Target,
) -> Result<Option<String>, RestError>
where
    S: ResourceStorage + ?Sized,
{
    let Some(id) = target.id.as_deref() else {
        return Ok(Some(target.reference.clone()));
    };
    let exists = storage
        .exists(tenant, target.resource_type, id)
        .await
        .map_err(RestError::from)?;
    Ok((!exists).then(|| target.reference.clone()))
}

/// The whole rule for a stored-data `$sql-run` (a stored or inline
/// ViewDefinition run against storage, and every dependency of a Library
/// subject): bound, shapes, then existence. A missing or deleted target is a
/// 400 with code `not-found` listing every such reference.
pub(crate) async fn check_stored_targets<S>(
    storage: &S,
    tenant: &TenantContext,
    patient: &[String],
    group: &[String],
) -> Result<(), RestError>
where
    S: ResourceStorage + ?Sized,
{
    check_values(patient, group)?;
    if patient.is_empty() && group.is_empty() {
        return Ok(());
    }
    let missing = missing_stored_targets(storage, tenant, patient, group).await?;
    if missing.is_empty() {
        return Ok(());
    }
    Err(RestError::ReferenceNotFound {
        message: missing_references_message(&missing),
    })
}

/// The client text listing missing `patient`/`group` references, shared by the
/// run paths (400) and the export kick-off (404).
pub(crate) fn missing_references_message(missing: &[String]) -> String {
    format!(
        "one or more patient/group references could not be resolved: {}",
        missing.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::response::IntoResponse;
    use helios_fhir::FhirVersion;
    use helios_persistence::error::{BackendError, ResourceError, StorageError, StorageResult};
    use helios_persistence::tenant::{TenantId, TenantPermissions};
    use helios_persistence::types::StoredResource;
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    /// A storage whose `read` answers by id: `live-*` exists, `gone-*` was
    /// deleted, `down-*` fails as unavailable, `slow-*` times out, anything
    /// else is absent. It records every read.
    #[derive(Default)]
    struct ByIdStorage {
        reads: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ResourceStorage for ByIdStorage {
        fn backend_name(&self) -> &'static str {
            "by-id"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            self.reads.lock().push(format!("{resource_type}/{id}"));
            if id.starts_with("live") {
                return Ok(Some(StoredResource::new(
                    resource_type,
                    id,
                    tenant.tenant_id().clone(),
                    json!({"resourceType": resource_type, "id": id}),
                    FhirVersion::R4,
                )));
            }
            if id.starts_with("gone") {
                return Err(StorageError::Resource(ResourceError::Gone {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    deleted_at: None,
                }));
            }
            if id.starts_with("down") {
                return Err(StorageError::Backend(BackendError::Unavailable {
                    backend_name: "by-id".to_string(),
                    message: "db-7 is restarting".to_string(),
                }));
            }
            if id.starts_with("slow") {
                return Err(StorageError::Backend(BackendError::Timeout {
                    backend_name: "by-id".to_string(),
                    message: "statement timeout on db-7".to_string(),
                }));
            }
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("refs"), TenantPermissions::full_access())
    }

    fn refs(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// The rendered `(status, OperationOutcome, Retry-After)` of an error.
    async fn render(error: RestError) -> (u16, Value, Option<String>) {
        let response = error.into_response();
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .map(|v| v.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap(), retry_after)
    }

    fn issue_text(outcome: &Value) -> String {
        outcome["issue"][0]["details"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// A missing and a deleted target are the same answer on a run path: 400
    /// `not-found` listing both as the caller wrote them; a live one is not
    /// listed.
    #[tokio::test]
    async fn missing_and_deleted_targets_answer_400_not_found() {
        let storage = ByIdStorage::default();
        let error = check_stored_targets(
            &storage,
            &tenant(),
            &refs(&["Patient/live-1", "Patient/absent", "gone-p"]),
            &refs(&["Group/gone-g", "Group/live-g"]),
        )
        .await
        .unwrap_err();
        let (status, outcome, _) = render(error).await;
        assert_eq!(status, 400, "{outcome}");
        assert_eq!(outcome["issue"][0]["code"], "not-found", "{outcome}");
        let text = outcome["issue"][0]["details"]["text"].as_str().unwrap();
        for named in ["Patient/absent", "gone-p", "Group/gone-g"] {
            assert!(text.contains(named), "{named}: {outcome}");
        }
        for live in ["live-1", "live-g"] {
            assert!(!text.contains(live), "{live}: {outcome}");
        }
    }

    /// Every live target passes, and each distinct `(type, id)` is read once.
    #[tokio::test]
    async fn live_targets_pass_and_each_is_read_once() {
        let storage = ByIdStorage::default();
        check_stored_targets(
            &storage,
            &tenant(),
            &refs(&["Patient/live-1", "live-1", "Patient/live-2"]),
            &refs(&["Group/live-1"]),
        )
        .await
        .expect("every target exists");
        let mut reads = storage.reads.lock().clone();
        reads.sort();
        assert_eq!(reads, ["Group/live-1", "Patient/live-1", "Patient/live-2"]);
    }

    /// Council C11 (A4): an existence check that fails because the backend is
    /// unavailable is a 503 `transient` with `Retry-After`, and one that times
    /// out a 504 `timeout`; neither is a 500 nor a missing target. Both come
    /// from the shared `RestError::from(StorageError)` mapping, so they read
    /// exactly as any other HFS read that fails this way.
    #[tokio::test]
    async fn an_existence_check_that_fails_with_unavailable_answers_503() {
        let storage = ByIdStorage::default();
        for (reference, status, code, retry_after) in [
            ("Patient/down-1", 503, "transient", true),
            ("Patient/slow-1", 504, "timeout", false),
        ] {
            let error = check_stored_targets(&storage, &tenant(), &refs(&[reference]), &[])
                .await
                .unwrap_err();
            let (got, outcome, header) = render(error).await;
            assert_eq!(got, status, "{reference}: {outcome}");
            assert_eq!(outcome["issue"][0]["code"], code, "{reference}: {outcome}");
            assert_eq!(header.is_some(), retry_after, "{reference}: {header:?}");
            assert!(
                !issue_text(&outcome).contains("could not be resolved"),
                "a failed read is not a missing target: {outcome}"
            );
        }
    }

    /// Shapes come first: an unusable value is a 400 `invalid` naming its
    /// parameter, and nothing is read from storage, even when another value
    /// names a missing target.
    #[tokio::test]
    async fn shapes_are_checked_before_any_existence_read() {
        let storage = ByIdStorage::default();
        for (patient, group, param) in [
            (
                refs(&["Patient/absent", "Patient/p1/_history/2"]),
                refs(&[]),
                "patient",
            ),
            (refs(&["Patient/absent"]), refs(&["Patient/p1"]), "group"),
            (
                refs(&[]),
                refs(&["http://example.org/fhir/Group/g1"]),
                "group",
            ),
            (
                refs(&["urn:uuid:0b0e8c86-5a8f-4d2e-9e4a-1d2c3b4a5f60"]),
                refs(&[]),
                "patient",
            ),
            (refs(&["#contained"]), refs(&[]), "patient"),
            (refs(&["Patient/"]), refs(&[]), "patient"),
        ] {
            let error = check_stored_targets(&storage, &tenant(), &patient, &group)
                .await
                .unwrap_err();
            let (status, outcome, _) = render(error).await;
            assert_eq!(status, 400, "{outcome}");
            assert_eq!(outcome["issue"][0]["code"], "invalid", "{outcome}");
            assert!(
                outcome["issue"][0]["details"]["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("'{param}'")),
                "{outcome}"
            );
        }
        assert!(storage.reads.lock().is_empty());
    }

    /// The 1000-value bound comes before any shape or existence check.
    #[tokio::test]
    async fn the_value_bound_comes_first() {
        let storage = ByIdStorage::default();
        let many: Vec<String> = (0..1001).map(|i| format!("Patient/absent-{i}")).collect();
        let error = check_stored_targets(&storage, &tenant(), &many, &[])
            .await
            .unwrap_err();
        assert!(matches!(error, RestError::BadRequest { .. }), "{error:?}");
        assert!(storage.reads.lock().is_empty());
    }
}
