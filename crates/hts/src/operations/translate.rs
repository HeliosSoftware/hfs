//! Handler for `POST /ConceptMap/$translate`.
//!
//! Accepts a FHIR Parameters resource naming the source concept — as `code` +
//! `system`, a `coding`, or a `codeableConcept` (R5: the `source*` spellings, and
//! the `target*` ones for reverse translation) — plus optional `url` (ConceptMap
//! URL) and `reverse`. Returns a FHIR Parameters resource with a `result` boolean
//! and zero or more `match` parts.
//!
//! # FHIR specification
//! <https://hl7.org/fhir/conceptmap-operation-translate.html>

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
use crate::traits::{ConceptMapOperations, TerminologyBackend};
use crate::types::TranslateRequest;

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    extract_codeable_concept, extract_coding, extract_parameter_array, find_str_param,
    parse_query_string, query_params_to_fhir_params,
};

/// Resolve a `(system, code)` pair from the Coding/CodeableConcept spellings of a
/// `$translate` input parameter.
///
/// The spec defines the source concept three ways — `code` + `system`, `coding`, or
/// `codeableConcept` (R5: `sourceCoding` / `sourceCodeableConcept`, and the `target*`
/// forms for reverse translation) — but this handler only ever read the scalar pair, so
/// a spec-legal `coding` request was rejected as "Missing required parameter". See #287.
///
/// A CodeableConcept may carry several codings; the first is used, matching how
/// `$validate-code` treats the same input.
fn coding_pair(params: &[Value], coding_name: &str, cc_name: &str) -> Option<(String, String)> {
    if let Some((system, code, _display)) = extract_coding(params, coding_name) {
        return Some((system, code));
    }
    extract_codeable_concept(params, cc_name)?
        .into_iter()
        .next()
}

/// Core translation logic shared by the POST and GET handlers.
///
/// Extracts the source concept (required — see [`coding_pair`] for the accepted
/// spellings) and optional `url` (ConceptMap URL), `source`, `target`,
/// `targetSystem`, `reverse`, and `date` from `params`, delegates to
/// [`ConceptMapOperations::translate`], and assembles the FHIR `Parameters`
/// response.
///
/// `reverse` is parsed from `"true"` / `"false"` strings so it works for both
/// GET (query-string) and POST (JSON boolean) inputs.
///
/// ## Returns
///
/// A FHIR `Parameters` resource with a `result` boolean and zero or more
/// `match` parts.  Each `match` part contains `equivalence` and
/// `relationship` codes, a `concept` (`valueCoding`) for the target side
/// of the matched ConceptMap element, an `originMap` canonical reference,
/// and (in reverse responses) a `source` (`valueCoding`) for the source
/// side of the matched element.
///
/// ## Errors
///
/// Returns [`HtsError::InvalidRequest`] when no source concept is supplied in any
/// of the accepted spellings.
pub(crate) async fn process_translate<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // R4 names: `code`, `system`. R5 names: `sourceCode`, `sourceSystem`,
    // `targetCode`, `targetSystem`. Accept either form.
    let mut source_code =
        find_str_param(&params, "sourceCode").or_else(|| find_str_param(&params, "code"));
    let mut target_code = find_str_param(&params, "targetCode");
    let mut source_system =
        find_str_param(&params, "sourceSystem").or_else(|| find_str_param(&params, "system"));
    let mut target_system = find_str_param(&params, "targetSystem");

    // The scalar pair wins when present; Coding/CodeableConcept only fill the gaps, so a
    // request mixing the spellings keeps the explicit `code`/`system` the caller named.
    if source_code.is_none() {
        if let Some((system, code)) = coding_pair(&params, "coding", "codeableConcept")
            .or_else(|| coding_pair(&params, "sourceCoding", "sourceCodeableConcept"))
        {
            source_code = Some(code);
            source_system = source_system.or(Some(system).filter(|s| !s.is_empty()));
        }
    }
    if target_code.is_none() {
        if let Some((system, code)) = coding_pair(&params, "targetCoding", "targetCodeableConcept")
        {
            target_code = Some(code);
            target_system = target_system.or(Some(system).filter(|s| !s.is_empty()));
        }
    }

    // Need at least one of source code (forward) or target code (reverse).
    if source_code.is_none() && target_code.is_none() {
        return Err(HtsError::InvalidRequest(
            "Missing required parameter: code or sourceCode (or targetCode for reverse)".into(),
        ));
    }

    // `reverse` arrives as valueBoolean (POST) or plain string "true"/"false" (GET).
    let reverse_flag = find_str_param(&params, "reverse")
        .map(|s| s == "true")
        .unwrap_or(false);
    // The request is reverse-mode if the caller asked for it explicitly
    // (`reverse=true`) or supplied `targetCode` instead of `sourceCode`.
    let is_reverse = reverse_flag || target_code.is_some();

    let req = TranslateRequest {
        url: find_str_param(&params, "url"),
        system: source_system,
        // `code` is the forward-mode lookup. Empty string when only
        // `targetCode` is supplied (reverse mode keyed on `target_code`).
        code: source_code.unwrap_or_default(),
        source: find_str_param(&params, "source"),
        target: find_str_param(&params, "target"),
        target_system,
        target_code,
        reverse: reverse_flag,
        date: find_str_param(&params, "date"),
    };

    let ctx = TenantContext::system();
    let resp = ConceptMapOperations::translate(state.backend(), &ctx, req).await?;

    // ── Build FHIR Parameters response ─────────────────────────────────────────
    //
    // The `match` parts come *before* `result` in the IG fixtures; emit in the
    // same order so byte-for-byte comparison passes.
    let mut parameter: Vec<Value> = Vec::with_capacity(resp.matches.len() + 2);

    for m in resp.matches {
        let mut parts: Vec<Value> = Vec::with_capacity(5);

        // `concept` Coding always first — fixtures rely on this ordering.
        // The IG translate fixtures expect bare {system, code} Codings here,
        // so we only emit `display` when the backend resolved one. (For now
        // it never does — see comment on `TranslateRow.display`.)
        let mut concept_coding = serde_json::Map::new();
        concept_coding.insert("system".into(), json!(m.concept_system));
        concept_coding.insert("code".into(), json!(m.concept_code));
        if let Some(disp) = m.concept_display.as_deref() {
            if !disp.is_empty() {
                concept_coding.insert("display".into(), json!(disp));
            }
        }
        parts.push(json!({
            "name": "concept",
            "valueCoding": Value::Object(concept_coding),
        }));

        // R4 uses `equivalence`; R5/R6 renamed it to `relationship`. The
        // tx-ecosystem fixtures mark each as `$optional$ version:N`, but the
        // validator's TxTesterSorters alphabetises the part list before
        // comparison. When we emit BOTH names the actual array has 4 parts
        // sorted as [concept, equivalence, relationship, source] while the
        // version-filtered expected has 3 parts sorted as [concept,
        // relationship, source] (R5 case), and position-1 mismatches with
        // "Expected:'relationship' Actual:'equivalence'". Emit only the
        // version-appropriate name so both arrays sort identically.
        #[cfg(any(feature = "R5", feature = "R6"))]
        parts.push(json!({"name": "relationship", "valueCode": m.equivalence}));
        #[cfg(not(any(feature = "R5", feature = "R6")))]
        parts.push(json!({"name": "equivalence", "valueCode": m.equivalence}));

        // `originMap` — canonical ConceptMap reference, with `|version` if known.
        // Only emitted on forward translations: the IG `translate/translate-reverse`
        // fixture omits originMap on reverse responses because the caller already
        // knows which CM was queried (they invoked it explicitly).
        if !is_reverse {
            if let Some(src) = m.source.as_deref() {
                let canonical = match m.map_version.as_deref() {
                    Some(v) if !v.is_empty() => format!("{src}|{v}"),
                    _ => src.to_owned(),
                };
                parts.push(json!({"name": "originMap", "valueCanonical": canonical}));
            }
        }

        // For reverse responses include the source-side Coding of the
        // matched ConceptMap element as a `source` part — IG `translate-
        // reverse` fixture expects this so the caller can read the
        // resolved source code. Skip in forward mode: the caller already
        // knows the source code they sent.
        if is_reverse {
            if let (Some(sys), Some(code)) = (m.source_system.as_deref(), m.source_code.as_deref())
            {
                parts.push(json!({
                    "name": "source",
                    "valueCoding": {
                        "system": sys,
                        "code": code
                    }
                }));
            }
        }

        parameter.push(json!({"name": "match", "part": parts}));
    }

    parameter.push(json!({"name": "result", "valueBoolean": resp.result}));

    if let Some(msg) = resp.message {
        parameter.push(json!({"name": "message", "valueString": msg}));
    }

    Ok(json!({
        "resourceType": "Parameters",
        "parameter": parameter
    }))
}

/// POST /ConceptMap/$translate
pub async fn translate_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_translate(&state, params).await?,
        format,
    ))
}

/// GET /ConceptMap/$translate?code=...&system=...
pub async fn get_translate_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_translate(&state, params).await?,
        format,
    ))
}

// ── Instance-level: /ConceptMap/{id}/$translate ───────────────────────────────

/// Inject (or replace) the `url` parameter in a params list.
fn inject_url(mut params: Vec<Value>, url: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("url"));
    let mut with_url = vec![json!({"name": "url", "valueUri": url})];
    with_url.append(&mut params);
    with_url
}

/// POST /ConceptMap/{id}/$translate
///
/// Resolves the ConceptMap canonical URL from its FHIR `id`, then delegates to
/// the same translate logic used by the system-level endpoint.
pub async fn translate_by_id_post<B: TerminologyBackend>(
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
        .resource_url_by_id("ConceptMap", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ConceptMap/{id}")))?;

    let raw_params = body
        .and_then(|Json(v)| extract_parameter_array(&v).ok())
        .unwrap_or_default();
    Ok(fhir_respond(
        process_translate(&state, inject_url(raw_params, url)).await?,
        format,
    ))
}

/// GET /ConceptMap/{id}/$translate?code=...&system=...
pub async fn get_translate_by_id<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ConceptMap", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ConceptMap/{id}")))?;

    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_translate(&state, inject_url(params, url)).await?,
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
                 VALUES ('cs-src', 'http://example.org/src', '1.0', 'Source CS',
                         'active', 'complete', '2024-01-01', '2024-01-01'),
                        ('cs-tgt', 'http://example.org/tgt', '1.0', 'Target CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs-src', 'A', 'Alpha'),
                        (2, 'cs-src', 'B', 'Beta'),
                        (10, 'cs-tgt', 'X', 'X-Ray'),
                        (11, 'cs-tgt', 'Y', 'Yankee');

                 INSERT INTO concept_maps
                     (id, url, version, source_uri, target_uri, status, created_at)
                 VALUES ('cm1', 'http://example.org/cm', '1.0',
                         'http://example.org/src', 'http://example.org/tgt',
                         'active', '2024-01-01');

                 INSERT INTO concept_map_elements
                     (map_id, source_system, source_code, target_system, target_code, equivalence)
                 VALUES ('cm1', 'http://example.org/src', 'A',
                         'http://example.org/tgt', 'X', 'equivalent'),
                        ('cm1', 'http://example.org/src', 'B',
                         'http://example.org/tgt', 'Y', 'wider');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ConceptMap/$translate",
                post(translate_handler::<SqliteTerminologyBackend>),
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

    // ── Happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn translate_returns_parameters_resource() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "Parameters");
    }

    #[tokio::test]
    async fn translate_result_true_on_match() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);
    }

    #[tokio::test]
    async fn translate_match_contains_concept_coding() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();

        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
        assert_eq!(concept["valueCoding"]["system"], "http://example.org/tgt");
        // The IG translate fixtures expect bare {system, code} Codings —
        // display is intentionally omitted from the output.
        assert!(concept["valueCoding"].get("display").is_none());
    }

    #[tokio::test]
    async fn translate_match_contains_equivalence() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();

        // The build emits `equivalence` for R4/R4B and `relationship` for R5/R6;
        // either name carries the same valueCode.
        let key = if cfg!(any(feature = "R5", feature = "R6")) {
            "relationship"
        } else {
            "equivalence"
        };
        let equiv = parts.iter().find(|p| p["name"] == key).unwrap();
        assert_eq!(equiv["valueCode"], "equivalent");
    }

    #[tokio::test]
    async fn translate_result_false_on_no_match() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "UNKNOWN"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn translate_reverse_finds_source_code() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "X"},
                {"name": "system", "valueUri": "http://example.org/tgt"},
                {"name": "reverse", "valueBoolean": true}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);

        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();
        // Reverse output: `concept` carries the supplied target Coding (X in tgt CS);
        // `source` carries the resolved source Coding (A in src CS).
        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
        assert_eq!(concept["valueCoding"]["system"], "http://example.org/tgt");
        let source = parts.iter().find(|p| p["name"] == "source").unwrap();
        assert_eq!(source["valueCoding"]["code"], "A");
        assert_eq!(source["valueCoding"]["system"], "http://example.org/src");
    }

    // ── Coding / CodeableConcept source spellings (#287) ───────────────────────

    /// The spec allows the source concept as a `coding`, but the handler only read the
    /// scalar `code` + `system` pair, so this shape 400'd as "Missing required
    /// parameter". Regression test for #287.
    #[tokio::test]
    async fn translate_accepts_coding_param() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "coding", "valueCoding": {
                    "system": "http://example.org/src",
                    "code": "A"
                }},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let concept = m["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "concept")
            .unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
    }

    /// R5 spelling of the same input.
    #[tokio::test]
    async fn translate_accepts_source_coding_param() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceCoding", "valueCoding": {
                    "system": "http://example.org/src",
                    "code": "A"
                }},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let result = json["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "result")
            .unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    /// A `codeableConcept` translates on its first coding.
    #[tokio::test]
    async fn translate_accepts_codeable_concept_param() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "codeableConcept", "valueCodeableConcept": {
                    "coding": [{"system": "http://example.org/src", "code": "A"}]
                }},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let result = json["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "result")
            .unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    /// `targetCoding` drives reverse mode the same way a bare `targetCode` does.
    #[tokio::test]
    async fn translate_accepts_target_coding_param_for_reverse() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "targetCoding", "valueCoding": {
                    "system": "http://example.org/tgt",
                    "code": "X"
                }}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let source = m["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "source")
            .unwrap();
        assert_eq!(source["valueCoding"]["code"], "A");
    }

    /// An explicit `code` outranks a `coding` sent alongside it, so a request mixing the
    /// spellings translates what the caller named rather than silently preferring one.
    #[tokio::test]
    async fn translate_scalar_code_wins_over_coding() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code",   "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"},
                {"name": "coding", "valueCoding": {
                    "system": "http://example.org/src",
                    "code": "no-such-code"
                }},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let concept = m["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "concept")
            .unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
    }

    // ── Error cases ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn translate_missing_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn translate_wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({"resourceType": "ConceptMap", "parameter": []});

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── R5 parameter names + no-URL translation (tx-ecosystem IG) ──────────────

    /// `sourceCode` + `sourceSystem` + `targetSystem` (no `url`) — R5 names.
    /// Mirrors the IG `translate/translate-1` fixture shape.
    #[tokio::test]
    async fn translate_r5_param_names_without_url_finds_match() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "sourceCode",   "valueCode": "A"},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = m["part"].as_array().unwrap();
        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
        assert_eq!(concept["valueCoding"]["system"], "http://example.org/tgt");
    }

    /// Reverse mode driven by `targetCode` + `sourceSystem` (no `reverse=true`).
    /// Mirrors `translate/translate-reverse`.
    #[tokio::test]
    async fn translate_reverse_via_target_code_emits_source_coding() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "targetCode",   "valueCode": "X"},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = m["part"].as_array().unwrap();

        // Reverse output: `concept` carries the *target* side of the
        // matched element (i.e. the supplied targetCode), and `source`
        // carries the *source* side (the resolved code). This matches
        // the IG `translate/translate-reverse` fixture exactly.
        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
        assert_eq!(concept["valueCoding"]["system"], "http://example.org/tgt");

        let source = parts.iter().find(|p| p["name"] == "source").unwrap();
        assert_eq!(source["valueCoding"]["code"], "A");
        assert_eq!(source["valueCoding"]["system"], "http://example.org/src");
    }

    /// IG `translate/translate-reverse` fixture pins the part ordering. The
    /// validator's TxTesterSorters alphabetises before comparison, so we just
    /// need the right SET of parts (one of equivalence/relationship per the
    /// build's FHIR version). originMap is suppressed in reverse mode.
    #[tokio::test]
    async fn translate_reverse_part_ordering_matches_ig_fixture() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "targetCode",   "valueCode": "X"},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });
        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = m["part"].as_array().unwrap();

        let names: Vec<&str> = parts.iter().filter_map(|p| p["name"].as_str()).collect();
        let equiv_or_rel = if cfg!(any(feature = "R5", feature = "R6")) {
            "relationship"
        } else {
            "equivalence"
        };
        assert_eq!(
            names,
            vec!["concept", equiv_or_rel, "source"],
            "reverse-mode parts must be concept/<equivalence|relationship>/source"
        );
    }

    /// `originMap` is emitted as `url|version` when the ConceptMap has a version.
    #[tokio::test]
    async fn translate_emits_origin_map_canonical_with_version() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "sourceCode",   "valueCode": "A"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        let m = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = m["part"].as_array().unwrap();
        let origin = parts.iter().find(|p| p["name"] == "originMap").unwrap();
        assert_eq!(origin["valueCanonical"], "http://example.org/cm|1.0");
    }

    /// Forward translation emits the version-appropriate name only —
    /// `equivalence` in R4/R4B, `relationship` in R5/R6 — so the validator's
    /// TxTesterSorters-alphabetised actual matches the version-filtered
    /// expected at every position.
    #[tokio::test]
    async fn translate_emits_version_appropriate_relationship_name() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "sourceCode",   "valueCode": "A"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;
        let parts = json["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "match")
            .unwrap()["part"]
            .as_array()
            .unwrap()
            .clone();

        if cfg!(any(feature = "R5", feature = "R6")) {
            let rel = parts.iter().find(|p| p["name"] == "relationship").unwrap();
            assert_eq!(rel["valueCode"], "equivalent");
            assert!(parts.iter().all(|p| p["name"] != "equivalence"));
        } else {
            let equiv = parts.iter().find(|p| p["name"] == "equivalence").unwrap();
            assert_eq!(equiv["valueCode"], "equivalent");
            assert!(parts.iter().all(|p| p["name"] != "relationship"));
        }
    }

    /// Forward responses do *not* include a `source` Coding — the caller
    /// already knows the source code they sent.
    #[tokio::test]
    async fn translate_forward_omits_source_coding_part() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "sourceCode",   "valueCode": "A"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;
        let parts = json["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "match")
            .unwrap()["part"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            parts.iter().all(|p| p["name"] != "source"),
            "forward response must not include `source` Coding part"
        );
    }

    /// Neither `code` nor `targetCode` → 400.
    #[tokio::test]
    async fn translate_missing_both_code_and_target_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "sourceSystem", "valueUri": "http://example.org/src"},
                {"name": "targetSystem", "valueUri": "http://example.org/tgt"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 400);
    }
}
