use crate::ValidationError;
use crate::error::{MalformedValidateCodeParameters, RemoteTerminologyError};
use crate::types::{TerminologyMembershipOutcome, TerminologyRemoteError};
use helios_fhirpath::error::FhirPathError;

fn extract_operation_outcome_diagnostics(body: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };

    if json.get("resourceType").and_then(|v| v.as_str()) != Some("OperationOutcome") {
        return Vec::new();
    }

    let mut out = Vec::new();
    if let Some(issues) = json.get("issue").and_then(|v| v.as_array()) {
        for issue in issues {
            if let Some(diag) = issue.get("diagnostics").and_then(|v| v.as_str()) {
                out.push(diag.to_string());
                continue;
            }

            if let Some(details_text) = issue
                .get("details")
                .and_then(|d| d.get("text"))
                .and_then(|v| v.as_str())
            {
                out.push(details_text.to_string());
            }
        }
    }

    out
}

/// Known prefix produced by [`helios_fhirpath::terminology_client::TerminologyClient`] on HTTP errors.
const VALIDATE_CODE_FAILURE_PREFIX: &str = "ValueSet validation failed with status ";

fn parse_validate_code_client_terminology_message(msg: &str) -> TerminologyRemoteError {
    if let Some(rest) = msg.strip_prefix(VALIDATE_CODE_FAILURE_PREFIX) {
        if let Some((status_and_suffix, body)) = rest.split_once(": ") {
            let status = status_and_suffix
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u16>().ok());
            let diagnostics = extract_operation_outcome_diagnostics(body);
            return TerminologyRemoteError {
                status,
                diagnostics,
                raw_body: Some(body.to_string()),
            };
        }
    }

    TerminologyRemoteError {
        status: None,
        diagnostics: Vec::new(),
        raw_body: Some(msg.to_string()),
    }
}

/// Convert a [`FhirPathError`] from the terminology HTTP client into structured remote metadata.
pub fn terminology_remote_from_fhir_path_error(err: &FhirPathError) -> TerminologyRemoteError {
    match err {
        FhirPathError::HttpError(code, body) => TerminologyRemoteError {
            status: Some(*code),
            diagnostics: extract_operation_outcome_diagnostics(body),
            raw_body: Some(body.clone()),
        },
        FhirPathError::TerminologyError(msg) => parse_validate_code_client_terminology_message(msg),
        FhirPathError::NetworkError(msg) | FhirPathError::ParseError(msg) => {
            TerminologyRemoteError {
                status: None,
                diagnostics: Vec::new(),
                raw_body: Some(msg.clone()),
            }
        }
        other => TerminologyRemoteError {
            status: None,
            diagnostics: vec![other.to_string()],
            raw_body: None,
        },
    }
}

/// Best-effort parsing of legacy string diagnostics (e.g. tests or older callers).
pub fn build_remote_terminology_error(msg: &str) -> TerminologyRemoteError {
    if msg.contains(VALIDATE_CODE_FAILURE_PREFIX) {
        return parse_validate_code_client_terminology_message(msg);
    }

    let status = msg
        .split("status ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse::<u16>().ok());

    if let Some((_, body)) = msg.split_once(": {") {
        let json_body = format!("{{{}", body);
        let diagnostics = extract_operation_outcome_diagnostics(&json_body);
        return TerminologyRemoteError {
            status,
            diagnostics,
            raw_body: Some(json_body),
        };
    }

    TerminologyRemoteError {
        status,
        diagnostics: Vec::new(),
        raw_body: Some(msg.to_string()),
    }
}

/// Parse the FHIR `Parameters` response returned by `ValueSet/$validate-code`.
///
/// Successful responses are converted into a [`TerminologyMembershipOutcome`],
/// preserving the membership result together with any server-provided message
/// and basic terminology metadata such as code, system, version, and display.
///
/// Malformed `Parameters` payloads produce [`ValidationError::RemoteTerminology`] with
/// [`RemoteTerminologyError::MalformedResponse`] ([`MalformedValidateCodeParameters`]).
pub fn parse_validate_vs_result(
    body: &serde_json::Value,
) -> Result<TerminologyMembershipOutcome, ValidationError> {
    if let Some(serde_json::Value::String(rt)) = body.get("resourceType") {
        if rt != "Parameters" {
            return Err(ValidationError::from(
                RemoteTerminologyError::MalformedResponse(
                    MalformedValidateCodeParameters::WrongResourceType { got: rt.clone() },
                ),
            ));
        }
    }

    let params = body
        .get("parameter")
        .and_then(|p| p.as_array())
        .ok_or_else(|| {
            ValidationError::from(RemoteTerminologyError::MalformedResponse(
                MalformedValidateCodeParameters::MissingParameterArray,
            ))
        })?;

    let mut result = None;
    let mut message = None;
    let mut diagnostics = Vec::new();
    let mut code = None;
    let mut display = None;
    let mut version = None;
    let mut system = None;

    for (index, p) in params.iter().enumerate() {
        if !p.is_object() {
            return Err(ValidationError::from(
                RemoteTerminologyError::MalformedResponse(
                    MalformedValidateCodeParameters::ParameterEntryNotObject { index },
                ),
            ));
        }

        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or_default();

        match name {
            "result" => {
                if let Some(v) = p.get("valueBoolean") {
                    if let Some(b) = v.as_bool() {
                        result = Some(b);
                    } else {
                        return Err(ValidationError::from(
                            RemoteTerminologyError::MalformedResponse(
                                MalformedValidateCodeParameters::ResultValueNotBoolean,
                            ),
                        ));
                    }
                } else if p
                    .as_object()
                    .is_some_and(|o| o.keys().any(|k| k.starts_with("value")))
                {
                    return Err(ValidationError::from(
                        RemoteTerminologyError::MalformedResponse(
                            MalformedValidateCodeParameters::ResultValueNotBoolean,
                        ),
                    ));
                }
            }
            "code" => {
                code = p
                    .get("valueString")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "display" => {
                display = p
                    .get("valueString")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "system" => {
                system = p
                    .get("valueString")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "version" => {
                version = p
                    .get("valueString")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "message" => {
                if let Some(msg) = p.get("valueString").and_then(|v| v.as_str()) {
                    let msg = msg.to_string();
                    message = Some(msg.clone());
                    diagnostics.push(msg);
                }
            }
            _ => {}
        }
    }

    match result {
        Some(is_member) => Ok(TerminologyMembershipOutcome {
            is_member,
            remote_validation_required: false,
            message,
            diagnostics,
            code,
            system,
            version,
            display,
            local_failure: None,
        }),
        None => Err(ValidationError::from(
            RemoteTerminologyError::MalformedResponse(
                MalformedValidateCodeParameters::MissingResultBoolean,
            ),
        )),
    }
}
