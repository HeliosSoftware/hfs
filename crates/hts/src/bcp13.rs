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
    url.split_once('|')
        .map(|(u, _)| u)
        .unwrap_or(url)
        == MIMETYPES_VS_URL
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
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '!' | '#' | '$' | '&' | '-' | '^' | '_'))
    }
    valid_token(ty) && sub.split('+').all(valid_token)
}

/// When `url` is the mimetypes ValueSet, validate `req.code` by BCP-13 syntax.
///
/// Returns `Some(response)` when this special-case applies; `None` otherwise.
pub fn validate_mimetypes_code(url: &str, req: &ValidateCodeRequest) -> Option<ValidateCodeResponse> {
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
        for code in ["text/cql", "application/elm+json", "application/xml", "application/fhir+json"] {
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

    #[test]
    fn mimetypes_validate_success_without_system() {
        let req = ValidateCodeRequest {
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
        };
        let resp = validate_mimetypes_code(MIMETYPES_VS_URL, &req).unwrap();
        assert!(resp.result);
        assert_eq!(resp.system.as_deref(), Some(BCP13_SYSTEM));
    }
}
