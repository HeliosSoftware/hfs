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
    http::{HeaderMap, header},
    response::Response,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::{CodeSystemOperations, SupplementInfo, TerminologyBackend, ValueSetOperations};
use crate::types::{ValidateCodeRequest, ValidateCodeResponse, ValidationIssue};

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    collect_canonical_params, extract_codeable_concept, extract_coding_full,
    extract_parameter_array, find_str_param, parse_query_string, query_params_to_fhir_params,
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
    let mut parameter: Vec<Value> = Vec::new();
    if let Some(c) = code {
        parameter.push(json!({"name": "code", "valueCode": c}));
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
    if let Some(unknown) = unknown_system {
        let text = format!(
            "A definition for CodeSystem {unknown} could not be found, so the code cannot be validated"
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
            expression: Some(expression),
            location: None,
            message_id: Some("UNKNOWN_CODESYSTEM".into()),
        });
    }

    // Determine the message string: when we have structured issues, sort
    // their texts alphabetically and join with `; ` (matches the IG fixture
    // convention). When we don't, fall back to the response's own `message`
    // (legacy single-message path used by older code in $translate, etc.).
    // Error-severity issues always contribute to the top-level `message`.
    // Inactive/status warnings (`INACTIVE_CONCEPT_FOUND`) also contribute —
    // the IG `inactive/validate-inactive-*` fixtures expect their text in
    // the top-level `message` parameter even though they are warnings.
    let message_str: Option<String> = if !issues.is_empty() {
        let mut texts: Vec<&str> = issues
            .iter()
            .filter(|i| {
                i.severity == "error"
                    || (i.severity == "warning"
                        && i.message_id.as_deref() == Some("INACTIVE_CONCEPT_FOUND"))
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
    // result is driven by error-severity issues when we have any; otherwise
    // honour the backend's `resp.result`.
    let final_result = if issues.is_empty() {
        resp.result
    } else {
        !issues.iter().any(|i| i.severity == "error")
    };
    parameter.push(json!({"name": "result", "valueBoolean": final_result}));
    if let Some(s) = system {
        parameter.push(json!({"name": "system", "valueUri": s}));
    }
    if let Some(v) = version {
        parameter.push(json!({"name": "version", "valueString": v}));
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

    // If the input system isn't stored, the IG expects an `x-unknown-system`
    // parameter pointing at the unknown URL (only when validate-code reported
    // result=false).
    let unknown_system = if !resp.result && stored_version.is_none() {
        effective_system
    } else {
        None
    };

    // Append info-level "Reference to <status> CodeSystem url|version" issues
    // when the validated CodeSystem carries a non-active standards-status —
    // matches the IG `deprecated/validate-*` fixtures.
    if let Some(sys) = effective_system {
        if let Ok(mut hits) = crate::traits::CodeSystemOperations::search(
            backend,
            ctx,
            crate::types::ResourceSearchQuery {
                url: Some(sys.to_string()),
                count: Some(1),
                ..Default::default()
            },
        )
        .await
        {
            if let Some(cs) = hits.pop() {
                for status in collect_status_check_codes(&cs) {
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
        }
    }

    // Mirror the same status-check emission for the validated ValueSet on
    // the VS-validate-code path. The IG `deprecated/validate-withdrawn`
    // fixture expects BOTH a deprecated-CS issue AND a withdrawn-VS issue.
    if let Some(vs_url) = value_set_url {
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
                for status in collect_status_check_codes(&vs) {
                    let vs_uri = match vs_version {
                        Some(v) => format!("{vs_url}|{v}"),
                        None => vs_url.to_string(),
                    };
                    resp.issues.push(ValidationIssue {
                        severity: "information".into(),
                        fhir_code: "business-rule".into(),
                        tx_code: "status-check".into(),
                        text: format!("Reference to {status} ValueSet {vs_uri}"),
                        expression: None,
                        location: None,
                        message_id: Some(status_message_id(&status).into()),
                    });
                }
            }
        }
    }

    build_validate_response(
        resp,
        code,
        effective_system,
        version.as_deref(),
        codeable_concept,
        unknown_system,
        request_path,
    )
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
fn append_used_supplements(value: &mut Value, supplements: &[SupplementInfo]) {
    if supplements.is_empty() {
        return;
    }
    if let Some(arr) = value.get_mut("parameter").and_then(|p| p.as_array_mut()) {
        for info in supplements {
            arr.push(json!({
                "name": "used-supplement",
                "valueCanonical": info.supplement_canonical,
            }));
        }
    }
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
    // mismatch message starts with either "Display mismatch:" (CodeSystem
    // path, see code_system.rs) or "Provided display ... does not match"
    // (ValueSet path, see finish_validate_code_response in value_set.rs).
    let msg = resp.message.as_deref().unwrap_or("");
    let looks_like_display_mismatch =
        msg.starts_with("Display mismatch:") || msg.contains("does not match stored display");
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
    let ctx = TenantContext::system();
    // ── Path 1: bare `code` parameter (requires `url` = CodeSystem canonical URL) ──
    if let Some(code) = find_str_param(&params, "code") {
        let system = find_str_param(&params, "url").ok_or_else(|| {
            HtsError::InvalidRequest(
                "Missing required parameter: url (CodeSystem canonical URL). \
                 Use `url`, not `system`, for CodeSystem/$validate-code."
                    .into(),
            )
        })?;
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
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    // ── Path 2: `coding` parameter (valueCoding — system+code bundled together) ──
    if let Some((system, code, coding_display, coding_version)) =
        extract_coding_full(&params, "coding")
    {
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
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_validate_code(&state, params).await?,
        format,
    ))
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
    Ok(fhir_respond(
        process_validate_code(&state, params).await?,
        format,
    ))
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

    // Flip `result` to false and set/replace `message` with the version-check
    // text (matches the IG fixtures: when check fires, `message` is the
    // check-error text alone).
    for p in params.iter_mut() {
        match p.get("name").and_then(|v| v.as_str()) {
            Some("result") => {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("valueBoolean".into(), Value::Bool(false));
                }
            }
            Some("message") => {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("valueString".into(), Value::String(text.clone()));
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
        let entry = json!({"name": "message", "valueString": text});
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
            let exists = ValueSetOperations::search(
                backend,
                ctx,
                crate::types::ResourceSearchQuery {
                    url: Some(bare_url.to_string()),
                    version: ver,
                    count: Some(1),
                    ..Default::default()
                },
            )
            .await
            .map(|hs| !hs.is_empty())
            .unwrap_or(false);
            if !exists {
                return Some(bare_url.to_string());
            }
        }
    }
    None
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
    // ValueSet/$validate-code always requires `url` (the ValueSet canonical URL).
    let url = find_str_param(&params, "url").ok_or_else(|| {
        HtsError::InvalidRequest("Missing required parameter: url (ValueSet canonical URL)".into())
    })?;

    let ctx = TenantContext::system();
    // ValueSet validate-code can carry useSupplement that targets ANY
    // CodeSystem in the VS expansion. We can't (yet) verify the target
    // matches a system in the VS without expanding, so pass `None` for
    // expected_target here — bad-supplement-not-found is still rejected.
    let supplements = resolve_supplements(state.backend(), &ctx, &params, None).await?;
    // Used to rewrite "...'url'..." → "...'url|version'..." in NotFound
    // messages so the IG-expected text format is met.
    let vs_version = find_str_param(&params, "valueSetVersion");

    // Detect a ValueSet whose compose.include[*].valueSet imports an
    // unresolvable ValueSet up-front. The IG `validation/simple-*-bad-import`
    // fixtures expect a single `not-found / Unable_to_resolve_value_Set_`
    // issue with text "A definition for the value Set 'X' could not be
    // found" — not the cascade of TX_GENERAL_CC_ERROR_MESSAGE/this-code-not-in-vs
    // that the regular CC fallback emits.
    if let Some(unresolved_vs_url) =
        detect_bad_vs_import(state.backend(), &ctx, &url, vs_version.as_deref()).await
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
        let vs_has_pin = source_vs
            .and_then(|vs| vs_include_pin_for_system(vs, system))
            .map(|opt_v| opt_v.is_some())
            .unwrap_or(false);
        if vs_has_pin {
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
            value_set_version: vs_version.clone(),
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
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
        // When force-system-version was active for this system, suppress the
        // backend's VS-pin mismatch issues — the forced version overrides the
        // VS pin entirely.
        if let (Some(sys), Some(forced)) = (system.as_deref(), req_version.as_deref()) {
            if find_pin_for_system(&force_pins, sys).is_some() {
                suppress_forced_version_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    sys,
                    &code,
                    forced,
                )
                .await;
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
                            location: None,
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
                            location: None,
                            message_id: Some("Coding_has_no_system__cannot_validate".into()),
                        },
                    ],
                    caused_by_unknown_system: None,
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
            value_set_version: vs_version.clone(),
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
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
        // When force-system-version was active for this system, suppress the
        // backend's VS-pin mismatch issues — the forced version overrides the
        // VS pin entirely.
        if let Some(forced) = req_version.as_deref() {
            if find_pin_for_system(&force_pins, &system).is_some() {
                suppress_forced_version_mismatch(
                    state.backend(),
                    &ctx,
                    &mut resp,
                    &system,
                    &code,
                    forced,
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
        let cc_req_version = find_str_param(&params, "version").or(system_version.clone());
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
            let req = ValidateCodeRequest {
                url: Some(url.clone()),
                value_set_version: vs_version.clone(),
                system: Some(system.clone()),
                code: code.clone(),
                version: per_coding_version.clone(),
                display: None,
                date: find_str_param(&params, "date"),
                include_abstract: params
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                    .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
                input_form: Some("codeableConcept".into()),
                lenient_display_validation: lenient_display,
            };
            let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
                .await
                .map_err(&rewrite)?;
            // When force-system-version was active for this system, suppress
            // the backend's VS-pin mismatch issues for this coding.
            if let Some(forced) = per_coding_version.as_deref() {
                if find_pin_for_system(&force_pins, &system).is_some() {
                    suppress_forced_version_mismatch(
                        state.backend(),
                        &ctx,
                        &mut resp,
                        &system,
                        &code,
                        forced,
                    )
                    .await;
                }
            }
            if resp.result {
                let resolved_version = resp.cs_version.clone();
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    Some(&url),
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
            if resp.issues.iter().any(|i| i.tx_code == "vs-invalid") {
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                    RequestPath::CodeableConcept,
                    Some(&url),
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
        for (idx, (system, code)) in codings.iter().enumerate() {
            // Look up the CS version for messaging.
            let cs_version = state
                .backend()
                .code_system_version_for_url(&ctx, system)
                .await
                .ok()
                .flatten();
            let cs_known = cs_version.is_some();
            // Per-coding lookup: does the code exist in the CS at all?
            let code_in_cs = if cs_known {
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
                };
                CodeSystemOperations::validate_code(state.backend(), &ctx, req)
                    .await
                    .map(|r| r.result)
                    .unwrap_or(false)
            } else {
                false
            };

            if cs_known && !code_in_cs {
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
            }

            // Per-coding "this code wasn't in VS" issue. The IG fixtures expect
            // severity=information and tx_code=this-code-not-in-vs.
            let display = coding_displays.get(&(system.clone(), code.clone()));
            let qualified = match display {
                Some(d) => format!("{system}#{code} ('{d}')"),
                None => format!("{system}#{code}"),
            };
            issues.push(ValidationIssue {
                severity: "information".into(),
                fhir_code: "code-invalid".into(),
                tx_code: "this-code-not-in-vs".into(),
                text: format!(
                    "The provided code '{qualified}' was not found in the value set '{url_with_version}'"
                ),
                expression: Some(format!("CodeableConcept.coding[{idx}].code")),
                location: None,
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
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            None,
            RequestPath::CodeableConcept,
        );
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    Err(HtsError::InvalidRequest(
        "Must provide one of: code, coding (valueCoding), or \
         codeableConcept (valueCodeableConcept)"
            .into(),
    ))
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
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_vs_validate_code(&state, params).await?,
        format,
    ))
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
    Ok(fhir_respond(
        process_vs_validate_code(&state, params).await?,
        format,
    ))
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
        let used = params
            .iter()
            .find(|p| p["name"] == "used-supplement")
            .expect("used-supplement parameter must be echoed");
        assert_eq!(
            used["valueCanonical"],
            "http://hl7.org/fhir/test/CodeSystem/supplement|0.1.1"
        );
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
