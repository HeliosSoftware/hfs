//! Fixed bounds on the size of `$sql-run` / `$sql-export` request inputs
//! (#1705). Each is a `400 Bad Request` that names the limit, checked before
//! the work it bounds starts.

use crate::error::RestError;

/// Most `subject` entries one `$sql-export` request may carry.
pub(crate) const MAX_EXPORT_SUBJECTS: usize = 64;

/// Most `patient` plus `group` values one request may carry.
pub(crate) const MAX_PATIENT_GROUP_VALUES: usize = 1000;

/// Most `context` entries one request may carry.
pub(crate) const MAX_CONTEXT_ENTRIES: usize = 256;

/// `depends-on` entries allowed per Library, as a multiple of
/// `HFS_SOF_SQLQUERY_MAX_VDS`.
pub(crate) const DEPENDS_ON_PER_MAX_VDS: usize = 4;

/// The per-Library `depends-on` limit for a given `HFS_SOF_SQLQUERY_MAX_VDS`.
pub(crate) fn max_depends_on(max_vds: usize) -> usize {
    max_vds.saturating_mul(DEPENDS_ON_PER_MAX_VDS)
}

/// Rejects a `$sql-export` request with more than [`MAX_EXPORT_SUBJECTS`]
/// `subject` entries.
pub(crate) fn check_export_subjects(count: usize) -> Result<(), RestError> {
    if count <= MAX_EXPORT_SUBJECTS {
        return Ok(());
    }
    Err(RestError::BadRequest {
        message: format!(
            "$sql-export accepts at most {MAX_EXPORT_SUBJECTS} `subject` entries per request; \
             this request has {count}"
        ),
    })
}

/// Rejects a request with more than [`MAX_PATIENT_GROUP_VALUES`] `patient`
/// plus `group` values.
pub(crate) fn check_patient_group_values(count: usize) -> Result<(), RestError> {
    if count <= MAX_PATIENT_GROUP_VALUES {
        return Ok(());
    }
    Err(RestError::BadRequest {
        message: format!(
            "at most {MAX_PATIENT_GROUP_VALUES} `patient` plus `group` values are accepted per \
             request; this request has {count}"
        ),
    })
}

/// Rejects a request with more than [`MAX_CONTEXT_ENTRIES`] `context` entries.
pub(crate) fn check_context_entries(count: usize) -> Result<(), RestError> {
    if count <= MAX_CONTEXT_ENTRIES {
        return Ok(());
    }
    Err(RestError::BadRequest {
        message: format!(
            "at most {MAX_CONTEXT_ENTRIES} `context` entries are accepted per request; this \
             request has {count}"
        ),
    })
}

/// The error for a Library that declares `count` `depends-on` entries where at
/// most `max` ([`max_depends_on`]) are accepted.
pub(crate) fn depends_on_limit_error(library: &str, count: usize, max: usize) -> RestError {
    RestError::BadRequest {
        message: format!(
            "Library '{library}' declares {count} `relatedArtifact` depends-on entries; at most \
             {max} ({DEPENDS_ON_PER_MAX_VDS} × HFS_SOF_SQLQUERY_MAX_VDS) are accepted per Library"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(err: RestError) -> String {
        let RestError::BadRequest { message } = err else {
            panic!("expected 400");
        };
        message
    }

    #[test]
    fn export_subjects_boundary() {
        assert!(check_export_subjects(64).is_ok());
        let msg = message(check_export_subjects(65).unwrap_err());
        assert!(msg.contains("64"), "{msg}");
    }

    #[test]
    fn patient_group_values_boundary() {
        assert!(check_patient_group_values(1000).is_ok());
        let msg = message(check_patient_group_values(1001).unwrap_err());
        assert!(msg.contains("1000"), "{msg}");
    }

    #[test]
    fn context_entries_boundary() {
        assert!(check_context_entries(256).is_ok());
        let msg = message(check_context_entries(257).unwrap_err());
        assert!(msg.contains("256"), "{msg}");
    }

    #[test]
    fn depends_on_limit_scales_with_max_vds_without_overflow() {
        assert_eq!(max_depends_on(16), 64);
        assert_eq!(max_depends_on(usize::MAX), usize::MAX);
        let msg = message(depends_on_limit_error("lib", 65, 64));
        assert!(
            msg.contains("65") && msg.contains("64") && msg.contains("'lib'"),
            "{msg}"
        );
    }
}
