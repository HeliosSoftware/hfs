//! Canonical validation for an atomic manifest artifact publication.
//!
//! A publication is a complete, self-consistent set of `SubmitFileRecord`
//! values, not a stream of independent writes. These helpers give every
//! backend the same view of logical file identity, deterministic order, and
//! value ranges before rows are made visible.

use crate::core::bulk_submit_worker::SubmitFileRecord;
use crate::error::{StorageError, StorageResult};

/// Terminal state written together with the complete artifact set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestPublicationStatus {
    /// Publish the artifacts as successful output.
    Completed,
    /// Record the manifest as failed while preserving its error artifacts.
    Failed {
        /// Terminal message retained with the manifest.
        error_message: String,
    },
}

/// Outcome of publishing a manifest's artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestPublicationResult {
    /// A previously unpublished manifest/generation was published.
    Published,
    /// The publication had already been committed with the same identity.
    AlreadyPublished,
}

fn invalid_publication(message: impl Into<String>) -> StorageError {
    StorageError::Validation(crate::error::ValidationError::InvalidResource {
        message: message.into(),
        details: Vec::new(),
    })
}

/// Sorts publication records by their logical identity and validates their
/// storage-representable ranges.
///
/// Two records have the same logical identity when they have the same
/// `file_type`, optional `resource_type`, and `part_index`. `manifest_url` and
/// the other content fields are intentionally excluded from that identity: a
/// publication may not silently resolve a disagreement about which generation
/// owns a file. This function returns cloned, deterministic records so callers
/// can compare and insert the exact canonical set.
pub(crate) fn canonical_publication_files(
    files: &[SubmitFileRecord],
) -> StorageResult<Vec<SubmitFileRecord>> {
    let mut seen = std::collections::HashSet::with_capacity(files.len());
    for file in files {
        if !matches!(file.file_type.as_str(), "output" | "error" | "deleted") {
            return Err(invalid_publication(format!(
                "unsupported publication file type: {:?}",
                file.file_type
            )));
        }
        if file.file_path.trim().is_empty() {
            return Err(invalid_publication(
                "publication file_path must not be empty",
            ));
        }
        if let Some(count_severity) = &file.count_severity {
            let Some(counts) = count_severity.as_object() else {
                return Err(invalid_publication(
                    "publication count_severity must be an object",
                ));
            };
            for value in counts.values() {
                if value.as_u64().is_none() {
                    return Err(invalid_publication(
                        "publication count_severity values must be nonnegative integers",
                    ));
                }
            }
        }
        if file.part_index > i32::MAX as u32 {
            return Err(invalid_publication(format!(
                "publication part_index exceeds i32::MAX: {}",
                file.part_index
            )));
        }
        if file.line_count > i64::MAX as u64 {
            return Err(invalid_publication(
                "publication line_count exceeds i64::MAX",
            ));
        }
        if file.byte_count > i64::MAX as u64 {
            return Err(invalid_publication(
                "publication byte_count exceeds i64::MAX",
            ));
        }
        let identity = (
            file.file_type.as_str(),
            file.resource_type.as_deref(),
            file.part_index,
        );
        if !seen.insert(identity) {
            return Err(invalid_publication(format!(
                "duplicate publication file identity: type={:?}, resource_type={:?}, part_index={}",
                file.file_type, file.resource_type, file.part_index
            )));
        }
    }

    let mut sorted = files.to_vec();
    sorted.sort_by_key(|file| {
        (
            file.file_type.clone(),
            file.resource_type.clone(),
            file.part_index,
        )
    });
    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn record(
        file_type: &str,
        resource_type: Option<&str>,
        part_index: u32,
        manifest_url: Option<&str>,
        count_severity: Option<Value>,
    ) -> SubmitFileRecord {
        SubmitFileRecord {
            manifest_url: manifest_url.map(str::to_string),
            file_type: file_type.to_string(),
            resource_type: resource_type.map(str::to_string),
            part_index,
            file_path: format!("{file_type}/part-{part_index}"),
            line_count: 1,
            byte_count: 1,
            count_severity,
        }
    }

    #[test]
    fn reordered_records_have_the_same_canonical_order_and_values() {
        let first = record(
            "error",
            Some("OperationOutcome"),
            0,
            Some("u"),
            Some(json!({"a": 1, "b": 2})),
        );
        let second = record("output", Some("Patient"), 0, Some("u"), None);
        let reversed = vec![second.clone(), first.clone()];
        let expected = vec![first, second];

        assert_eq!(canonical_publication_files(&reversed).unwrap(), expected);
    }

    #[test]
    fn none_and_empty_resource_type_are_distinct_identities() {
        let files = vec![
            record("output", None, 0, Some("u"), None),
            record("output", Some(""), 0, Some("u"), None),
        ];

        let canonical = canonical_publication_files(&files).unwrap();
        assert_eq!(canonical.len(), 2);
        assert_eq!(canonical[0].resource_type, None);
        assert_eq!(canonical[1].resource_type, Some(String::new()));
    }

    #[test]
    fn duplicate_logical_identity_is_rejected_even_when_exact() {
        let file = record("error", Some("OperationOutcome"), 0, Some("u"), None);
        let identical = vec![file.clone(), file];

        let error = canonical_publication_files(&identical).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("duplicate publication file identity")
        );
    }

    #[test]
    fn changed_metadata_and_severity_are_retained_as_inequal() {
        let unchanged = record(
            "error",
            Some("OperationOutcome"),
            0,
            Some("u"),
            Some(json!({"error": 1})),
        );
        let mut changed = unchanged.clone();
        changed.manifest_url = Some("u-2".to_string());
        changed.count_severity = Some(json!({"error": 2}));

        assert_ne!(unchanged, changed);
        let canonical = canonical_publication_files(std::slice::from_ref(&changed)).unwrap();
        assert_eq!(canonical[0].manifest_url, Some("u-2".to_string()));
        assert_eq!(canonical[0].count_severity, Some(json!({"error": 2})));
    }

    #[test]
    fn invalid_kind_path_and_severity_are_rejected() {
        let bad_kind = record("failure", Some("Patient"), 0, Some("u"), None);
        assert!(
            canonical_publication_files(&[bad_kind])
                .unwrap_err()
                .to_string()
                .contains("unsupported publication file type")
        );

        let mut empty_path = record("output", Some("Patient"), 0, Some("u"), None);
        empty_path.file_path = "   ".to_string();
        assert!(
            canonical_publication_files(&[empty_path])
                .unwrap_err()
                .to_string()
                .contains("file_path must not be empty")
        );

        for severity in [
            json!("error"),
            json!({"error": -1}),
            json!({"error": 1.5}),
            json!({"error": null}),
        ] {
            let bad_severity = record(
                "error",
                Some("OperationOutcome"),
                0,
                Some("u"),
                Some(severity),
            );
            assert!(
                canonical_publication_files(&[bad_severity])
                    .unwrap_err()
                    .to_string()
                    .contains("count_severity")
            );
        }
    }

    #[test]
    fn values_that_do_not_fit_signed_sql_columns_are_rejected() {
        let mut too_many_lines = record("output", Some("Patient"), 0, Some("u"), None);
        too_many_lines.line_count = (i64::MAX as u64) + 1;
        assert!(canonical_publication_files(&[too_many_lines]).is_err());

        let mut too_many_bytes = record("output", Some("Patient"), 0, Some("u"), None);
        too_many_bytes.byte_count = (i64::MAX as u64) + 1;
        assert!(canonical_publication_files(&[too_many_bytes]).is_err());

        let too_large_index = record(
            "output",
            Some("Patient"),
            (i32::MAX as u32) + 1,
            Some("u"),
            None,
        );
        assert!(canonical_publication_files(&[too_large_index]).is_err());
    }
}
