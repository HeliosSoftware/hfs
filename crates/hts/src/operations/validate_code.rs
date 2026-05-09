//! Handlers for `POST /CodeSystem/$validate-code` and
//! `POST /ValueSet/$validate-code`.
//!
//! Both operations accept a FHIR Parameters resource and return a FHIR
//! Parameters resource with a boolean `result`, optional `message`, and
//! optional `display`.
//!
//! **CodeSystem/$validate-code** requires the `url` parameter (CodeSystem
//! canonical URL). Sending `system` instead returns HTTP 400.
//!
//! **ValueSet/$validate-code** requires the `url` parameter (ValueSet
//! canonical URL) and optionally accepts `system` (to scope the lookup to a
//! specific code system within the expanded value set).
//!
//! # FHIR specifications
//! - CodeSystem: <https://hl7.org/fhir/codesystem-operation-validate-code.html>
//! - ValueSet:   <https://hl7.org/fhir/valueset-operation-validate-code.html>
use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use std::sync::Arc;

use crate::error::HtsError;
use crate::state::{AppState, VALIDATE_CODE_HANDLER_CACHE_MAX, ValidateCodeHandlerCache};
use crate::traits::{CodeSystemOperations, SupplementInfo, TerminologyBackend, ValueSetOperations};
use crate::types::{ValidateCodeRequest, ValidateCodeResponse, ValidationIssue};

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    collect_canonical_params, extract_codeable_concept, extract_coding_full,
    extract_parameter_array, find_resource_param, find_str_param, parse_query_string,
    query_params_to_fhir_params,
};

/// Identifies which FHIR `$validate-code` input form the operations layer is
/// rendering a response for. Used to keep `OperationOutcome.issue.location`
/// on each emitted issue aligned with the FHIRPath the IG fixtures expect:
/// the bare-code path uses `code` / `system`, while the Coding and
/// CodeableConcept paths use `Coding.code` / `Coding.system`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestPath {
    /// `code` (+ optional `system`/`version`/`display`) parameter.
    BareCode,
    /// `coding` (`valueCoding`) parameter.
    Coding,
    /// `codeableConcept` (`valueCodeableConcept`) parameter.
    CodeableConcept,
}

/// Render a single [`ValidationIssue`] as a FHIR `OperationOutcome.issue`.
fn render_issue(issue: &ValidationIssue) -> Value {
    let mut json_issue = json!({
        "severity": issue.severity,
        "code": issue.fhir_code,
        "details": {
            "coding": [{
                "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                "code": issue.tx_code,
            }],
            "text": issue.text,
        }
    });
    if let Some(msg_id) = issue.message_id.as_deref() {
        json_issue.as_object_mut().unwrap().insert(
            "extension".into(),
            json!([{
                "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                "valueString": msg_id
            }]),
        );
    }
    if let Some(loc) = issue.location.as_deref() {
        json_issue
            .as_object_mut()
            .unwrap()
            .insert("location".into(), json!([loc]));
    }
    if let Some(expr) = issue.expression.as_deref() {
        json_issue
            .as_object_mut()
            .unwrap()
            .insert("expression".into(), json!([expr]));
    }
    json_issue
}

/// Serialize a [`ValidateCodeResponse`] into a FHIR Parameters JSON value.
///
/// Always includes `result` (boolean). When `resp.issues` is non-empty (or
/// `unknown_system` is supplied), wraps every concern in a multi-entry
/// `OperationOutcome` under the `issues` parameter and joins the issue
/// texts (alphabetically, semicolon-separated) into the top-level `message`
/// parameter — matching the IG tx-ecosystem fixture convention. Falls back
/// to the legacy single-issue path when only `resp.message` is set.
///
/// Echoes `code`, `system`, and `version` (when known) so the IG fixtures
/// can confirm what we validated.
fn build_validate_response(
    resp: ValidateCodeResponse,
    code: Option<&str>,
    system: Option<&str>,
    version: Option<&str>,
    codeable_concept: Option<&Value>,
    unknown_system: Option<&str>,
    request_path: RequestPath,
) -> Value {
    build_validate_response_inner(
        resp,
        code,
        system,
        version,
        codeable_concept,
        unknown_system,
        request_path,
        false,
    )
}

/// Like `build_validate_response`, but with a flag indicating whether the
/// synthesized `UNKNOWN_CODESYSTEM` issue should omit its `location` field.
/// Used by `build_validate_response_async` for the IG `regex-bad/validate-
/// regex-bad` and similar fixtures where the caller's `system` URL is not in
/// the resolved VS's `compose.include[].system` list — those expect only
/// `expression` on the UNKNOWN_CODESYSTEM issue, not `location`.
fn build_validate_response_inner(
    resp: ValidateCodeResponse,
    code: Option<&str>,
    system: Option<&str>,
    version: Option<&str>,
    codeable_concept: Option<&Value>,
    unknown_system: Option<&str>,
    request_path: RequestPath,
    suppress_unknown_system_location: bool,
) -> Value {
    let mut parameter: Vec<Value> = Vec::new();
    // For the CodeableConcept request path: when the validation produced an
    // `UNKNOWN_CODESYSTEM_VERSION` issue whose caused-by canonical names a
    // version that doesn't appear in any of the input codings, the unknown
    // version came from the VS include's bad pin (not the caller's data).
    // The IG fixtures (`codeableconcept-v10-vs1wb-*`, `codeableconcept-vnn-vs1wb-*`)
    // drop `code` / `system` / `version` echoes in this case because those
    // values describe the VS structure, not the caller's input. When the
    // unknown version IS the caller's coding version (e.g. `vbb-vs10` with
    // coding.version=2.4.0), the echoes ARE expected — the IG `vbb-vs10-*`
    // fixtures show the coding's bad version + the VS's resolved version.
    let suppress_cc_echoes = matches!(request_path, RequestPath::CodeableConcept)
        && resp
            .issues
            .iter()
            .any(|i| i.message_id.as_deref() == Some("UNKNOWN_CODESYSTEM_VERSION"))
        && {
            let coding_versions: Vec<String> = codeable_concept
                .and_then(|cc| cc.get("coding"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            c.get("version")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Extract the version part of `caused_by_unknown_system` (after
            // the `|`). When it doesn't match any coding's version, the
            // failure is from the VS include's bad pin.
            let caused_version: Option<String> =
                resp.caused_by_unknown_system.as_deref().and_then(|c| {
                    c.split_once('|').map(|(_, v)| v.to_string())
                });
            match caused_version {
                Some(v) => !coding_versions.iter().any(|cv| cv == &v),
                None => true,
            }
        };
    if let Some(c) = code {
        if !suppress_cc_echoes {
            parameter.push(json!({"name": "code", "valueCode": c}));
        }
    }
    if let Some(cc) = codeable_concept {
        parameter.push(json!({"name": "codeableConcept", "valueCodeableConcept": cc}));
    }
    if let Some(display) = resp.display {
        parameter.push(json!({"name": "display", "valueString": display}));
    }
    // The IG fixtures expect a top-level `inactive` parameter when the
    // validated concept is inactive (status retired/deprecated/withdrawn/
    // inactive); kept alphabetical between display and issues.
    if resp.inactive == Some(true) {
        parameter.push(json!({"name": "inactive", "valueBoolean": true}));
    }
    // Compose the issue list: backend-provided issues first, then synthesise
    // an `unknown CodeSystem` issue from the operations layer when the input
    // system isn't stored. The IG fixtures (e.g.
    // validation/simple-coding-bad-system) expect both a `code-invalid` /
    // `not-in-vs` issue (from the backend) AND a `not-found` / `not-found`
    // issue pointing at the unknown CodeSystem URL.
    let mut issues: Vec<ValidationIssue> = resp.issues.clone();
    // Rewrite Coding.X locations to bare X for the bare-code request path
    // (per IG `validation-simple-code-bad-code`: location is `code` not
    // `Coding.code` when there is no Coding wrapper in the request).
    if matches!(request_path, RequestPath::BareCode) {
        for issue in &mut issues {
            // Rewrite FHIRPath expression paths for bare-code requests:
            // `Coding.code` → `code`, `Coding.system` → `system`, `Coding` → drop.
            for field in [&mut issue.expression, &mut issue.location] {
                if let Some(path) = field.as_deref() {
                    if let Some(stripped) = path.strip_prefix("Coding.") {
                        *field = Some(stripped.to_string());
                    } else if path == "Coding" {
                        *field = None;
                    }
                }
            }
        }
    }
    // For the CodeableConcept request path, rewrite `Coding.X` → `CodeableConcept.coding[0].X`
    // for issues that don't already use a `CodeableConcept.*` path. The IG
    // `validation/simple-codeableconcept-bad-display*` fixtures expect the
    // CodeableConcept-rooted path on display-mismatch issues.
    if matches!(request_path, RequestPath::CodeableConcept) {
        for issue in &mut issues {
            for field in [&mut issue.expression, &mut issue.location] {
                if let Some(path) = field.as_deref() {
                    if let Some(stripped) = path.strip_prefix("Coding.") {
                        *field = Some(format!("CodeableConcept.coding[0].{stripped}"));
                    }
                }
            }
        }
    }
    if let Some(unknown) = unknown_system {
        // The IG fixtures (regex-bad, errors/unknown-system1) consistently
        // wrap the unknown CodeSystem URL in single quotes regardless of
        // whether it's absolute (http://, urn:) or relative.  Relative
        // references additionally trigger the Terminology_TX_System_Relative
        // companion issue below.
        let is_absolute = unknown.starts_with("http://")
            || unknown.starts_with("https://")
            || unknown.starts_with("urn:");
        let text = format!(
            "A definition for CodeSystem '{unknown}' could not be found, so the code cannot be validated"
        );
        let expression = match request_path {
            RequestPath::BareCode => "system".to_string(),
            _ => "Coding.system".to_string(),
        };
        issues.push(ValidationIssue {
            severity: "error".into(),
            fhir_code: "not-found".into(),
            tx_code: "not-found".into(),
            text,
            expression: Some(expression.clone()),
            location: if suppress_unknown_system_location {
                None
            } else {
                Some(expression)
            },
            message_id: Some("UNKNOWN_CODESYSTEM".into()),
        });
        // Local/relative reference: also emit Terminology_TX_System_Relative.
        if !is_absolute {
            let rel_expr = match request_path {
                RequestPath::BareCode => "system".to_string(),
                _ => "Coding.system".to_string(),
            };
            issues.push(ValidationIssue {
                severity: "error".into(),
                fhir_code: "invalid".into(),
                tx_code: "invalid-data".into(),
                text: "Coding.system must be an absolute reference, not a local reference"
                    .to_string(),
                expression: Some(rel_expr.clone()),
                location: Some(rel_expr),
                message_id: Some("Terminology_TX_System_Relative".into()),
            });
        }
    }

    // Determine the message string: when we have structured issues, sort
    // their texts alphabetically and join with `; ` (matches the IG fixture
    // convention). When we don't, fall back to the response's own `message`
    // (legacy single-message path used by older code in $translate, etc.).
    // Error-severity issues always contribute to the top-level `message`.
    // Inactive/status warnings (`INACTIVE_CONCEPT_FOUND`) also contribute —
    // the IG `inactive/validate-inactive-*` fixtures expect their text in
    // the top-level `message` parameter even though they are warnings.
    // Lenient-display-validation warnings also contribute — the IG
    // `validation/simple-coding-bad-displayW` fixture echoes the warning
    // text in `message` even though severity=warning.
    // Info-severity language warnings (NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_*)
    // also contribute — IG `display/validation-right-de-en` etc. expect them.
    let message_str: Option<String> = if !issues.is_empty() {
        let mut texts: Vec<&str> = issues
            .iter()
            .filter(|i| {
                i.severity == "error"
                    || (i.severity == "warning"
                        && (i.message_id.as_deref() == Some("INACTIVE_CONCEPT_FOUND")
                            || i.message_id.as_deref() == Some("DEPRECATED_CONCEPT_FOUND")
                            || i.message_id.as_deref()
                                == Some("Coding_has_no_system__cannot_validate")
                            || i.tx_code == "invalid-display"))
                    || (i.severity == "information"
                        && i.message_id.as_deref().is_some_and(|m| {
                            m == "NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_OK"
                        }))
            })
            .map(|i| i.text.as_str())
            .collect();
        if texts.is_empty() {
            None
        } else {
            texts.sort();
            Some(texts.join("; "))
        }
    } else {
        resp.message.clone()
    };

    if !issues.is_empty() {
        let oo_issues: Vec<Value> = issues.iter().map(render_issue).collect();
        parameter.push(json!({
            "name": "issues",
            "resource": {
                "resourceType": "OperationOutcome",
                "issue": oo_issues,
            }
        }));
    } else if let Some(msg) = message_str.as_deref() {
        // Legacy fallback: no structured issues but we still have a message
        // (e.g. an unknown ValueSet path in postgres backend). Emit a single
        // catch-all OperationOutcome so the response shape stays compatible
        // with older fixture matchers.
        let (issue_code, tx_code) = if resp.result {
            ("invalid", "invalid-display")
        } else {
            ("code-invalid", "not-in-vs")
        };
        let severity = if resp.result { "warning" } else { "error" };
        parameter.push(json!({
            "name": "issues",
            "resource": {
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": severity,
                    "code": issue_code,
                    "details": {
                        "coding": [{
                            "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                            "code": tx_code,
                        }],
                        "text": msg,
                    },
                    "expression": ["Coding.code"],
                }]
            }
        }));
    }
    if let Some(msg) = message_str.as_deref() {
        parameter.push(json!({"name": "message", "valueString": msg}));
    }
    // `normalized-code` parameter — surfaces the canonical-case code when the
    // backend matched via the `caseSensitive: false` fallback. The IG
    // `case/case-coding-insensitive-code1-{2,3}` fixtures place this between
    // `issues` and `result` in the response Parameters.
    if let Some(ref nc) = resp.normalized_code {
        parameter.push(json!({"name": "normalized-code", "valueCode": nc}));
    }
    // result is driven by error-severity issues when we have any; otherwise
    // honour the backend's `resp.result`.
    let final_result = if issues.is_empty() {
        resp.result
    } else {
        !issues.iter().any(|i| i.severity == "error")
    };
    parameter.push(json!({"name": "result", "valueBoolean": final_result}));
    // `status` parameter — surfaces the concept's standards-status extension
    // (e.g. "deprecated", "withdrawn"). The IG `extensions/validate-code-inactive`
    // fixture echoes this between `result` and `system`.
    if let Some(ref status) = resp.concept_status {
        parameter.push(json!({"name": "status", "valueCode": status}));
    }
    if let Some(s) = system {
        // Suppress for CC path with UNKNOWN_CODESYSTEM_VERSION
        // (see `suppress_cc_echoes` above for rationale).
        if !suppress_cc_echoes {
            parameter.push(json!({"name": "system", "valueUri": s}));
        }
    }
    if let Some(v) = version {
        if !suppress_cc_echoes {
            parameter.push(json!({"name": "version", "valueString": v}));
        }
    }
    if let Some(u) = unknown_system {
        parameter.push(json!({"name": "x-unknown-system", "valueCanonical": u}));
    }
    if let Some(ref canonical) = resp.caused_by_unknown_system {
        parameter.push(json!({"name": "x-caused-by-unknown-system", "valueCanonical": canonical}));
    }
    json!({
        "resourceType": "Parameters",
        "parameter": parameter
    })
}

/// Look up the `status` property of a concept (e.g. `retired`, `deprecated`,
/// `withdrawn`, `inactive`). Returns `None` when the concept has no status
/// Extract a VS-implied `displayLanguage` from a stored ValueSet resource.
///
/// The IG `validation/simple-coding-bad-language-vs(lang)` fixtures attach a
/// language constraint to the VS itself rather than supplying `displayLanguage`
/// in the request. Two sources are checked, in priority order:
///
/// 1. `compose.extension[valueset-expansion-parameter]` whose nested `name=displayLanguage`
///    extension carries the language code (`valueCode`).
/// 2. Top-level `ValueSet.language` — used when no expansion-parameter override exists.
///
/// Returns `None` when the VS is absent or carries no language hint.
fn vs_implied_display_language(vs: &Value) -> Option<String> {
    // 1. compose.extension[valueset-expansion-parameter] -> displayLanguage
    if let Some(exts) = vs
        .get("compose")
        .and_then(|c| c.get("extension"))
        .and_then(|e| e.as_array())
    {
        for ext in exts {
            let url_match = ext.get("url").and_then(|u| u.as_str())
                == Some("http://hl7.org/fhir/StructureDefinition/valueset-expansion-parameter");
            if !url_match {
                continue;
            }
            // Nested extension array: { name: "displayLanguage" } + { value: "<code>" }.
            let inner = match ext.get("extension").and_then(|e| e.as_array()) {
                Some(a) => a,
                None => continue,
            };
            let mut is_display_language = false;
            let mut lang_value: Option<String> = None;
            for sub in inner {
                let sub_url = sub.get("url").and_then(|u| u.as_str()).unwrap_or("");
                match sub_url {
                    "name" => {
                        if sub.get("valueCode").and_then(|v| v.as_str()) == Some("displayLanguage")
                        {
                            is_display_language = true;
                        }
                    }
                    "value" => {
                        if let Some(s) = sub
                            .get("valueCode")
                            .and_then(|v| v.as_str())
                            .or_else(|| sub.get("valueString").and_then(|v| v.as_str()))
                        {
                            lang_value = Some(s.to_string());
                        }
                    }
                    _ => {}
                }
            }
            if is_display_language {
                if let Some(v) = lang_value {
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
    }
    // 2. Top-level ValueSet.language.
    if let Some(s) = vs.get("language").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// property, when the property value is `active` or `inactive` (the generic
/// status), or when the lookup fails. Used to drive the second
/// "has a status of <X>" warning for non-`inactive` inactive concepts.
async fn lookup_concept_status<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system: &str,
    code: &str,
) -> Option<String> {
    let req = crate::types::LookupRequest {
        system: system.to_string(),
        code: code.to_string(),
        version: None,
        display_language: None,
        expression: None,
        properties: vec!["status".to_string()],
        date: None,
        use_supplements: vec![],
    };
    let resp = CodeSystemOperations::lookup(backend, ctx, req).await.ok()?;
    for prop in resp.properties {
        if prop.code == "status" {
            let status = prop.value;
            if status != "active" && status != "inactive" && !status.is_empty() {
                return Some(status);
            }
        }
    }
    None
}

/// Apply IG-style language-aware display validation to a `validate-code`
/// response.
///
/// Rewrites or inserts the `invalid-display` issue using the canonical
/// wording the IG fixtures expect (`display/`, `language2/`, `validation/`):
///
///   "Wrong Display Name 'X' for <system>#<code>. Valid display is 'Y' (lang)
///    (for the language(s) 'L')"
///
/// or, when `displayLanguage` was requested but the CodeSystem has no
/// designation in that language and the supplied display does match the
/// default-language display:
///
///   "There are no valid display names found for the code <system>#<code>
///    for language(s) 'L'. The display is 'Y' which is the default language
///    display"  (info severity, `NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_OK`)
///
/// or, when no display in the requested language exists AND the supplied
/// display doesn't match either:
///
///   "Wrong Display Name 'X' for <system>#<code>. There are no valid display
///    names found for language(s) 'L'. Default display is 'Y'"
///   (error severity, `NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_ERR`)
///
/// Also updates `resp.display` to the language-preferred designation value
/// when one matches `displayLanguage`.
#[allow(clippy::too_many_arguments)]
async fn apply_language_display_validation<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system_url: &str,
    code: &str,
    cs_resource: &Value,
    display_language: Option<&str>,
    expected_display: Option<&str>,
    supplements: &[SupplementInfo],
    resp: &mut ValidateCodeResponse,
) {
    // CodeSystem.language is the language of the primary `display` field.
    // None when the CS doesn't declare a language.
    let cs_language: Option<String> = cs_resource
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // Fetch all designations for this concept. Failures (e.g. unknown system)
    // are non-fatal — fall back to an empty list so the default-display logic
    // still runs.
    let designations = backend
        .concept_designations(ctx, system_url, &[code.to_string()])
        .await
        .ok()
        .and_then(|mut m| m.remove(code))
        .unwrap_or_default();

    // Pull in designations contributed by any applied supplements so the
    // valid-display universe matches what `rescue_via_supplements` accepts.
    // This is what lets `display="ectenoot"` validate against a base CS with
    // no Dutch designation when a supplement provides one (IG
    // `extensions/validate-coding-good-supplement` fixtures).
    let supplement_designations: Vec<crate::traits::ConceptDesignation> = if supplements.is_empty()
    {
        Vec::new()
    } else {
        let supp_urls: Vec<String> = supplements
            .iter()
            .map(|s| {
                s.supplement_canonical
                    .split('|')
                    .next()
                    .unwrap_or(&s.supplement_canonical)
                    .to_string()
            })
            .collect();
        backend
            .supplement_designations(ctx, &supp_urls, &[code.to_string()])
            .await
            .ok()
            .and_then(|mut m| m.remove(code))
            .unwrap_or_default()
    };

    let default_display: Option<String> = resp.display.clone();

    // Collect (display_value, language_tag_opt) pairs for every valid display:
    // the default display tagged with the CS language, plus every designation
    // that has a language attached.
    // A designation only counts as a "valid display" alternative when it
    // either has no `use.code` (default = display) or carries the FHIR-standard
    // `display` use. Designations with a non-display use (e.g.
    // `olde-english`, `consumer-name`) are alternative-purpose terms, not
    // displays — including them in the "Valid display is …" message
    // misrepresents what counts as a correct display, and the IG
    // `batch/batch-validate-bad` fixture expects them excluded.
    fn is_display_alternative(use_code: Option<&str>) -> bool {
        match use_code {
            None => true,
            Some(c) if c.eq_ignore_ascii_case("display") => true,
            _ => false,
        }
    }

    let mut displays_for_lang: Vec<(String, Option<String>)> = Vec::new();
    if let Some(d) = default_display.as_deref() {
        displays_for_lang.push((d.to_string(), cs_language.clone()));
    }
    for desig in &designations {
        if !desig.value.is_empty() && is_display_alternative(desig.use_code.as_deref()) {
            displays_for_lang.push((desig.value.clone(), desig.language.clone()));
        }
    }
    for desig in &supplement_designations {
        if !desig.value.is_empty()
            && is_display_alternative(desig.use_code.as_deref())
            && !displays_for_lang
                .iter()
                .any(|(v, _)| v.eq_ignore_ascii_case(&desig.value))
        {
            displays_for_lang.push((desig.value.clone(), desig.language.clone()));
        }
    }

    // The "language(s) 'L'" tail of the IG message. `--` means "no language
    // requested" (the request omitted `displayLanguage` entirely).
    let lang_tail: String = match display_language {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "--".to_string(),
    };

    // The displayLanguage parameter may carry a comma-separated list (e.g.
    // `de,it,zh` per the IG `validation/simple-code-good-language` fixture).
    // Split on `,` (trim each) so we can match any of the requested languages.
    let requested_langs: Vec<String> = display_language
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // Find the "preferred" display in any of the requested languages, if any.
    // Returns the first designation whose language equals one of the requested
    // languages (case-insensitive, exact match — IG fixtures don't exercise
    // language-tag fallback like `de-CH` → `de`).
    let preferred_for_lang: Option<&(String, Option<String>)> =
        if requested_langs.is_empty() {
            None
        } else {
            displays_for_lang.iter().find(|(_, lang_opt)| {
                lang_opt.as_deref().is_some_and(|l| {
                    requested_langs
                        .iter()
                        .any(|req| l.eq_ignore_ascii_case(req))
                })
            })
        };

    // Surface the language-preferred display on the response (overriding the
    // CS default). The IG `display/validation-right-de-ende-response` fixture
    // expects `display=Anzeige1` even though the request supplied
    // `displayLanguage=de` alongside the (matching) German designation.
    if let Some((value, _)) = preferred_for_lang {
        resp.display = Some(value.clone());
    }

    // Without a caller-supplied expected_display there's nothing to
    // language-validate — return now. resp.display has already been updated.
    let Some(expected) = expected_display else {
        return;
    };
    if expected.is_empty() {
        return;
    }

    // Skip language-aware validation when:
    //   1. No `displayLanguage` was requested, AND
    //   2. The backend already accepted the supplied display (no
    //      `invalid-display` issue is present),
    // because in that case the display has been validated against a broader
    // set than just our concept designations (e.g. a supplement designation
    // rescued the response in `rescue_via_supplements`).  Re-running our
    // narrower check here would spuriously turn an accepted display into a
    // mismatch error.
    let has_existing_invalid_display = resp
        .issues
        .iter()
        .any(|i| i.tx_code == "invalid-display");
    if display_language.is_none() && !has_existing_invalid_display {
        return;
    }

    // Decide whether the supplied display is valid.
    //   - If a requested displayLanguage is set AND a designation in that
    //     language exists, the only valid display is that designation.
    //   - Otherwise (no displayLanguage, or the requested language has no
    //     designation), any (default | designation) value is accepted.
    let display_matches: bool = if let Some((value, _)) = preferred_for_lang {
        value.eq_ignore_ascii_case(expected)
    } else {
        displays_for_lang
            .iter()
            .any(|(v, _)| v.eq_ignore_ascii_case(expected))
    };

    // Determine whether the CS has any display in any of the requested
    // languages. `displayLanguage` may be a comma-separated list.
    let has_display_in_lang: bool = if requested_langs.is_empty() {
        true
    } else {
        displays_for_lang.iter().any(|(_, lang_opt)| {
            lang_opt.as_deref().is_some_and(|x| {
                requested_langs
                    .iter()
                    .any(|req| x.eq_ignore_ascii_case(req))
            })
        })
    };

    // Capture the severity of the existing `invalid-display` issue so the
    // lenient-display-validation case (backend-emitted "warning" severity)
    // can be preserved when we rebuild the issue with the language-aware
    // text. Only honoured when the new issue is itself a mismatch error.
    let prior_invalid_display_severity: Option<String> = resp
        .issues
        .iter()
        .find(|i| i.tx_code == "invalid-display")
        .map(|i| i.severity.clone());

    // Strip any pre-existing `invalid-display` issue the backend emitted —
    // we will rebuild it (or skip it) using the language-aware wording.
    resp.issues.retain(|i| i.tx_code != "invalid-display");

    // Always emit `Coding.display` here; the downstream BareCode rewriter
    // in `build_validate_response` strips the `Coding.` prefix when the
    // request used the bare-code form, so we don't branch on the request
    // path here. Many IG `parameters/` and `validation/` fixtures omit
    // `location` from the issue body and don't mark it `$optional-properties$`,
    // so we only emit `expression` to stay strict-comparator-safe.
    let expression: Option<String> = Some("Coding.display".to_string());
    let location: Option<String> = None;

    if display_matches {
        // The supplied display is valid. If the request asked for a language
        // that the CS doesn't have a designation in, emit the
        // `NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_OK` info-level notice.
        if !has_display_in_lang && cs_language.is_some() {
            let default = default_display.as_deref().unwrap_or("");
            let text = format!(
                "There are no valid display names found for the code {system_url}#{code} for language(s) '{lang_tail}'. The display is '{default}' which is the default language display"
            );
            resp.issues.push(ValidationIssue {
                severity: "information".into(),
                fhir_code: "invalid".into(),
                tx_code: "invalid-display".into(),
                text: text.clone(),
                expression,
                location,
                message_id: Some("NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_OK".into()),
            });
            resp.result = true;
            // Replace any backend-set message that is now stale.
            resp.message = Some(text);
        } else {
            // Display matches AND the requested language is satisfied (or no
            // language was requested). Drop any stale message.
            resp.message = None;
            resp.result = true;
        }
    } else {
        // Supplied display is wrong. Pick the wording variant.

        // Whitespace-only mismatch detection: if the supplied display equals
        // some valid display after collapsing whitespace, the IG uses the
        // `Display_Name_WS_*` message-id variant (text wording is identical).
        fn collapse_ws(s: &str) -> String {
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        let expected_collapsed = collapse_ws(expected);
        let is_whitespace_only_mismatch = displays_for_lang
            .iter()
            .any(|(v, _)| collapse_ws(v).eq_ignore_ascii_case(&expected_collapsed))
            && displays_for_lang
                .iter()
                .all(|(v, _)| !v.eq_ignore_ascii_case(expected));

        let (text, message_id) = if !has_display_in_lang
            && cs_language.is_some()
            && display_language.is_some()
        {
            // No designation in the requested language — fall back to default.
            let default = default_display.as_deref().unwrap_or("");
            let txt = format!(
                "Wrong Display Name '{expected}' for {system_url}#{code}. There are no valid display names found for language(s) '{lang_tail}'. Default display is '{default}'"
            );
            (txt, "NO_VALID_DISPLAY_FOUND_NONE_FOR_LANG_ERR".to_string())
        } else if is_whitespace_only_mismatch {
            let valid_str = format_valid_displays(&displays_for_lang, display_language);
            let txt = format!(
                "Wrong Display Name '{expected}' for {system_url}#{code}. {valid_str} (for the language(s) '{lang_tail}')"
            );
            (
                txt,
                "Display_Name_WS_for__should_be_one_of__instead_of".to_string(),
            )
        } else {
            // CS has a display in the requested language (or no language
            // requested): list the valid display(s).
            let valid_str = format_valid_displays(&displays_for_lang, display_language);
            let txt = format!(
                "Wrong Display Name '{expected}' for {system_url}#{code}. {valid_str} (for the language(s) '{lang_tail}')"
            );
            (
                txt,
                "Display_Name_for__should_be_one_of__instead_of".to_string(),
            )
        };

        // Honour `lenient-display-validation`: if the original
        // backend-emitted issue was a warning, preserve that severity (and
        // keep result=true).  Otherwise this is a hard mismatch error.
        let severity = prior_invalid_display_severity
            .as_deref()
            .filter(|s| *s == "warning")
            .map(str::to_string)
            .unwrap_or_else(|| "error".to_string());
        let lenient = severity == "warning";
        resp.issues.push(ValidationIssue {
            severity,
            fhir_code: "invalid".into(),
            tx_code: "invalid-display".into(),
            text: text.clone(),
            expression,
            location,
            message_id: Some(message_id),
        });
        resp.message = Some(text);
        resp.result = lenient;
    }
}

/// Inspect the concept's stored `extension[]` and `designation[].extension[]`
/// arrays for `structuredefinition-standards-status` markers. When the concept
/// itself is marked `deprecated`/`withdrawn`, emit a `DEPRECATED_CONCEPT_FOUND`
/// warning and surface the status as `resp.concept_status` (rendered as the
/// top-level `status` parameter).  When a designation matching the supplied
/// `expected_display` is marked deprecated/withdrawn, emit an
/// `INACTIVE_DISPLAY_FOUND` warning and replace `resp.display` with the
/// concept's primary display so the response advertises the still-valid name.
///
/// Drives the IG `extensions/validate-code-inactive` and
/// `extensions/validate-code-inactive-display` fixtures.
async fn apply_concept_extension_status<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system_url: &str,
    code: &str,
    request_path: RequestPath,
    expected_display: Option<&str>,
    resp: &mut ValidateCodeResponse,
) {
    let entry_map = match backend
        .concept_resource_entries(ctx, system_url, &[code.to_string()])
        .await
    {
        Ok(m) => m,
        Err(_) => return,
    };
    let Some(entry) = entry_map.get(code) else {
        return;
    };

    // (a) Concept-level standards-status extension.
    if let Some(exts) = entry.get("extension").and_then(|e| e.as_array()) {
        for ext in exts {
            if ext.get("url").and_then(|u| u.as_str())
                != Some(
                    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                )
            {
                continue;
            }
            let status_code = match ext.get("valueCode").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue,
            };
            if !matches!(status_code, "deprecated" | "withdrawn") {
                continue;
            }
            // Surface as top-level `status` parameter.
            if resp.concept_status.is_none() {
                resp.concept_status = Some(status_code.to_string());
            }
            // Emit DEPRECATED_CONCEPT_FOUND warning. Skip if already present
            // (e.g. multiple paths might call this).
            let already = resp
                .issues
                .iter()
                .any(|i| i.message_id.as_deref() == Some("DEPRECATED_CONCEPT_FOUND"));
            if !already {
                let text = format!(
                    "The concept '{code}' is {status_code} and its use should be reviewed"
                );
                let (loc_path, expr_path) = match request_path {
                    RequestPath::BareCode => {
                        ("code".to_string(), "code".to_string())
                    }
                    RequestPath::CodeableConcept => (
                        "CodeableConcept.coding[0].code".to_string(),
                        "CodeableConcept.coding[0].code".to_string(),
                    ),
                    _ => ("Coding.code".to_string(), "Coding.code".to_string()),
                };
                resp.issues.push(ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text,
                    expression: Some(expr_path),
                    location: Some(loc_path),
                    message_id: Some("DEPRECATED_CONCEPT_FOUND".into()),
                });
            }
            break;
        }
    }

    // (b) Designation-level standards-status: only fires when the caller
    // supplied a `display` that matches one of the concept's designations,
    // AND that designation carries a deprecated/withdrawn status. The IG
    // `validate-code-inactive-display` fixture expects:
    //   - resp.display = the concept's primary display (rescuing it)
    //   - INACTIVE_DISPLAY_FOUND warning naming the supplied (now-inactive)
    //     display and the still-valid display(s).
    let Some(expected) = expected_display else {
        return;
    };
    if expected.is_empty() {
        return;
    }
    let Some(designations) = entry.get("designation").and_then(|d| d.as_array()) else {
        return;
    };
    let primary_display: Option<String> = entry
        .get("display")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    for desig in designations {
        let value = match desig.get("value").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        if !value.eq_ignore_ascii_case(expected) {
            continue;
        }
        let Some(d_exts) = desig.get("extension").and_then(|e| e.as_array()) else {
            continue;
        };
        let mut desig_status: Option<&str> = None;
        for d_ext in d_exts {
            if d_ext.get("url").and_then(|u| u.as_str())
                != Some(
                    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                )
            {
                continue;
            }
            if let Some(c) = d_ext.get("valueCode").and_then(|v| v.as_str()) {
                if matches!(c, "deprecated" | "withdrawn") {
                    desig_status = Some(c);
                    break;
                }
            }
        }
        let Some(_status_code) = desig_status else {
            continue;
        };
        // Replace the response display with the concept's primary display so
        // the response advertises the canonical (active) name.
        if let Some(pd) = primary_display.as_deref() {
            resp.display = Some(pd.to_string());
        }
        // Drop any pre-existing display-mismatch issue — the supplied display
        // matched a known designation, just an inactive one.
        resp.issues.retain(|i| i.tx_code != "invalid-display");
        let valid_str = match primary_display.as_deref() {
            Some(d) => format!("\"{d}\""),
            None => "(none)".to_string(),
        };
        // The IG fixture wording uses "(status = deprecated)" regardless of
        // whether the designation is marked deprecated or withdrawn — the
        // concept of "inactive display" subsumes both per the IG.
        let text = format!(
            "'{expected}' is no longer considered a correct display for code '{code}' (status = deprecated). The correct display is one of {valid_str}."
        );
        let already = resp
            .issues
            .iter()
            .any(|i| i.message_id.as_deref() == Some("INACTIVE_DISPLAY_FOUND"));
        if !already {
            let (loc_path, expr_path) = match request_path {
                RequestPath::BareCode => {
                    ("display".to_string(), "display".to_string())
                }
                RequestPath::CodeableConcept => (
                    "CodeableConcept.coding[0].display".to_string(),
                    "CodeableConcept.coding[0].display".to_string(),
                ),
                _ => (
                    "Coding.display".to_string(),
                    "Coding.display".to_string(),
                ),
            };
            resp.issues.push(ValidationIssue {
                severity: "warning".into(),
                fhir_code: "invalid".into(),
                tx_code: "display-comment".into(),
                text,
                expression: Some(expr_path),
                location: Some(loc_path),
                message_id: Some("INACTIVE_DISPLAY_FOUND".into()),
            });
        }
        // Mark result=true so the response is "validated with warnings".
        resp.result = true;
        resp.message = None;
        break;
    }
}

/// Format the "Valid display is ..." segment of an IG `invalid-display`
/// message.
///
/// When `display_language` is set, candidates are restricted to designations
/// that match that language; otherwise every (display, language) pair is
/// considered. With one candidate the wording is `"Valid display is 'Y' (lang)"`;
/// with multiple it becomes
/// `"Valid display is one of N choices: 'A' (en) or 'B' (de)"`. When a
/// candidate has no language tag the `(lang)` suffix is dropped.
fn format_valid_displays(
    displays_for_lang: &[(String, Option<String>)],
    display_language: Option<&str>,
) -> String {
    // When a language is requested, restrict to displays whose tag matches
    // (case-insensitive). If nothing matches, fall back to the full set so
    // the message still names the default — the IG `display/validation-wrong-de-none`
    // fixture (CS has no language, request has displayLanguage=de) expects
    // the response to point at the default display rather than an empty list.
    let candidates: Vec<&(String, Option<String>)> = if let Some(req) =
        display_language.filter(|s| !s.is_empty())
    {
        let filtered: Vec<&(String, Option<String>)> = displays_for_lang
            .iter()
            .filter(|(_, l)| l.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(req)))
            .collect();
        if filtered.is_empty() {
            displays_for_lang.iter().collect()
        } else {
            filtered
        }
    } else {
        displays_for_lang.iter().collect()
    };

    let render_one = |entry: &(String, Option<String>)| -> String {
        match entry.1.as_deref() {
            Some(lang) if !lang.is_empty() => format!("'{}' ({})", entry.0, lang),
            _ => format!("'{}'", entry.0),
        }
    };

    match candidates.len() {
        0 => "Valid display is unknown".to_string(),
        1 => format!("Valid display is {}", render_one(candidates[0])),
        n => {
            let parts: Vec<String> = candidates.iter().map(|e| render_one(e)).collect();
            format!(
                "Valid display is one of {} choices: {}",
                n,
                parts.join(" or ")
            )
        }
    }
}

/// Build a validate-code response and resolve the system's version via a
/// backend lookup (so the response can echo `version` per the IG fixtures).
///
/// The version echoed in the response is taken from `resp.cs_version` — the
/// version the backend **actually resolved and used** during validation.  This
/// is set by the storage layer to the CS version it picked (latest stored
/// when no version was pinned, or the exact version it fell back to when the
/// requested version didn't exist).  A separate DB lookup is still done for
/// `x-unknown-system` detection and status-check issue generation.
#[allow(clippy::too_many_arguments)]
async fn build_validate_response_async<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    mut resp: ValidateCodeResponse,
    code: Option<&str>,
    system: Option<&str>,
    codeable_concept: Option<&Value>,
    request_path: RequestPath,
    value_set_url: Option<&str>,
    display_language: Option<&str>,
    expected_display: Option<&str>,
    supplements: &[SupplementInfo],
) -> Value {
    // For inactive concepts whose underlying status is more specific than
    // "inactive" (e.g. `retired`, `deprecated`, `withdrawn`), the IG
    // `inactive/validate-inactive-3*` fixtures expect TWO warning issues:
    // one with text "...has a status of inactive..." (the canonical wording
    // already emitted by the backend) AND a second with text using the
    // specific status code (e.g. "...has a status of retired..."). Detect
    // that case here by looking up the concept's `status` property and
    // appending a second issue when needed.
    if resp.inactive == Some(true) {
        let inferred_system = resp.system.clone();
        let lookup_system: Option<&str> = system.or(inferred_system.as_deref());
        if let (Some(sys), Some(cd)) = (lookup_system, code) {
            if let Some(specific_status) = lookup_concept_status(backend, ctx, sys, cd).await {
                // Surface as top-level `status` parameter (e.g. "retired",
                // "deprecated", "withdrawn"). The IG `batch/batch-validate`
                // fixture expects this when the underlying CS concept has a
                // `status` property set to a non-active value.
                if resp.concept_status.is_none() {
                    resp.concept_status = Some(specific_status.clone());
                }
                let already_has_specific = resp.issues.iter().any(|i| {
                    i.message_id.as_deref() == Some("INACTIVE_CONCEPT_FOUND")
                        && i.text
                            .contains(&format!("has a status of {specific_status} and"))
                });
                if !already_has_specific {
                    let inactive_issue = resp.issues.iter().find(|i| {
                        i.message_id.as_deref() == Some("INACTIVE_CONCEPT_FOUND")
                            && i.text.contains("has a status of inactive")
                    });
                    if let Some(template) = inactive_issue.cloned() {
                        let new_text = format!(
                            "The concept '{cd}' has a status of {specific_status} and its use should be reviewed"
                        );
                        resp.issues.push(ValidationIssue {
                            severity: template.severity,
                            fhir_code: template.fhir_code,
                            tx_code: template.tx_code,
                            text: new_text,
                            expression: template.expression,
                            location: template.location,
                            message_id: template.message_id,
                        });
                    }
                }
            }
        }
    }
    // Prefer the system the caller passed; otherwise fall back to whatever
    // the backend inferred from the VS expansion (e.g. inferSystem=true).
    let inferred_system = resp.system.clone();
    let effective_system: Option<&str> = system.or(inferred_system.as_deref());

    // Look up the stored CS version for `x-unknown-system` detection and
    // status-check issue generation.
    let stored_version = if let Some(s) = effective_system {
        backend
            .code_system_version_for_url(ctx, s)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    // Use the version the backend actually resolved and used.  The backend
    // populates `resp.cs_version` with the CS version it picked; fall back
    // to the stored_version (latest) when the backend didn't set it (e.g.
    // older backends or paths that bypass finish_validate_code_response).
    let version: Option<String> = resp.cs_version.take().or(stored_version.clone());

    // Search to determine if the system URL exists as a CodeSystem. This is a
    // more reliable existence check than `stored_version.is_some()` — a CS
    // that has no `version` field yields `stored_version = None` despite
    // existing. We need this distinction to avoid spurious `x-unknown-system`
    // / `UNKNOWN_CODESYSTEM` emissions for stored-but-versionless CSes.
    let cs_resource: Option<Value> = if let Some(sys) = effective_system {
        crate::traits::CodeSystemOperations::search(
            backend,
            ctx,
            crate::types::ResourceSearchQuery {
                url: Some(sys.to_string()),
                count: Some(1),
                ..Default::default()
            },
        )
        .await
        .ok()
        .and_then(|mut hits| hits.pop())
    } else {
        None
    };
    let cs_exists = cs_resource.is_some();

    // Detect when the system URL is actually a stored ValueSet (not a
    // CodeSystem). In that case the IG expects a `Terminology_TX_System_ValueSet2`
    // issue rather than `UNKNOWN_CODESYSTEM`, and no `x-unknown-system` param
    // (see `validation/simple-coding-bad-system2`).
    let system_is_value_set = if !cs_exists {
        if let Some(sys) = effective_system {
            crate::traits::ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(sys.to_string()),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .map(|hits| !hits.is_empty())
            .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    // If the input system isn't stored as a CS (and isn't a known VS), the IG
    // expects an `x-unknown-system` parameter pointing at the unknown URL
    // (only when validate-code reported result=false).
    //
    // VS-context wrinkle: the IG fixture `errors/errors-unknown-system1` —
    // where the caller's `system` matches a VS-include's system AND the
    // system is unknown — expects ONLY the `UNKNOWN_CODESYSTEM` issue
    // (no `not-in-vs` companion) AND `x-caused-by-unknown-system` (not
    // `x-unknown-system`). The companion fixture `errors-unknown-system2`
    // — where the caller's `system` is unknown but DIFFERENT from any VS
    // include — keeps both issues and `x-unknown-system`. Differentiate by
    // checking whether the caller's `effective_system` literally appears in
    // the resolved VS's compose.include[].system list.
    let system_unknown = !resp.result && !cs_exists && !system_is_value_set;
    let mut suppress_not_in_vs_for_unknown = false;
    // Set when the standard `unknown_system` UNKNOWN_CODESYSTEM emission
    // should drop its `location` field — IG fixtures (e.g. `regex-bad/
    // validate-regex-bad`) where the caller's `system` is unknown AND not
    // referenced by any VS include expect only `expression` on that issue.
    let mut suppress_unknown_system_location = false;
    let unknown_system = if system_unknown {
        let sys_matches_vs_include = if let (Some(vs_url), Some(sys)) =
            (value_set_url, effective_system)
        {
            // Look up the VS to see if its compose.include[] mentions `sys`.
            let bare_url = vs_url.split('|').next().unwrap_or(vs_url).to_string();
            let vs_hit = crate::traits::ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(bare_url),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|mut h| h.pop());
            vs_hit
                .as_ref()
                .and_then(|v| v.get("compose"))
                .and_then(|c| c.get("include"))
                .and_then(|i| i.as_array())
                .map(|incs| {
                    incs.iter().any(|inc| {
                        inc.get("system").and_then(|s| s.as_str()) == Some(sys)
                    })
                })
                .unwrap_or(false)
        } else {
            false
        };
        if sys_matches_vs_include {
            // Only set caused_by when nothing else has claimed it — version
            // mismatch detection (`detect_cs_version_mismatch`) sets this on
            // CSes that exist at *some* version but not the requested one,
            // and we don't want to clobber that diagnostic with the simpler
            // unknown-system canonical.
            if resp.caused_by_unknown_system.is_none() {
                if let Some(sys) = effective_system {
                    resp.caused_by_unknown_system = Some(sys.to_string());
                }
            }
            suppress_not_in_vs_for_unknown = true;
            // Synthesise the UNKNOWN_CODESYSTEM issue inline (since we're
            // returning `None` for `unknown_system`, `build_validate_response`
            // won't add it in its standard path). The IG fixture expects a
            // single UNKNOWN_CODESYSTEM issue with location/expression `system`
            // (or `Coding.system` for non-bare paths).
            if let Some(sys) = effective_system {
                let already = resp
                    .issues
                    .iter()
                    .any(|i| i.message_id.as_deref() == Some("UNKNOWN_CODESYSTEM"));
                if !already {
                    let expression = match request_path {
                        RequestPath::BareCode => "system".to_string(),
                        _ => "Coding.system".to_string(),
                    };
                    let text = format!(
                        "A definition for CodeSystem '{sys}' could not be found, so the code cannot be validated"
                    );
                    resp.issues.push(ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "not-found".into(),
                        tx_code: "not-found".into(),
                        text,
                        expression: Some(expression.clone()),
                        location: Some(expression),
                        message_id: Some("UNKNOWN_CODESYSTEM".into()),
                    });
                }
            }
            None
        } else {
            // System is unknown AND does not match any VS include — set the
            // flag to drop `location` from the synthesized UNKNOWN_CODESYSTEM
            // issue when a VS context is in play. The errors-unknown-system2
            // fixture marks `location` `$optional-properties$` so omitting it
            // is conformant there too. The CodeSystem-only path
            // (value_set_url is None) keeps location for backward compat.
            if value_set_url.is_some() {
                suppress_unknown_system_location = true;
            }
            effective_system
        }
    } else {
        None
    };
    if suppress_not_in_vs_for_unknown {
        resp.issues.retain(|i| {
            i.message_id.as_deref() != Some("None_of_the_provided_codes_are_in_the_value_set_one")
                && i.tx_code != "not-in-vs"
        });
    }

    // When the input system URL is a stored ValueSet rather than a
    // CodeSystem, synthesize the IG-expected `Terminology_TX_System_ValueSet2`
    // issue instead of the unknown-system issue.
    if system_is_value_set {
        if let Some(sys) = effective_system {
            let already_has = resp.issues.iter().any(|i| {
                i.message_id.as_deref() == Some("Terminology_TX_System_ValueSet2")
            });
            if !already_has {
                let expression = match request_path {
                    RequestPath::BareCode => "system".to_string(),
                    _ => "Coding.system".to_string(),
                };
                resp.issues.push(ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "invalid".into(),
                    tx_code: "invalid-data".into(),
                    text: format!(
                        "The Coding references a value set, not a code system ('{sys}')"
                    ),
                    expression: Some(expression),
                    location: None,
                    message_id: Some("Terminology_TX_System_ValueSet2".into()),
                });
            }
        }
    }

    // Append info-level "Reference to <status> CodeSystem url|version" issues
    // when the validated CodeSystem carries a non-active standards-status —
    // matches the IG `deprecated/validate-*` fixtures.
    if let (Some(sys), Some(cs)) = (effective_system, cs_resource.as_ref()) {
        for status in collect_status_check_codes(cs) {
            let cs_uri = match version.as_deref() {
                Some(v) => format!("{sys}|{v}"),
                None => sys.to_string(),
            };
            resp.issues.push(ValidationIssue {
                severity: "information".into(),
                fhir_code: "business-rule".into(),
                tx_code: "status-check".into(),
                text: format!("Reference to {status} CodeSystem {cs_uri}"),
                expression: None,
                location: None,
                message_id: Some(status_message_id(&status).into()),
            });
        }
    }


    // Apply IG-style language-aware display validation: rewrite (or insert) the
    // `invalid-display` issue using the canonical "Wrong Display Name 'X' for
    // url#code. ..." text the IG fixtures (display/, language2/, validation/)
    // expect, and surface a language-appropriate response `display`.
    //
    // This runs only when we have a stored CodeSystem (so we can read
    // `CodeSystem.language` and `concept.designation[].language`) and an
    // `expected_display` was supplied (otherwise there is nothing to validate).
    if let (Some(sys), Some(cs), Some(cd)) = (effective_system, cs_resource.as_ref(), code) {
        apply_language_display_validation(
            backend,
            ctx,
            sys,
            cd,
            cs,
            display_language,
            expected_display,
            supplements,
            &mut resp,
        )
        .await;
    }

    // Concept-level standards-status extension scan: detect
    // `structuredefinition-standards-status: deprecated/withdrawn` on the
    // concept itself or on any of its designations, and emit the IG
    // `extensions/validate-code-inactive` warnings.
    if let (Some(sys), Some(cd)) = (effective_system, code) {
        apply_concept_extension_status(
            backend,
            ctx,
            sys,
            cd,
            request_path,
            expected_display,
            &mut resp,
        )
        .await;
    }

    // Mirror the same status-check emission for the validated ValueSet on
    // the VS-validate-code path. The IG `deprecated/validate-withdrawn`
    // fixture expects BOTH a deprecated-CS issue AND a withdrawn-VS issue.
    //
    // The IG `deprecated/not-withdrawn-validate` fixture additionally
    // requires walking `compose.include[].valueSet[]` chains: validating a
    // code via a VS whose compose imports a withdrawn VS yields TWO
    // status-check issues (one for the validated CS + one for each chained
    // VS that carries a non-active standards-status).
    if let Some(vs_url) = value_set_url {
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut to_visit: Vec<String> = vec![vs_url.to_string()];
        while let Some(current_url) = to_visit.pop() {
            // Strip any `|version` suffix for the visited-set so a
            // versionless and a pinned reference to the same VS aren't
            // walked twice.
            let bare = current_url
                .split('|')
                .next()
                .unwrap_or(&current_url)
                .to_string();
            if !visited.insert(bare.clone()) {
                continue;
            }
            let (lookup_url, lookup_ver): (String, Option<String>) =
                match current_url.split_once('|') {
                    Some((u, v)) => (u.to_string(), Some(v.to_string())),
                    None => (current_url.clone(), None),
                };
            let mut hits = match crate::traits::ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(lookup_url.clone()),
                    version: lookup_ver.clone(),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            {
                Ok(h) => h,
                Err(_) => continue,
            };
            let Some(vs) = hits.pop() else { continue };
            let vs_version = vs.get("version").and_then(|v| v.as_str());
            for status in collect_status_check_codes(&vs) {
                let vs_uri = match vs_version {
                    Some(v) => format!("{lookup_url}|{v}"),
                    None => lookup_url.clone(),
                };
                let text = format!("Reference to {status} ValueSet {vs_uri}");
                // De-dupe across the chain: skip if we already emitted the
                // exact same status-check text via a different reachable URL.
                let already = resp
                    .issues
                    .iter()
                    .any(|i| i.text == text && i.tx_code == "status-check");
                if already {
                    continue;
                }
                resp.issues.push(ValidationIssue {
                    severity: "information".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "status-check".into(),
                    text,
                    expression: None,
                    location: None,
                    message_id: Some(status_message_id(&status).into()),
                });
            }
            // Enqueue any nested compose.include[].valueSet[] refs.
            if let Some(includes) = vs
                .get("compose")
                .and_then(|c| c.get("include"))
                .and_then(|v| v.as_array())
            {
                for inc in includes {
                    if let Some(refs) = inc.get("valueSet").and_then(|v| v.as_array()) {
                        for r in refs {
                            if let Some(s) = r.as_str() {
                                to_visit.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // CONCEPT_DEPRECATED_IN_VALUESET: when a code is validated against a VS
    // whose `compose.include[].concept[]` entry for that code carries a
    // `valueset-deprecated` extension OR a `structuredefinition-standards-status`
    // extension valued `deprecated/withdrawn`, emit a warning. Drives the IG
    // `deprecated/deprecating-validate*` fixtures.
    if let (Some(vs_url), Some(cd), Some(sys)) = (value_set_url, code, effective_system) {
        if resp.result {
            if let Ok(mut hits) = crate::traits::ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(vs_url.to_string()),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            {
                if let Some(vs) = hits.pop() {
                    let vs_version = vs.get("version").and_then(|v| v.as_str());
                    let vs_uri = match vs_version {
                        Some(v) => format!("{vs_url}|{v}"),
                        None => vs_url.to_string(),
                    };
                    if let Some(includes) = vs
                        .get("compose")
                        .and_then(|c| c.get("include"))
                        .and_then(|v| v.as_array())
                    {
                        'find_marker: for inc in includes {
                            let inc_sys = inc.get("system").and_then(|s| s.as_str());
                            if inc_sys != Some(sys) {
                                continue;
                            }
                            let Some(concepts) =
                                inc.get("concept").and_then(|c| c.as_array())
                            else {
                                continue;
                            };
                            for c in concepts {
                                if c.get("code").and_then(|v| v.as_str()) != Some(cd) {
                                    continue;
                                }
                                if !concept_marked_deprecated(c) {
                                    continue;
                                }
                                let already = resp.issues.iter().any(|i| {
                                    i.message_id.as_deref()
                                        == Some("CONCEPT_DEPRECATED_IN_VALUESET")
                                });
                                if already {
                                    break 'find_marker;
                                }
                                let text = format!(
                                    "The presence of the concept '{cd}' in the system '{sys}' in the value set {vs_uri} is marked with a status of deprecated and its use should be reviewed"
                                );
                                let (loc_path, expr_path) = match request_path {
                                    RequestPath::BareCode => {
                                        ("code".to_string(), "code".to_string())
                                    }
                                    RequestPath::CodeableConcept => (
                                        "CodeableConcept.coding[0].code".to_string(),
                                        "CodeableConcept.coding[0].code".to_string(),
                                    ),
                                    _ => {
                                        ("Coding.code".to_string(), "Coding.code".to_string())
                                    }
                                };
                                resp.issues.push(ValidationIssue {
                                    severity: "warning".into(),
                                    fhir_code: "business-rule".into(),
                                    tx_code: "code-comment".into(),
                                    text,
                                    expression: Some(expr_path),
                                    location: Some(loc_path),
                                    message_id: Some(
                                        "CONCEPT_DEPRECATED_IN_VALUESET".into(),
                                    ),
                                });
                                break 'find_marker;
                            }
                        }
                    }
                }
            }
        }
    }

    build_validate_response_inner(
        resp,
        code,
        effective_system,
        version.as_deref(),
        codeable_concept,
        unknown_system,
        request_path,
        suppress_unknown_system_location,
    )
}

/// Detect whether a `compose.include[].concept[]` JSON object is marked as
/// deprecated via either the `valueset-deprecated` extension (valueCode "true")
/// or a `structuredefinition-standards-status` extension valued
/// `deprecated`/`withdrawn`. Drives the IG `deprecated/deprecating-validate*`
/// fixtures' `CONCEPT_DEPRECATED_IN_VALUESET` warning emission.
fn concept_marked_deprecated(concept: &Value) -> bool {
    let Some(exts) = concept.get("extension").and_then(|e| e.as_array()) else {
        return false;
    };
    for ext in exts {
        let Some(url) = ext.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        match url {
            "http://hl7.org/fhir/StructureDefinition/valueset-deprecated" => {
                let v = ext
                    .get("valueCode")
                    .and_then(|v| v.as_str())
                    .or_else(|| ext.get("valueBoolean").and_then(|v| v.as_str()));
                let truthy = v.map(|s| s.eq_ignore_ascii_case("true")).unwrap_or(false)
                    || ext.get("valueBoolean").and_then(|v| v.as_bool()) == Some(true);
                if truthy {
                    return true;
                }
            }
            "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status" => {
                if let Some(code) = ext.get("valueCode").and_then(|v| v.as_str()) {
                    if matches!(code, "deprecated" | "withdrawn") {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Collect the standards-status codes (deprecated, withdrawn, draft, etc.)
/// declared on a CodeSystem or ValueSet resource_json. Used by the
/// validate-code response builder to emit IG `MSG_DEPRECATED`-style
/// info-level issues. Returns at most one of each status, in the order:
/// extension first, then `experimental`, then `status`.
fn collect_status_check_codes(resource: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |code: &str| {
        if !code.is_empty() && !out.iter().any(|c| c == code) {
            out.push(code.to_string());
        }
    };
    if let Some(exts) = resource.get("extension").and_then(|e| e.as_array()) {
        for ext in exts {
            if ext.get("url").and_then(|u| u.as_str())
                == Some(
                    "http://hl7.org/fhir/StructureDefinition/structuredefinition-standards-status",
                )
            {
                if let Some(code) = ext.get("valueCode").and_then(|v| v.as_str()) {
                    push_unique(code);
                }
            }
        }
    }
    if resource.get("experimental").and_then(|v| v.as_bool()) == Some(true) {
        push_unique("experimental");
    }
    let status = resource
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(status, "draft" | "retired") {
        push_unique(status);
    }
    out
}

fn status_message_id(status: &str) -> &'static str {
    match status {
        "deprecated" => "MSG_DEPRECATED",
        "withdrawn" => "MSG_WITHDRAWN",
        "experimental" => "MSG_EXPERIMENTAL",
        "draft" => "MSG_DRAFT",
        "retired" => "MSG_RETIRED",
        _ => "MSG_DEPRECATED",
    }
}

/// Resolve every `useSupplement` request param against the backend.
///
/// For each supplement URL provided by the caller:
/// - Verify a stored CodeSystem exists with that URL **and** `content =
///   supplement` (via `supplement_target`).
/// - When `expected_target` is `Some`, also enforce that the supplement's
///   `supplements` URL matches it (so a supplement targeting CS-A cannot
///   silently apply to CS-B).
///
/// Returns the resolved [`SupplementInfo`] list on success — operations
/// layer code merges supplement-derived data into the response. Returns
/// `HtsError::NotFound` when any supplement is unknown / mistargeted, so
/// the IG fixtures' `bad-supplement` cases produce a 4xx OperationOutcome.
async fn resolve_supplements<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    params: &[Value],
    expected_target: Option<&str>,
) -> Result<Vec<SupplementInfo>, HtsError> {
    let mut out = Vec::new();
    for s in params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("useSupplement"))
        .filter_map(|p| {
            p.get("valueCanonical")
                .or_else(|| p.get("valueUri"))
                .and_then(|v| v.as_str())
        })
    {
        let bare = s.split('|').next().unwrap_or(s);
        let info = backend.supplement_target(ctx, bare).await?;
        let info = match info {
            Some(i) => i,
            None => {
                return Err(HtsError::NotFound(format!(
                    "Required supplement not found: {bare}"
                )));
            }
        };
        if let Some(target) = expected_target {
            if info.target_url != target {
                return Err(HtsError::NotFound(format!(
                    "Required supplement not found: {bare}"
                )));
            }
        }
        out.push(info);
    }
    Ok(out)
}

/// True when `expected` matches the concept's stored display OR any
/// supplement designation value (case-insensitive ASCII compare, the same
/// rule used inside the backend's display check). Used to "rescue" a
/// validate-code response whose only failure was a display mismatch that
/// is in fact resolved by an applied supplement.
async fn display_matches_supplement<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    supplements: &[SupplementInfo],
    system_url: &str,
    code: &str,
    expected: &str,
) -> bool {
    if supplements.is_empty() {
        return false;
    }
    let supp_urls: Vec<String> = supplements
        .iter()
        .map(|s| {
            s.supplement_canonical
                .split('|')
                .next()
                .unwrap_or(&s.supplement_canonical)
                .to_string()
        })
        .collect();
    let codes = vec![code.to_string()];
    let designs = match backend
        .supplement_designations(ctx, &supp_urls, &codes)
        .await
    {
        Ok(d) => d,
        Err(_) => return false,
    };
    let _ = system_url; // supplements are already filtered by their own URL list
    if let Some(list) = designs.get(code) {
        for d in list {
            if d.value.eq_ignore_ascii_case(expected) {
                return true;
            }
        }
    }
    false
}

/// Append a `used-supplement` parameter to a built validate-code response,
/// once per applied supplement. The value is the supplement's canonical
/// (`url|version` when available). Mutates `value` in place.
///
/// Note: the IG `parameters/parameters-validate-supplement-*` fixtures do
/// NOT echo `used-supplement` on validate-code responses (only on $expand
/// and $lookup). Currently a no-op so we don't pollute validate-code output
/// with the parameter — kept callable so the call sites compile unchanged.
fn append_used_supplements(_value: &mut Value, _supplements: &[SupplementInfo]) {
    // intentionally empty — see doc comment
}

/// If `resp` reports `result=false` solely because of a display mismatch,
/// and the supplied display in fact matches one of the supplement-derived
/// alt-display designations, mutate `resp` in place to clear the message
/// and set `result=true`. No-op when no supplements are applied or when
/// the response wasn't a display-mismatch failure.
async fn rescue_via_supplements<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    supplements: &[SupplementInfo],
    system_url: &str,
    code: &str,
    expected_display: Option<&str>,
    resp: &mut ValidateCodeResponse,
) {
    if supplements.is_empty() || resp.result {
        return;
    }
    let Some(expected) = expected_display else {
        return;
    };
    // Heuristic: only "rescue" display-mismatch failures, not
    // code-not-in-VS or unknown-code rejections. The backend's display
    // mismatch message starts with one of:
    //   - "Display mismatch:" (CodeSystem path, see code_system.rs)
    //   - "Provided display ... does not match" (legacy ValueSet path)
    //   - "Wrong Display Name ..." (IG-canonical ValueSet path, see
    //     finish_validate_code_response in value_set.rs)
    let msg = resp.message.as_deref().unwrap_or("");
    let looks_like_display_mismatch = msg.starts_with("Display mismatch:")
        || msg.contains("does not match stored display")
        || msg.starts_with("Wrong Display Name ")
        || msg.contains("Wrong whitespace in Display Name ");
    if !looks_like_display_mismatch {
        return;
    }
    if display_matches_supplement(backend, ctx, supplements, system_url, code, expected).await {
        resp.result = true;
        resp.message = None;
        // Drop the structured issues too — the backend emitted an
        // `invalid-display` error that is no longer applicable now that the
        // supplement has supplied a matching designation. Without this the
        // build_validate_response final_result computation would still see
        // an error-severity issue and force result=false.
        resp.issues.clear();
    }
}

/// Apply the `activeOnly=true` request-parameter semantics to the response of
/// a VS-bound validation. The IG `validation/simple-coding-bad-code-inactive`
/// fixture validates an inactive code against a VS that includes it, but with
/// `activeOnly=true` — the IG expects the code to be treated as
/// "not in VS" because the activeOnly filter would have excluded it from the
/// expansion. Specifically:
///   - `result` flips to `false`,
///   - a `code-rule` business-rule error ("…is valid but is not active") is
///     added,
///   - a `not-in-vs` `code-invalid` error is added.
///
/// The pre-existing `code-comment` warning ("…has a status of inactive…") is
/// kept. No-op when `active_only` isn't true, the response isn't currently a
/// pass, or the concept isn't inactive. Mutates `resp` in place.
fn apply_active_only_inactive(
    active_only: bool,
    resp: &mut ValidateCodeResponse,
    code: &str,
    system_url: &str,
    vs_url: &str,
    vs_version: Option<&str>,
) {
    if !active_only || !resp.result || resp.inactive != Some(true) {
        return;
    }
    let url_with_version = match vs_version {
        Some(v) => format!("{vs_url}|{v}"),
        None => vs_url.to_string(),
    };
    // Build the two new issues. The IG fixture orders them as:
    //   [code-rule (error), not-in-vs (error), <pre-existing code-comment warning>]
    // — i.e. errors first, warnings retained at the end. Insert at index 0
    // so the existing inactive-warning slides to the back.
    let code_rule_text = format!("The concept '{code}' is valid but is not active");
    let not_in_vs_text = format!(
        "The provided code '{system_url}#{code}' was not found in the value set '{url_with_version}'"
    );
    let mut prefix: Vec<ValidationIssue> = Vec::new();
    if !resp.issues.iter().any(|i| i.text == code_rule_text) {
        prefix.push(ValidationIssue {
            severity: "error".into(),
            fhir_code: "business-rule".into(),
            tx_code: "code-rule".into(),
            text: code_rule_text,
            expression: Some("Coding.code".into()),
            location: Some("Coding.code".into()),
            message_id: Some("STATUS_CODE_WARNING_CODE".into()),
        });
    }
    if !resp.issues.iter().any(|i| i.text == not_in_vs_text) {
        prefix.push(ValidationIssue {
            severity: "error".into(),
            fhir_code: "code-invalid".into(),
            tx_code: "not-in-vs".into(),
            text: not_in_vs_text,
            expression: Some("Coding.code".into()),
            location: Some("Coding.code".into()),
            message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
        });
    }
    if !prefix.is_empty() {
        prefix.append(&mut resp.issues);
        resp.issues = prefix;
    }
    resp.result = false;
    // Recompute message from sorted error texts (matches the convention used
    // elsewhere in this file).
    let mut texts: Vec<&str> = resp
        .issues
        .iter()
        .filter(|i| i.severity != "information")
        .map(|i| i.text.as_str())
        .collect();
    texts.sort_unstable();
    if !texts.is_empty() {
        resp.message = Some(texts.join("; "));
    }
}

/// Build a CODESYSTEM_CS_NO_SUPPLEMENT failure response: when the caller's
/// `system` URL points at a stored CodeSystem whose `content = supplement`,
/// CodeSystem/$validate-code must reject the call (a supplement is not a
/// valid Coding.system per FHIR R5 §4.7.10). Returns `Some(value)` when the
/// system is a supplement and the response should be returned immediately.
async fn supplement_url_in_coding_error<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system_url: &str,
    code: Option<&str>,
    request_path: RequestPath,
) -> Option<Value> {
    let info = match backend.supplement_target(ctx, system_url).await {
        Ok(Some(i)) => i,
        _ => return None,
    };
    let canonical = &info.supplement_canonical;
    let text = format!(
        "CodeSystem {canonical} is a supplement, so can't be used as a value in Coding.system"
    );
    let expression = match request_path {
        RequestPath::BareCode => "system".to_string(),
        _ => "Coding.system".to_string(),
    };
    let issue = ValidationIssue {
        severity: "error".into(),
        fhir_code: "invalid".into(),
        tx_code: "invalid-data".into(),
        text: text.clone(),
        expression: Some(expression),
        location: None,
        message_id: Some("CODESYSTEM_CS_NO_SUPPLEMENT".into()),
    };
    Some(build_validate_response(
        ValidateCodeResponse {
            result: false,
            message: Some(text),
            display: None,
            system: None,
            cs_version: None,
            inactive: None,
            issues: vec![issue],
            caused_by_unknown_system: None,
            concept_status: None,
            normalized_code: None,
        },
        code,
        Some(system_url),
        None,
        None,
        None,
        request_path,
    ))
}

/// Build a canonical cache key for the `$validate-code` handler-response cache.
///
/// Returns `None` when caching MUST be skipped because the response is
/// effectively unique-per-request:
///
/// * any parameter carries an inline `resource` body (`valueSet`, `tx-resource`,
///   `system`, …) — those vary on every distinct compose / supplement payload
///   and would pollute the cache;
/// * the request includes `default-valueset-version`, `force-system-version`,
///   `system-version`, `check-system-version`, or `useSupplement` — these
///   force slow paths whose outcome depends on global terminology state in
///   ways that the simple per-params key cannot fully capture safely.
///
/// Otherwise every `(name, valueXxx)` pair is serialised as a compact JSON
/// fragment and the fragments are sorted by name (stable for repeated
/// parameter names: their relative order is preserved as a secondary key
/// because we rely on `sort_by_key` for the primary axis).  The resulting
/// string is the cache key.
fn build_validate_code_cache_key(params: &[Value]) -> Option<String> {
    // Reject params that depend on inline FHIR resources or on session-scoped
    // version pins / supplements — caching those would be either wasteful or
    // outright unsafe (the response can vary even for the same params if the
    // backend's supplement state shifts mid-run).
    const SKIP_NAMES: &[&str] = &[
        "useSupplement",
        "default-valueset-version",
        "force-system-version",
        "system-version",
        "check-system-version",
    ];
    let mut frags: Vec<(String, String)> = Vec::with_capacity(params.len());
    for p in params {
        // FHIR Parameters entry MUST have a `name` — defensively skip any that
        // don't (caching of malformed input is irrelevant; the slow path will
        // produce the same error response either way).
        let name = match p.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return None,
        };
        // Inline resources: bail.  Even a single `resource` field on any param
        // means we can't cheaply build a stable, compact key.
        if p.get("resource").is_some() {
            return None;
        }
        if SKIP_NAMES.contains(&name) {
            return None;
        }
        // Compact JSON of the whole entry — captures every `valueXxx`,
        // `valueCoding{system,code,version,display}`, `valueCodeableConcept`,
        // including booleans like `lenient-display-validation`.  The
        // serialiser preserves field order from the input map — this is fine
        // here because the k6 driver and tx-ecosystem fixtures send identical
        // bytes per request.  In the unlikely event of a key collision the
        // worst case is a cache miss, never an incorrect response: identical
        // canonical params => identical handler output by construction.
        let frag = match serde_json::to_string(p) {
            Ok(s) => s,
            Err(_) => return None,
        };
        frags.push((name.to_string(), frag));
    }
    frags.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::with_capacity(frags.iter().map(|(_, f)| f.len() + 1).sum());
    for (i, (_, f)) in frags.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(f);
    }
    Some(out)
}

/// Fetch a cached `$validate-code` response by canonical key.
fn validate_code_cache_get(
    cache: &ValidateCodeHandlerCache,
    key: &str,
) -> Option<Arc<Value>> {
    cache.read().ok()?.get(key).cloned()
}

/// Insert a successfully-built `$validate-code` response into the per-AppState
/// cache.  Drops new entries silently once the cache reaches
/// [`VALIDATE_CODE_HANDLER_CACHE_MAX`].
fn validate_code_cache_put(
    cache: &ValidateCodeHandlerCache,
    key: String,
    value: Arc<Value>,
) {
    if let Ok(mut guard) = cache.write() {
        if guard.len() >= VALIDATE_CODE_HANDLER_CACHE_MAX {
            return;
        }
        guard.insert(key, value);
    }
}

/// Core validate-code logic for `CodeSystem/$validate-code`.
///
/// Accepts three input forms (checked in priority order):
///
/// 1. **`code`** parameter — requires `url` (CodeSystem canonical URL); `system`
///    is intentionally not accepted here (FHIR spec distinction).
/// 2. **`coding`** (`valueCoding`) — system and code bundled in a single object.
/// 3. **`codeableConcept`** (`valueCodeableConcept`) — returns `true` if *any*
///    coding in the concept is valid.
///
/// ## Returns
///
/// A FHIR `Parameters` resource with `result` (boolean), optional `display`
/// (on success), and optional `message` (on display mismatch or failure).
///
/// ## Errors
///
/// Returns [`HtsError::InvalidRequest`] when none of the three input forms are
/// present, or when `url` is absent for the bare-code form.
pub(crate) async fn process_validate_code<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // ── Handler-level response cache (CS path) ───────────────────────────────
    // Skips ALL pre-call helpers (resolve_supplements, supplement_url_in_coding_error,
    // CodeSystemOperations::validate_code) when the same canonical params have
    // produced a response earlier in this AppState's lifetime.  Cleared on
    // every bundle import / CRUD write via `clear_expand_cache`.
    let cache_key = build_validate_code_cache_key(&params);
    if let Some(ref key) = cache_key {
        if let Some(cached) = validate_code_cache_get(&state.cs_validate_code_handler_cache, key) {
            return Ok((*cached).clone());
        }
    }
    let result = process_validate_code_inner(state, params).await;
    if let (Ok(value), Some(key)) = (&result, cache_key) {
        validate_code_cache_put(
            &state.cs_validate_code_handler_cache,
            key,
            Arc::new(value.clone()),
        );
    }
    result
}

async fn process_validate_code_inner<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    let ctx = TenantContext::system();
    // The IG `display/`, `language2/`, and parts of `validation/` test groups
    // pin the response display + invalid-display issue text against the
    // requested `displayLanguage` parameter. Pulled here so all three input
    // forms (code / coding / codeableConcept) can pass it to the post-build
    // language-aware display validator.
    let display_language: Option<String> = find_str_param(&params, "displayLanguage");
    // Reject malformed BCP-47 `displayLanguage` early — IG
    // `display/validation-wrong-de-en-bad` and the language2 group expect a
    // 4xx OperationOutcome with `code=processing` and the
    // INVALID_DISPLAY_NAME message-id.  We use a sentinel-prefixed
    // `VsInvalid` error so the handler can render the correct shape.
    if let Some(ref lang) = display_language {
        if !is_well_formed_display_language(lang) {
            return Err(HtsError::VsInvalid(format!(
                "{}{lang}",
                INVALID_DISPLAY_LANGUAGE_PREFIX
            )));
        }
    }
    // ── Path 1: bare `code` parameter (requires `url` = CodeSystem canonical URL) ──
    if let Some(code) = find_str_param(&params, "code") {
        let system = find_str_param(&params, "url").ok_or_else(|| {
            HtsError::InvalidRequest(
                "Missing required parameter: url (CodeSystem canonical URL). \
                 Use `url`, not `system`, for CodeSystem/$validate-code."
                    .into(),
            )
        })?;
        // Reject when the `url` resolves to a supplement (FHIR R5 §4.7.10):
        // supplements aren't a valid Coding.system value. Matches the IG
        // `extensions/validate-coding-bad-supplement-url` fixture.
        if let Some(value) = supplement_url_in_coding_error(
            state.backend(),
            &ctx,
            &system,
            Some(&code),
            RequestPath::BareCode,
        )
        .await
        {
            return Ok(value);
        }
        let supplements =
            resolve_supplements(state.backend(), &ctx, &params, Some(&system)).await?;
        let display = find_str_param(&params, "display");
        let req_version = find_str_param(&params, "version");
        let req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system.clone()),
            code: code.clone(),
            version: req_version.clone(),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            input_form: Some("code".into()),
            lenient_display_validation: params
                .iter()
                .find(|p| {
                    p.get("name").and_then(|v| v.as_str()) == Some("lenient-display-validation")
                })
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            default_value_set_versions: std::collections::HashMap::new(),
        };
        let mut resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
        rescue_via_supplements(
            state.backend(),
            &ctx,
            &supplements,
            &system,
            &code,
            display.as_deref(),
            &mut resp,
        )
        .await;
        let mut value = build_validate_response_async(
            state.backend(),
            &ctx,
            resp,
            Some(&code),
            Some(&system),
            None,
            RequestPath::BareCode,
            None,
            display_language.as_deref(),
            display.as_deref(),
            &supplements,
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    // ── Path 2: `coding` parameter (valueCoding — system+code bundled together) ──
    if let Some((system, code, coding_display, coding_version)) =
        extract_coding_full(&params, "coding")
    {
        // Reject when the Coding.system points at a supplement.
        if let Some(value) = supplement_url_in_coding_error(
            state.backend(),
            &ctx,
            &system,
            Some(&code),
            RequestPath::Coding,
        )
        .await
        {
            return Ok(value);
        }
        // Coding.display takes precedence over a top-level `display` param —
        // the IG fixtures pin display via the Coding so the server can
        // report a mismatch.
        let display = coding_display.or_else(|| find_str_param(&params, "display"));
        // Coding.version takes precedence over a top-level `version` param.
        let req_version = coding_version.or_else(|| find_str_param(&params, "version"));
        let supplements =
            resolve_supplements(state.backend(), &ctx, &params, Some(&system)).await?;
        let req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system.clone()),
            code: code.clone(),
            version: req_version.clone(),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            input_form: Some("coding".into()),
            lenient_display_validation: params
                .iter()
                .find(|p| {
                    p.get("name").and_then(|v| v.as_str()) == Some("lenient-display-validation")
                })
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            default_value_set_versions: std::collections::HashMap::new(),
        };
        let mut resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
        rescue_via_supplements(
            state.backend(),
            &ctx,
            &supplements,
            &system,
            &code,
            display.as_deref(),
            &mut resp,
        )
        .await;
        let mut value = build_validate_response_async(
            state.backend(),
            &ctx,
            resp,
            Some(&code),
            Some(&system),
            None,
            RequestPath::Coding,
            None,
            display_language.as_deref(),
            display.as_deref(),
            &supplements,
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    // ── Path 3: `codeableConcept` parameter (multiple codings — true if any matches) ──
    if let Some(codings) = extract_codeable_concept(&params, "codeableConcept") {
        if codings.is_empty() {
            return Err(HtsError::InvalidRequest(
                "codeableConcept parameter has no valid coding entries".into(),
            ));
        }
        // Bad-supplement rejection still applies — we don't yet know which
        // coding's system will win, so verify each supplement is *known* (no
        // target enforcement until we know the matched coding's system).
        let _ = resolve_supplements(state.backend(), &ctx, &params, None).await?;
        // Capture the original valueCodeableConcept so we can echo it in the response.
        let cc_value = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("codeableConcept"))
            .and_then(|p| p.get("valueCodeableConcept"))
            .cloned();
        // The IG fixtures expect the LAST matching coding to win (when several
        // codings in a CodeableConcept all validate, the response echoes the
        // last one). Iterate in reverse so the earliest "yes" we find is the
        // last entry in the input.
        let cc_req_version = find_str_param(&params, "version");
        let cs_lenient = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("lenient-display-validation"))
            .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()));
        for (system, code) in codings.into_iter().rev() {
            let req = ValidateCodeRequest {
                url: None,
                value_set_version: None,
                system: Some(system.clone()),
                code: code.clone(),
                version: cc_req_version.clone(),
                display: None,
                date: find_str_param(&params, "date"),
                include_abstract: params
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                    .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
                input_form: Some("codeableConcept".into()),
                lenient_display_validation: cs_lenient,
                default_value_set_versions: std::collections::HashMap::new(),
            };
            let resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
            if resp.result {
                return Ok(build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    None,
                    display_language.as_deref(),
                    None,
                    &[],
                )
                .await);
            }
        }
        // No coding matched
        return Ok(build_validate_response(
            ValidateCodeResponse {
                result: false,
                message: Some("None of the provided codings were found in any CodeSystem".into()),
                display: None,
                system: None,
                cs_version: None,
                inactive: None,
                issues: vec![],
                caused_by_unknown_system: None,
                concept_status: None,
                normalized_code: None,
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            None,
            RequestPath::CodeableConcept,
        ));
    }

    Err(HtsError::InvalidRequest(
        "Must provide one of: code, coding (valueCoding), or \
         codeableConcept (valueCodeableConcept)"
            .into(),
    ))
}

/// POST /CodeSystem/$validate-code
pub async fn validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let mut params = extract_parameter_array(&body)?;
    crate::operations::expand::inject_accept_language(&headers, &mut params);
    match process_validate_code(&state, params).await {
        Ok(v) => Ok(fhir_respond(v, format)),
        Err(e) => match invalid_display_language_response(&e) {
            Some(resp) => Ok(resp),
            None => Err(e),
        },
    }
}

/// GET /CodeSystem/$validate-code?url=...&code=...
pub async fn get_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    match process_validate_code(&state, params).await {
        Ok(v) => Ok(fhir_respond(v, format)),
        Err(e) => match invalid_display_language_response(&e) {
            Some(resp) => Ok(resp),
            None => Err(e),
        },
    }
}

// ── ValueSet/$validate-code ────────────────────────────────────────────────────

/// Returns true if `version` satisfies the wildcard `pattern`.
/// "1.x" matches "1.0.0", "1.2.0", etc. "1.0.x" matches "1.0.0", "1.0.1".
/// "1.x.x" matches "1.0.0", "1.2.3" (segment-wise: each "x" is any segment).
/// Mirrors the helper in `backends/sqlite/value_set.rs`.
fn version_satisfies_wildcard(version: &str, pattern: &str) -> bool {
    if pattern == "x" {
        return true;
    }
    let pat_segs: Vec<&str> = pattern.split('.').collect();
    let ver_segs: Vec<&str> = version.split('.').collect();

    let ends_with_x = pat_segs.last().is_some_and(|s| *s == "x");
    if !ends_with_x && pat_segs.len() != ver_segs.len() {
        return false;
    }
    if ends_with_x && ver_segs.len() < pat_segs.len() - 1 {
        return false;
    }
    for (i, ps) in pat_segs.iter().enumerate() {
        if *ps == "x" {
            continue;
        }
        match ver_segs.get(i) {
            Some(vs) if vs == ps => {}
            _ => return false,
        }
    }
    true
}

/// Pull the include-pinned version for `system_url` out of a ValueSet
/// resource. Returns `Some(Some(v))` when an include for that system pins a
/// specific version, `Some(None)` for a versionless include match, and
/// `None` when no include matches the system at all. Used by the IG-style
/// version-param resolver to skip applying a default when the VS already
/// pins the include.
fn vs_include_pin_for_system(vs: &Value, system_url: &str) -> Option<Option<String>> {
    let includes = vs.get("compose")?.get("include")?.as_array()?;
    for inc in includes {
        if inc.get("system").and_then(|v| v.as_str()) == Some(system_url) {
            let ver = inc
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            return Some(ver);
        }
    }
    None
}

/// Resolve a (possibly wildcard) version pattern against the set of stored
/// versions for a CodeSystem URL. Picks the highest matching version.
/// Returns `None` when no stored version matches (or the CS is unknown).
async fn resolve_cs_version_pattern<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system_url: &str,
    pattern: &str,
) -> Option<String> {
    // Exact (non-wildcard) version: just return it as-is. The backend will
    // detect mismatches against stored data when relevant.
    if !pattern.contains(".x") && pattern != "x" {
        return Some(pattern.to_string());
    }
    let hits = CodeSystemOperations::search(
        backend,
        ctx,
        crate::types::ResourceSearchQuery {
            url: Some(system_url.to_string()),
            count: Some(50),
            ..Default::default()
        },
    )
    .await
    .ok()?;
    let mut versions: Vec<String> = hits
        .iter()
        .filter_map(|cs| {
            cs.get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|v| version_satisfies_wildcard(v, pattern))
        .collect();
    versions.sort();
    versions.pop()
}

/// Find the first `(system, version_pattern)` pair matching `target_system`
/// in a list collected via [`collect_canonical_params`].
fn find_pin_for_system<'a>(pins: &'a [(String, String)], target_system: &str) -> Option<&'a str> {
    pins.iter()
        .find(|(s, _)| s == target_system)
        .map(|(_, v)| v.as_str())
}

/// Strip VS-pin-mismatch issues from a backend response when a
/// `force-system-version` parameter overrode the version selection. The
/// backend's mismatch detector looks at the request's version vs the VS
/// compose pin; when the operations layer has *forced* a different version
/// for that system (potentially making the VS pin moot), the resulting
/// mismatch issue is incorrect. Removes `VALUESET_VALUE_MISMATCH` and the
/// paired `UNKNOWN_CODESYSTEM_VERSION` issues, flips `result` back to true
/// (when the only barriers were those), clears `cs_version` echo to the
/// forced value, and clears `caused_by_unknown_system`. Also attempts to
/// repopulate `resp.display` from the forced version when possible (the
/// expansion may have been computed against a different version).
async fn suppress_forced_version_mismatch<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: &mut crate::types::ValidateCodeResponse,
    system_url: &str,
    code: &str,
    forced_version: &str,
) {
    let had_mismatch = resp
        .issues
        .iter()
        .any(|i| i.message_id.as_deref() == Some("VALUESET_VALUE_MISMATCH"));
    if !had_mismatch {
        return;
    }
    resp.issues.retain(|i| {
        let mid = i.message_id.as_deref();
        !matches!(
            mid,
            Some("VALUESET_VALUE_MISMATCH") | Some("UNKNOWN_CODESYSTEM_VERSION")
        )
    });
    resp.caused_by_unknown_system = None;
    // If no error-severity issues remain, treat the validation as a pass.
    let any_error = resp.issues.iter().any(|i| i.severity == "error");
    if !any_error {
        resp.result = true;
        resp.message = None;
        resp.cs_version = Some(forced_version.to_string());
        // Look up the display at the forced version via a CodeSystem-level
        // validate-code (cheaper than a generic $lookup) so the response
        // reflects the canonical display for the forced version, not the
        // expansion's chosen version.
        let cs_req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system_url.to_string()),
            code: code.to_string(),
            version: Some(forced_version.to_string()),
            display: None,
            date: None,
            include_abstract: None,
            input_form: None,
            lenient_display_validation: None,
            default_value_set_versions: std::collections::HashMap::new(),
        };
        if let Ok(cs_resp) = CodeSystemOperations::validate_code(backend, ctx, cs_req).await {
            if cs_resp.result {
                if let Some(d) = cs_resp.display {
                    resp.display = Some(d);
                }
            }
        }
    }
}

/// Strip a `Some(None)` (versionless include) `VALUESET_VALUE_MISMATCH` from
/// a backend response when a `system-version` (DEFAULT) parameter applied for
/// the system. The default *is* the effective VS version when the include is
/// versionless, so the backend's mismatch detector — which compares the
/// caller's version against the latest stored CS version — produces a
/// spurious error.
///
/// Mutates `resp` in-place: removes the mismatch issue, clears
/// `caused_by_unknown_system`, restores `cs_version` to the default-applied
/// version, and re-runs a CodeSystem-level validate at that version to
/// repopulate `display`. When all errors are gone, flips `result=true`.
async fn suppress_default_versionless_mismatch<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: &mut crate::types::ValidateCodeResponse,
    system_url: &str,
    code: &str,
    default_version: &str,
) {
    let had_mismatch = resp
        .issues
        .iter()
        .any(|i| i.message_id.as_deref() == Some("VALUESET_VALUE_MISMATCH"));
    if !had_mismatch {
        return;
    }
    resp.issues
        .retain(|i| i.message_id.as_deref() != Some("VALUESET_VALUE_MISMATCH"));
    let any_error = resp.issues.iter().any(|i| i.severity == "error");
    if !any_error {
        resp.result = true;
        resp.message = None;
        resp.cs_version = Some(default_version.to_string());
        resp.caused_by_unknown_system = None;
        let cs_req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system_url.to_string()),
            code: code.to_string(),
            version: Some(default_version.to_string()),
            display: None,
            date: None,
            include_abstract: None,
            input_form: None,
            lenient_display_validation: None,
            default_value_set_versions: std::collections::HashMap::new(),
        };
        if let Ok(cs_resp) = CodeSystemOperations::validate_code(backend, ctx, cs_req).await {
            if cs_resp.result {
                if let Some(d) = cs_resp.display {
                    resp.display = Some(d);
                }
            }
        }
    }
}

/// Transform the backend's `VALUESET_VALUE_MISMATCH` (and the warning-severity
/// `VALUESET_VALUE_MISMATCH_DEFAULT`) into the IG-mandated
/// `VALUESET_VALUE_MISMATCH_CHANGED` (severity=error) when:
///
/// 1. The VS include for `system_url` is versionless,
/// 2. A `system-version` (DEFAULT) pin applied for that system, and
/// 3. The caller supplied an explicit version that disagrees with the default.
///
/// The IG fixtures (`code-vbb-vsnn-default`, `coding-vbb-vsnn-default`,
/// `codeableconcept-vbb-vsnn-default`) expect the response to reflect the
/// default-applied version (`<default>`) on the top-level `version` echo, with
/// the mismatch text reading "...version 'X' resulting from the version '<orig>'
/// in the ValueSet include is different to the one in the value ('<orig>')"
/// rather than the legacy "...for the versionless include..." form.
///
/// Mutates `resp` in-place. Does nothing when no mismatch issue is present
/// (e.g. when the caller's version equals the default — the base
/// `suppress_default_versionless_mismatch` helper already handles that case).
async fn transform_default_versionless_mismatch_to_changed<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: &mut crate::types::ValidateCodeResponse,
    system_url: &str,
    code: &str,
    default_version: &str,
    original_version: &str,
    text_version: &str,
) {
    // Find any backend-emitted mismatch issue (error or warning variant).
    let mismatch_idx = resp
        .issues
        .iter()
        .position(|i| {
            matches!(
                i.message_id.as_deref(),
                Some("VALUESET_VALUE_MISMATCH") | Some("VALUESET_VALUE_MISMATCH_DEFAULT")
            )
        });
    let Some(idx) = mismatch_idx else {
        return;
    };
    // Replace the issue with the IG-canonical CHANGED form. The text format:
    //   "The code system '<sys>' version '<resolved_or_pattern>' resulting
    //    from the version '<vs_include_version>' in the ValueSet include is
    //    different to the one in the value ('<caller_version>')"
    //
    // - `text_version` is the resolved CS version for system-version=...|1.0.0
    //   pins (matches the resolved row), and the wildcard pattern itself for
    //   check-system-version=...|1.0.x pins (the pattern is preserved in the
    //   IG fixtures' text even though the matched CS row is 1.0.0).
    // - The "resulting from the version 'X' in the ValueSet include" piece
    //   names the include's pinned version. The caller of this helper only
    //   reaches this branch when the include is *versionless*, so X is `""`.
    // - The trailing `('Y')` is the caller's actual version.
    let new_text = format!(
        "The code system '{system_url}' version '{text_version}' resulting from the version '' in the ValueSet include is different to the one in the value ('{original_version}')"
    );
    let original_loc = resp.issues[idx].location.clone();
    let original_expr = resp.issues[idx].expression.clone();
    resp.issues[idx] = crate::types::ValidationIssue {
        severity: "error".into(),
        fhir_code: "invalid".into(),
        tx_code: "vs-invalid".into(),
        text: new_text,
        expression: original_expr,
        location: original_loc,
        message_id: Some("VALUESET_VALUE_MISMATCH_CHANGED".into()),
    };
    // The backend echoed the *latest* CS version on this failure path. The
    // IG fixtures expect the default-applied version instead, so we override
    // the response's cs_version + re-resolve the display at that version.
    resp.cs_version = Some(default_version.to_string());
    let cs_req = ValidateCodeRequest {
        url: None,
        value_set_version: None,
        system: Some(system_url.to_string()),
        code: code.to_string(),
        version: Some(default_version.to_string()),
        display: None,
        date: None,
        include_abstract: None,
        input_form: None,
        lenient_display_validation: None,
        default_value_set_versions: std::collections::HashMap::new(),
    };
    if let Ok(cs_resp) = CodeSystemOperations::validate_code(backend, ctx, cs_req).await {
        if cs_resp.result {
            if let Some(d) = cs_resp.display {
                resp.display = Some(d);
            }
        }
    }
    // Recompute the response message from the (possibly transformed) issue
    // texts so the top-level `message` reflects the new wording.
    let mut texts: Vec<&str> = resp
        .issues
        .iter()
        .filter(|i| i.severity == "error")
        .map(|i| i.text.as_str())
        .collect();
    texts.sort_unstable();
    if !texts.is_empty() {
        resp.message = Some(texts.join("; "));
    }
}

/// Override the response's echoed `cs_version` (and re-resolve `display`) to
/// the default-applied version when:
///
/// 1. A `system-version` (DEFAULT) pin applied for `system_url`,
/// 2. The backend emitted `UNKNOWN_CODESYSTEM_VERSION` because the VS-include
///    pinned a non-existent version, and
/// 3. The default version exists in the stored CS rows.
///
/// The IG `code-vnn-vs1wb-default` / `coding-vnn-vs1wb-default` fixtures
/// expect the response to echo the default-applied version (1.0.0) on the
/// top-level `version` parameter rather than the latest stored CS version
/// (1.2.0) — the default takes effect even though the include pin itself is
/// invalid. Mutates `resp` in-place; no-op when no UNKNOWN issue is present
/// or the existing echo already matches the default.
async fn apply_default_to_unknown_version_echo<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: &mut crate::types::ValidateCodeResponse,
    system_url: &str,
    code: &str,
    default_version: &str,
) {
    let has_unknown = resp
        .issues
        .iter()
        .any(|i| i.message_id.as_deref() == Some("UNKNOWN_CODESYSTEM_VERSION"));
    if !has_unknown {
        return;
    }
    if resp.cs_version.as_deref() == Some(default_version) {
        return;
    }
    resp.cs_version = Some(default_version.to_string());
    let cs_req = ValidateCodeRequest {
        url: None,
        value_set_version: None,
        system: Some(system_url.to_string()),
        code: code.to_string(),
        version: Some(default_version.to_string()),
        display: None,
        date: None,
        include_abstract: None,
        input_form: None,
        lenient_display_validation: None,
        default_value_set_versions: std::collections::HashMap::new(),
    };
    if let Ok(cs_resp) = CodeSystemOperations::validate_code(backend, ctx, cs_req).await {
        if cs_resp.result {
            if let Some(d) = cs_resp.display {
                resp.display = Some(d);
            }
        }
    }
}

/// Look up all stored `CodeSystem.version` strings for `system_url` (sorted
/// ascending). Used by the force-caller-version-unknown helper to (a) decide
/// whether the caller's version is actually unknown and (b) format the
/// "Valid versions: …" suffix in the UNKNOWN_CODESYSTEM_VERSION message.
async fn cs_stored_versions<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    system_url: &str,
) -> Vec<String> {
    let hits = match CodeSystemOperations::search(
        backend,
        ctx,
        crate::types::ResourceSearchQuery {
            url: Some(system_url.to_string()),
            count: Some(50),
            ..Default::default()
        },
    )
    .await
    {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let mut versions: Vec<String> = hits
        .iter()
        .filter_map(|cs| {
            cs.get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    versions.sort();
    versions
}

/// Format "X", "X or Y", or "X, Y or Z" — mirrors the SQLite backend's
/// `format_valid_versions_msg` so the operations-layer-emitted UNKNOWN
/// message text matches the IG fixtures verbatim.
fn format_valid_versions_msg_op(versions: &[String]) -> String {
    match versions {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let (last, rest) = versions.split_last().unwrap();
            format!("{} or {}", rest.join(", "), last)
        }
    }
}

/// Inject the IG-required `VALUESET_VALUE_MISMATCH_CHANGED` +
/// `UNKNOWN_CODESYSTEM_VERSION` failure pair when a `force-system-version`
/// pin has overridden the caller's *unknown* version.
///
/// Triggered by the operations-layer caller when:
/// 1. A `force-system-version` pin applies for `system_url`,
/// 2. The caller supplied an explicit version (`original_version`),
/// 3. That version does NOT satisfy the force pattern, and
/// 4. That version is NOT a stored CodeSystem version.
///
/// Without this transformation the response would (incorrectly) report
/// success — the upstream `req_version` is rewritten to the resolved force
/// version before the backend is invoked, so the backend never sees the
/// caller's unknown version.
///
/// IG fixtures driving this branch: `code/coding/codeableconcept-vbb-vs10-force`
/// and `…-vbb-vsnn-force` (see `tests/version/`). The mismatch text format is:
///
///   "The code system '<sys>' version '<force_pattern>' resulting from the
///    version '<vs_include_version_or_empty>' in the ValueSet include is
///    different to the one in the value ('<original_version>')"
///
/// Mutates `resp` in-place: appends issues, sets `result=false`, sets
/// `cs_version` to the resolved force version, sets
/// `caused_by_unknown_system=<sys>|<original_version>`, and recomputes
/// `message` from the new error texts.
async fn apply_force_caller_version_unknown_failure<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: &mut crate::types::ValidateCodeResponse,
    system_url: &str,
    original_version: &str,
    force_pattern: &str,
    vs_include_version: Option<&str>,
    resolved_force_version: &str,
    request_path: RequestPath,
) {
    // Don't double-apply when the failure shape is already present.
    if resp
        .issues
        .iter()
        .any(|i| i.message_id.as_deref() == Some("VALUESET_VALUE_MISMATCH_CHANGED"))
    {
        return;
    }

    let stored = cs_stored_versions(backend, ctx, system_url).await;
    if stored.iter().any(|v| v == original_version) {
        // Caller's version is actually known — fall back to the standard
        // suppression / mismatch detection path.
        return;
    }

    // Strip any pre-existing mismatch issues (we replace them with the
    // CHANGED + UNKNOWN pair). The base `suppress_forced_version_mismatch`
    // helper already removed VALUESET_VALUE_MISMATCH and
    // UNKNOWN_CODESYSTEM_VERSION when it ran, but defend against re-runs.
    resp.issues.retain(|i| {
        !matches!(
            i.message_id.as_deref(),
            Some("VALUESET_VALUE_MISMATCH")
                | Some("VALUESET_VALUE_MISMATCH_DEFAULT")
                | Some("UNKNOWN_CODESYSTEM_VERSION")
        )
    });

    let (version_loc, system_loc) = match request_path {
        RequestPath::BareCode => ("version", "system"),
        RequestPath::CodeableConcept => (
            "CodeableConcept.coding[0].version",
            "CodeableConcept.coding[0].system",
        ),
        RequestPath::Coding => ("Coding.version", "Coding.system"),
    };

    let inc_ver_text = vs_include_version.unwrap_or("");
    let mismatch_text = format!(
        "The code system '{system_url}' version '{force_pattern}' resulting from the version '{inc_ver_text}' in the ValueSet include is different to the one in the value ('{original_version}')"
    );
    let valid_str = format_valid_versions_msg_op(&stored);
    let unknown_text = format!(
        "A definition for CodeSystem '{system_url}' version '{original_version}' could not be found, so the code cannot be validated. Valid versions: {valid_str}"
    );

    resp.issues.push(crate::types::ValidationIssue {
        severity: "error".into(),
        fhir_code: "invalid".into(),
        tx_code: "vs-invalid".into(),
        text: mismatch_text.clone(),
        expression: Some(version_loc.into()),
        location: Some(version_loc.into()),
        message_id: Some("VALUESET_VALUE_MISMATCH_CHANGED".into()),
    });
    resp.issues.push(crate::types::ValidationIssue {
        severity: "error".into(),
        fhir_code: "not-found".into(),
        tx_code: "not-found".into(),
        text: unknown_text.clone(),
        expression: Some(system_loc.into()),
        location: Some(system_loc.into()),
        message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
    });

    resp.result = false;
    resp.cs_version = Some(resolved_force_version.to_string());
    resp.caused_by_unknown_system = Some(format!("{system_url}|{original_version}"));

    // Order matches the IG fixtures: UNKNOWN first, then MISMATCH.
    resp.message = Some(format!("{unknown_text}; {mismatch_text}"));
}

/// Pull the `version` valueString out of an already-built validate-code
/// response (FHIR Parameters resource). Used as a fallback when the backend
/// did not populate `resp.cs_version` directly.
fn extract_response_version(response: &Value) -> Option<String> {
    response
        .get("parameter")
        .and_then(|v| v.as_array())?
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("version"))
        .and_then(|p| p.get("valueString").and_then(|v| v.as_str()))
        .map(str::to_string)
}

/// Append the IG-mandated VALUESET_VERSION_CHECK error issue to a built
/// validate-code response when the resolved CS version doesn't satisfy the
/// `check-system-version` pattern. Mutates `response` in-place: appends an
/// issue to the OperationOutcome (creating it if absent), flips `result` to
/// false, sets `message`, and adjusts the displayed `version` echo when
/// needed. The location/expression varies with the request shape.
fn apply_check_version_failure(
    response: &mut Value,
    system_url: &str,
    resolved_version: &str,
    pattern: &str,
    request_path: RequestPath,
) {
    let location = match request_path {
        RequestPath::BareCode => "version",
        RequestPath::CodeableConcept => "CodeableConcept.coding[0].version",
        RequestPath::Coding => "Coding.version",
    };
    let text = format!(
        "The version '{resolved_version}' is not allowed for system '{system_url}': required \
         to be '{pattern}' by a version-check parameter"
    );
    let issue = json!({
        "extension": [{
            "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
            "valueString": "VALUESET_VERSION_CHECK"
        }],
        "severity": "error",
        "code": "exception",
        "details": {
            "coding": [{
                "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                "code": "version-error"
            }],
            "text": text,
        },
        "location": [location],
        "expression": [location],
    });

    let params = match response.get_mut("parameter").and_then(|v| v.as_array_mut()) {
        Some(a) => a,
        None => return,
    };

    // The IG fixtures suppress VALUESET_VERSION_CHECK when an
    // UNKNOWN_CODESYSTEM_VERSION error already invalidates the response —
    // the version-check is meaningless if the version itself is unknown
    // (cases: vbb-vsnn-check, vnn-vs1wb-check). Detect that on the existing
    // OperationOutcome before pushing.
    let already_has_unknown_version = params.iter().any(|p| {
        if p.get("name").and_then(|v| v.as_str()) != Some("issues") {
            return false;
        }
        let issues = match p
            .get("resource")
            .and_then(|r| r.get("issue"))
            .and_then(|v| v.as_array())
        {
            Some(a) => a,
            None => return false,
        };
        issues.iter().any(|iss| {
            iss.get("extension")
                .and_then(|e| e.as_array())
                .map(|exts| {
                    exts.iter().any(|ext| {
                        ext.get("url").and_then(|u| u.as_str())
                            == Some("http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id")
                            && ext.get("valueString").and_then(|v| v.as_str())
                                == Some("UNKNOWN_CODESYSTEM_VERSION")
                    })
                })
                .unwrap_or(false)
        })
    });
    if already_has_unknown_version {
        return;
    }

    // Locate (or create) the `issues` parameter and push our new issue.
    let mut found_issues = false;
    for p in params.iter_mut() {
        if p.get("name").and_then(|v| v.as_str()) == Some("issues") {
            if let Some(oo) = p.get_mut("resource") {
                if let Some(arr) = oo.get_mut("issue").and_then(|v| v.as_array_mut()) {
                    arr.push(issue.clone());
                } else {
                    oo["issue"] = json!([issue.clone()]);
                }
                found_issues = true;
                break;
            }
        }
    }
    if !found_issues {
        params.push(json!({
            "name": "issues",
            "resource": {
                "resourceType": "OperationOutcome",
                "issue": [issue],
            }
        }));
    }

    // Recompose the top-level `message` string from ALL error-severity issue
    // texts in the OperationOutcome (sorted alphabetically, joined with `; `),
    // matching the convention used in `build_validate_response` so the message
    // includes both the new VALUESET_VERSION_CHECK text AND any pre-existing
    // VALUESET_VALUE_MISMATCH / UNKNOWN_CODESYSTEM_VERSION texts. The IG
    // fixtures (e.g. code-v10-vs20-check, code-v10-vsnn-check) expect the
    // mismatch and version-check messages joined together when both are
    // present.
    let combined_message: String = {
        let mut texts: Vec<String> = Vec::new();
        for p in params.iter() {
            if p.get("name").and_then(|v| v.as_str()) != Some("issues") {
                continue;
            }
            let issues = match p
                .get("resource")
                .and_then(|r| r.get("issue"))
                .and_then(|v| v.as_array())
            {
                Some(a) => a,
                None => continue,
            };
            for iss in issues {
                let sev = iss.get("severity").and_then(|v| v.as_str()).unwrap_or("");
                if sev != "error" {
                    continue;
                }
                if let Some(t) = iss
                    .get("details")
                    .and_then(|d| d.get("text"))
                    .and_then(|v| v.as_str())
                {
                    texts.push(t.to_string());
                }
            }
        }
        texts.sort();
        texts.join("; ")
    };
    let final_message = if combined_message.is_empty() {
        text.clone()
    } else {
        combined_message
    };

    // Flip `result` to false and set/replace `message` with the recomposed
    // text (which includes the version-check error plus any prior errors).
    for p in params.iter_mut() {
        match p.get("name").and_then(|v| v.as_str()) {
            Some("result") => {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("valueBoolean".into(), Value::Bool(false));
                }
            }
            Some("message") => {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("valueString".into(), Value::String(final_message.clone()));
                }
            }
            _ => {}
        }
    }
    // If `message` was absent, append it just after `issues`.
    let has_message = params
        .iter()
        .any(|p| p.get("name").and_then(|v| v.as_str()) == Some("message"));
    if !has_message {
        // Insert message right before `result` to preserve spec ordering.
        let result_idx = params
            .iter()
            .position(|p| p.get("name").and_then(|v| v.as_str()) == Some("result"));
        let entry = json!({"name": "message", "valueString": final_message});
        match result_idx {
            Some(i) => params.insert(i, entry),
            None => params.push(entry),
        }
    }
}

/// Inspect the compose.include[*].valueSet entries of the named ValueSet and
/// return the first canonical URL that does not resolve to a stored
/// ValueSet (after stripping any `|version` suffix). Returns `None` when the
/// VS isn't found, has no compose.include, has no valueSet imports, or every
/// import resolves successfully.
///
/// The IG `validation/simple-*-bad-import` fixtures expect a single
/// `not-found / Unable_to_resolve_value_Set_` issue when an import cannot
/// be resolved — this helper drives the early-exit detection in
/// `process_vs_validate_code`.
async fn detect_bad_vs_import<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    vs_url: &str,
    vs_version: Option<&str>,
    default_vs_versions: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut hits = ValueSetOperations::search(
        backend,
        ctx,
        crate::types::ResourceSearchQuery {
            url: Some(vs_url.to_string()),
            version: vs_version.map(str::to_string),
            count: Some(1),
            ..Default::default()
        },
    )
    .await
    .ok()?;
    let vs = hits.pop()?;
    let includes = vs
        .get("compose")
        .and_then(|c| c.get("include"))
        .and_then(|v| v.as_array())?;
    for inc in includes {
        let imports = match inc.get("valueSet").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => continue,
        };
        for imp in imports {
            let canonical = match imp.as_str() {
                Some(s) => s,
                None => continue,
            };
            let (bare_url, ver) = match canonical.split_once('|') {
                Some((u, v)) => (u, Some(v.to_string())),
                None => (canonical, None),
            };
            // Apply a `default-valueset-version` pin when the import does
            // not carry an explicit `|version`. The IG
            // `valueset-version/coding-indirect-zero-pinned-wrong` fixture
            // pins a non-existent version on a versionless import and the
            // failure text must name the pinned `<url>|<version>`.
            let (lookup_ver, reported) = match (ver, default_vs_versions.get(bare_url)) {
                (Some(v), _) => {
                    let r = format!("{bare_url}|{v}");
                    (Some(v), r)
                }
                (None, Some(default_v)) => {
                    let r = format!("{bare_url}|{default_v}");
                    (Some(default_v.clone()), r)
                }
                (None, None) => (None, bare_url.to_string()),
            };
            let exists = ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(bare_url.to_string()),
                    version: lookup_ver,
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .map(|hs| !hs.is_empty())
            .unwrap_or(false);
            if !exists {
                return Some(reported);
            }
        }
    }
    None
}

/// Inspect the named ValueSet's `valueset-supplement` extensions and verify
/// every referenced supplement CodeSystem is loaded. Returns the resolved
/// [`SupplementInfo`] list on success so callers can auto-apply the supplements
/// (matches the IG `extensions/validate-coding-good-supplement` fixtures —
/// the supplement's designations rescue display mismatches and the
/// `valueset-deprecated` concept extension surfaces as a warning). Returns
/// `Err(NotFound)` when any referenced supplement CS is missing — matches
/// `extensions/validate-*-bad-supplement` 4xx fixtures.
async fn enforce_vs_supplement_extensions<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    vs_url: &str,
    vs_version: Option<&str>,
) -> Result<Vec<SupplementInfo>, HtsError> {
    let mut hits = match ValueSetOperations::search(
        backend,
        ctx,
        crate::types::ResourceSearchQuery {
            url: Some(vs_url.to_string()),
            version: vs_version.map(str::to_string),
            count: Some(1),
            ..Default::default()
        },
    )
    .await
    {
        Ok(h) => h,
        Err(_) => return Ok(Vec::new()),
    };
    let vs = match hits.pop() {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };
    let exts = match vs.get("extension").and_then(|e| e.as_array()) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut out: Vec<SupplementInfo> = Vec::new();
    for ext in exts {
        if ext.get("url").and_then(|u| u.as_str())
            != Some("http://hl7.org/fhir/StructureDefinition/valueset-supplement")
        {
            continue;
        }
        let raw = match ext
            .get("valueCanonical")
            .or_else(|| ext.get("valueUri"))
            .and_then(|v| v.as_str())
        {
            Some(s) => s,
            None => continue,
        };
        let bare = raw.split('|').next().unwrap_or(raw);
        match backend.supplement_target(ctx, bare).await? {
            Some(info) => out.push(info),
            None => {
                return Err(HtsError::NotFound(format!(
                    "Required supplement not found: {bare}"
                )));
            }
        }
    }
    Ok(out)
}

/// `ValueSet/$validate-code` against an inline `valueSet` body.
///
/// Drives the IG `validation/validate-contained-{good,bad}` fixtures: the
/// caller supplies a `valueSet` resource whose `compose.include[].valueSet[]`
/// chain references a `#contained` ValueSet alongside an external canonical.
/// We expand the inline VS via the backend (which resolves `#contained` refs
/// from the inline body before falling back to the local store), then check
/// membership of the supplied code within the resulting expansion.
///
/// Display lookups for the "Display 1" / "inactive" / "version" echo
/// parameters delegate to the underlying CodeSystem via
/// `CodeSystemOperations::validate_code`.
async fn process_inline_vs_validate_code<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
    vs_resource: Value,
) -> Result<Value, HtsError> {
    let ctx = TenantContext::system();

    // Extract the input coding/code (priority: coding → code). For the
    // `coding` form, an empty `system` (Coding without a system field)
    // collapses to None per FHIR spec semantics.
    let (in_system, in_code, in_display) =
        if let Some((sys, cd, disp, _ver)) = extract_coding_full(&params, "coding") {
            let sys_opt = if sys.is_empty() { None } else { Some(sys) };
            (sys_opt, cd, disp)
        } else if let Some(cd) = find_str_param(&params, "code") {
            (find_str_param(&params, "system"), cd, find_str_param(&params, "display"))
        } else {
            return Err(HtsError::InvalidRequest(
                "Must provide one of: code, coding (valueCoding), or codeableConcept \
                 (valueCodeableConcept)"
                    .into(),
            ));
        };

    // Determine the request path so issue locations / parameter echoes match
    // the IG fixture conventions.
    let req_path = if extract_coding_full(&params, "coding").is_some() {
        RequestPath::Coding
    } else {
        RequestPath::BareCode
    };

    // The inline VS is anonymous (no top-level `url`) in the IG fixtures —
    // surface "(unidentified)" in `not-in-vs` text per the expected output.
    let vs_label = vs_resource
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(unidentified)".to_string());

    // Expand the inline VS via the backend. The SQLite backend's expand path
    // walks `compose.include[].valueSet[]` references using the inline
    // `contained[]` array as the first lookup tier, so `#vs1` resolves to the
    // inline contained ValueSet.
    let expand_req = crate::types::ExpandRequest {
        url: None,
        value_set_version: None,
        value_set: Some(vs_resource.clone()),
        filter: None,
        count: None,
        offset: None,
        max_expansion_size: Some(state.max_expansion_size),
        date: None,
        hierarchical: None,
        hierarchical_explicit: false,
        tx_resources: vec![],
        force_system_versions: std::collections::HashMap::new(),
        system_version_defaults: std::collections::HashMap::new(),
        default_value_set_versions: std::collections::HashMap::new(),
    };
    let expansion = ValueSetOperations::expand(state.backend(), &ctx, expand_req).await?;

    // Membership check: match by (system, code) when system is supplied,
    // else by code alone (and infer the system from the matched entry).
    let matched: Option<&crate::types::ExpansionContains> =
        expansion.contains.iter().find(|c| {
            c.code == in_code
                && in_system
                    .as_deref()
                    .map(|s| c.system == s)
                    .unwrap_or(true)
        });

    let resolved_system: Option<String> = matched
        .map(|c| c.system.clone())
        .or_else(|| in_system.clone());

    if let Some(concept) = matched {
        // Look up canonical display + CodeSystem version via the CS
        // validate-code path. The expansion entry's `display` is sufficient
        // for the membership echo, but the CS path also computes
        // `cs_version` and triggers display-mismatch detection.
        let cs_req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(concept.system.clone()),
            code: in_code.clone(),
            version: None,
            display: in_display.clone(),
            date: None,
            include_abstract: None,
            input_form: Some(match req_path {
                RequestPath::BareCode => "code".into(),
                RequestPath::Coding => "coding".into(),
                RequestPath::CodeableConcept => "codeableConcept".into(),
            }),
            lenient_display_validation: None,
            default_value_set_versions: std::collections::HashMap::new(),
        };
        let mut cs_resp = CodeSystemOperations::validate_code(state.backend(), &ctx, cs_req)
            .await
            .unwrap_or_else(|_| ValidateCodeResponse {
                result: true,
                display: concept.display.clone(),
                ..Default::default()
            });
        // Prefer the expansion-supplied display (already language-resolved
        // by the backend) when the CS lookup didn't return one.
        if cs_resp.display.is_none() {
            cs_resp.display = concept.display.clone();
        }
        // The CS path may set `system` to None; surface the resolved one so
        // the response echoes it back per the IG fixtures.
        if cs_resp.system.is_none() {
            cs_resp.system = resolved_system.clone();
        }
        // Look up inactive flag for the matched concept; the IG
        // `validate-contained-good` fixture expects an `inactive=true`
        // top-level parameter and a pair of INACTIVE_CONCEPT_FOUND warnings
        // (one for the generic `inactive` status, one for the specific
        // status `retired`/`deprecated`/`withdrawn` from the
        // structuredefinition-standards-status extension).
        let flags_map = CodeSystemOperations::concept_expansion_flags(
            state.backend(),
            &ctx,
            &concept.system,
            &[in_code.clone()],
        )
        .await
        .ok()
        .unwrap_or_default();
        let is_inactive = flags_map
            .get(&in_code)
            .map(|f| f.inactive)
            .unwrap_or(false);
        if is_inactive {
            cs_resp.inactive = Some(true);
            // Generic INACTIVE_CONCEPT_FOUND warning. Mirrors the SQLite
            // backend's VS path so the operations layer's
            // `lookup_concept_status` follow-up (in
            // `build_validate_response_async`) can surface a second issue
            // when the specific status is retired/deprecated/withdrawn.
            let already = cs_resp.issues.iter().any(|i| {
                i.message_id.as_deref() == Some("INACTIVE_CONCEPT_FOUND")
                    && i.text.contains("has a status of inactive")
            });
            if !already {
                cs_resp.issues.push(ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text: format!(
                        "The concept '{in_code}' has a status of inactive and its use should be reviewed"
                    ),
                    expression: Some("Coding".into()),
                    // The inline-VS path's IG `validate-contained-good`
                    // fixture pins the INACTIVE_CONCEPT_FOUND issue WITHOUT
                    // a `location` (only `expression`).  The URL-based
                    // path (e.g. `simple-coding-bad-code-inactive`) does
                    // expect both — that issue is built elsewhere.
                    location: None,
                    message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
                });
            }
        }
        // Force result=true for membership-only success — display mismatches
        // surface as issues, not as a hard membership failure.
        let has_error = cs_resp.issues.iter().any(|i| i.severity == "error");
        cs_resp.result = !has_error;
        let value = build_validate_response_async(
            state.backend(),
            &ctx,
            cs_resp,
            Some(&in_code),
            resolved_system.as_deref(),
            None,
            req_path,
            None, // no VS canonical URL to surface
            find_str_param(&params, "displayLanguage").as_deref(),
            in_display.as_deref(),
            &[],
        )
        .await;
        return Ok(value);
    }

    // Membership miss — emit the IG `not-in-vs` issue text. Format the
    // qualified code per IG convention: `system#code ('display')`.
    let qualified = match (in_system.as_deref(), in_display.as_deref()) {
        (Some(s), Some(d)) => format!("{s}#{in_code} ('{d}')"),
        (Some(s), None) => format!("{s}#{in_code}"),
        (None, Some(d)) => format!("{in_code} ('{d}')"),
        (None, None) => in_code.clone(),
    };
    let text = format!(
        "The provided code '{qualified}' was not found in the value set '{vs_label}'"
    );
    let issue = ValidationIssue {
        severity: "error".into(),
        fhir_code: "code-invalid".into(),
        tx_code: "not-in-vs".into(),
        text,
        expression: Some(match req_path {
            RequestPath::BareCode => "code".into(),
            _ => "Coding.code".into(),
        }),
        location: None,
        message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
    };
    // Look up the CS version for the `version` echo so the IG fixture can
    // confirm which CS row was checked even though the code wasn't found.
    let cs_version_lookup = match resolved_system.as_deref() {
        Some(s) => CodeSystemOperations::code_system_version_for_url(state.backend(), &ctx, s)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    // When the code is not in the VS BUT the user supplied a `display` AND the
    // code IS known in the underlying CodeSystem, additionally check the
    // display against that CS's canonical display. If they diverge, emit a
    // SECOND warning-severity issue ("Wrong Display Name 'X' for ...") so the
    // tx-ecosystem `batch/batch-validate-bad` fixture sees both signals.
    //
    // The display-mismatch is severity=warning (not error) because the primary
    // failure is membership; the wrong display is informational on top of
    // that. validation[0] of the same fixture (code IS in VS, display wrong)
    // continues to emit its display-mismatch as severity=error via the
    // membership-hit branch above — that path is untouched.
    let mut issues = vec![issue];
    let mut canonical_display: Option<String> = None;
    if let (Some(sys), Some(disp)) = (resolved_system.as_deref(), in_display.as_deref()) {
        let cs_req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(sys.to_string()),
            code: in_code.clone(),
            version: None,
            display: Some(disp.to_string()),
            date: None,
            include_abstract: None,
            input_form: Some(match req_path {
                RequestPath::BareCode => "code".into(),
                RequestPath::Coding => "coding".into(),
                RequestPath::CodeableConcept => "codeableConcept".into(),
            }),
            lenient_display_validation: params
                .iter()
                .find(|p| {
                    p.get("name").and_then(|v| v.as_str()) == Some("lenient-display-validation")
                })
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            default_value_set_versions: std::collections::HashMap::new(),
        };
        if let Ok(cs_resp) =
            CodeSystemOperations::validate_code(state.backend(), &ctx, cs_req).await
        {
            // Pick up the canonical display so the response's `display` echo
            // is the CS-known value rather than the user's wrong input. The
            // IG fixture marks `display` as `$optional$:true` with the
            // canonical value — the validator tolerates either presence
            // (with that value) or absence, but rejects a divergent value.
            canonical_display = cs_resp.display.clone();
            // Take any display-mismatch issues (tx_code = "invalid-display")
            // and append a warning-severity copy. The membership-miss
            // demotes severity from error to warning.
            for iss in cs_resp.issues {
                if iss.tx_code == "invalid-display" && iss.severity == "error" {
                    issues.push(ValidationIssue {
                        severity: "warning".into(),
                        ..iss
                    });
                } else if iss.tx_code == "invalid-display" {
                    // Already warning/info (e.g. lenient-display-validation
                    // already downgraded it). Pass through as-is.
                    issues.push(iss);
                }
            }
        }
    }

    let resp = ValidateCodeResponse {
        result: false,
        message: None,
        display: canonical_display.or_else(|| in_display.clone()),
        system: resolved_system.clone(),
        cs_version: cs_version_lookup,
        inactive: None,
        issues,
        caused_by_unknown_system: None,
        concept_status: None,
        normalized_code: None,
    };
    let value = build_validate_response_async(
        state.backend(),
        &ctx,
        resp,
        Some(&in_code),
        resolved_system.as_deref(),
        None,
        req_path,
        None,
        find_str_param(&params, "displayLanguage").as_deref(),
        in_display.as_deref(),
        &[],
    )
    .await;
    Ok(value)
}

/// Core validate-code logic for `ValueSet/$validate-code`.
///
/// Always requires the `url` parameter (ValueSet canonical URL).  The optional
/// `system` parameter can further scope the check to a specific code system
/// within the expanded value set.
///
/// Supports the same three input forms as [`process_validate_code`] (bare
/// `code`, `coding`, and `codeableConcept`), with the same priority order.
///
/// Unlike `CodeSystem/$validate-code`, a missing or unknown ValueSet URL
/// returns `result = false` (not an error), consistent with the FHIR spec's
/// intent to treat absence of a value set as a negative match.
pub(crate) async fn process_vs_validate_code<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // ── Handler-level response cache (VS path) ───────────────────────────────
    // The VS validate-code handler is dominated by pre-call helpers that hit
    // the DB even when no work is needed:
    //   * `enforce_vs_supplement_extensions` (always runs ValueSetOperations::search)
    //   * `detect_bad_vs_import` (additional DB calls)
    //   * `resolve_supplements` / `supplement_url_in_coding_error`
    // A warm hit here returns the previously-built JSON response directly,
    // skipping all of those.  Cleared on every bundle import / CRUD write.
    let cache_key = build_validate_code_cache_key(&params);
    if let Some(ref key) = cache_key {
        if let Some(cached) = validate_code_cache_get(&state.vs_validate_code_handler_cache, key) {
            return Ok((*cached).clone());
        }
    }
    let result = process_vs_validate_code_inner(state, params).await;
    if let (Ok(value), Some(key)) = (&result, cache_key) {
        validate_code_cache_put(
            &state.vs_validate_code_handler_cache,
            key,
            Arc::new(value.clone()),
        );
    }
    result
}

async fn process_vs_validate_code_inner<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // ValueSet/$validate-code accepts either `url` (canonical URL of a stored
    // ValueSet) or `valueSet` (an inline ValueSet resource). The IG
    // `validation/validate-contained-{good,bad}` fixtures exercise the inline
    // form where the supplied ValueSet's `compose.include[].valueSet[]` chain
    // names a `#contained` fragment alongside an external canonical reference.
    //
    // When only `valueSet` is supplied, hand off to a dedicated inline-VS
    // validator that resolves contained refs from the inline body before
    // falling back to the local store / tx-resources.
    if find_str_param(&params, "url").is_none() {
        if let Some(vs_resource) = find_resource_param(&params, "valueSet") {
            return process_inline_vs_validate_code(state, params, vs_resource).await;
        }
    }
    // ValueSet/$validate-code requires `url` (the ValueSet canonical URL) when
    // no inline `valueSet` body was supplied.
    let url = find_str_param(&params, "url").ok_or_else(|| {
        HtsError::InvalidRequest("Missing required parameter: url (ValueSet canonical URL)".into())
    })?;

    let ctx = TenantContext::system();
    // The IG `display/`, `language2/`, and parts of `validation/` test groups
    // pin the response display + invalid-display issue text against the
    // requested `displayLanguage` parameter. Pulled here so all three input
    // forms (code / coding / codeableConcept) can pass it to the post-build
    // language-aware display validator.
    let mut display_language: Option<String> = find_str_param(&params, "displayLanguage");
    // Reject malformed BCP-47 displayLanguage early — IG
    // `display/validation-wrong-de-en-bad` expects 4xx + INVALID_DISPLAY_NAME.
    if let Some(ref lang) = display_language {
        if !is_well_formed_display_language(lang) {
            return Err(HtsError::VsInvalid(format!(
                "{}{lang}",
                INVALID_DISPLAY_LANGUAGE_PREFIX
            )));
        }
    }
    // VS-implied displayLanguage: the IG `validation/simple-coding-bad-language-vs`
    // and `-vslang` fixtures pin the language on the ValueSet itself rather
    // than supplying `displayLanguage` in the request. Look the VS up once and,
    // if the caller didn't supply `displayLanguage`, adopt the VS-pinned one
    // so the language-aware display validator (`apply_language_display_validation`)
    // rejects displays that don't match the VS's pinned language.
    if display_language.is_none() {
        let vs_for_lang = ValueSetOperations::search(
            state.backend(),
            &ctx,
            crate::types::ResourceSearchQuery {
                url: Some(url.clone()),
                version: find_str_param(&params, "valueSetVersion"),
                count: Some(1),
                ..Default::default()
            },
        )
        .await
        .ok()
        .and_then(|mut hits| hits.pop());
        if let Some(vs) = vs_for_lang.as_ref() {
            if let Some(lang) = vs_implied_display_language(vs) {
                display_language = Some(lang);
            }
        }
    }
    // ValueSet validate-code can carry useSupplement that targets ANY
    // CodeSystem in the VS expansion. We can't (yet) verify the target
    // matches a system in the VS without expanding, so pass `None` for
    // expected_target here — bad-supplement-not-found is still rejected.
    let mut supplements = resolve_supplements(state.backend(), &ctx, &params, None).await?;
    // Used to rewrite "...'url'..." → "...'url|version'..." in NotFound
    // messages so the IG-expected text format is met.
    let vs_version = find_str_param(&params, "valueSetVersion");

    // `default-valueset-version` pins (canonical URL → version pin).  Used
    // here so `detect_bad_vs_import` can apply the pin to versionless
    // imports — and reused below to build the ValidateCodeRequest.
    let default_vs_pin_pairs_early: Vec<(String, String)> =
        collect_canonical_params(&params, "default-valueset-version");
    let default_value_set_versions_early: std::collections::HashMap<String, String> =
        default_vs_pin_pairs_early.iter().cloned().collect();

    // The VS may pin one or more CS supplements via the `valueset-supplement`
    // extension. Reject the request with 4xx if any of those supplements is
    // not loaded — matches the IG `extensions/validate-*-bad-supplement`
    // fixtures. The returned list of resolved SupplementInfo gets merged into
    // the active `supplements` list so VS-pinned supplements auto-rescue
    // displays just like an explicit `useSupplement` would (per IG
    // `extensions/validate-coding-good-supplement`).
    let vs_ext_supplements =
        enforce_vs_supplement_extensions(state.backend(), &ctx, &url, vs_version.as_deref())
            .await?;
    for s in vs_ext_supplements {
        if !supplements
            .iter()
            .any(|existing| existing.supplement_canonical == s.supplement_canonical)
        {
            supplements.push(s);
        }
    }

    // Detect a ValueSet whose compose.include[*].valueSet imports an
    // unresolvable ValueSet up-front. The IG `validation/simple-*-bad-import`
    // fixtures expect a single `not-found / Unable_to_resolve_value_Set_`
    // issue with text "A definition for the value Set 'X' could not be
    // found" — not the cascade of TX_GENERAL_CC_ERROR_MESSAGE/this-code-not-in-vs
    // that the regular CC fallback emits.
    if let Some(unresolved_vs_url) = detect_bad_vs_import(
        state.backend(),
        &ctx,
        &url,
        vs_version.as_deref(),
        &default_value_set_versions_early,
    )
    .await
    {
        let cc_value = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("codeableConcept"))
            .and_then(|p| p.get("valueCodeableConcept"))
            .cloned();
        let req_path = if extract_codeable_concept(&params, "codeableConcept").is_some() {
            RequestPath::CodeableConcept
        } else if extract_coding_full(&params, "coding").is_some() {
            RequestPath::Coding
        } else {
            RequestPath::BareCode
        };
        let text =
            format!("A definition for the value Set '{unresolved_vs_url}' could not be found");
        let issue = ValidationIssue {
            severity: "error".into(),
            fhir_code: "not-found".into(),
            tx_code: "not-found".into(),
            text,
            expression: None,
            location: None,
            message_id: Some("Unable_to_resolve_value_Set_".into()),
        };
        let mut value = build_validate_response(
            ValidateCodeResponse {
                result: false,
                message: None,
                display: None,
                system: None,
                cs_version: None,
                inactive: None,
                issues: vec![issue],
                caused_by_unknown_system: None,
                concept_status: None,
                normalized_code: None,
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            None,
            req_path,
        );
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }
    // systemVersion pins the CS version to use for this validation call.
    // Falls back when the explicit `version` param is absent.
    let system_version = find_str_param(&params, "systemVersion");
    let lenient_display = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("lenient-display-validation"))
        .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()));

    // ── IG-style version pin parameters ─────────────────────────────────────
    // The FHIR R5 IG `version/parameters-*-version.json` profiles inject these
    // into the request body to steer CodeSystem version selection:
    //   - `force-system-version` (FORCE): override Coding.version / version /
    //     systemVersion / VS-pinned version.
    //   - `system-version`        (DEFAULT): apply only when neither the
    //     request nor the VS pins a version for the matching system.
    //   - `check-system-version`  (CHECK): same DEFAULT semantics as
    //     system-version PLUS a post-check that emits VALUESET_VERSION_CHECK
    //     when the resolved CS version doesn't satisfy the pattern.
    let force_pins: Vec<(String, String)> =
        collect_canonical_params(&params, "force-system-version");
    let default_pins: Vec<(String, String)> = collect_canonical_params(&params, "system-version");
    let check_pins: Vec<(String, String)> =
        collect_canonical_params(&params, "check-system-version");
    // `check` also acts as a DEFAULT — merge for the default lookup.
    let mut effective_defaults: Vec<(String, String)> = default_pins.clone();
    effective_defaults.extend(check_pins.iter().cloned());
    // `default-valueset-version` pins were already parsed earlier (so the
    // bad-import detector can honour them).
    let default_value_set_versions = default_value_set_versions_early.clone();
    // Promote a default-valueset-version pin matching the request's `url` to
    // an effective `valueSetVersion` so the backend resolves the correct
    // (url, version) row up front.
    let effective_vs_version: Option<String> = vs_version.clone().or_else(|| {
        default_value_set_versions
            .get(url.as_str())
            .map(|s| s.to_owned())
    });

    // Look up the source ValueSet once so we can ask whether a given system
    // is pinned in any include (drives the "default applies only if VS
    // doesn't pin" rule). Only worth doing when there are version-pin
    // parameters to apply.
    let source_vs: Option<Value> = if !force_pins.is_empty() || !effective_defaults.is_empty() {
        ValueSetOperations::search(
            state.backend(),
            &ctx,
            crate::types::ResourceSearchQuery {
                url: Some(url.clone()),
                version: vs_version.clone(),
                count: Some(20),
                ..Default::default()
            },
        )
        .await
        .ok()
        .and_then(|mut hits| {
            // Pick the same VS row the backend will use:
            //   - if vs_version was supplied, take the unique match
            //   - otherwise, pick the highest version (matches
            //     `resolve_value_set_versioned` ordering).
            if vs_version.is_some() {
                hits.into_iter().next()
            } else {
                hits.sort_by(|a, b| {
                    let av = a.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    let bv = b.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    av.cmp(bv)
                });
                hits.pop()
            }
        })
    } else {
        None
    };

    // Helper: resolve the effective `version` for a given system based on the
    // priority order:  force > explicit (Coding.version / version /
    // systemVersion) > VS-pin > default (system-version / check-system-version)
    // > (None, backend will fall back to latest).
    //
    // Wildcards are resolved to a concrete stored version where possible to
    // avoid the backend's mismatch detector flagging the wildcard against a
    // VS pin.  Inlined per call site (cannot use a closure here because the
    // body needs `.await` and would require `futures::BoxFuture`).
    async fn resolve_version_for_system<B: TerminologyBackend>(
        backend: &B,
        ctx: &TenantContext,
        system: &str,
        original: Option<String>,
        force_pins: &[(String, String)],
        effective_defaults: &[(String, String)],
        source_vs: Option<&Value>,
    ) -> Option<String> {
        // 1. Force always wins.
        if let Some(pat) = find_pin_for_system(force_pins, system) {
            return Some(
                resolve_cs_version_pattern(backend, ctx, system, pat)
                    .await
                    .unwrap_or_else(|| pat.to_string()),
            );
        }
        // 2. Explicit caller-supplied version.
        if original.is_some() {
            return original;
        }
        // 3. VS-pinned include version (handled by backend).
        // Skip applying a default ONLY when the VS include for this system has
        // an explicit version pin. A versionless include (`Some(None)`) does
        // not pin the version — the system-version / check-system-version
        // default should still apply (per IG `coding-vnn-vsnn-default` etc.,
        // which expect the default to drive the effective CS version when the
        // VS include has no explicit version).
        let vs_has_explicit_pin = source_vs
            .and_then(|vs| vs_include_pin_for_system(vs, system))
            .map(|opt_v| opt_v.is_some())
            .unwrap_or(false);
        if vs_has_explicit_pin {
            return None;
        }
        // 4. Default from system-version / check-system-version.
        if let Some(pat) = find_pin_for_system(effective_defaults, system) {
            return Some(
                resolve_cs_version_pattern(backend, ctx, system, pat)
                    .await
                    .unwrap_or_else(|| pat.to_string()),
            );
        }
        None
    }
    let rewrite = |e: HtsError| -> HtsError {
        match (e, vs_version.as_deref()) {
            (HtsError::NotFound(msg), Some(v)) => {
                let needle = format!("'{url}'");
                let replacement = format!("'{url}|{v}'");
                HtsError::NotFound(msg.replace(&needle, &replacement))
            }
            (e, _) => e,
        }
    };

    // ── Path 1: bare `code` parameter ────────────────────────────────────────────
    if let Some(code) = find_str_param(&params, "code") {
        let system = find_str_param(&params, "system");
        let display = find_str_param(&params, "display");
        let original_version = find_str_param(&params, "version").or(system_version.clone());
        let req_version = if let Some(sys) = system.as_deref() {
            resolve_version_for_system(
                state.backend(),
                &ctx,
                sys,
                original_version.clone(),
                &force_pins,
                &effective_defaults,
                source_vs.as_ref(),
            )
            .await
        } else {
            original_version.clone()
        };
        let req = ValidateCodeRequest {
            url: Some(url.clone()),
            value_set_version: effective_vs_version.clone(),
            system: system.clone(),
            code: code.clone(),
            version: req_version.clone(),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            input_form: Some("code".into()),
            lenient_display_validation: lenient_display,
            default_value_set_versions: default_value_set_versions.clone(),
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
        // When force-system-version was active for this system, suppress the
        // backend's VS-pin mismatch issues — the forced version overrides the
        // VS pin entirely.
        if let (Some(sys), Some(forced)) = (system.as_deref(), req_version.as_deref()) {
            if let Some(force_pat) = find_pin_for_system(&force_pins, sys) {
                let force_pat = force_pat.to_string();
                suppress_forced_version_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    sys,
                    &code,
                    forced,
                )
                .await;
                // When the caller's original version is incompatible with the
                // force pattern AND not a known stored CS version, the IG
                // expects a CHANGED + UNKNOWN failure pair (the suppress above
                // turned a passing validation into a success — re-fail it).
                // Drives `code-vbb-vs10-force` and `code-vbb-vsnn-force`.
                if let Some(orig) = original_version.as_deref() {
                    let satisfies = if force_pat.contains(".x") || force_pat == "x" {
                        version_satisfies_wildcard(orig, &force_pat)
                    } else {
                        orig == force_pat.as_str()
                    };
                    if !satisfies {
                        let inc_ver = source_vs
                            .as_ref()
                            .and_then(|vs| vs_include_pin_for_system(vs, sys))
                            .unwrap_or(None);
                        apply_force_caller_version_unknown_failure(
                            state.backend(),
                            &ctx,
                            &mut resp,
                            sys,
                            orig,
                            &force_pat,
                            inc_ver.as_deref(),
                            forced,
                            RequestPath::BareCode,
                        )
                        .await;
                    }
                }
            }
        }
        // When system-version (DEFAULT) applied — i.e. caller had no version,
        // VS include is versionless, and a default pin matches this system —
        // the default IS the effective VS version, so any
        // `VALUESET_VALUE_MISMATCH` from the backend's "versionless include"
        // branch is spurious. Drop it.
        if let (Some(sys), Some(default_v)) = (system.as_deref(), req_version.as_deref()) {
            let default_applied = original_version.is_none()
                && find_pin_for_system(&force_pins, sys).is_none()
                && find_pin_for_system(&effective_defaults, sys).is_some()
                && source_vs
                    .as_ref()
                    .and_then(|vs| vs_include_pin_for_system(vs, sys))
                    .map(|opt| opt.is_none())
                    .unwrap_or(false);
            if default_applied {
                suppress_default_versionless_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    sys,
                    &code,
                    default_v,
                )
                .await;
            }
        }
        // When the caller DID supply a version AND a system-version default
        // is in effect AND the VS include for this system is versionless,
        // the backend's MISMATCH_DEFAULT (warning) needs to be transformed
        // into VALUESET_VALUE_MISMATCH_CHANGED (error) — see
        // `transform_default_versionless_mismatch_to_changed` for rationale.
        // Skip when the caller's version satisfies the default pattern (e.g.
        // check-system-version=1.0.x and the caller passed 1.0.0): there's
        // no actual mismatch to report.
        if let Some(sys) = system.as_deref() {
            if let (Some(orig), Some(default_pat)) = (
                original_version.as_deref(),
                find_pin_for_system(&effective_defaults, sys),
            ) {
                let vs_versionless = source_vs
                    .as_ref()
                    .and_then(|vs| vs_include_pin_for_system(vs, sys))
                    .map(|opt| opt.is_none())
                    .unwrap_or(false);
                let no_force = find_pin_for_system(&force_pins, sys).is_none();
                let satisfies = if default_pat.contains(".x") || default_pat == "x" {
                    version_satisfies_wildcard(orig, default_pat)
                } else {
                    orig == default_pat
                };
                if vs_versionless && no_force && !satisfies {
                    let resolved_default =
                        resolve_cs_version_pattern(state.backend(), &ctx, sys, default_pat)
                            .await
                            .unwrap_or_else(|| default_pat.to_string());
                    transform_default_versionless_mismatch_to_changed(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        sys,
                        &code,
                        &resolved_default,
                        orig,
                        default_pat,
                    )
                    .await;
                }
            }
        }
        // When the caller supplied no version AND a system-version default is
        // in effect AND the backend emitted UNKNOWN_CODESYSTEM_VERSION (because
        // the VS-include pinned a non-existent version), override the echoed
        // CS version with the default so the response reflects the
        // default-applied semantics rather than the latest stored CS version.
        if let Some(sys) = system.as_deref() {
            if original_version.is_none()
                && find_pin_for_system(&force_pins, sys).is_none()
            {
                if let Some(default_pat) = find_pin_for_system(&effective_defaults, sys) {
                    let resolved_default =
                        resolve_cs_version_pattern(state.backend(), &ctx, sys, default_pat)
                            .await
                            .unwrap_or_else(|| default_pat.to_string());
                    apply_default_to_unknown_version_echo(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        sys,
                        &code,
                        &resolved_default,
                    )
                    .await;
                }
            }
        }
        if let Some(sys) = system.as_deref() {
            rescue_via_supplements(
                state.backend(),
                &ctx,
                &supplements,
                sys,
                &code,
                display.as_deref(),
                &mut resp,
            )
            .await;
        }
        // `activeOnly=true` semantics — see Path 2 handling for rationale.
        let active_only = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("activeOnly"))
            .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        // Fall back to the stored VS version when the caller didn't supply
        // a `valueSetVersion` — keeps the not-in-vs message consistent with
        // the IG fixture's `<url>|<version>` form (mirrors the Path 2 fix).
        let effective_vs_version_for_msg: Option<String> =
            if effective_vs_version.is_some() {
                effective_vs_version.clone()
            } else {
                ValueSetOperations::search(
                    state.backend(),
                    &ctx,
                    crate::types::ResourceSearchQuery {
                        url: Some(url.clone()),
                        count: Some(1),
                        ..Default::default()
                    },
                )
                .await
                .ok()
                .and_then(|mut hits| hits.pop())
                .and_then(|vs| {
                    vs.get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
            };
        if let Some(sys) = system.as_deref() {
            apply_active_only_inactive(
                active_only,
                &mut resp,
                &code,
                sys,
                &url,
                effective_vs_version_for_msg.as_deref(),
            );
        }
        // Capture cs_version BEFORE moving resp into build_validate_response_async,
        // so we can post-validate against the check pattern.
        let resolved_version = resp.cs_version.clone();
        let mut value = build_validate_response_async(
            state.backend(),
            &ctx,
            resp,
            Some(&code),
            system.as_deref(),
            None,
            RequestPath::BareCode,
            Some(&url),
            display_language.as_deref(),
            display.as_deref(),
            &supplements,
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        // Apply check-system-version post-check (only when no other error
        // already invalidated the result; the IG fixtures show that the
        // version-check error is the dominant issue when present).
        if let Some(sys) = system.as_deref() {
            if let Some(pat) = find_pin_for_system(&check_pins, sys) {
                let actual = resolved_version
                    .clone()
                    .or_else(|| extract_response_version(&value));
                if let Some(v) = actual.as_deref() {
                    if !version_satisfies_wildcard(v, pat) {
                        apply_check_version_failure(&mut value, sys, v, pat, RequestPath::BareCode);
                    }
                }
            }
        }
        return Ok(value);
    }

    // ── Path 2: `coding` parameter (valueCoding) ──────────────────────────────
    if let Some((system, code, coding_display, coding_version)) =
        extract_coding_full(&params, "coding")
    {
        // Empty system from extract_coding means the Coding had no system
        // field. Per the IG fixtures, that should produce result=false with
        // a "Coding has no system" message rather than matching by code
        // alone.
        if system.is_empty() {
            // The IG `validation/simple-coding-no-system` fixture expects two
            // issues: an error-level not-in-vs (the code clearly isn't in the
            // VS expansion since we have no system to anchor it) plus a
            // warning-level invalid-data with the canonical
            // "Coding has no system. A code with no system has no defined
            // meaning..." text. Result is false because of the error issue.
            // Need vs_version to format the not-in-vs URL with `|version`.
            let vs_version_owned = crate::traits::ValueSetOperations::search(
                state.backend(),
                &ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(url.clone()),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|mut hits| {
                hits.pop().and_then(|vs| {
                    vs.get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
            });
            let vs_qualified = match vs_version_owned.as_deref() {
                Some(v) => format!("{url}|{v}"),
                None => url.clone(),
            };
            let not_in_vs_text = format!(
                "The provided code '#{code}' was not found in the value set '{vs_qualified}'"
            );
            let no_system_text =
                "Coding has no system. A code with no system has no defined meaning, \
                 and it cannot be validated. A system should be provided"
                    .to_string();
            return Ok(build_validate_response(
                ValidateCodeResponse {
                    result: false,
                    message: Some(no_system_text.clone()),
                    display: None,
                    system: None,
                    cs_version: None,
                    inactive: None,
                    issues: vec![
                        ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "code-invalid".into(),
                            tx_code: "not-in-vs".into(),
                            text: not_in_vs_text,
                            expression: Some("Coding.code".into()),
                            location: Some("Coding.code".into()),
                            message_id: Some(
                                "None_of_the_provided_codes_are_in_the_value_set_one".into(),
                            ),
                        },
                        ValidationIssue {
                            severity: "warning".into(),
                            fhir_code: "invalid".into(),
                            tx_code: "invalid-data".into(),
                            text: no_system_text,
                            expression: Some("Coding".into()),
                            location: Some("Coding".into()),
                            message_id: Some("Coding_has_no_system__cannot_validate".into()),
                        },
                    ],
                    caused_by_unknown_system: None,
                    concept_status: None,
                    normalized_code: None,
                },
                Some(&code),
                None,
                None,
                None,
                None,
                RequestPath::Coding,
            ));
        }
        // Coding.display takes precedence over a top-level `display` param —
        // the IG fixtures pin display via the Coding so the server can
        // report a mismatch.
        let display = coding_display.or_else(|| find_str_param(&params, "display"));
        // Coding.version > explicit `version` param > systemVersion pin.
        let original_version = coding_version
            .or_else(|| find_str_param(&params, "version"))
            .or(system_version.clone());
        let req_version = resolve_version_for_system(
            state.backend(),
            &ctx,
            &system,
            original_version.clone(),
            &force_pins,
            &effective_defaults,
            source_vs.as_ref(),
        )
        .await;
        let req = ValidateCodeRequest {
            url: Some(url.clone()),
            value_set_version: effective_vs_version.clone(),
            system: Some(system.clone()),
            code: code.clone(),
            version: req_version.clone(),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            input_form: Some("coding".into()),
            lenient_display_validation: lenient_display,
            default_value_set_versions: default_value_set_versions.clone(),
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
        // When force-system-version was active for this system, suppress the
        // backend's VS-pin mismatch issues — the forced version overrides the
        // VS pin entirely.
        if let Some(forced) = req_version.as_deref() {
            if let Some(force_pat) = find_pin_for_system(&force_pins, &system) {
                let force_pat = force_pat.to_string();
                suppress_forced_version_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    &system,
                    &code,
                    forced,
                )
                .await;
                // When the caller's original version is incompatible with the
                // force pattern AND not a known stored CS version, the IG
                // expects a CHANGED + UNKNOWN failure pair. Drives
                // `coding-vbb-vs10-force` and `coding-vbb-vsnn-force`.
                if let Some(orig) = original_version.as_deref() {
                    let satisfies = if force_pat.contains(".x") || force_pat == "x" {
                        version_satisfies_wildcard(orig, &force_pat)
                    } else {
                        orig == force_pat.as_str()
                    };
                    if !satisfies {
                        let inc_ver = source_vs
                            .as_ref()
                            .and_then(|vs| vs_include_pin_for_system(vs, &system))
                            .unwrap_or(None);
                        apply_force_caller_version_unknown_failure(
                            state.backend(),
                            &ctx,
                            &mut resp,
                            &system,
                            orig,
                            &force_pat,
                            inc_ver.as_deref(),
                            forced,
                            RequestPath::Coding,
                        )
                        .await;
                    }
                }
            }
        }
        // When system-version (DEFAULT) applied for this system + the VS
        // include is versionless + caller had no version, the default IS the
        // effective VS version. Drop the spurious VALUESET_VALUE_MISMATCH the
        // backend emits from comparing latest-stored vs the default version.
        if let Some(default_v) = req_version.as_deref() {
            let default_applied = original_version.is_none()
                && find_pin_for_system(&force_pins, &system).is_none()
                && find_pin_for_system(&effective_defaults, &system).is_some()
                && source_vs
                    .as_ref()
                    .and_then(|vs| vs_include_pin_for_system(vs, &system))
                    .map(|opt| opt.is_none())
                    .unwrap_or(false);
            if default_applied {
                suppress_default_versionless_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    &system,
                    &code,
                    default_v,
                )
                .await;
            }
        }
        // Caller-supplied version + default-pin + versionless include →
        // transform backend mismatch into VALUESET_VALUE_MISMATCH_CHANGED.
        if let (Some(orig), Some(default_pat)) = (
            original_version.as_deref(),
            find_pin_for_system(&effective_defaults, &system),
        ) {
            let vs_versionless = source_vs
                .as_ref()
                .and_then(|vs| vs_include_pin_for_system(vs, &system))
                .map(|opt| opt.is_none())
                .unwrap_or(false);
            let no_force = find_pin_for_system(&force_pins, &system).is_none();
            let satisfies = if default_pat.contains(".x") || default_pat == "x" {
                version_satisfies_wildcard(orig, default_pat)
            } else {
                orig == default_pat
            };
            if vs_versionless && no_force && !satisfies {
                let resolved_default =
                    resolve_cs_version_pattern(state.backend(), &ctx, &system, default_pat)
                        .await
                        .unwrap_or_else(|| default_pat.to_string());
                transform_default_versionless_mismatch_to_changed(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    &system,
                    &code,
                    &resolved_default,
                    orig,
                    default_pat,
                )
                .await;
            }
        }
        // No caller version + default-pin + UNKNOWN_CODESYSTEM_VERSION →
        // override echoed version with default.
        if original_version.is_none()
            && find_pin_for_system(&force_pins, &system).is_none()
        {
            if let Some(default_pat) = find_pin_for_system(&effective_defaults, &system) {
                let resolved_default =
                    resolve_cs_version_pattern(state.backend(), &ctx, &system, default_pat)
                        .await
                        .unwrap_or_else(|| default_pat.to_string());
                apply_default_to_unknown_version_echo(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    &system,
                    &code,
                    &resolved_default,
                )
                .await;
            }
        }
        rescue_via_supplements(
            state.backend(),
            &ctx,
            &supplements,
            &system,
            &code,
            display.as_deref(),
            &mut resp,
        )
        .await;
        // `activeOnly=true` semantics — when the validated code is inactive,
        // the activeOnly filter would have excluded it from the expansion.
        // Flip result=false and add the missing not-in-vs / code-rule issues.
        // Drives the IG `validation/simple-coding-bad-code-inactive` fixture.
        let active_only = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("activeOnly"))
            .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        // For the not-in-vs message format, fall back to the stored VS's
        // version when the caller didn't supply one (the IG
        // `validation/simple-coding-bad-code-inactive` fixture expects the
        // message to reference `<url>|<stored-version>`, not the bare URL).
        let effective_vs_version_for_msg: Option<String> =
            if effective_vs_version.is_some() {
                effective_vs_version.clone()
            } else {
                ValueSetOperations::search(
                    state.backend(),
                    &ctx,
                    crate::types::ResourceSearchQuery {
                        url: Some(url.clone()),
                        count: Some(1),
                        ..Default::default()
                    },
                )
                .await
                .ok()
                .and_then(|mut hits| hits.pop())
                .and_then(|vs| {
                    vs.get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
            };
        apply_active_only_inactive(
            active_only,
            &mut resp,
            &code,
            &system,
            &url,
            effective_vs_version_for_msg.as_deref(),
        );
        let resolved_version = resp.cs_version.clone();
        let mut value = build_validate_response_async(
            state.backend(),
            &ctx,
            resp,
            Some(&code),
            Some(&system),
            None,
            RequestPath::Coding,
            Some(&url),
            display_language.as_deref(),
            display.as_deref(),
            &supplements,
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        // Apply check-system-version post-check.
        if let Some(pat) = find_pin_for_system(&check_pins, &system) {
            let actual = resolved_version
                .clone()
                .or_else(|| extract_response_version(&value));
            if let Some(v) = actual.as_deref() {
                if !version_satisfies_wildcard(v, pat) {
                    apply_check_version_failure(&mut value, &system, v, pat, RequestPath::Coding);
                }
            }
        }
        return Ok(value);
    }

    // ── Path 3: `codeableConcept` parameter (true if any coding is in the ValueSet) ──
    if let Some(codings) = extract_codeable_concept(&params, "codeableConcept") {
        if codings.is_empty() {
            return Err(HtsError::InvalidRequest(
                "codeableConcept parameter has no valid coding entries".into(),
            ));
        }
        let cc_value = params
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("codeableConcept"))
            .and_then(|p| p.get("valueCodeableConcept"))
            .cloned();
        // Capture per-coding `display` and `version` from the original
        // CodeableConcept. `display` is used for the IG `permutations/bad-cc*`
        // text format; `version` is needed so the per-coding CS version check
        // fires correctly (the coding's version is NOT a top-level parameter).
        let coding_displays: std::collections::HashMap<(String, String), String> = cc_value
            .as_ref()
            .and_then(|cc| cc.get("coding").and_then(|v| v.as_array()))
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        let s = c.get("system").and_then(|v| v.as_str())?.to_string();
                        let cd = c.get("code").and_then(|v| v.as_str())?.to_string();
                        let d = c.get("display").and_then(|v| v.as_str())?.to_string();
                        Some(((s, cd), d))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let coding_versions: std::collections::HashMap<(String, String), String> = cc_value
            .as_ref()
            .and_then(|cc| cc.get("coding").and_then(|v| v.as_array()))
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        let s = c.get("system").and_then(|v| v.as_str())?.to_string();
                        let cd = c.get("code").and_then(|v| v.as_str())?.to_string();
                        let v = c.get("version").and_then(|v| v.as_str())?.to_string();
                        Some(((s, cd), v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The IG fixtures expect the LAST matching coding to win (when several
        // codings in a CodeableConcept all validate, the response echoes the
        // last one). Iterate in reverse so the earliest "yes" we find is the
        // last entry in the input.
        //
        // Also track per-coding `unknown-code` failures (codes that don't
        // exist in their CS) so we can surface them in the response even when
        // a different coding succeeds. The IG `permutations/simple-bad-cc2-*`
        // fixtures expect: when a CC has BOTH a bad coding (unknown CS code)
        // AND a good coding, the response echoes the good coding's metadata
        // (code/display/system/version) but `result=false` and surfaces the
        // bad coding's `Unknown_Code_in_Version` error +
        // `None_of_the_provided_codes_are_in_the_value_set_one` info.
        let cc_req_version = find_str_param(&params, "version").or(system_version.clone());
        // Map (system, code) → original CC index (preserved through reverse
        // iteration) so per-coding failure issues reference
        // `CodeableConcept.coding[N]` with the input order's N.
        let coding_index: std::collections::HashMap<(String, String), usize> = codings
            .iter()
            .enumerate()
            .map(|(i, (s, c))| ((s.clone(), c.clone()), i))
            .collect();
        for (system, code) in codings.clone().into_iter().rev() {
            // Prefer the per-coding version (embedded in the CC) over the
            // top-level `version` parameter so that version-mismatch detection
            // fires correctly for each coding.
            let original_version = coding_versions
                .get(&(system.clone(), code.clone()))
                .cloned()
                .or(cc_req_version.clone());
            let per_coding_version = resolve_version_for_system(
                state.backend(),
                &ctx,
                &system,
                original_version.clone(),
                &force_pins,
                &effective_defaults,
                source_vs.as_ref(),
            )
            .await;
            // Plumb the per-coding display down to the backend so it can emit
            // an `invalid-display` issue when the supplied display doesn't
            // match any of the concept's display/designation values. The IG
            // `validation/simple-codeableconcept-bad-display(W)` fixtures
            // expect this validation to fire just like the bare-Coding path.
            let coding_display_for_req = coding_displays
                .get(&(system.clone(), code.clone()))
                .cloned();
            let req = ValidateCodeRequest {
                url: Some(url.clone()),
                value_set_version: effective_vs_version.clone(),
                system: Some(system.clone()),
                code: code.clone(),
                version: per_coding_version.clone(),
                display: coding_display_for_req.clone(),
                date: find_str_param(&params, "date"),
                include_abstract: params
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                    .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
                input_form: Some("codeableConcept".into()),
                lenient_display_validation: lenient_display,
                default_value_set_versions: default_value_set_versions.clone(),
            };
            let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
                .await
                .map_err(&rewrite)?;
            // When force-system-version was active for this system, suppress
            // the backend's VS-pin mismatch issues for this coding.
            if let Some(forced) = per_coding_version.as_deref() {
                if let Some(force_pat) = find_pin_for_system(&force_pins, &system) {
                    let force_pat = force_pat.to_string();
                    suppress_forced_version_mismatch(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        &system,
                        &code,
                        forced,
                    )
                    .await;
                    // When the per-coding original version is incompatible with
                    // the force pattern AND not a known stored CS version, the
                    // IG expects a CHANGED + UNKNOWN failure pair. Drives
                    // `codeableconcept-vbb-vs10-force` and
                    // `codeableconcept-vbb-vsnn-force`.
                    if let Some(orig) = original_version.as_deref() {
                        let satisfies = if force_pat.contains(".x") || force_pat == "x" {
                            version_satisfies_wildcard(orig, &force_pat)
                        } else {
                            orig == force_pat.as_str()
                        };
                        if !satisfies {
                            let inc_ver = source_vs
                                .as_ref()
                                .and_then(|vs| vs_include_pin_for_system(vs, &system))
                                .unwrap_or(None);
                            apply_force_caller_version_unknown_failure(
                                state.backend(),
                                &ctx,
                                &mut resp,
                                &system,
                                orig,
                                &force_pat,
                                inc_ver.as_deref(),
                                forced,
                                RequestPath::CodeableConcept,
                            )
                            .await;
                        }
                    }
                }
            }
            // When system-version (DEFAULT) applied for this coding's system +
            // the VS include is versionless + this coding had no version, the
            // default IS the effective VS version. Drop the spurious
            // VALUESET_VALUE_MISMATCH from the backend.
            if let Some(default_v) = per_coding_version.as_deref() {
                let default_applied = original_version.is_none()
                    && find_pin_for_system(&force_pins, &system).is_none()
                    && find_pin_for_system(&effective_defaults, &system).is_some()
                    && source_vs
                        .as_ref()
                        .and_then(|vs| vs_include_pin_for_system(vs, &system))
                        .map(|opt| opt.is_none())
                        .unwrap_or(false);
                if default_applied {
                    suppress_default_versionless_mismatch(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        &system,
                        &code,
                        default_v,
                    )
                    .await;
                }
            }
            // Per-coding caller-supplied version + default-pin + versionless
            // include → transform mismatch into VALUESET_VALUE_MISMATCH_CHANGED.
            if let (Some(orig), Some(default_pat)) = (
                original_version.as_deref(),
                find_pin_for_system(&effective_defaults, &system),
            ) {
                let vs_versionless = source_vs
                    .as_ref()
                    .and_then(|vs| vs_include_pin_for_system(vs, &system))
                    .map(|opt| opt.is_none())
                    .unwrap_or(false);
                let no_force = find_pin_for_system(&force_pins, &system).is_none();
                let satisfies = if default_pat.contains(".x") || default_pat == "x" {
                    version_satisfies_wildcard(orig, default_pat)
                } else {
                    orig == default_pat
                };
                if vs_versionless && no_force && !satisfies {
                    let resolved_default =
                        resolve_cs_version_pattern(state.backend(), &ctx, &system, default_pat)
                            .await
                            .unwrap_or_else(|| default_pat.to_string());
                    transform_default_versionless_mismatch_to_changed(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        &system,
                        &code,
                        &resolved_default,
                        orig,
                        default_pat,
                    )
                    .await;
                }
            }
            // No caller version + default-pin + UNKNOWN_CODESYSTEM_VERSION →
            // override echoed version with default.
            if original_version.is_none()
                && find_pin_for_system(&force_pins, &system).is_none()
            {
                if let Some(default_pat) =
                    find_pin_for_system(&effective_defaults, &system)
                {
                    let resolved_default = resolve_cs_version_pattern(
                        state.backend(),
                        &ctx,
                        &system,
                        default_pat,
                    )
                    .await
                    .unwrap_or_else(|| default_pat.to_string());
                    apply_default_to_unknown_version_echo(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        &system,
                        &code,
                        &resolved_default,
                    )
                    .await;
                }
            }
            // Treat a coding as "in VS" when either:
            //   - the backend confirmed it (`resp.result == true`), OR
            //   - the only error is `invalid-display` (i.e. the code+system
            //     was found in the VS expansion, but the supplied display
            //     didn't match any of the concept's display/designation
            //     values). The IG `validation/complex-codeableconcept-full`
            //     fixture expects this case to suppress the generic
            //     `TX_GENERAL_CC_ERROR_MESSAGE` + per-coding `this-code-not-in-vs`
            //     for the in-VS coding, and emit the `Display_Name_for_*`
            //     `invalid-display` issue instead.
            let in_vs_bad_display_only = !resp.result
                && resp.issues.iter().any(|i| i.tx_code == "invalid-display")
                && !resp.issues.iter().any(|i| {
                    matches!(
                        i.tx_code.as_str(),
                        "not-in-vs" | "this-code-not-in-vs" | "invalid-code" | "not-found"
                            | "vs-invalid"
                    )
                });
            if resp.result || in_vs_bad_display_only {
                let resolved_version = resp.cs_version.clone();
                let coding_display = coding_displays
                    .get(&(system.clone(), code.clone()))
                    .cloned();
                // ── Walk remaining codings (those we haven't reached yet in
                // reverse iteration, i.e. earlier in input order) and check
                // for hard `unknown-code` failures. If any exist, the IG
                // `permutations/simple-bad-cc2-*` fixtures expect us to echo
                // THIS coding's metadata but mark result=false and surface
                // the bad coding's issues.
                let success_idx = coding_index
                    .get(&(system.clone(), code.clone()))
                    .copied()
                    .unwrap_or(0);
                let mut accumulated_issues: Vec<ValidationIssue> = Vec::new();
                let vs_version_owned = crate::traits::ValueSetOperations::search(
                    state.backend(),
                    &ctx,
                    crate::types::ResourceSearchQuery {
                        url: Some(url.clone()),
                        version: vs_version.clone(),
                        count: Some(1),
                        ..Default::default()
                    },
                )
                .await
                .ok()
                .and_then(|mut hits| {
                    hits.pop().and_then(|vs| {
                        vs.get("version")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                });
                let url_with_version = match vs_version_owned.as_deref() {
                    Some(v) => format!("{url}|{v}"),
                    None => url.clone(),
                };
                for (other_idx, (other_system, other_code)) in codings.iter().enumerate() {
                    if other_idx == success_idx {
                        continue;
                    }
                    let cs_exists = crate::traits::CodeSystemOperations::search(
                        state.backend(),
                        &ctx,
                        crate::types::ResourceSearchQuery {
                            url: Some(other_system.clone()),
                            count: Some(1),
                            ..Default::default()
                        },
                    )
                    .await
                    .ok()
                    .map(|hits| !hits.is_empty())
                    .unwrap_or(false);
                    if !cs_exists {
                        continue;
                    }
                    let cs_version = state
                        .backend()
                        .code_system_version_for_url(&ctx, other_system)
                        .await
                        .ok()
                        .flatten();
                    // Per-coding lookup: does the code exist in the CS at all?
                    let req = ValidateCodeRequest {
                        url: None,
                        value_set_version: None,
                        system: Some(other_system.clone()),
                        code: other_code.clone(),
                        version: None,
                        display: None,
                        date: None,
                        include_abstract: None,
                        input_form: None,
                        lenient_display_validation: None,
                        default_value_set_versions: std::collections::HashMap::new(),
                    };
                    let code_in_cs =
                        CodeSystemOperations::validate_code(state.backend(), &ctx, req)
                            .await
                            .map(|r| r.result)
                            .unwrap_or(false);
                    if code_in_cs {
                        continue;
                    }
                    // Hard failure: emit `Unknown_Code_in_Version` error +
                    // `None_of_the_provided_codes_are_in_the_value_set_one` info.
                    let cs_text = match cs_version.as_deref() {
                        Some(v) => format!(
                            "Unknown code '{other_code}' in the CodeSystem \
                             '{other_system}' version '{v}'"
                        ),
                        None => format!(
                            "Unknown code '{other_code}' in the CodeSystem '{other_system}'"
                        ),
                    };
                    accumulated_issues.push(ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "invalid-code".into(),
                        text: cs_text,
                        expression: Some(format!("CodeableConcept.coding[{other_idx}].code")),
                        location: None,
                        message_id: Some("Unknown_Code_in_Version".into()),
                    });
                    let other_disp =
                        coding_displays.get(&(other_system.clone(), other_code.clone()));
                    let other_ver =
                        coding_versions.get(&(other_system.clone(), other_code.clone()));
                    let qualified = match (other_ver, other_disp) {
                        (Some(v), Some(d)) => {
                            format!("{other_system}|{v}#{other_code} ('{d}')")
                        }
                        (Some(v), None) => format!("{other_system}|{v}#{other_code}"),
                        (None, Some(d)) => format!("{other_system}#{other_code} ('{d}')"),
                        (None, None) => format!("{other_system}#{other_code}"),
                    };
                    accumulated_issues.push(ValidationIssue {
                        severity: "information".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "this-code-not-in-vs".into(),
                        text: format!(
                            "The provided code '{qualified}' was not found in the \
                             value set '{url_with_version}'"
                        ),
                        expression: Some(format!(
                            "CodeableConcept.coding[{other_idx}].code"
                        )),
                        location: None,
                        message_id: Some(
                            "None_of_the_provided_codes_are_in_the_value_set_one".into(),
                        ),
                    });
                }
                let has_bad_codings = !accumulated_issues.is_empty();
                let mut hybrid_resp = resp.clone();
                if has_bad_codings {
                    hybrid_resp.result = false;
                    hybrid_resp.issues.extend(accumulated_issues);
                    // Promote the first error issue's text to `message`
                    // (matches IG fixture: top-level `message` echoes the
                    // unknown-code error text).
                    if let Some(first_err) =
                        hybrid_resp.issues.iter().find(|i| i.severity == "error")
                    {
                        hybrid_resp.message = Some(first_err.text.clone());
                    }
                }
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    hybrid_resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    Some(&url),
                    display_language.as_deref(),
                    coding_display.as_deref(),
                    &supplements,
                )
                .await;
                append_used_supplements(&mut value, &supplements);
                // Apply check-system-version post-check.
                if let Some(pat) = find_pin_for_system(&check_pins, &system) {
                    let actual = resolved_version
                        .clone()
                        .or_else(|| extract_response_version(&value));
                    if let Some(v) = actual.as_deref() {
                        if !version_satisfies_wildcard(v, pat) {
                            apply_check_version_failure(
                                &mut value,
                                &system,
                                v,
                                pat,
                                RequestPath::CodeableConcept,
                            );
                        }
                    }
                }
                return Ok(value);
            }
            // Propagate version-mismatch failures — they carry the correct
            // VALUESET_VALUE_MISMATCH / UNKNOWN_CODESYSTEM_VERSION issues and
            // must not be replaced by the generic "no valid coding" fallback.
            //
            // Two trigger conditions:
            //   1. tx_code == "vs-invalid"  → the original mismatch path
            //      (multi-version overload, regex-bad VS pin, …).
            //   2. message_id == "UNKNOWN_CODESYSTEM_VERSION" AND this is a
            //      single-coding CC  → fired by `detect_vs_pin_unknown` in the
            //      SQLite backend when the VS include pins a CS version that
            //      doesn't exist (e.g. the `version-w-bad` fixture pins
            //      `version="1"` against a CS that only has `1.0.0` / `1.2.0`).
            //      Without this, the CC path drops the diagnostic and emits
            //      the generic `TX_GENERAL_CC_ERROR_MESSAGE` instead — the IG
            //      `codeableconcept-vnn-vs1wb` family expects the
            //      `UNKNOWN_CODESYSTEM_VERSION` issue + `x-caused-by-unknown-system`
            //      parameter. Limited to single-coding to avoid short-circuiting
            //      the reverse loop before a later (good) coding gets visited
            //      in multi-coding CCs.
            let has_unknown_cs_version = codings.len() == 1
                && resp
                    .issues
                    .iter()
                    .any(|i| i.message_id.as_deref() == Some("UNKNOWN_CODESYSTEM_VERSION"));
            if resp.issues.iter().any(|i| i.tx_code == "vs-invalid") || has_unknown_cs_version {
                let resolved_version = resp.cs_version.clone();
                let coding_display = coding_displays
                    .get(&(system.clone(), code.clone()))
                    .cloned();
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    Some(&url),
                    display_language.as_deref(),
                    coding_display.as_deref(),
                    &supplements,
                )
                .await;
                append_used_supplements(&mut value, &supplements);
                // Apply check-system-version post-check on the failure path
                // too. The IG `codeableconcept-v10-vs20-check` /
                // `-v10-vsnn-check` fixtures expect the version-check error
                // alongside the pre-existing mismatch issue.
                if let Some(pat) = find_pin_for_system(&check_pins, &system) {
                    let actual = resolved_version
                        .clone()
                        .or_else(|| extract_response_version(&value));
                    if let Some(v) = actual.as_deref() {
                        if !version_satisfies_wildcard(v, pat) {
                            apply_check_version_failure(
                                &mut value,
                                &system,
                                v,
                                pat,
                                RequestPath::CodeableConcept,
                            );
                        }
                    }
                }
                return Ok(value);
            }
            // Display-only failure: the coding's code+system matched the VS
            // but the supplied display didn't match the concept's known
            // displays/designations. The IG
            // `validation/simple-codeableconcept-bad-display(W)` fixtures
            // (single-coding CC) expect this coding to win — echo its data
            // with the backend-emitted `invalid-display` issue intact.
            //
            // Limited to the single-coding case so multi-coding CCs (e.g.
            // `complex-codeableconcept-full`) still fall through to the
            // comprehensive issue-collection path below.
            //
            // Detected by: (a) we have an `invalid-display` issue, and
            // (b) we don't also have a "code not in VS" / "code not in CS"
            // failure (which would mean the code itself didn't validate).
            let has_invalid_display = resp
                .issues
                .iter()
                .any(|i| i.tx_code == "invalid-display");
            let has_real_failure = resp.issues.iter().any(|i| {
                matches!(
                    i.tx_code.as_str(),
                    "not-in-vs" | "this-code-not-in-vs" | "invalid-code" | "not-found"
                )
            });
            if codings.len() == 1 && has_invalid_display && !has_real_failure {
                let coding_display = coding_displays
                    .get(&(system.clone(), code.clone()))
                    .cloned();
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    Some(&url),
                    display_language.as_deref(),
                    coding_display.as_deref(),
                    &supplements,
                )
                .await;
                append_used_supplements(&mut value, &supplements);
                return Ok(value);
            }
        }

        // No coding matched. The IG `permutations/bad-cc*` fixtures expect:
        //   1. one error code-invalid/not-in-vs "No valid coding was found ..."
        //   2. per-coding error code-invalid/invalid-code "Unknown code 'X' in
        //      the CodeSystem 'sys' version 'Y'" when the code isn't in CS
        //   3. per-coding info code-invalid/this-code-not-in-vs "The provided
        //      code 'sys#code ('Display')' was not found in the value set ..."
        let vs_version_owned = crate::traits::ValueSetOperations::search(
            state.backend(),
            &ctx,
            crate::types::ResourceSearchQuery {
                url: Some(url.clone()),
                version: vs_version.clone(),
                count: Some(1),
                ..Default::default()
            },
        )
        .await
        .ok()
        .and_then(|mut hits| {
            hits.pop().and_then(|vs| {
                vs.get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
        });
        let url_with_version = match vs_version_owned.as_deref() {
            Some(v) => format!("{url}|{v}"),
            None => url.clone(),
        };

        // `valueset-membership-only=true` (IG
        // `validation/complex-codeableconcept-vsonly`) tells the server to
        // report only VS-membership issues; per-CodeSystem diagnostics
        // (`Unknown_Code_in_Version`, `UNKNOWN_CODESYSTEM`) are suppressed.
        let membership_only = params
            .iter()
            .find(|p| {
                p.get("name").and_then(|v| v.as_str()) == Some("valueset-membership-only")
            })
            .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool()))
            .unwrap_or(false);

        // TX_GENERAL_CC_ERROR_MESSAGE: top-level "no valid coding" error.
        // The IG fixtures do NOT expect location or expression on this issue.
        let mut issues: Vec<ValidationIssue> = vec![ValidationIssue {
            severity: "error".into(),
            fhir_code: "code-invalid".into(),
            tx_code: "not-in-vs".into(),
            text: format!("No valid coding was found for the value set '{url_with_version}'"),
            expression: None,
            location: None,
            message_id: Some("TX_GENERAL_CC_ERROR_MESSAGE".into()),
        }];

        // For each coding, emit per-coding issues based on whether the
        // CodeSystem and code exist.
        // Track unknown CSes (per-coding) so we can emit `x-unknown-system`
        // and the per-CS `UNKNOWN_CODESYSTEM` issue once per coding when the
        // referenced CS isn't stored.
        let mut single_unknown_system: Option<String> = None;
        for (idx, (system, code)) in codings.iter().enumerate() {
            // Use a real existence check (search by URL) rather than relying
            // on `code_system_version_for_url` — a stored CS that has no
            // `version` field would otherwise look "unknown" here.
            let cs_exists = crate::traits::CodeSystemOperations::search(
                state.backend(),
                &ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(system.clone()),
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .map(|hits| !hits.is_empty())
            .unwrap_or(false);
            // Look up the CS version for messaging (best-effort; may be None
            // even when cs_exists=true if the CS has no `version` field).
            let cs_version = state
                .backend()
                .code_system_version_for_url(&ctx, system)
                .await
                .ok()
                .flatten();
            // Per-coding lookup: does the code exist in the CS at all?
            let code_in_cs = if cs_exists {
                let req = ValidateCodeRequest {
                    url: None,
                    value_set_version: None,
                    system: Some(system.clone()),
                    code: code.clone(),
                    version: None,
                    display: None,
                    date: None,
                    include_abstract: None,
                    input_form: None,
                    lenient_display_validation: None,
                    default_value_set_versions: std::collections::HashMap::new(),
                };
                CodeSystemOperations::validate_code(state.backend(), &ctx, req)
                    .await
                    .map(|r| r.result)
                    .unwrap_or(false)
            } else {
                false
            };

            if cs_exists && !code_in_cs && !membership_only {
                let cs_text = match cs_version.as_deref() {
                    Some(v) => {
                        format!("Unknown code '{code}' in the CodeSystem '{system}' version '{v}'")
                    }
                    None => format!("Unknown code '{code}' in the CodeSystem '{system}'"),
                };
                issues.push(ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "code-invalid".into(),
                    tx_code: "invalid-code".into(),
                    text: cs_text,
                    expression: Some(format!("CodeableConcept.coding[{idx}].code")),
                    location: None,
                    message_id: Some("Unknown_Code_in_Version".into()),
                });
            } else if !cs_exists && !membership_only {
                // CS not found: emit per-coding UNKNOWN_CODESYSTEM issue. Per
                // IG fixture (validation/simple-codeableconcept-bad-system),
                // text quotes the CS URL with single-quotes. Location goes
                // on .system, expression too.
                //
                // When the coding carries a `version` for the unknown system,
                // the IG `simple-codeableconcept-bad-version2` fixture expects
                // the version-aware variant `UNKNOWN_CODESYSTEM_VERSION_NONE`
                // with text "...version 'X' could not be found ... No versions
                // of this code system are known". The `_NONE` suffix marks the
                // case where the system itself is unknown (zero stored
                // versions), distinguishing it from `UNKNOWN_CODESYSTEM_VERSION`
                // (the system exists but the requested version doesn't).
                let coding_version = coding_versions
                    .get(&(system.clone(), code.clone()))
                    .cloned();
                let (text, message_id) = match coding_version.as_deref() {
                    Some(v) => (
                        format!(
                            "A definition for CodeSystem '{system}' version '{v}' could not be found, \
                             so the code cannot be validated. No versions of this code system are known"
                        ),
                        "UNKNOWN_CODESYSTEM_VERSION_NONE",
                    ),
                    None => (
                        format!(
                            "A definition for CodeSystem '{system}' could not be found, so the code cannot be validated"
                        ),
                        "UNKNOWN_CODESYSTEM",
                    ),
                };
                let loc = format!("CodeableConcept.coding[{idx}].system");
                issues.push(ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "not-found".into(),
                    tx_code: "not-found".into(),
                    text,
                    expression: Some(loc.clone()),
                    location: Some(loc),
                    message_id: Some(message_id.into()),
                });
                // Track first unknown CS for the `x-unknown-system` param.
                if single_unknown_system.is_none() {
                    single_unknown_system = Some(system.clone());
                }
            }

            // Per-coding "this code wasn't in VS" issue. The IG fixtures expect
            // severity=information and tx_code=this-code-not-in-vs.
            let display = coding_displays.get(&(system.clone(), code.clone()));
            // Include the coding's version (when present) in the qualified
            // form, per the IG `simple-codeableconcept-bad-version2` fixture
            // which expects `system|version#code` for codings that carry an
            // explicit version. Versionless codings still use `system#code`.
            let coding_version_for_qual = coding_versions.get(&(system.clone(), code.clone()));
            let qualified = match (coding_version_for_qual, display) {
                (Some(v), Some(d)) => format!("{system}|{v}#{code} ('{d}')"),
                (Some(v), None) => format!("{system}|{v}#{code}"),
                (None, Some(d)) => format!("{system}#{code} ('{d}')"),
                (None, None) => format!("{system}#{code}"),
            };
            // For unknown systems, expression also goes to a location[] entry
            // (matches IG `simple-codeableconcept-bad-version2` which has both
            // location and expression on the not-in-vs issue).
            let coding_loc = format!("CodeableConcept.coding[{idx}].code");
            let location_for_issue = if !cs_exists {
                Some(coding_loc.clone())
            } else {
                None
            };
            issues.push(ValidationIssue {
                severity: "information".into(),
                fhir_code: "code-invalid".into(),
                tx_code: "this-code-not-in-vs".into(),
                text: format!(
                    "The provided code '{qualified}' was not found in the value set '{url_with_version}'"
                ),
                expression: Some(coding_loc),
                location: location_for_issue,
                message_id: Some(
                    "None_of_the_provided_codes_are_in_the_value_set_one".into(),
                ),
            });
        }

        let mut value = build_validate_response(
            ValidateCodeResponse {
                result: false,
                message: None,
                display: None,
                system: None,
                cs_version: None,
                inactive: None,
                issues,
                caused_by_unknown_system: None,
                concept_status: None,
                normalized_code: None,
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            // We've already emitted UNKNOWN_CODESYSTEM issue(s) inline above
            // with the IG-correct CodeableConcept location/expression. Pass
            // None here to avoid build_validate_response synthesising a
            // duplicate (with the generic Coding.system location).
            None,
            RequestPath::CodeableConcept,
        );
        // Append `x-unknown-system` for the first unknown CS encountered.
        // Matches the IG `validation/simple-codeableconcept-bad-system`
        // fixture which expects exactly one such param.
        if let Some(unknown) = single_unknown_system.as_deref() {
            if let Some(arr) = value.get_mut("parameter").and_then(|p| p.as_array_mut()) {
                arr.push(json!({
                    "name": "x-unknown-system",
                    "valueCanonical": unknown,
                }));
            }
        }
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    Err(HtsError::InvalidRequest(
        "Must provide one of: code, coding (valueCoding), or \
         codeableConcept (valueCodeableConcept)"
            .into(),
    ))
}

/// Sentinel marker prepended to a [`HtsError::VsInvalid`] when a
/// validate-code request fails because `displayLanguage` is not a
/// well-formed BCP-47 language tag. Picked up by
/// [`invalid_display_language_response`] to format the IG-spec
/// OperationOutcome shape (`code=processing`, `INVALID_DISPLAY_NAME`).
const INVALID_DISPLAY_LANGUAGE_PREFIX: &str = "__INVALID_DISPLAY_LANGUAGE__:";

/// Returns `true` when `lang` is a syntactically plausible BCP-47 tag, OR a
/// comma-separated list of such tags (e.g. `de,it,zh` or `en, en-AU`).
///
/// We reject the tag forms that the IG `display/validation-wrong-de-en-bad`
/// fixture expects to fail: empty, leading hyphen, trailing hyphen, double
/// hyphen, or non-ASCII letters in the primary subtag. This is intentionally
/// loose — we don't validate against the IANA registry — so any reasonable
/// language code (e.g. `de`, `en-US`, `zh-Hans-CN`) still passes.
///
/// FHIR R5 `displayLanguage` accepts a comma-separated preference list — IG
/// `validation/simple-*-language*` fixtures pass `de,it,zh` / `en, en-AU` and
/// expect the server to interpret each comma-separated token as a language
/// preference rather than rejecting the whole string. We split on `,`, trim
/// surrounding whitespace, and require every non-empty token to be
/// well-formed; an empty token (e.g. trailing comma) makes the whole input
/// malformed.
fn is_well_formed_display_language(lang: &str) -> bool {
    fn is_single_tag_well_formed(tag: &str) -> bool {
        if tag.is_empty()
            || tag.starts_with('-')
            || tag.ends_with('-')
            || tag.contains("--")
        {
            return false;
        }
        let primary = tag.split('-').next().unwrap_or("");
        (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic())
    }
    if lang.is_empty() {
        return false;
    }
    // Comma-separated list: every non-empty token must be well-formed; a
    // bare comma (e.g. `,` or `de,`) is malformed.
    if lang.contains(',') {
        return lang
            .split(',')
            .all(|t| is_single_tag_well_formed(t.trim()));
    }
    is_single_tag_well_formed(lang)
}

/// If `err` carries the `__INVALID_DISPLAY_LANGUAGE__` sentinel, format the
/// 4xx OperationOutcome the IG `display/validation-wrong-de-en-bad` and
/// language2 fixtures expect.  Returns `None` when `err` is unrelated.
fn invalid_display_language_response(err: &HtsError) -> Option<Response> {
    use axum::response::IntoResponse;
    let HtsError::VsInvalid(msg) = err else {
        return None;
    };
    let lang = msg.strip_prefix(INVALID_DISPLAY_LANGUAGE_PREFIX)?;
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                "valueString": "INVALID_DISPLAY_NAME"
            }],
            "severity": "error",
            "code": "processing",
            "details": {
                "coding": [{
                    "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                    "code": "invalid-display"
                }],
                "text": format!("Invalid displayLanguage: '{lang}'"),
            }
        }]
    });
    Some((StatusCode::BAD_REQUEST, Json(body)).into_response())
}

/// Build a `code: "processing"` cycle-detection OperationOutcome for
/// validate-code paths so they match the IG `big/big-circle-validate` shape
/// (the same fixture used by `$expand`'s cyclic_reference_response, but
/// reachable via VS-validate-code as well).  Returns `None` when `err` is
/// not a cycle so the caller falls through to the generic [`HtsError`]
/// [`IntoResponse`] path.
fn vs_cyclic_validate_response(err: &HtsError) -> Option<Response> {
    use axum::response::IntoResponse;
    let HtsError::VsInvalid(msg) = err else {
        return None;
    };
    if !msg.starts_with("Cyclic reference detected when excluding ") {
        return None;
    }
    let body = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "extension": [{
                "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                "valueString": "VALUESET_CIRCULAR_REFERENCE"
            }],
            "severity": "error",
            "code": "processing",
            "details": {
                "coding": [{
                    "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                    "code": "vs-invalid"
                }],
                "text": msg
            },
            "diagnostics": msg
        }]
    });
    Some((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response())
}

/// POST /ValueSet/$validate-code
pub async fn vs_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let mut params = extract_parameter_array(&body)?;
    crate::operations::expand::inject_accept_language(&headers, &mut params);
    match process_vs_validate_code(&state, params).await {
        Ok(v) => Ok(fhir_respond(v, format)),
        Err(e) => {
            if let Some(resp) = invalid_display_language_response(&e) {
                return Ok(resp);
            }
            match vs_cyclic_validate_response(&e) {
                Some(resp) => Ok(resp),
                None => Err(e),
            }
        }
    }
}

/// GET /ValueSet/$validate-code?url=...&code=...
pub async fn get_vs_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    match process_vs_validate_code(&state, params).await {
        Ok(v) => Ok(fhir_respond(v, format)),
        Err(e) => {
            if let Some(resp) = invalid_display_language_response(&e) {
                return Ok(resp);
            }
            match vs_cyclic_validate_response(&e) {
                Some(resp) => Ok(resp),
                None => Err(e),
            }
        }
    }
}

// ── Instance-level: /ValueSet/{id}/$validate-code ─────────────────────────────

/// Inject (or replace) the `url` parameter in a params list.
fn inject_url(mut params: Vec<Value>, url: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("url"));
    let mut with_url = vec![json!({"name": "url", "valueUri": url})];
    with_url.append(&mut params);
    with_url
}

/// POST /ValueSet/{id}/$validate-code
///
/// Resolves the ValueSet canonical URL from its FHIR `id`, then delegates to
/// the same validate-code logic used by the system-level endpoint.
pub async fn vs_validate_by_id_post<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let raw_params = body
        .and_then(|Json(v)| extract_parameter_array(&v).ok())
        .unwrap_or_default();
    Ok(fhir_respond(
        process_vs_validate_code(&state, inject_url(raw_params, url)).await?,
        format,
    ))
}

/// GET /ValueSet/{id}/$validate-code?code=...
pub async fn get_vs_validate_by_id<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_vs_validate_code(&state, inject_url(params, url)).await?,
        format,
    ))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;

    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::state::AppState;

    fn make_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs1', 'http://example.org/cs', '1.0', 'Example CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'ABC', 'Alpha Beta Charlie');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/CodeSystem/$validate-code",
                post(validate_code_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn post_json(app: Router, uri: &str, body: Value) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn valid_code_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);
    }

    #[tokio::test]
    async fn valid_code_returns_display() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let display_param = params.iter().find(|p| p["name"] == "display").unwrap();
        assert_eq!(display_param["valueString"], "Alpha Beta Charlie");
    }

    #[tokio::test]
    async fn system_param_rejected_with_400() {
        // FHIR spec requires `url`; sending `system` is not accepted.
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn unknown_code_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "NOPE"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn unknown_system_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://unknown.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn display_match_has_no_message() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"},
                {"name": "display", "valueString": "Alpha Beta Charlie"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        assert!(
            params.iter().all(|p| p["name"] != "message"),
            "no message expected when display matches"
        );
    }

    #[tokio::test]
    async fn display_mismatch_returns_false_with_message() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"},
                {"name": "display", "valueString": "Wrong Display"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(
            result_param["valueBoolean"], false,
            "display mismatch makes result=false per FHIR spec"
        );

        let has_message = params.iter().any(|p| p["name"] == "message");
        assert!(has_message, "message expected for display mismatch");
    }

    #[tokio::test]
    async fn missing_url_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn missing_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "CodeSystem",
            "parameter": []
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── ValueSet/$validate-code tests ──────────────────────────────────────────

    fn make_vs_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();

        // Seed directly via SQL (same pattern as other operation handler tests).
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs-vs', 'http://example.org/cs', '1.0', 'TestCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs-vs', 'A', 'Alpha'),
                        (2, 'cs-vs', 'B', 'Beta'),
                        (3, 'cs-vs', 'C', 'Gamma');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at)
                 VALUES ('vs-main', 'http://example.org/vs', 'TestVS', 'active',
                         '{\"include\":[{\"system\":\"http://example.org/cs\",\"concept\":[{\"code\":\"A\"},{\"code\":\"B\"}]}]}',
                         '2024-01-01', '2024-01-01');",
            )
            .unwrap();
        }

        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$validate-code",
                post(vs_validate_code_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn vs_code_in_set_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn vs_code_not_in_set_returns_false() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "C" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_missing_url_returns_400() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn vs_missing_code_returns_400() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn vs_unknown_value_set_returns_404() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://unknown.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn vs_returns_display_for_valid_code() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let display = params.iter().find(|p| p["name"] == "display").unwrap();
        assert_eq!(display["valueString"], "Alpha");
    }

    // ── valueCoding input ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn cs_validate_coding_valid_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "coding",
                    "valueCoding": {
                        "system": "http://example.org/cs",
                        "code": "ABC",
                        "display": "Alpha Beta Charlie"
                    }
                }
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn cs_validate_coding_unknown_code_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "coding",
                "valueCoding": {"system": "http://example.org/cs", "code": "UNKNOWN"}
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_validate_coding_in_set_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {
                    "name": "coding",
                    "valueCoding": {
                        "system": "http://example.org/cs",
                        "code": "A"
                    }
                }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    // ── valueCodeableConcept input ────────────────────────────────────────────

    #[tokio::test]
    async fn cs_validate_codeable_concept_one_match_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://other.org/cs", "code": "NOPE"},
                        {"system": "http://example.org/cs", "code": "ABC"}
                    ]
                }
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn cs_validate_codeable_concept_no_match_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://example.org/cs", "code": "X"},
                        {"system": "http://example.org/cs", "code": "Y"}
                    ]
                }
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_validate_codeable_concept_one_match_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {
                    "name": "codeableConcept",
                    "valueCodeableConcept": {
                        "coding": [
                            {"system": "http://example.org/cs", "code": "C"}, // not in VS
                            {"system": "http://example.org/cs", "code": "A"}  // in VS
                        ]
                    }
                }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn no_input_param_returns_400() {
        let app = make_app();
        // No code, coding, or codeableConcept — should be rejected
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── Supplement-aware display matching (IG `parameters-validate-supplement-good`) ──

    fn make_supplement_vs_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at, resource_json)
                 VALUES ('base', 'http://hl7.org/fhir/test/CodeSystem/extensions', '5.0.0',
                         'ExtensionsTestCodeSystem', 'active', 'complete',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\"}');

                 INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at, resource_json)
                 VALUES ('supp', 'http://hl7.org/fhir/test/CodeSystem/supplement', '0.1.1',
                         'SupplementCS', 'active', 'supplement',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\",\"supplements\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (10, 'base', 'code1', 'Display 1'),
                        (11, 'supp', 'code1', NULL);

                 INSERT INTO concept_designations (concept_id, language, value)
                 VALUES (10, 'de', 'Mein erster Code'),
                        (11, 'nl', 'ectenoot');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at, resource_json)
                 VALUES ('vs-extns', 'http://hl7.org/fhir/test/ValueSet/extensions-all-ns',
                         'ExtensionsValueSetAllNS', 'active',
                         '{\"include\":[{\"system\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}]}',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"ValueSet\"}');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$validate-code",
                post(vs_validate_code_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn vs_validate_supplement_display_matches_via_supplement_designation() {
        let app = make_supplement_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://hl7.org/fhir/test/ValueSet/extensions-all-ns"},
                {"name": "coding", "valueCoding": {
                    "system": "http://hl7.org/fhir/test/CodeSystem/extensions",
                    "code": "code1",
                    "display": "ectenoot"
                }},
                {"name": "useSupplement", "valueCanonical": "http://hl7.org/fhir/test/CodeSystem/supplement"}
            ]
        });
        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(
            result["valueBoolean"], true,
            "supplement designation 'ectenoot' should be accepted as alt display"
        );
        // IG parameters-validate-supplement-good response does NOT echo
        // used-supplement on $validate-code (only on $expand and $lookup),
        // so we don't assert its presence here. result=true is the proof
        // that the supplement designation rescued the display match.
    }

    #[tokio::test]
    async fn vs_validate_supplement_omitted_then_display_mismatch_fails() {
        // Mirror IG `parameters-validate-supplement-none-response`: same
        // request shape but no useSupplement → result=false because
        // 'ectenoot' is not in the base CS.
        let app = make_supplement_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://hl7.org/fhir/test/ValueSet/extensions-all-ns"},
                {"name": "coding", "valueCoding": {
                    "system": "http://hl7.org/fhir/test/CodeSystem/extensions",
                    "code": "code1",
                    "display": "ectenoot"
                }}
            ]
        });
        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_validate_unknown_supplement_returns_404() {
        let app = make_supplement_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://hl7.org/fhir/test/ValueSet/extensions-all-ns"},
                {"name": "coding", "valueCoding": {
                    "system": "http://hl7.org/fhir/test/CodeSystem/extensions",
                    "code": "code1"
                }},
                {"name": "useSupplement", "valueCanonical": "http://does-not-exist/cs"}
            ]
        });
        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 404);
    }

    // ── Multi-issue OperationOutcome ─────────────────────────────────────────

    #[tokio::test]
    async fn vs_validate_unknown_system_emits_two_issues() {
        // Mirror IG fixture validation/simple-coding-bad-system: when the
        // Coding's system isn't loaded, the OperationOutcome should carry
        // BOTH a `code-invalid`/`not-in-vs` issue (code not in VS) and a
        // `not-found`/`not-found` issue (CodeSystem unknown).
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {
                    "name": "coding",
                    "valueCoding": {
                        "system": "http://unknown.org/cs",
                        "code": "anything"
                    }
                }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let issues_param = params.iter().find(|p| p["name"] == "issues").unwrap();
        let issues = issues_param["resource"]["issue"].as_array().unwrap();
        assert_eq!(
            issues.len(),
            2,
            "expected 2 issues (code-invalid + not-found), got {issues:?}"
        );
        // One of the two issues must be code-invalid + not-in-vs.
        assert!(
            issues.iter().any(|i| {
                i["code"] == "code-invalid" && i["details"]["coding"][0]["code"] == "not-in-vs"
            }),
            "missing code-invalid/not-in-vs issue: {issues:?}"
        );
        // The other must be not-found / not-found pointing at the unknown CS.
        assert!(
            issues.iter().any(|i| {
                i["code"] == "not-found" && i["details"]["coding"][0]["code"] == "not-found"
            }),
            "missing not-found/not-found issue: {issues:?}"
        );
        // x-unknown-system parameter still echoed.
        assert!(
            params.iter().any(|p| p["name"] == "x-unknown-system"
                && p["valueCanonical"] == "http://unknown.org/cs"),
            "missing x-unknown-system param"
        );
    }

    #[tokio::test]
    async fn vs_validate_no_system_on_coding_emits_invalid_data_issue() {
        // Coding without `system` is a structural problem — emit
        // `invalid` / `invalid-data` rather than a generic not-in-vs issue.
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {"name": "coding", "valueCoding": {"code": "A"}}
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
        let issues = params.iter().find(|p| p["name"] == "issues").unwrap()["resource"]["issue"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            issues.iter().any(|i| {
                i["code"] == "invalid" && i["details"]["coding"][0]["code"] == "invalid-data"
            }),
            "expected invalid/invalid-data issue: {issues:?}"
        );
    }
}
