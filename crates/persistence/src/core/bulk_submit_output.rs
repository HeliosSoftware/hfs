//! Stable, submit-only artifact locators for output-store keys.
//!
//! The locator is a bounded SHA-256 digest of the complete artifact scope. It
//! is stored in the internal `ExportPartKey::resource_type` field only; the
//! worker continues to keep the protocol kind and resource type in database
//! rows. The submission-level `job_id` is unchanged so cleanup remains grouped
//! by submission.

use sha2::{Digest, Sha256};

use crate::core::bulk_export_output::ExportPartKey;
use crate::core::bulk_submit::SubmissionId;
use crate::core::bulk_submit_input::submission_output_job_id;
use crate::tenant::TenantContext;

const LOCATOR_PREFIX: &str = "submit-v1-";
const LOCATOR_VERSION: u8 = 1;

/// Builds a stable route locator for one submit artifact.
///
/// The tuple includes the tenant, submitter, submission ID, manifest ID,
/// protocol file type, optional resource type, part index, and fencing token.
/// JSON makes `None` and an empty resource type distinct while preserving the
/// order of every identity field.
pub fn submit_artifact_locator(
    tenant: &TenantContext,
    id: &SubmissionId,
    manifest_id: &str,
    file_type: &str,
    resource_type: Option<&str>,
    part_index: u32,
    fencing_token: u64,
) -> String {
    let payload = serde_json::to_vec(&(
        LOCATOR_VERSION,
        tenant.tenant_id().as_str(),
        id.submitter.as_str(),
        id.submission_id.as_str(),
        manifest_id,
        file_type,
        resource_type,
        part_index,
        fencing_token,
    ))
    .expect("canonical submit locator payload must be valid JSON");

    let digest = Sha256::digest(&payload);
    let mut locator = String::with_capacity(LOCATOR_PREFIX.len() + 64);
    locator.push_str(LOCATOR_PREFIX);
    for byte in digest.iter() {
        locator.push_str(&format!("{byte:02x}"));
    }
    locator
}

/// Builds the internal output-store key for a submit artifact.
///
/// The locator binds the complete artifact identity. The key keeps the real
/// protocol kind and the submission-level job ID, while using the locator as a
/// bounded internal resource-type component safe for filesystem and S3 scratch
/// names.
pub fn submit_artifact_key(
    tenant: &TenantContext,
    id: &SubmissionId,
    manifest_id: &str,
    file_type: &str,
    resource_type: Option<&str>,
    part_index: u32,
    fencing_token: u64,
) -> ExportPartKey {
    let locator = submit_artifact_locator(
        tenant,
        id,
        manifest_id,
        file_type,
        resource_type,
        part_index,
        fencing_token,
    );

    ExportPartKey {
        tenant_id: tenant.tenant_id().as_str().to_owned(),
        job_id: submission_output_job_id(id),
        resource_type: locator,
        file_type: file_type.to_owned(),
        part_index,
        fencing_token,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant(value: &str) -> TenantContext {
        TenantContext::new(TenantId::new(value), TenantPermissions::full_access())
    }

    #[test]
    fn every_identity_component_changes_the_locator() {
        let base_tenant = tenant("tenant-a");
        let id = SubmissionId::new("submitter-a", "submission-a");
        let manifest = "manifest-a";
        let kind = "output";
        let resource = Some("Patient");
        let part = 2;
        let token = 7;
        let base =
            submit_artifact_locator(&base_tenant, &id, manifest, kind, resource, part, token);
        // The v1 encoding is persistent: changing it would change stored keys.
        assert_eq!(
            base,
            "submit-v1-2eb5ff8b2cef4a178ee29a131e69c1fdf258a2ddd05eec7edf5b9590048ad44a"
        );

        let changed_submitter = SubmissionId::new("submitter-b", "submission-a");
        let changed_submission = SubmissionId::new("submitter-a", "submission-b");
        let variants = [
            submit_artifact_locator(
                &tenant("tenant-b"),
                &id,
                manifest,
                kind,
                resource,
                part,
                token,
            ),
            submit_artifact_locator(
                &base_tenant,
                &changed_submitter,
                manifest,
                kind,
                resource,
                part,
                token,
            ),
            submit_artifact_locator(
                &base_tenant,
                &changed_submission,
                manifest,
                kind,
                resource,
                part,
                token,
            ),
            submit_artifact_locator(&base_tenant, &id, "manifest-b", kind, resource, part, token),
            submit_artifact_locator(&base_tenant, &id, manifest, "error", resource, part, token),
            submit_artifact_locator(&base_tenant, &id, manifest, kind, None, part, token),
            submit_artifact_locator(&base_tenant, &id, manifest, kind, Some(""), part, token),
            submit_artifact_locator(&base_tenant, &id, manifest, kind, resource, 3, token),
            submit_artifact_locator(&base_tenant, &id, manifest, kind, resource, part, 8),
        ];

        let unique: std::collections::HashSet<_> = variants.iter().collect();
        assert_eq!(
            unique.len(),
            variants.len(),
            "including None versus Some(\"\")"
        );
        for variant in variants {
            assert_ne!(base, variant);
        }
    }

    #[test]
    fn arbitrary_ids_produce_a_bounded_safe_internal_component() {
        let safe_tenant = tenant("tenant-safe");
        let long = "s".repeat(4096);
        let id = SubmissionId::new(&long, "../../unsafe-submission");
        let manifest = "../../unsafe-manifest";
        let resource = Some("Patient/../Patient");
        let locator =
            submit_artifact_locator(&safe_tenant, &id, manifest, "output", resource, 1, 2);

        assert_eq!(locator.len(), LOCATOR_PREFIX.len() + 64);
        assert!(locator.starts_with(LOCATOR_PREFIX));
        assert!(
            locator[LOCATOR_PREFIX.len()..].bytes().all(|byte| {
                byte.is_ascii_hexdigit() && matches!(byte, b'0'..=b'9' | b'a'..=b'f')
            })
        );

        let key = submit_artifact_key(&safe_tenant, &id, manifest, "output", resource, 1, 2);
        assert_eq!(key.tenant_id, "tenant-safe");
        assert_eq!(key.job_id.as_str(), submission_output_job_id(&id).as_str());
        assert_eq!(key.resource_type, locator);
        assert_eq!(key.file_type, "output");
        assert_eq!(key.part_index, 1);
        assert_eq!(key.fencing_token, 2);
    }
}
