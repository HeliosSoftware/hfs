//! Pipeline and orchestration errors for the validation engine.
//!
//! See **[`Errors.md`](../Errors.md)** in the `fhir-validation` crate for the full error model,
//! mapping to [`crate::ValidationIssue`], and terminology binding flows.
//!
//! - [`ValidationError`] is returned from FHIRPath evaluation, profile extraction, remote
//!   terminology calls, and similar **non-issue** failure paths (before they are converted into
//!   [`crate::ValidationIssue`] where applicable).
//! - **Local coded value semantics** use [`TerminologyValidationError`] from generated
//!   ValueSet helpers; when those must propagate through the same orchestration type, they appear as
//!   [`ValidationError::LocalTerminology`].
//! - **Remote** `$validate-code` and related failures are grouped under [`ValidationError::RemoteTerminology`]
//!   ([`RemoteTerminologyError`]). To turn these into [`crate::ValidationIssue`] rows on binding paths, use
//!   [`crate::validation_error_to_issues`] or [`ValidationError::to_binding_issues`].
//! - **Request assembly / local orchestration** failures before any network call uses [`ValidationError::InvalidRequest`]
//!   ([`TerminologyRequestInvalid`]).
//!
//! **`Error::source`:** [`ValidationError::FhirPath`], [`ValidationError::LocalTerminology`], and
//! [`ValidationError::InvalidRequest`] forward to their inner types; other variants do not chain a nested
//! [`std::error::Error`].

use crate::profile::structure_definition_extract::StructureDefinitionExtractMessage;
use crate::terminology::types::TerminologyRemoteError;
use helios_fhir::TerminologyValidationError;
use std::fmt;

/// Local validation of a terminology request before any HTTP call (e.g. [`crate::terminology::requests::ValidateVsRequest::validate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminologyRequestInvalid {
    pub message: String,
}

impl fmt::Display for TerminologyRequestInvalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for TerminologyRequestInvalid {}

/// Why a `$validate-code` `Parameters` JSON response could not be parsed into a membership outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MalformedValidateCodeParameters {
    /// Response JSON is not FHIR `Parameters` with a `parameter` array.
    MissingParameterArray,
    /// `resourceType` was present but not `Parameters` (FHIR is case-sensitive).
    WrongResourceType { got: String },
    /// `parameter` was an array but an entry was not a JSON object.
    ParameterEntryNotObject { index: usize },
    /// A `result` parameter part existed, but `valueBoolean` was missing, not a JSON boolean, or another
    /// `value*` was sent instead of `valueBoolean`.
    ResultValueNotBoolean,
    /// No usable boolean `result` (e.g., no `result,` part, or only empty parts without `valueBoolean`).
    MissingResultBoolean,
}

impl fmt::Display for MalformedValidateCodeParameters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingParameterArray => {
                f.write_str("Expected FHIR Parameters resource with a `parameter` array")
            }
            Self::WrongResourceType { got } => {
                write!(f, "Expected resourceType Parameters, got {got:?}",)
            }
            Self::ParameterEntryNotObject { index } => {
                write!(f, "Parameters.parameter[{index}] must be a JSON object",)
            }
            Self::ResultValueNotBoolean => f.write_str(
                "$validate-code Parameters response `result` must use boolean `valueBoolean`",
            ),
            Self::MissingResultBoolean => {
                f.write_str("$validate-code Parameters response did not include a boolean `result`")
            }
        }
    }
}

/// Remote terminology failures: HTTP / transport metadata, or malformed `$validate-code` `Parameters` JSON.
///
/// This enum is `#[non_exhaustive]` so new protocol or transport classifications can be added without
/// a major version bump for downstream `match` sites (they must include a wildcard arm).
///
/// **Note:** Request-shape failures **before** any network call belong on [`ValidationError::InvalidRequest`],
/// not here. This type covers responses and transport after a request is sent (or attempted).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RemoteTerminologyError {
    /// Upstream HTTP / terminology server outcome: status, parsed diagnostics, optional body.
    ///
    /// Named **`Upstream`** (not `Remote`) so call sites read clearly next to
    /// [`RemoteTerminologyError`] / [`ValidationError::RemoteTerminology`].
    Upstream(TerminologyRemoteError),
    /// `$validate-code` returned JSON that is not a usable FHIR `Parameters` result (e.g. missing
    /// `parameter` or boolean `result`).
    MalformedResponse(MalformedValidateCodeParameters),
}

impl fmt::Display for RemoteTerminologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upstream(err) => {
                if !err.diagnostics.is_empty() {
                    write!(f, "{}", err.diagnostics.join("; "))
                } else if let Some(body) = &err.raw_body {
                    write!(f, "{}", body)
                } else if let Some(status) = err.status {
                    write!(
                        f,
                        "Remote terminology validation failed with status {}",
                        status
                    )
                } else {
                    write!(f, "Remote terminology validation failed")
                }
            }
            Self::MalformedResponse(reason) => write!(f, "{}", reason),
        }
    }
}

impl std::error::Error for RemoteTerminologyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // `TerminologyRemoteError` is a structured value type, not `Error`; string variants have no chain.
        None
    }
}

impl RemoteTerminologyError {
    /// HTTP / upstream outcome when this is [`RemoteTerminologyError::Upstream`].
    pub fn as_upstream(&self) -> Option<&TerminologyRemoteError> {
        match self {
            Self::Upstream(e) => Some(e),
            _ => None,
        }
    }

    /// Structured parse failure when this is [`RemoteTerminologyError::MalformedResponse`].
    pub fn as_malformed_parameters(&self) -> Option<&MalformedValidateCodeParameters> {
        match self {
            Self::MalformedResponse(m) => Some(m),
            _ => None,
        }
    }
}

/// Errors raised while evaluating invariants, parsing profiles, or running terminology-backed validation.
///
/// This enum is `#[non_exhaustive]` so new orchestration failures can be added without forcing every
/// downstream `match` to break in a minor release (callers outside this crate should use a wildcard arm).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ValidationError {
    FhirPath(helios_fhirpath_support::EvaluationError),
    /// Local terminology failure from generated ValueSet helpers (e.g. invalid input).
    LocalTerminology(TerminologyValidationError),
    /// Remote `$validate-code` (or related) failures and protocol issues after a request is in flight.
    RemoteTerminology(RemoteTerminologyError),
    /// Terminology request failed local structural validation before any HTTP call.
    InvalidRequest(TerminologyRequestInvalid),
    InvalidStructureDefinition(StructureDefinitionExtractMessage),
    Internal(String),
}

impl ValidationError {
    /// Human-readable diagnostics when reporting a remote terminology failure on a ValueSet binding.
    ///
    /// Used when `member_of` or similar returns `Err(ValidationError::…)` so callers get consistent
    /// wording with ValueSet context.
    pub fn remote_binding_failure_diagnostics(&self, valueset_url: &str) -> String {
        match self {
            ValidationError::RemoteTerminology(r) => match r {
                RemoteTerminologyError::MalformedResponse(reason) => {
                    format!(
                        "Remote terminology validation failed for ValueSet '{}': {}",
                        valueset_url, reason
                    )
                }
                RemoteTerminologyError::Upstream(remote) => {
                    if !remote.diagnostics.is_empty() {
                        return format!(
                            "Remote terminology validation failed for ValueSet '{}': {}",
                            valueset_url,
                            remote.diagnostics.join("; ")
                        );
                    }
                    if let Some(body) = &remote.raw_body {
                        return format!(
                            "Remote terminology validation failed for ValueSet '{}': {}",
                            valueset_url, body
                        );
                    }

                    if let Some(status) = remote.status {
                        return format!(
                            "Remote terminology validation failed for ValueSet '{}' with status {}",
                            valueset_url, status
                        );
                    }

                    format!(
                        "Remote terminology validation failed for ValueSet '{}'",
                        valueset_url
                    )
                }
            },
            // Not a remote payload issue; surface [`Display`] (e.g. [`InvalidRequest`], [`LocalTerminology`]).
            _ => self.to_string(),
        }
    }

    /// [`TerminologyRequestInvalid`] when this is [`ValidationError::InvalidRequest`].
    pub fn as_invalid_request(&self) -> Option<&TerminologyRequestInvalid> {
        match self {
            Self::InvalidRequest(e) => Some(e),
            _ => None,
        }
    }

    /// [`RemoteTerminologyError`] when this is [`ValidationError::RemoteTerminology`].
    pub fn as_remote_terminology(&self) -> Option<&RemoteTerminologyError> {
        match self {
            Self::RemoteTerminology(e) => Some(e),
            _ => None,
        }
    }

    /// Nested malformed `$validate-code` shape when this is
    /// `RemoteTerminology(MalformedResponse(…))`.
    pub fn as_remote_malformed_parameters(&self) -> Option<&MalformedValidateCodeParameters> {
        self.as_remote_terminology()
            .and_then(RemoteTerminologyError::as_malformed_parameters)
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FhirPath(e) => write!(f, "{}", e),
            Self::LocalTerminology(e) => write!(f, "{}", e),
            Self::InvalidRequest(e) => write!(f, "{}", e),
            Self::InvalidStructureDefinition(e) => write!(f, "{}", e),
            Self::RemoteTerminology(r) => write!(f, "{}", r),
            Self::Internal(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FhirPath(e) => Some(e),
            Self::LocalTerminology(e) => Some(e),
            Self::InvalidRequest(e) => Some(e),
            // `RemoteTerminology` wraps structured transport data without an `Error` chain today.
            _ => None,
        }
    }
}

/// Short stable label for metrics and structured logging (not localized text).
pub fn validation_error_kind_label(err: &ValidationError) -> &'static str {
    match err {
        ValidationError::FhirPath(_) => "fhir_path",
        ValidationError::LocalTerminology(_) => "local_terminology",
        ValidationError::RemoteTerminology(_) => "remote_terminology",
        ValidationError::InvalidRequest(_) => "invalid_request",
        ValidationError::InvalidStructureDefinition(_) => "invalid_structure_definition",
        ValidationError::Internal(_) => "other",
    }
}

/// Subtype label for [`RemoteTerminologyError`] when logging `member_of` failures.
pub fn remote_terminology_error_kind_label(err: &RemoteTerminologyError) -> &'static str {
    match err {
        RemoteTerminologyError::Upstream(_) => "remote_http",
        RemoteTerminologyError::MalformedResponse(m) => {
            malformed_validate_code_parameters_kind_label(m)
        }
    }
}

/// Fine-grained label for [`MalformedValidateCodeParameters`] (metrics / tracing).
pub fn malformed_validate_code_parameters_kind_label(
    err: &MalformedValidateCodeParameters,
) -> &'static str {
    match err {
        MalformedValidateCodeParameters::MissingParameterArray => "missing_parameter_array",
        MalformedValidateCodeParameters::WrongResourceType { .. } => "wrong_resource_type",
        MalformedValidateCodeParameters::ParameterEntryNotObject { .. } => {
            "parameter_entry_not_object"
        }
        MalformedValidateCodeParameters::ResultValueNotBoolean => "result_value_not_boolean",
        MalformedValidateCodeParameters::MissingResultBoolean => "missing_result_boolean",
    }
}

impl From<helios_fhirpath_support::EvaluationError> for ValidationError {
    fn from(e: helios_fhirpath_support::EvaluationError) -> Self {
        Self::FhirPath(e)
    }
}

impl From<StructureDefinitionExtractMessage> for ValidationError {
    fn from(msg: StructureDefinitionExtractMessage) -> Self {
        Self::InvalidStructureDefinition(msg)
    }
}

impl From<TerminologyRemoteError> for ValidationError {
    fn from(e: TerminologyRemoteError) -> Self {
        Self::RemoteTerminology(RemoteTerminologyError::Upstream(e))
    }
}

impl From<RemoteTerminologyError> for ValidationError {
    fn from(e: RemoteTerminologyError) -> Self {
        Self::RemoteTerminology(e)
    }
}
