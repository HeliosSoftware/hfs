//! BCP-13 (RFC 2046) MIME type helpers for the FHIR `mimetypes` ValueSet.
//!
//! The HL7 `mimetypes` ValueSet composes all codes from `urn:ietf:bcp:13`, which
//! is an unbounded code system — tx.fhir.org accepts any syntactically valid
//! `type/subtype` rather than materialising every MIME type. HTS mirrors that
//! behaviour when the BCP-13 CodeSystem is not loaded locally.

use crate::types::{ValidateCodeRequest, ValidateCodeResponse, ValidationIssue};

pub const BCP13_SYSTEM: &str = "urn:ietf:bcp:13";
pub const MIMETYPES_VS_URL: &str = "http://hl7.org/fhir/ValueSet/mimetypes";

/// True when `url` is the FHIR core MimeType value set (optional `|version` suffix).
pub fn is_mimetypes_valueset_url(url: &str) -> bool {
    url.split_once('|').map(|(u, _)| u).unwrap_or(url) == MIMETYPES_VS_URL
}

/// Best-effort RFC 2045 `type/subtype` syntax check (no IANA registry lookup).
///
/// Accepts structured syntax suffixes (`application/elm+json`, `application/fhir+xml`)
/// and ignores parameter suffixes after `;`.
pub fn is_valid_mime_type(code: &str) -> bool {
    let main = code.split(';').next().unwrap_or(code);
    let Some((ty, sub)) = main.split_once('/') else {
        return false;
    };
    if ty.is_empty() || sub.is_empty() {
        return false;
    }
    fn valid_token(token: &str) -> bool {
        !token.is_empty()
            && token.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '!' | '#' | '$' | '&' | '-' | '^' | '_')
            })
    }
    valid_token(ty) && sub.split('+').all(valid_token)
}

/// When `url` is the mimetypes ValueSet, validate `req.code` by BCP-13 syntax.
///
/// Returns `Some(response)` when this special-case applies; `None` otherwise.
pub fn validate_mimetypes_code(
    url: &str,
    req: &ValidateCodeRequest,
) -> Option<ValidateCodeResponse> {
    if !is_mimetypes_valueset_url(url) {
        return None;
    }
    if let Some(sys) = req.system.as_deref().filter(|s| !s.is_empty()) {
        if sys != BCP13_SYSTEM {
            return None;
        }
    }

    let vs_qualified = if let Some(v) = req.value_set_version.as_deref().filter(|s| !s.is_empty()) {
        format!("{MIMETYPES_VS_URL}|{v}")
    } else if let Some((_, v)) = url.split_once('|') {
        format!("{MIMETYPES_VS_URL}|{v}")
    } else {
        MIMETYPES_VS_URL.to_string()
    };

    if is_valid_mime_type(&req.code) {
        return Some(ValidateCodeResponse {
            result: true,
            message: None,
            display: None,
            system: Some(BCP13_SYSTEM.into()),
            cs_version: None,
            inactive: None,
            issues: vec![],
            caused_by_unknown_system: None,
            concept_status: None,
            normalized_code: None,
        });
    }

    let not_in_vs_text = format!(
        "The provided code '#{}' was not found in the value set '{vs_qualified}'",
        req.code
    );
    Some(ValidateCodeResponse {
        result: false,
        message: Some(not_in_vs_text.clone()),
        display: None,
        system: None,
        cs_version: None,
        inactive: None,
        issues: vec![ValidationIssue {
            severity: "error".into(),
            fhir_code: "code-invalid".into(),
            tx_code: "not-in-vs".into(),
            text: not_in_vs_text,
            expression: Some("code".into()),
            location: Some("code".into()),
            message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
        }],
        caused_by_unknown_system: None,
        concept_status: None,
        normalized_code: None,
    })
}

/// MIME syntax validation for a stored mimetypes ValueSet that includes all of
/// BCP-13.
///
/// Call after resolving the stored row (including any requested date), for
/// canonical and instance requests alike, and use the response only when
/// BCP-13 is not loaded locally. Harmless compose metadata and display requests
/// keep syntax validation. Subsets, exclusions, modifiers and CodeSystem version
/// pins still require normal terminology evaluation; syntax alone cannot
/// establish their meaning.
pub fn validate_stored_mimetypes_code(
    url: &str,
    compose_json: Option<&str>,
    req: &ValidateCodeRequest,
) -> Option<ValidateCodeResponse> {
    if !is_mimetypes_valueset_url(url) || req.version.is_some() {
        return None;
    }

    let compose: serde_json::Value = serde_json::from_str(compose_json?).ok()?;
    if !compose_includes_all_bcp13(&compose) {
        return None;
    }

    validate_mimetypes_code(url, req)
}

fn compose_includes_all_bcp13(compose: &serde_json::Value) -> bool {
    let Some(compose) = compose.as_object() else {
        return false;
    };
    // Only these compose fields have known, unrestricted semantics here.
    // Inactive filtering cannot remove intrinsic MIME codes without a local
    // CodeSystem; local terminology always takes the normal evaluation path.
    if !compose.keys().all(|key| {
        matches!(
            key.as_str(),
            "include" | "exclude" | "inactive" | "id" | "extension"
        )
    }) || !has_no_elements(compose.get("exclude"))
    {
        return false;
    }
    let Some(includes) = compose.get("include").and_then(serde_json::Value::as_array) else {
        return false;
    };
    !includes.is_empty()
        && includes.iter().all(|include| {
            let Some(include) = include.as_object() else {
                return false;
            };
            include.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "system" | "concept" | "filter" | "valueSet" | "version" | "id" | "extension"
                )
            }) && include.get("system").and_then(serde_json::Value::as_str) == Some(BCP13_SYSTEM)
                && ["concept", "filter", "valueSet"]
                    .iter()
                    .all(|key| has_no_elements(include.get(*key)))
                && match include.get("version") {
                    None | Some(serde_json::Value::Null) => true,
                    Some(serde_json::Value::String(version)) => version.is_empty(),
                    _ => false,
                }
        })
}

fn has_no_elements(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::Array(elements)) => elements.is_empty(),
        _ => false,
    }
}

/// Build a canonical mimetypes VS URL from a ValueSet resource JSON body.
pub fn mimetypes_url_from_resource(vs: &serde_json::Value) -> Option<String> {
    let base = vs.get("url")?.as_str()?;
    if !is_mimetypes_valueset_url(base) {
        return None;
    }
    let version = vs
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    Some(match version {
        Some(v) => format!("{base}|{v}"),
        None => base.to_string(),
    })
}

/// True when compose.include references only `urn:ietf:bcp:13` (mimetypes VS shape).
pub fn compose_is_bcp13_only(vs: &serde_json::Value) -> bool {
    let Some(includes) = vs
        .get("compose")
        .and_then(|c| c.get("include"))
        .and_then(|i| i.as_array())
    else {
        return false;
    };
    !includes.is_empty()
        && includes.iter().all(|inc| {
            inc.get("system")
                .and_then(|s| s.as_str())
                .map(|s| s == BCP13_SYSTEM)
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_ig_mime_types() {
        for code in [
            "text/cql",
            "application/elm+json",
            "application/xml",
            "application/fhir+json",
        ] {
            assert!(is_valid_mime_type(code), "{code}");
        }
    }

    #[test]
    fn rejects_obviously_invalid_mime_types() {
        assert!(!is_valid_mime_type(""));
        assert!(!is_valid_mime_type("not-a-mime"));
        assert!(!is_valid_mime_type("/subtype"));
        assert!(!is_valid_mime_type("type/"));
    }

    fn mimetypes_request() -> ValidateCodeRequest {
        ValidateCodeRequest {
            selected_value_set_id: None,
            url: Some(format!("{MIMETYPES_VS_URL}|4.0.1")),
            value_set_version: Some("4.0.1".into()),
            system: None,
            code: "text/cql".into(),
            version: None,
            display: None,
            date: None,
            include_abstract: None,
            input_form: None,
            lenient_display_validation: None,
            default_value_set_versions: Default::default(),
        }
    }

    #[test]
    fn mimetypes_validate_success_without_system() {
        let req = mimetypes_request();
        let resp = validate_mimetypes_code(MIMETYPES_VS_URL, &req).unwrap();
        assert!(resp.result);
        assert_eq!(resp.system.as_deref(), Some(BCP13_SYSTEM));
    }

    #[test]
    fn stored_whole_system_uses_mime_syntax_and_requested_version() {
        let compose = r#"{"include":[{"system":"urn:ietf:bcp:13"}]}"#;
        let mut req = mimetypes_request();
        let response = validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(compose), &req)
            .expect("unrestricted stored compose");
        assert!(response.result);
        assert_eq!(response.system.as_deref(), Some(BCP13_SYSTEM));

        req.code = "invalid".into();
        let response = validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(compose), &req)
            .expect("invalid syntax is still handled");
        assert!(!response.result);
        assert!(response.message.unwrap().contains("mimetypes|4.0.1"));

        req.value_set_version = None;
        let response = validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(compose), &req)
            .expect("unversioned stored compose");
        assert!(!response.message.unwrap().contains("mimetypes|"));
    }

    #[test]
    fn stored_mime_restrictions_require_normal_evaluation() {
        let req = mimetypes_request();
        let unrestricted = serde_json::json!({"include": [{"system": BCP13_SYSTEM}]});
        for (field, value) in [
            ("concept", serde_json::json!([{"code": "application/json"}])),
            (
                "filter",
                serde_json::json!([{"property": "code", "op": "=", "value": "text/cql"}]),
            ),
            (
                "valueSet",
                serde_json::json!(["http://example.org/restriction"]),
            ),
            ("version", serde_json::json!("1.0")),
            ("modifierExtension", serde_json::json!([])),
        ] {
            let mut compose = unrestricted.clone();
            compose["include"][0][field] = value;
            assert!(
                validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(&compose.to_string()), &req)
                    .is_none(),
                "include.{field}"
            );
        }
        for (field, value) in [
            (
                "exclude",
                serde_json::json!([{"system": BCP13_SYSTEM, "concept": [{"code": "text/cql"}]}]),
            ),
            ("lockedDate", serde_json::json!("2020-01-01")),
            ("modifierExtension", serde_json::json!([])),
        ] {
            let mut compose = unrestricted.clone();
            compose[field] = value;
            assert!(
                validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(&compose.to_string()), &req)
                    .is_none(),
                "compose.{field}"
            );
        }
        for compose in [
            "null",
            "{}",
            "{",
            r#"{"include":[]}"#,
            r#"{"include":[{"system":"other"}]}"#,
        ] {
            assert!(
                validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(compose), &req).is_none(),
                "{compose}"
            );
        }
        assert!(validate_stored_mimetypes_code(MIMETYPES_VS_URL, None, &req).is_none());
    }

    #[test]
    fn stored_mime_metadata_display_and_resolved_date_preserve_syntax_validation() {
        let compose = serde_json::json!({
            "id": "compose-metadata",
            "extension": [{"url": "http://example.org/note", "valueString": "metadata"}],
            "inactive": false,
            "exclude": [],
            "include": [{
                "system": BCP13_SYSTEM,
                "id": "include-metadata",
                "extension": [{"url": "http://example.org/note", "valueString": "metadata"}],
                "concept": [],
                "filter": [],
                "valueSet": [],
                "version": ""
            }]
        });
        let mut req = mimetypes_request();
        req.display = Some("Clinical Quality Language".into());
        req.date = Some("2020-01-01".into());
        let response =
            validate_stored_mimetypes_code(MIMETYPES_VS_URL, Some(&compose.to_string()), &req)
                .expect("metadata and already-resolved date do not restrict membership");
        assert!(response.result);
        assert_eq!(response.system.as_deref(), Some(BCP13_SYSTEM));
    }

    #[test]
    fn stored_mime_request_constraints_require_normal_evaluation() {
        let compose = Some(r#"{"include":[{"system":"urn:ietf:bcp:13"}]}"#);

        let mut versioned = mimetypes_request();
        versioned.version = Some("unknown".into());
        assert!(validate_stored_mimetypes_code(MIMETYPES_VS_URL, compose, &versioned).is_none());

        let mut other_system = mimetypes_request();
        other_system.system = Some("http://example.org/other".into());
        assert!(validate_stored_mimetypes_code(MIMETYPES_VS_URL, compose, &other_system).is_none());

        let other_value_set = "http://example.org/other";
        let req = mimetypes_request();
        assert!(validate_stored_mimetypes_code(other_value_set, compose, &req).is_none());
    }
}
