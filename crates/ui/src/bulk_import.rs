//! Bulk Import workspace (`/ui/bulk-import`) — HFS as a Bulk Data Submit
//! *provider* (#527).
//!
//! The user assembles named **submissions**: a destination (the Data
//! Recipient's FHIR base URL), an authentication choice, and one or more
//! **manifests** — each a Bulk Export Manifest URL plus the per-manifest
//! kick-off options. Submitting a manifest POSTs a `$bulk-submit` `Parameters`
//! resource to the recipient (the exact vocabulary HFS's own consumer side
//! parses), and every attempt is appended to the submission's log.
//!
//! State lives in the per-user settings document under the reserved
//! `byTenant.<tenant>.bulkImport` subtree, object-keyed by id so single-entry
//! merge patches never clobber siblings. The log is a bounded array — the
//! settings document has a hard size cap.

use askama::Template;
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, current_status, render, settings_user_key};

/// Log entries kept per submission. The whole settings document is capped at
/// 256 KiB server-side, so the log must stay bounded.
const LOG_CAP: usize = 100;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub recipient_base_url: String,
    /// `none` or `backend-services`.
    #[serde(default)]
    pub auth: String,
    /// The submitter Identifier, coordinated out-of-band with the recipient.
    /// Empty falls back to `urn:helios:hfs:bulk-submit` / the submission id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub submitter_system: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub submitter_value: String,
    /// Recipient status-poll URL (the `$bulk-submit-status` kick-off's
    /// Content-Location) plus the latest poll observations.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poll_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub progress: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_url: String,
    /// `not-started` | `in-progress` | `stopped` | `completed`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub manifests: serde_json::Map<String, Value>,
    #[serde(default)]
    pub log: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub manifest_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fhir_base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_format: String,
    /// `Name: value` lines, one header per line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_request_headers: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_submitted_at: String,
}

/// The `byTenant.<tenant>.bulkImport.submissions` object for this user.
async fn load_submissions(
    state: &WebState,
    user_key: &str,
    tenant: &str,
) -> serde_json::Map<String, Value> {
    let Some(store) = &state.settings else {
        return serde_json::Map::new();
    };
    store
        .get_settings(user_key)
        .await
        .ok()
        .flatten()
        .and_then(|s| {
            s.document
                .get("byTenant")?
                .get(tenant)?
                .get("bulkImport")?
                .get("submissions")?
                .as_object()
                .cloned()
        })
        .unwrap_or_default()
}

/// Merge-patches one submission entry (or removes it when `value` is `Null`).
async fn patch_submission(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    value: Value,
) -> Result<(), String> {
    let Some(store) = &state.settings else {
        return Err("settings store unavailable".to_string());
    };
    let patch = json!({
        "byTenant": { tenant: { "bulkImport": { "submissions": { id: value } } } }
    });
    store
        .patch_settings(user_key, patch, None)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Replaces one submission wholesale. RFC 7386 merges objects key-by-key, so a
/// plain patch cannot *remove* a manifest or trim the log; writing the full
/// serialized submission under its id does, without touching siblings.
async fn store_submission(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    submission: &Submission,
) -> Result<(), String> {
    // Null out the entry first so nested leftovers (deleted manifests, dropped
    // log lines) don't survive the merge.
    patch_submission(state, user_key, tenant, id, Value::Null).await?;
    let value = serde_json::to_value(submission).map_err(|e| e.to_string())?;
    patch_submission(state, user_key, tenant, id, value).await
}

fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn push_log(submission: &mut Submission, message: String) {
    submission
        .log
        .push(json!({ "at": now_stamp(), "message": message }));
    if submission.log.len() > LOG_CAP {
        let drop = submission.log.len() - LOG_CAP;
        submission.log.drain(..drop);
    }
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

struct SubmissionRow {
    id: String,
    name: String,
    status_label: String,
    created_date: String,
    manifest_count: usize,
    destination: String,
}

struct ManifestRow {
    id: String,
    manifest_url: String,
    last_submitted_at: String,
}

struct LogLine {
    at: String,
    message: String,
}

fn status_label(i18n: &I18n, status: &str) -> String {
    match status {
        "in-progress" => i18n.t("bulk-import-status-in-progress"),
        "stopped" => i18n.t("bulk-import-status-stopped"),
        "completed" => i18n.t("bulk-import-status-completed"),
        _ => i18n.t("bulk-import-status-not-started"),
    }
}

#[derive(Template)]
#[template(path = "pages/bulk-import.html")]
struct BulkImportPage {
    status: crate::Status,
    i18n: I18n,
    active_page: &'static str,
    available: bool,
    rows: Vec<SubmissionRow>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/bulk-import-detail.html")]
struct BulkImportDetailPage {
    status: crate::Status,
    i18n: I18n,
    active_page: &'static str,
    id: String,
    name: String,
    recipient: String,
    submitter_display: String,
    created_at: String,
    status_label: String,
    auth: String,
    manifests: Vec<ManifestRow>,
    log: Vec<LogLine>,
    error: Option<String>,
}

/// Inline fragment returned by the test-authentication button.
#[derive(Template)]
#[template(path = "partials/bulk_import_test_auth.html")]
struct TestAuthResult {
    ok: bool,
    message: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn parse_submission(value: &Value) -> Submission {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// `GET /ui/bulk-import` — the submissions list.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(state.version, rv.0, &rt);
    let available = state.settings.is_some();
    let user_key = settings_user_key(principal.as_deref());

    let mut rows = Vec::new();
    if available {
        let map = load_submissions(&state, &user_key, &rt.id).await;
        for (id, value) in &map {
            let s = parse_submission(value);
            rows.push(SubmissionRow {
                id: id.clone(),
                status_label: status_label(&i18n, &s.status),
                created_date: s.created_at.clone(),
                manifest_count: s.manifests.len(),
                destination: s.recipient_base_url,
                name: s.name,
            });
        }
        // Most recent first, matching the design's default sort.
        rows.sort_by(|a, b| b.created_date.cmp(&a.created_date));
    }

    render(BulkImportPage {
        status,
        i18n,
        active_page: "bulk-import",
        available,
        rows,
        error: None,
    })
}

#[derive(Deserialize)]
pub struct CreateForm {
    pub name: String,
    #[serde(default)]
    pub recipient_base_url: String,
    #[serde(default)]
    pub auth: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub token_url: String,
    #[serde(default)]
    pub submitter_system: String,
    #[serde(default)]
    pub submitter_value: String,
    #[serde(default)]
    pub submission_id: String,
}

/// `POST /ui/bulk-import` — create a submission, then land on its detail page.
pub async fn create(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    axum::Form(form): axum::Form<CreateForm>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    // The user may pin the submission id (it must be unique per submitter,
    // coordinated with the recipient); empty generates one.
    let id = match form.submission_id.trim() {
        "" => uuid::Uuid::new_v4().to_string(),
        pinned => pinned.to_string(),
    };
    let submission = Submission {
        name: form.name.trim().to_string(),
        recipient_base_url: form
            .recipient_base_url
            .trim()
            .trim_end_matches('/')
            .to_string(),
        auth: if form.auth == "backend-services" {
            form.auth
        } else {
            "none".to_string()
        },
        submitter_system: form.submitter_system.trim().to_string(),
        submitter_value: form.submitter_value.trim().to_string(),
        poll_url: String::new(),
        progress: String::new(),
        result: Value::Null,
        client_id: form.client_id.trim().to_string(),
        token_url: form.token_url.trim().to_string(),
        status: "not-started".to_string(),
        created_at: now_stamp(),
        manifests: serde_json::Map::new(),
        log: Vec::new(),
    };
    match store_submission(&state, &user_key, &rt.id, &id, &submission).await {
        Ok(()) => Redirect::to(&format!("/ui/bulk-import/{id}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// Loads one submission or answers with the list page redirect.
async fn load_one(state: &WebState, user_key: &str, tenant: &str, id: &str) -> Option<Submission> {
    load_submissions(state, user_key, tenant)
        .await
        .get(id)
        .map(parse_submission)
}

/// `GET /ui/bulk-import/{id}` — the submission detail.
pub async fn detail(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(state.version, rv.0, &rt);
    let user_key = settings_user_key(principal.as_deref());

    let Some(s) = load_one(&state, &user_key, &rt.id, &id).await else {
        return Redirect::to("/ui/bulk-import").into_response();
    };

    let mut manifests: Vec<ManifestRow> = s
        .manifests
        .iter()
        .map(|(mid, value)| {
            let m: Manifest = serde_json::from_value(value.clone()).unwrap_or_default();
            ManifestRow {
                id: mid.clone(),
                manifest_url: m.manifest_url,
                last_submitted_at: m.last_submitted_at,
            }
        })
        .collect();
    manifests.sort_by(|a, b| a.id.cmp(&b.id));

    let log: Vec<LogLine> = s
        .log
        .iter()
        .rev()
        .map(|entry| LogLine {
            at: entry
                .get("at")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            message: entry
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
        .collect();

    let label = status_label(&i18n, &s.status);
    render(BulkImportDetailPage {
        status,
        i18n,
        active_page: "bulk-import",
        submitter_display: {
            let system = if s.submitter_system.is_empty() {
                "urn:helios:hfs:bulk-submit"
            } else {
                &s.submitter_system
            };
            let value = if s.submitter_value.is_empty() {
                &id
            } else {
                &s.submitter_value
            };
            format!("{system} | {value}")
        },
        id,
        name: s.name,
        recipient: s.recipient_base_url,
        created_at: s.created_at,
        status_label: label,
        auth: s.auth,
        manifests,
        log,
        error: None,
    })
}

/// `POST /ui/bulk-import/{id}/delete`.
pub async fn delete(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let _ = patch_submission(&state, &user_key, &rt.id, &id, Value::Null).await;
    Redirect::to("/ui/bulk-import").into_response()
}

#[derive(Deserialize)]
pub struct ManifestForm {
    pub manifest_url: String,
    #[serde(default)]
    pub fhir_base_url: String,
    #[serde(default)]
    pub output_format: String,
    #[serde(default)]
    pub file_request_headers: String,
}

/// `POST /ui/bulk-import/{id}/manifests` — add a manifest.
pub async fn add_manifest(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
    axum::Form(form): axum::Form<ManifestForm>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await else {
        return Redirect::to("/ui/bulk-import").into_response();
    };
    let manifest = Manifest {
        manifest_url: form.manifest_url.trim().to_string(),
        fhir_base_url: form.fhir_base_url.trim().to_string(),
        output_format: form.output_format.trim().to_string(),
        file_request_headers: form.file_request_headers.trim().to_string(),
        last_submitted_at: String::new(),
    };
    let mid = uuid::Uuid::new_v4().to_string();
    s.manifests
        .insert(mid, serde_json::to_value(&manifest).unwrap_or(Value::Null));
    let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

/// `POST /ui/bulk-import/{id}/manifests/{mid}/delete`.
pub async fn delete_manifest(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path((id, mid)): Path<(String, String)>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        s.manifests.remove(&mid);
        let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

// ---------------------------------------------------------------------------
// Submitting
// ---------------------------------------------------------------------------

/// Builds the `$bulk-submit` kick-off `Parameters`, mirroring the vocabulary
/// HFS's own consumer parses (`helios-rest` `parse_submit_request`).
fn kickoff_parameters(
    submission: &Submission,
    id: &str,
    status: &str,
    manifest: Option<&Manifest>,
    replaces: Option<&str>,
) -> Value {
    let system = if submission.submitter_system.is_empty() {
        "urn:helios:hfs:bulk-submit"
    } else {
        &submission.submitter_system
    };
    let value = if submission.submitter_value.is_empty() {
        id
    } else {
        &submission.submitter_value
    };
    let mut parameter = vec![
        json!({ "name": "submitter", "valueIdentifier": { "system": system, "value": value } }),
        json!({ "name": "submissionId", "valueString": id }),
        json!({ "name": "submissionStatus", "valueCoding": {
            "system": "http://hl7.org/fhir/event-status", "code": status } }),
    ];
    if let Some(m) = manifest {
        // valueUrl per the Bulk Data IG (STU4 ballot: both parameters are
        // type `url`). The SMART reference recipient reads valueString
        // instead -- an off-spec leniency gap on its side, not ours.
        parameter.push(json!({ "name": "manifestUrl", "valueUrl": m.manifest_url }));
        // fhirBaseUrl is required alongside manifestUrl; an empty field falls
        // back to the manifest URL's origin, matching the reference provider
        // (`new URL(manifestUrl).origin`).
        let base = if m.fhir_base_url.is_empty() {
            url_origin(&m.manifest_url)
        } else {
            m.fhir_base_url.clone()
        };
        parameter.push(json!({ "name": "fhirBaseUrl", "valueUrl": base }));
        if !m.output_format.is_empty() {
            parameter.push(json!({ "name": "outputFormat", "valueString": m.output_format }));
        }
        if let Some(old) = replaces {
            parameter.push(json!({ "name": "replacesManifestUrl", "valueUrl": old }));
        }
        for line in m.file_request_headers.lines() {
            if let Some((name, value)) = line.split_once(':') {
                parameter.push(json!({ "name": "fileRequestHeader", "part": [
                    { "name": "headerName", "valueString": name.trim() },
                    { "name": "headerValue", "valueString": value.trim() },
                ]}));
            }
        }
    }
    json!({ "resourceType": "Parameters", "parameter": parameter })
}

/// `scheme://authority` of a URL, dropping the path — the reference provider's
/// fallback for an empty FHIR base.
fn url_origin(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    match url[scheme_end + 3..].find('/') {
        Some(path_start) => url[..scheme_end + 3 + path_start].to_string(),
        None => url.to_string(),
    }
}

/// Computes the RFC 7638 thumbprint of a private key as the `kid`.
/// Supports ES384 (P-384) and RS384. Returns `None` when the PEM cannot be parsed.
fn signing_kid(pem: &str, alg: &str) -> Option<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    match alg {
        // RS384 thumbprints are deliberately not derived: the only pure-Rust
        // RSA implementation carries RUSTSEC-2023-0071 (Marvin Attack) with
        // no fixed release — the same reason jwe.rs rejects RSA-OAEP. An
        // RS384 assertion goes out without a kid; the key is registered with
        // the recipient out-of-band.
        "RS384" => None,
        _ => {
            use p384::elliptic_curve::sec1::ToEncodedPoint;
            use p384::pkcs8::DecodePrivateKey;

            let secret = p384::SecretKey::from_pkcs8_pem(pem)
                .or_else(|_| p384::SecretKey::from_sec1_pem(pem))
                .ok()?;
            let point = secret.public_key().to_encoded_point(false);
            let x = URL_SAFE_NO_PAD.encode(point.x()?);
            let y = URL_SAFE_NO_PAD.encode(point.y()?);
            let canonical = format!(r#"{{"crv":"P-384","kty":"EC","x":"{x}","y":"{y}"}}"#);
            Some(URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes())))
        }
    }
}

/// Mints a SMART Backend Services access token (`client_credentials` +
/// `private_key_jwt`) against the submission's token endpoint. The signing key
/// is the server-wide `HFS_BULK_SUBMIT_PRIVATE_KEY`, shared with the consumer
/// side's protected-file fetches; the client id is per-submission.
async fn backend_services_token(client_id: &str, token_url: &str) -> Result<String, String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    if client_id.is_empty() || token_url.is_empty() {
        return Err("missing client_id or token URL".to_string());
    }
    let pem = std::env::var("HFS_BULK_SUBMIT_PRIVATE_KEY")
        .map_err(|_| "HFS_BULK_SUBMIT_PRIVATE_KEY is not configured on this server".to_string())?;
    let alg = std::env::var("HFS_BULK_SUBMIT_SIGNING_ALG").unwrap_or_else(|_| "ES384".to_string());
    let (algorithm, key) = match alg.as_str() {
        "RS384" => (
            Algorithm::RS384,
            EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| e.to_string())?,
        ),
        _ => (
            Algorithm::ES384,
            EncodingKey::from_ec_pem(pem.as_bytes()).map_err(|e| e.to_string())?,
        ),
    };
    let claims = json!({
        "iss": client_id,
        "sub": client_id,
        "aud": token_url,
        "exp": Utc::now().timestamp() + 300,
        "jti": uuid::Uuid::new_v4().to_string(),
    });
    let mut header = Header::new(algorithm);
    header.kid = signing_kid(&pem, &alg);
    let assertion = encode(&header, &claims, &key).map_err(|e| e.to_string())?;

    let response = reqwest::Client::new()
        .post(token_url)
        .form(&[
            ("grant_type", "client_credentials"),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            ),
            ("client_assertion", &assertion),
            // The scope the recipient's $bulk-submit endpoints require —
            // HFS's own consumer and the SMART reference both gate on it.
            ("scope", "system/bulk-submit"),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token endpoint answered {status}"));
    }
    body.get("access_token")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "token response carried no access_token".to_string())
}

/// POSTs a kick-off to the recipient, returning `(status, body-excerpt)`.
async fn post_kickoff(
    submission: &Submission,
    parameters: &Value,
) -> Result<(u16, String), String> {
    let target = format!(
        "{}/$bulk-submit",
        submission.recipient_base_url.trim_end_matches('/')
    );
    let mut request = reqwest::Client::new()
        .post(&target)
        .header("Content-Type", "application/fhir+json")
        .header("Accept", "application/fhir+json")
        .timeout(std::time::Duration::from_secs(15))
        .json(parameters);
    if submission.auth == "backend-services" {
        let token = backend_services_token(&submission.client_id, &submission.token_url).await?;
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let mut body = response.text().await.unwrap_or_default();
    body.truncate(300);
    Ok((status, body))
}

/// Kicks off recipient-side status tracking: `POST $bulk-submit-status`
/// (submitter + submissionId, `Prefer: respond-async`), returning the poll
/// URL the recipient hands back in `Content-Location`.
async fn status_kickoff(submission: &Submission, id: &str) -> Result<String, String> {
    let target = format!(
        "{}/$bulk-submit-status",
        submission.recipient_base_url.trim_end_matches('/')
    );
    // Only the identifying parameters ride the status kick-off.
    let parameters = kickoff_parameters(submission, id, "", None, None);
    let identifying: Vec<Value> = parameters["parameter"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["name"] == "submitter" || p["name"] == "submissionId")
        .cloned()
        .collect();
    let body = json!({ "resourceType": "Parameters", "parameter": identifying });

    let mut request = reqwest::Client::new()
        .post(&target)
        .header("Content-Type", "application/fhir+json")
        .header("Prefer", "respond-async")
        .timeout(std::time::Duration::from_secs(15))
        .json(&body);
    if submission.auth == "backend-services" {
        let token = backend_services_token(&submission.client_id, &submission.token_url).await?;
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    response
        .headers()
        .get("content-location")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .ok_or_else(|| format!("status kick-off answered {status} without Content-Location"))
}

/// One poll of the recipient's status URL: `202` records `X-Progress`, `200`
/// records the status manifest as the submission's result, anything else is
/// logged and polling stops (the poll URL is cleared).
async fn poll_status(submission: &mut Submission) {
    let poll_url = submission.poll_url.clone();
    let response = match reqwest::Client::new()
        .get(&poll_url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            push_log(submission, format!("Status poll failed: {e}"));
            return;
        }
    };
    match response.status().as_u16() {
        202 => {
            let progress = response
                .headers()
                .get("x-progress")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("in progress")
                .to_string();
            if submission.progress != progress {
                push_log(submission, format!("Status: {progress}"));
            }
            submission.progress = progress;
        }
        200 => {
            let manifest: Value = response.json().await.unwrap_or(Value::Null);
            let outputs = manifest["output"].as_array().map(Vec::len).unwrap_or(0);
            let errors = manifest["error"].as_array().map(Vec::len).unwrap_or(0);
            submission.result = json!({
                "completedAt": now_stamp(),
                "outputs": outputs,
                "errors": errors,
            });
            submission.progress = String::new();
            submission.poll_url = String::new();
            push_log(
                submission,
                format!(
                    "Status: got 200 OK — processing finished ({outputs} outputs, {errors} error files)."
                ),
            );
        }
        429 => {
            push_log(
                submission,
                "Status poll throttled (429); backing off.".to_string(),
            );
        }
        other => {
            push_log(
                submission,
                format!("Status poll answered {other}; polling stopped."),
            );
            submission.poll_url = String::new();
        }
    }
}

/// `POST /ui/bulk-import/{id}/manifests/{mid}/submit`.
pub async fn submit_manifest(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path((id, mid)): Path<(String, String)>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        submit_with_id(&mut s, &id, Some(&mid)).await;
        let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

/// `POST /ui/bulk-import/{id}/submit-all`.
pub async fn submit_all(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        submit_with_id(&mut s, &id, None).await;
        let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

async fn submit_with_id(submission: &mut Submission, id: &str, only: Option<&str>) {
    let mids: Vec<String> = submission.manifests.keys().cloned().collect();
    for mid in mids {
        if only.is_some_and(|o| o != mid) {
            continue;
        }
        submit_one_with_id(submission, id, &mid).await;
    }
}

async fn submit_one_with_id(submission: &mut Submission, id: &str, mid: &str) {
    let Some(m) = submission
        .manifests
        .get(mid)
        .and_then(|v| serde_json::from_value::<Manifest>(v.clone()).ok())
    else {
        return;
    };
    push_log(
        submission,
        format!("Submitting manifest \"{}\"...", m.manifest_url),
    );
    let parameters = kickoff_parameters(submission, id, "in-progress", Some(&m), None);
    match post_kickoff(submission, &parameters).await {
        Ok((status, _)) if (200..300).contains(&status) => {
            push_log(
                submission,
                format!("Manifest accepted by the recipient ({status})."),
            );
            submission.status = "in-progress".to_string();
            if let Some(entry) = submission.manifests.get_mut(mid) {
                entry["lastSubmittedAt"] = json!(now_stamp());
            }
            // Start recipient-side status tracking on the first accepted
            // manifest; later submissions reuse the same poll URL.
            if submission.poll_url.is_empty() {
                match status_kickoff(submission, id).await {
                    Ok(poll_url) => {
                        push_log(submission, "Bulk status kick-off request".to_string());
                        submission.poll_url = poll_url;
                        submission.result = Value::Null;
                    }
                    Err(e) => {
                        push_log(submission, format!("Status kick-off failed: {e}"));
                    }
                }
            }
        }
        Ok((status, body)) => {
            push_log(submission, "Bulk Submit request failed!".to_string());
            push_log(
                submission,
                format!(
                    "Failed to submit manifest {}: {status} {}",
                    m.manifest_url,
                    body.replace('\n', " ")
                ),
            );
        }
        Err(e) => {
            push_log(submission, "Bulk Submit request failed!".to_string());
            push_log(
                submission,
                format!("Failed to submit manifest {}: {e}", m.manifest_url),
            );
        }
    }
}

/// `POST /ui/bulk-import/{id}/abort` — status-only kick-off, `stopped`.
pub async fn abort(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    set_status(state, rt, principal, id, "stopped").await
}

/// `POST /ui/bulk-import/{id}/complete` — status-only kick-off, `completed`.
pub async fn complete(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    set_status(state, rt, principal, id, "completed").await
}

async fn set_status(
    state: WebState,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    id: String,
    status: &str,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        push_log(&mut s, format!("Marking submission {status}..."));
        let parameters = kickoff_parameters(&s, &id, status, None, None);
        match post_kickoff(&s, &parameters).await {
            Ok((code, _)) if (200..300).contains(&code) => {
                push_log(&mut s, format!("Recipient acknowledged ({code})."));
                s.status = status.to_string();
            }
            Ok((code, body)) => {
                push_log(
                    &mut s,
                    format!("Recipient rejected the status change: {code} {body}"),
                );
            }
            Err(e) => {
                push_log(&mut s, format!("Status change failed: {e}"));
            }
        }
        let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

/// The recipient-status card fragment, polled by htmx while a poll URL is
/// live. Each fetch performs at most one poll against the recipient, so the
/// cadence is the page's `every 5s` trigger — no background tasks.
#[derive(Template)]
#[template(path = "partials/bulk_import_status.html")]
struct StatusCard {
    i18n: I18n,
    id: String,
    polling: bool,
    progress: String,
    outputs: u64,
    errors: u64,
    completed_at: String,
}

/// `GET /ui/bulk-import/{id}/status` — one poll, then the refreshed card.
pub async fn status_fragment(
    State(state): State<WebState>,
    locale: RequestLocale,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let user_key = settings_user_key(principal.as_deref());
    let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !s.poll_url.is_empty() {
        poll_status(&mut s).await;
        let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
    }
    render(StatusCard {
        i18n,
        id,
        polling: !s.poll_url.is_empty(),
        progress: s.progress.clone(),
        outputs: s.result["outputs"].as_u64().unwrap_or(0),
        errors: s.result["errors"].as_u64().unwrap_or(0),
        completed_at: s.result["completedAt"].as_str().unwrap_or("").to_string(),
    })
}

/// `GET /ui/bulk-import/keys` — redirects to the canonical JWKS endpoint.
///
/// The authoritative key set is now served at
/// `/.well-known/bulk-submit-jwks.json` by the REST layer, which derives the
/// JWK directly from `HFS_BULK_SUBMIT_PRIVATE_KEY` (#529). This redirect keeps
/// any existing bookmarks working.
pub async fn keys() -> Response {
    axum::response::Redirect::permanent("/.well-known/bulk-submit-jwks.json").into_response()
}

/// `GET /ui/bulk-import/empty-manifest.json` — an empty Bulk Export Manifest.
/// Aborting a single manifest is spec'd as replacing it with an empty one;
/// this is the empty one, hosted where the recipient can fetch it.
pub async fn empty_manifest() -> Response {
    axum::Json(json!({
        "transactionTime": now_stamp(),
        "request": "",
        "requiresAccessToken": false,
        "output": [],
        "error": []
    }))
    .into_response()
}

/// `POST /ui/bulk-import/{id}/manifests/{mid}/replace` — submit a new
/// manifest carrying `replacesManifestUrl` = the old one, then store the
/// replacement under the same id.
pub async fn replace_manifest(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path((id, mid)): Path<(String, String)>,
    axum::Form(form): axum::Form<ManifestForm>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        let old_url = s
            .manifests
            .get(&mid)
            .and_then(|m| m["manifestUrl"].as_str())
            .unwrap_or_default()
            .to_string();
        if !old_url.is_empty() {
            let replacement = Manifest {
                manifest_url: form.manifest_url.trim().to_string(),
                fhir_base_url: form.fhir_base_url.trim().to_string(),
                output_format: form.output_format.trim().to_string(),
                file_request_headers: form.file_request_headers.trim().to_string(),
                last_submitted_at: String::new(),
            };
            push_log(
                &mut s,
                format!(
                    "Replacing manifest \"{old_url}\" with \"{}\"...",
                    replacement.manifest_url
                ),
            );
            let parameters =
                kickoff_parameters(&s, &id, "in-progress", Some(&replacement), Some(&old_url));
            match post_kickoff(&s, &parameters).await {
                Ok((code, _)) if (200..300).contains(&code) => {
                    push_log(&mut s, format!("Replacement accepted ({code})."));
                    let mut entry = serde_json::to_value(&replacement).unwrap_or(Value::Null);
                    entry["lastSubmittedAt"] = json!(now_stamp());
                    s.manifests.insert(mid, entry);
                }
                Ok((code, body)) => {
                    push_log(
                        &mut s,
                        format!("Replacement rejected: {code} {}", body.replace('\n', " ")),
                    );
                }
                Err(e) => {
                    push_log(&mut s, format!("Replacement failed: {e}"));
                }
            }
            let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
        }
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

/// `POST /ui/bulk-import/{id}/manifests/{mid}/abort` — abort one manifest by
/// replacing it with the empty manifest this server hosts. The empty
/// manifest's URL is derived from the request's Host header, since that is
/// the address the recipient reached us... the address the *browser* reached
/// us on, which is the best externally-visible base the UI can know.
pub async fn abort_manifest(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: axum::http::HeaderMap,
    Path((id, mid)): Path<(String, String)>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    if let Some(mut s) = load_one(&state, &user_key, &rt.id, &id).await {
        let old_url = s
            .manifests
            .get(&mid)
            .and_then(|m| m["manifestUrl"].as_str())
            .unwrap_or_default()
            .to_string();
        if !old_url.is_empty() {
            let host = headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("localhost:8080");
            let empty = Manifest {
                manifest_url: format!("http://{host}/ui/bulk-import/empty-manifest.json"),
                fhir_base_url: format!("http://{host}"),
                ..Default::default()
            };
            push_log(&mut s, format!("Aborting manifest \"{old_url}\"..."));
            let parameters =
                kickoff_parameters(&s, &id, "in-progress", Some(&empty), Some(&old_url));
            match post_kickoff(&s, &parameters).await {
                Ok((code, _)) if (200..300).contains(&code) => {
                    push_log(&mut s, format!("Abort accepted ({code})."));
                    if let Some(entry) = s.manifests.get_mut(&mid) {
                        entry["abortedAt"] = json!(now_stamp());
                    }
                }
                Ok((code, body)) => {
                    push_log(
                        &mut s,
                        format!("Abort rejected: {code} {}", body.replace('\n', " ")),
                    );
                }
                Err(e) => {
                    push_log(&mut s, format!("Abort failed: {e}"));
                }
            }
            let _ = store_submission(&state, &user_key, &rt.id, &id, &s).await;
        }
    }
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

#[derive(Deserialize)]
pub struct TestAuthForm {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub token_url: String,
}

/// `POST /ui/bulk-import/test-auth` — htmx fragment with the outcome of a
/// backend-services token mint against the given endpoint.
pub async fn test_auth(
    locale: RequestLocale,
    axum::Form(form): axum::Form<TestAuthForm>,
) -> Response {
    let i18n = I18n::new(locale);
    let (ok, message) = match backend_services_token(&form.client_id, &form.token_url).await {
        Ok(_) => (true, i18n.t("bulk-import-test-auth-ok")),
        Err(e) => (false, e),
    };
    render(TestAuthResult { ok, message })
}
