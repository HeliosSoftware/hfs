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
    extract_codeable_concept, extract_coding, extract_parameter_array, find_str_param,
    parse_query_string, query_params_to_fhir_params,
};

/// Render a single [`ValidationIssue`] as a FHIR `OperationOutcome.issue`.
///
/// The resulting JSON includes the `operationoutcome-message-id` extension
/// when the issue carries one, plus `details.coding[]` with the tx-issue-type
/// coding and `details.text`. `expression` and `location` echo the structured
/// FHIRPath location.
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
        json_issue
            .as_object_mut()
            .unwrap()
            .insert("expression".into(), json!([loc]));
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
    if let Some(unknown) = unknown_system {
        let text = format!(
            "A definition for CodeSystem {unknown} could not be found, so the code cannot be validated"
        );
        issues.push(ValidationIssue {
            severity: "error".into(),
            fhir_code: "not-found".into(),
            tx_code: "not-found".into(),
            text,
            location: Some("Coding.system".into()),
            message_id: Some("UNKNOWN_CODESYSTEM".into()),
        });
    }

    // Determine the message string: when we have structured issues, sort
    // their texts alphabetically and join with `; ` (matches the IG fixture
    // convention). When we don't, fall back to the response's own `message`
    // (legacy single-message path used by older code in $translate, etc.).
    let message_str: Option<String> = if !issues.is_empty() {
        let mut texts: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
        texts.sort();
        Some(texts.join("; "))
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
    json!({
        "resourceType": "Parameters",
        "parameter": parameter
    })
}

/// Build a validate-code response and resolve the system's version via a
/// backend lookup (so the response can echo `version` per the IG fixtures).
async fn build_validate_response_async<B: TerminologyBackend>(
    backend: &B,
    ctx: &TenantContext,
    resp: ValidateCodeResponse,
    code: Option<&str>,
    system: Option<&str>,
    codeable_concept: Option<&Value>,
) -> Value {
    // Prefer the system the caller passed; otherwise fall back to whatever
    // the backend inferred from the VS expansion (e.g. inferSystem=true).
    let inferred_system = resp.system.clone();
    let effective_system: Option<&str> = system.or(inferred_system.as_deref());

    let version = if let Some(s) = effective_system {
        backend
            .code_system_version_for_url(ctx, s)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    // If the input system isn't stored, the IG expects an `x-unknown-system`
    // parameter pointing at the unknown URL (only when validate-code reported
    // result=false).
    let unknown_system = if !resp.result && version.is_none() {
        effective_system
    } else {
        None
    };
    build_validate_response(
        resp,
        code,
        effective_system,
        version.as_deref(),
        codeable_concept,
        unknown_system,
    )
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
        let req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system.clone()),
            code: code.clone(),
            version: find_str_param(&params, "version"),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
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
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    // ── Path 2: `coding` parameter (valueCoding — system+code bundled together) ──
    if let Some((system, code, coding_display)) = extract_coding(&params, "coding") {
        // Coding.display takes precedence over a top-level `display` param —
        // the IG fixtures pin display via the Coding so the server can
        // report a mismatch.
        let display = coding_display.or_else(|| find_str_param(&params, "display"));
        let supplements =
            resolve_supplements(state.backend(), &ctx, &params, Some(&system)).await?;
        let req = ValidateCodeRequest {
            url: None,
            value_set_version: None,
            system: Some(system.clone()),
            code: code.clone(),
            version: find_str_param(&params, "version"),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
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
        for (system, code) in codings.into_iter().rev() {
            let req = ValidateCodeRequest {
                url: None,
                value_set_version: None,
                system: Some(system.clone()),
                code: code.clone(),
                version: find_str_param(&params, "version"),
                display: None,
                date: find_str_param(&params, "date"),
                include_abstract: params
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                    .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
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
                inactive: None,
                issues: vec![],
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            None,
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
        let req = ValidateCodeRequest {
            url: Some(url.clone()),
            value_set_version: vs_version.clone(),
            system: system.clone(),
            code: code.clone(),
            version: find_str_param(&params, "version"),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
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
        let mut value = build_validate_response_async(
            state.backend(),
            &ctx,
            resp,
            Some(&code),
            system.as_deref(),
            None,
        )
        .await;
        append_used_supplements(&mut value, &supplements);
        return Ok(value);
    }

    // ── Path 2: `coding` parameter (valueCoding) ──────────────────────────────
    if let Some((system, code, coding_display)) = extract_coding(&params, "coding") {
        // Empty system from extract_coding means the Coding had no system
        // field. Per the IG fixtures, that should produce result=false with
        // a "Coding has no system" message rather than matching by code
        // alone.
        if system.is_empty() {
            // The IG fixtures expect a single `invalid` / `invalid-data`
            // issue here, not a generic `code-invalid` / `not-in-vs`. Build
            // it as a structured issue so the message text matches the
            // fixture and result=false flows naturally.
            let text = "No System defined; Coding has no system - cannot validate".to_string();
            return Ok(build_validate_response(
                ValidateCodeResponse {
                    result: false,
                    message: Some(text.clone()),
                    display: None,
                    system: None,
                    inactive: None,
                    issues: vec![ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "invalid".into(),
                        tx_code: "invalid-data".into(),
                        text,
                        location: Some("Coding.system".into()),
                        message_id: Some("UNABLE_TO_INFER_CODESYSTEM".into()),
                    }],
                },
                Some(&code),
                None,
                None,
                None,
                None,
            ));
        }
        // Coding.display takes precedence over a top-level `display` param —
        // the IG fixtures pin display via the Coding so the server can
        // report a mismatch.
        let display = coding_display.or_else(|| find_str_param(&params, "display"));
        let req = ValidateCodeRequest {
            url: Some(url.clone()),
            value_set_version: vs_version.clone(),
            system: Some(system.clone()),
            code: code.clone(),
            version: find_str_param(&params, "version"),
            display: display.clone(),
            date: find_str_param(&params, "date"),
            include_abstract: params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
        };
        let mut resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
            .await
            .map_err(&rewrite)?;
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
        )
        .await;
        append_used_supplements(&mut value, &supplements);
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
        // The IG fixtures expect the LAST matching coding to win (when several
        // codings in a CodeableConcept all validate, the response echoes the
        // last one). Iterate in reverse so the earliest "yes" we find is the
        // last entry in the input.
        for (system, code) in codings.into_iter().rev() {
            let req = ValidateCodeRequest {
                url: Some(url.clone()),
                value_set_version: vs_version.clone(),
                system: Some(system.clone()),
                code: code.clone(),
                version: find_str_param(&params, "version"),
                display: None,
                date: find_str_param(&params, "date"),
                include_abstract: params
                    .iter()
                    .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("abstract"))
                    .and_then(|p| p.get("valueBoolean").and_then(|v| v.as_bool())),
            };
            let resp = ValueSetOperations::validate_code(state.backend(), &ctx, req)
                .await
                .map_err(&rewrite)?;
            if resp.result {
                let mut value = build_validate_response_async(
                    state.backend(),
                    &ctx,
                    resp,
                    Some(&code),
                    Some(&system),
                    cc_value.as_ref(),
                )
                .await;
                append_used_supplements(&mut value, &supplements);
                return Ok(value);
            }
        }
        let mut value = build_validate_response(
            ValidateCodeResponse {
                result: false,
                message: Some("None of the provided codings were found in the ValueSet".into()),
                display: None,
                system: None,
                inactive: None,
                issues: vec![],
            },
            None,
            None,
            None,
            cc_value.as_ref(),
            None,
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
