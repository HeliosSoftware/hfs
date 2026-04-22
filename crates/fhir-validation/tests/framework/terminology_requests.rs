use fhir_validation::terminology::requests::ValidateVsRequest;
use helios_fhir::PrecisionDateTime;
use serde_json::json;

fn parameters(req: &ValidateVsRequest) -> Vec<serde_json::Value> {
    req.to_parameters_json()
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn has_param(params: &[serde_json::Value], name: &str) -> bool {
    params
        .iter()
        .any(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))
}

fn find_param<'a>(params: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))
        .unwrap_or_else(|| panic!("missing parameter: {name}"))
}

#[test]
fn validate_accepts_code_with_system() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        system: Some("http://example.org/CodeSystem/test".into()),
        ..Default::default()
    };

    assert!(req.validate().is_ok());
}

#[test]
fn validate_accepts_code_with_context() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        context: Some("http://example.org/context".into()),
        ..Default::default()
    };

    assert!(req.validate().is_ok());
}

#[test]
fn validate_accepts_code_with_infer_system() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        infer_system: Some(true),
        ..Default::default()
    };

    assert!(req.validate().is_ok());
}

#[test]
fn validate_rejects_code_without_system_context_or_infer_system() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        ..Default::default()
    };

    let err = req.validate().unwrap_err();
    assert!(
        err.contains("either system, context, or infer_system=true should be set"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_rejects_multiple_code_inputs() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        coding: Some(json!({
            "system": "http://example.org/CodeSystem/test",
            "code": "x"
        })),
        system: Some("http://example.org/CodeSystem/test".into()),
        ..Default::default()
    };

    let err = req.validate().unwrap_err();
    assert!(
        err.contains("Exactly one of code, coding, or codeable_concept must be set"),
        "unexpected error: {err}"
    );
}

#[test]
fn to_parameters_json_emits_url_code_and_system() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        system: Some("http://example.org/CodeSystem/test".into()),
        ..Default::default()
    };

    let params = parameters(&req);

    assert!(has_param(&params, "url"));
    assert!(has_param(&params, "code"));
    assert!(has_param(&params, "system"));

    let url = find_param(&params, "url");
    assert_eq!(
        url.get("valueUri").and_then(|v| v.as_str()),
        Some("http://example.org/ValueSet/test")
    );

    let code = find_param(&params, "code");
    assert_eq!(code.get("valueCode").and_then(|v| v.as_str()), Some("male"));

    let system = find_param(&params, "system");
    assert_eq!(
        system.get("valueUri").and_then(|v| v.as_str()),
        Some("http://example.org/CodeSystem/test")
    );
}

#[test]
fn to_parameters_json_emits_context() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        context: Some("http://example.org/context".into()),
        ..Default::default()
    };

    let params = parameters(&req);

    let context = find_param(&params, "context");
    assert_eq!(
        context.get("valueUri").and_then(|v| v.as_str()),
        Some("http://example.org/context")
    );
}

#[test]
fn to_parameters_json_emits_manifest_and_tx_resource() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        context: Some("http://example.org/context".into()),
        manifest: Some("http://example.org/Library/manifest".into()),
        tx_resource: Some(vec![
            json!({ "resourceType": "ValueSet" }),
            json!({ "resourceType": "CodeSystem" }),
        ]),
        ..Default::default()
    };

    let params = parameters(&req);

    let manifest = find_param(&params, "manifest");
    assert_eq!(
        manifest.get("valueCanonical").and_then(|v| v.as_str()),
        Some("http://example.org/Library/manifest")
    );

    let tx_resources: Vec<_> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("tx-resource"))
        .collect();

    assert_eq!(tx_resources.len(), 2);
    assert!(tx_resources.iter().all(|p| p.get("resource").is_some()));
}

#[test]
fn to_parameters_json_emits_version_parameters() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        context: Some("http://example.org/context".into()),
        system_version2: Some(vec![
            "http://example.org/CodeSystem/test|1.0.0".into(),
            "http://example.org/CodeSystem/test|2.0.0".into(),
        ]),
        check_system_version: Some(vec![
            "http://example.org/CodeSystem/test|1.0.0".into(),
            "http://example.org/CodeSystem/test|2.0.0".into(),
        ]),
        default_valueset_version: Some(vec!["http://example.org/ValueSet/test|1.0.0".into()]),
        check_valueset_version: Some(vec!["http://example.org/ValueSet/test|2.0.0".into()]),
        force_valueset_version: Some(vec!["http://example.org/ValueSet/test|3.0.0".into()]),
        ..Default::default()
    };

    let params = parameters(&req);

    let system_version_count = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("system-version"))
        .count();
    assert_eq!(system_version_count, 2);

    let check_system_version_count = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("check-system-version"))
        .count();
    assert_eq!(check_system_version_count, 2);

    let default_valueset_version_count = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("default-valueset-version"))
        .count();
    assert_eq!(default_valueset_version_count, 1);

    let check_valueset_version_count = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("check-valueset-version"))
        .count();
    assert_eq!(check_valueset_version_count, 1);

    let force_valueset_version_count = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("force-valueset-version"))
        .count();
    assert_eq!(force_valueset_version_count, 1);
}

#[test]
fn to_parameters_json_emits_date_and_flags() {
    let req = ValidateVsRequest {
        valueset_url: "http://example.org/ValueSet/test".into(),
        code: Some("male".into()),
        context: Some("http://example.org/context".into()),
        date: PrecisionDateTime::parse("2026-03-28T13:00:00Z"),
        abstract_ok: Some(true),
        infer_system: Some(true),
        lenient_display_validation: Some(true),
        valueset_membership_only: Some(true),
        ..Default::default()
    };

    let params = parameters(&req);

    let date = find_param(&params, "date");
    assert_eq!(
        date.get("valueDateTime").and_then(|v| v.as_str()),
        Some("2026-03-28T13:00:00Z")
    );

    let abstract_param = find_param(&params, "abstract");
    assert_eq!(
        abstract_param.get("valueBoolean").and_then(|v| v.as_bool()),
        Some(true)
    );

    let infer_system = find_param(&params, "inferSystem");
    assert_eq!(
        infer_system.get("valueBoolean").and_then(|v| v.as_bool()),
        Some(true)
    );

    let lenient = find_param(&params, "lenient-display-validation");
    assert_eq!(
        lenient.get("valueBoolean").and_then(|v| v.as_bool()),
        Some(true)
    );

    let membership_only = find_param(&params, "valueset-membership-only");
    assert_eq!(
        membership_only
            .get("valueBoolean")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}
