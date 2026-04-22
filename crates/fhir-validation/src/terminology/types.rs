use helios_fhir::TerminologyValidationError;

/// Outcome of a narrow [`crate::terminology::service::TerminologyService::member_of`] check.
#[derive(Debug, Clone)]
pub struct TerminologyMembershipOutcome {
    pub is_member: bool,
    /// True when local terminology cannot decide membership (e.g. ValueSet composition not
    /// materialized locally). This is **not** a proven non-member result.
    pub remote_validation_required: bool,
    pub message: Option<String>,
    pub diagnostics: Vec<String>,
    pub system: Option<String>,
    pub code: Option<String>,
    pub version: Option<String>,
    pub display: Option<String>,
    /// When the backend used the same structured errors as local `validate_coding` (for example
    /// [`TerminologyValidationError::WrongDisplay`]), this preserves that variant so binding
    /// validation can map it through [`crate::binding::common::local_error_to_issues`].
    ///
    /// Remote `$validate-code` responses do not set this; they only populate `message` / metadata.
    pub local_failure: Option<TerminologyValidationError>,
}

#[derive(Debug, Clone)]
pub struct TerminologyRemoteError {
    pub status: Option<u16>,
    pub diagnostics: Vec<String>,
    pub raw_body: Option<String>,
}
