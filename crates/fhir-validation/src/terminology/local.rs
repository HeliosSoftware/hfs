//! Synchronous terminology membership checks using generated ValueSet helpers in [`helios_fhir`].
//!
//! This is intended for sync validation paths when no remote terminology server is available.
//! Supported FHIR versions follow the workspace’s embedded terminology indices (`R4` and `R5` when
//! those features are enabled). Other enabled versions return a non-member outcome until matching
//! local indices exist.

use crate::ValidationError;
use crate::error::validation_error_kind_label;
use crate::terminology::service::TerminologyServiceSync;
use crate::terminology::types::TerminologyMembershipOutcome;
use helios_fhir::FhirVersion;
use helios_fhir::TerminologyValidationError;
use tracing::warn;

/// Terminology service that delegates to embedded `helios_fhir` ValueSet validation (`validate_code` /
/// `validate_coding`).
///
/// This mirrors the validator’s local-first binding checks: it answers membership using the same
/// generated tables as [`helios_fhir::r4::terminology`] / [`helios_fhir::r5::terminology`] when those
/// modules are enabled for the build.
#[derive(Debug, Clone, Copy)]
pub struct LocalTerminologyService {
    fhir_version: FhirVersion,
}

impl LocalTerminologyService {
    pub const fn new(fhir_version: FhirVersion) -> Self {
        Self { fhir_version }
    }

    #[inline]
    fn not_member() -> TerminologyMembershipOutcome {
        TerminologyMembershipOutcome {
            is_member: false,
            remote_validation_required: false,
            message: None,
            diagnostics: Vec::new(),
            system: None,
            code: None,
            version: None,
            display: None,
            local_failure: None,
        }
    }

    #[inline]
    fn member() -> TerminologyMembershipOutcome {
        TerminologyMembershipOutcome {
            is_member: true,
            remote_validation_required: false,
            message: None,
            diagnostics: Vec::new(),
            system: None,
            code: None,
            version: None,
            display: None,
            local_failure: None,
        }
    }
}

/// Non-member outcome carrying a structured [`TerminologyValidationError`] from generated
/// `validate_coding` / `validate_code` (for example display mismatch).
fn non_member_with_local_failure(err: TerminologyValidationError) -> TerminologyMembershipOutcome {
    let msg = err.to_string();
    TerminologyMembershipOutcome {
        is_member: false,
        remote_validation_required: false,
        message: Some(msg.clone()),
        diagnostics: vec![msg],
        system: None,
        code: None,
        version: None,
        display: None,
        local_failure: Some(err),
    }
}

fn map_validation_error(
    err: TerminologyValidationError,
) -> Result<TerminologyMembershipOutcome, ValidationError> {
    match err {
        TerminologyValidationError::InvalidInput(_) => Err(ValidationError::LocalTerminology(err)),
        TerminologyValidationError::MissingSystem(_) => Ok(LocalTerminologyService::not_member()),
        TerminologyValidationError::RemoteValidationRequired(msg) => {
            Ok(TerminologyMembershipOutcome {
                is_member: false,
                remote_validation_required: true,
                message: Some(msg),
                diagnostics: Vec::new(),
                system: None,
                code: None,
                version: None,
                display: None,
                local_failure: None,
            })
        }
        TerminologyValidationError::UnknownCode { .. }
        | TerminologyValidationError::NotInValueSet { .. }
        | TerminologyValidationError::WrongDisplay { .. } => Ok(non_member_with_local_failure(err)),
    }
}

#[cfg(feature = "R4")]
fn r4_coding(system: Option<&str>, code: &str, display: Option<&str>) -> helios_fhir::r4::Coding {
    use helios_fhir::Element;
    use helios_fhir::r4::{Code, Coding, Uri};

    Coding {
        id: None,
        extension: None,
        system: system.map(|s| Uri {
            id: None,
            extension: None,
            value: Some(s.to_string()),
        }),
        version: None,
        code: Some(Code {
            id: None,
            extension: None,
            value: Some(code.to_string()),
        }),
        display: display.map(|d| Element {
            id: None,
            extension: None,
            value: Some(d.to_string()),
        }),
        user_selected: None,
    }
}

#[cfg(feature = "R5")]
fn r5_coding(system: Option<&str>, code: &str, display: Option<&str>) -> helios_fhir::r5::Coding {
    use helios_fhir::Element;
    use helios_fhir::r5::{Code, Coding, Uri};

    Coding {
        id: None,
        extension: None,
        system: system.map(|s| Uri {
            id: None,
            extension: None,
            value: Some(s.to_string()),
        }),
        version: None,
        code: Some(Code {
            id: None,
            extension: None,
            value: Some(code.to_string()),
        }),
        display: display.map(|d| Element {
            id: None,
            extension: None,
            value: Some(d.to_string()),
        }),
        user_selected: None,
    }
}

#[cfg(feature = "R4")]
fn member_of_r4(
    valueset_url: &str,
    system: Option<&str>,
    code: &str,
    display: Option<&str>,
) -> Result<TerminologyMembershipOutcome, ValidationError> {
    use helios_fhir::r4::terminology;

    let res = if system.is_some() {
        let coding = r4_coding(system, code, display);
        terminology::validate_coding(valueset_url, &coding)
    } else {
        terminology::validate_code(valueset_url, code)
    };

    match res {
        Ok(()) => Ok(LocalTerminologyService::member()),
        Err(e) => map_validation_error(e),
    }
}

#[cfg(feature = "R5")]
fn member_of_r5(
    valueset_url: &str,
    system: Option<&str>,
    code: &str,
    display: Option<&str>,
) -> Result<TerminologyMembershipOutcome, ValidationError> {
    use helios_fhir::r5::terminology;

    let res = if system.is_some() {
        let coding = r5_coding(system, code, display);
        terminology::validate_coding(valueset_url, &coding)
    } else {
        terminology::validate_code(valueset_url, code)
    };

    match res {
        Ok(()) => Ok(LocalTerminologyService::member()),
        Err(e) => map_validation_error(e),
    }
}

impl TerminologyServiceSync for LocalTerminologyService {
    fn member_of(
        &self,
        valueset_url: &str,
        system: Option<&str>,
        code: &str,
        display: Option<&str>,
    ) -> Result<TerminologyMembershipOutcome, ValidationError> {
        let out = match self.fhir_version {
            #[cfg(feature = "R4")]
            FhirVersion::R4 => member_of_r4(valueset_url, system, code, display),
            #[cfg(feature = "R5")]
            FhirVersion::R5 => member_of_r5(valueset_url, system, code, display),
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => Ok(Self::not_member()),
            #[cfg(feature = "R6")]
            FhirVersion::R6 => Ok(Self::not_member()),
        };
        if let Err(ref e) = out {
            warn!(
                valueset_url = %valueset_url,
                error_kind = validation_error_kind_label(e),
                "local terminology member_of error"
            );
        }
        out
    }
}
