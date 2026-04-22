//! Tests for terminology helper error mapping and `$validate-code` parsing.

use fhir_validation::terminology::helpers::{
    build_remote_terminology_error, parse_validate_vs_result,
    terminology_remote_from_fhir_path_error,
};
use fhir_validation::{
    MalformedValidateCodeParameters, RemoteTerminologyError, TerminologyRequestInvalid,
    ValidationError,
};
use helios_fhir::TerminologyValidationError;
use helios_fhirpath::error::FhirPathError;
use helios_fhirpath_support::EvaluationError;
use serde_json::json;

#[test]
fn fhir_path_validation_error_chains_source() {
    let inner = EvaluationError::TypeError("x".to_string());
    let err = ValidationError::FhirPath(inner.clone());
    let src = std::error::Error::source(&err).expect("source");
    assert_eq!(src.to_string(), inner.to_string());
}

#[test]
fn local_terminology_validation_error_chains_source() {
    let inner = TerminologyValidationError::InvalidInput("bad".to_string());
    let err = ValidationError::LocalTerminology(inner.clone());
    let src = std::error::Error::source(&err).expect("source");
    assert_eq!(src.to_string(), inner.to_string());
}

#[test]
fn terminology_remote_from_http_error_extracts_status_and_body() {
    let body = r#"{"resourceType":"OperationOutcome","issue":[{"diagnostics":"bad"}]}"#;
    let err = FhirPathError::HttpError(502, body.to_string());
    let remote = terminology_remote_from_fhir_path_error(&err);
    assert_eq!(remote.status, Some(502));
    assert_eq!(remote.diagnostics, vec!["bad".to_string()]);
    assert_eq!(remote.raw_body.as_deref(), Some(body));
}

#[test]
fn terminology_remote_from_validate_code_message_parses_status_prefix() {
    let msg = "ValueSet validation failed with status 404 Not Found: {\"resourceType\":\"OperationOutcome\"}";
    let err = FhirPathError::TerminologyError(msg.to_string());
    let remote = terminology_remote_from_fhir_path_error(&err);
    assert_eq!(remote.status, Some(404));
    assert_eq!(
        remote.raw_body.as_deref(),
        Some(r#"{"resourceType":"OperationOutcome"}"#)
    );
}

#[test]
fn parse_validate_vs_result_ok() {
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "result", "valueBoolean": true},
            {"name": "message", "valueString": "ok"}
        ]
    });
    let out = parse_validate_vs_result(&body).unwrap();
    assert!(out.is_member);
    assert_eq!(out.message.as_deref(), Some("ok"));
}

#[test]
fn parse_validate_vs_result_ok_without_resource_type() {
    let body = json!({
        "parameter": [
            {"name": "result", "valueBoolean": false}
        ]
    });
    let out = parse_validate_vs_result(&body).unwrap();
    assert!(!out.is_member);
}

#[test]
fn parse_validate_vs_result_wrong_resource_type() {
    let body = json!({
        "resourceType": "OperationOutcome",
        "parameter": []
    });
    let err = parse_validate_vs_result(&body).unwrap_err();
    match err {
        ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
            MalformedValidateCodeParameters::WrongResourceType { got },
        )) if got == "OperationOutcome" => {}
        _ => panic!("unexpected {err:?}"),
    }
}

#[test]
fn parse_validate_vs_result_parameter_entry_not_object() {
    let body = json!({
        "resourceType": "Parameters",
        "parameter": ["not-an-object"]
    });
    let err = parse_validate_vs_result(&body).unwrap_err();
    match err {
        ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
            MalformedValidateCodeParameters::ParameterEntryNotObject { index: 0 },
        )) => {}
        _ => panic!("unexpected {err:?}"),
    }
}

#[test]
fn parse_validate_vs_result_result_not_boolean() {
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "result", "valueString": "true"}]
    });
    let err = parse_validate_vs_result(&body).unwrap_err();
    match err {
        ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
            MalformedValidateCodeParameters::ResultValueNotBoolean,
        )) => {}
        _ => panic!("unexpected {err:?}"),
    }
}

#[test]
fn invalid_request_chains_source() {
    let inner = TerminologyRequestInvalid {
        message: "bad request".to_string(),
    };
    let err = ValidationError::InvalidRequest(inner.clone());
    let src = std::error::Error::source(&err).expect("source");
    assert_eq!(src.to_string(), inner.message);
}

#[test]
fn validation_error_as_remote_malformed_parameters() {
    let err = ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
        MalformedValidateCodeParameters::MissingParameterArray,
    ));
    assert_eq!(
        err.as_remote_malformed_parameters(),
        Some(&MalformedValidateCodeParameters::MissingParameterArray)
    );
}

#[test]
fn parse_validate_vs_result_missing_parameter_array() {
    let body = json!({"resourceType": "Parameters"});
    let err = parse_validate_vs_result(&body).unwrap_err();
    match err {
        ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
            MalformedValidateCodeParameters::MissingParameterArray,
        )) => {}
        _ => panic!("unexpected {err:?}"),
    }
}

#[test]
fn parse_validate_vs_result_missing_result() {
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "message", "valueString": "x"}]
    });
    let err = parse_validate_vs_result(&body).unwrap_err();
    match err {
        ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
            MalformedValidateCodeParameters::MissingResultBoolean,
        )) => {}
        _ => panic!("unexpected {err:?}"),
    }
}

#[test]
fn build_remote_terminology_error_legacy_json_suffix() {
    let msg = r#"upstream status 500: {"resourceType":"OperationOutcome","issue":[{"diagnostics":"legacy"}]}"#;
    let remote = build_remote_terminology_error(msg);
    assert_eq!(remote.diagnostics, vec!["legacy".to_string()]);
}
