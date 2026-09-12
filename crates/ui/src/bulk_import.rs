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
//! State lives in the tenant-scoped provider store
//! ([`BulkProviderStore`](helios_persistence::core::BulkProviderStore), #772):
//! one whole JSON document per submission, written under optimistic
//! versioning and visible to every operator of the tenant. The log is a
//! bounded array so a chatty run cannot grow a document without limit.

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
use crate::{RequestTenant, RequestVersion, WebState, current_status, render};

fn public_url_with_segments<'a>(
    public_base_url: &str,
    segments: impl IntoIterator<Item = &'a str>,
) -> String {
    let mut url = reqwest::Url::parse(public_base_url)
        .expect("WebState requires a valid HTTP(S) public base URL");
    {
        let mut path = url
            .path_segments_mut()
            .expect("HTTP(S) public base URL supports path segments");
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    url.to_string().trim_end_matches('/').to_string()
}

fn recipient_base_url(state: &WebState, tenant: &RequestTenant) -> String {
    recipient_base_url_value(
        &state.public_base_url,
        state.tenant_path_routing,
        &tenant.id,
    )
}

fn recipient_base_url_value(
    public_base_url: &str,
    tenant_path_routing: bool,
    tenant_id: &str,
) -> String {
    if tenant_path_routing {
        public_url_with_segments(public_base_url, [tenant_id])
    } else {
        public_base_url.to_string()
    }
}

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
    /// The recipient's `Retry-After`, materialized: no poll goes out before
    /// this instant (#790). Empty means poll freely.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next_poll_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub progress: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_url: String,
    /// `not-started` | `in-progress` | `failed` | `stopped` | `completed`.
    /// `failed` is resolvable: a new submit returns it to `in-progress`, and
    /// Abort/Complete can close it out.
    #[serde(default)]
    pub status: String,
    /// Why the last status change (Abort / Mark completed) did not reach the
    /// recipient — kept on the submission so a failed Abort is visible on the
    /// import itself and not only in the log (#968). Named for the operation,
    /// not for Abort: both buttons go through `set_status` and both leave the
    /// submission running when the kick-off fails. The value is the
    /// recipient's or transport's own untranslated diagnosis; the sentence
    /// around it is localized at render time (`status_error_message`), so the
    /// banner reads in the *viewer's* language rather than in the language of
    /// whoever pressed the button. Cleared by the next successful status
    /// change; `serde(default)` keeps submissions stored before #968
    /// deserializing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status_error: String,
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
    pub created_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_submitted_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aborted_at: String,
}

/// The tenant context provider-store calls run under. `/ui` sits outside
/// the auth layer today (#320), so the effective tenant is the request's.
fn tenant_ctx(rt: &RequestTenant) -> helios_persistence::tenant::TenantContext {
    helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new(&rt.id),
        helios_persistence::tenant::TenantPermissions::full_access(),
    )
}

/// Every submission of the tenant, with ids — the list page's view. Entries
/// that fail to decode are skipped rather than sinking the page.
async fn load_all(state: &WebState, rt: &RequestTenant) -> Vec<(String, Submission)> {
    let Some(store) = &state.bulk_provider else {
        return Vec::new();
    };
    match store.list_provider_submissions(&tenant_ctx(rt)).await {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                let parsed = serde_json::from_value(row.document).ok()?;
                Some((row.id, parsed))
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to list bulk-import submissions");
            Vec::new()
        }
    }
}

/// One submission plus the stored version its next write must match.
async fn load_one(state: &WebState, rt: &RequestTenant, id: &str) -> Option<(Submission, i64)> {
    let store = state.bulk_provider.as_ref()?;
    let row = store
        .get_provider_submission(&tenant_ctx(rt), id)
        .await
        .ok()
        .flatten()?;
    let parsed = serde_json::from_value(row.document).ok()?;
    Some((parsed, row.version))
}

/// Persists one submission as a whole document under optimistic versioning
/// (#766, reworked per #772: the provider store replaced the per-user
/// settings subtree, so there is no merge-patch to race and no
/// delete-then-write window). `expected` of `Some(0)` asserts creation.
async fn save(
    state: &WebState,
    rt: &RequestTenant,
    id: &str,
    submission: &Submission,
    expected: Option<i64>,
) -> Result<i64, String> {
    let Some(store) = &state.bulk_provider else {
        return Err("bulk provider store unavailable".to_string());
    };
    let document = serde_json::to_value(submission).map_err(|e| e.to_string())?;
    store
        .put_provider_submission(&tenant_ctx(rt), id, document, expected)
        .await
        .map(|stored| stored.version)
        .map_err(|e| e.to_string())
}

/// Saves and, on failure, logs — for the paths whose response is a redirect
/// either way. A version conflict here means another handler (usually the 5s
/// status poller) wrote first; the caller's state is stale and the next
/// load/poll re-derives it, so the lost write is benign but still logged.
async fn save_or_warn(
    state: &WebState,
    rt: &RequestTenant,
    id: &str,
    submission: &Submission,
    expected: Option<i64>,
) {
    if let Err(e) = save(state, rt, id, submission, expected).await {
        tracing::warn!(submission = %id, error = %e, "failed to persist bulk-import submission");
    }
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
    destination: String,
}

struct LogLine {
    at: String,
    message: String,
}

/// The banner text for a submission whose last Abort / Mark completed never
/// reached the recipient (#968), or an empty string when the last status
/// change went through. The stored value is a technical diagnosis in whatever
/// words the recipient or the transport used; only the sentence around it is
/// translated, the same split the ViewDefinition lint catalog uses.
fn status_error_message(i18n: &I18n, submission: &Submission) -> String {
    if submission.status_error.is_empty() {
        String::new()
    } else {
        i18n.t_arg(
            "bulk-import-status-error",
            "detail",
            submission.status_error.clone(),
        )
    }
}

/// The submission's log as `partials/bulk_import_log.html` wants it:
/// newest-first, so the detail page's first paint and the status fragment's
/// out-of-band refresh agree on the order (#955).
fn log_lines(submission: &Submission) -> Vec<LogLine> {
    submission
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
        .collect()
}

fn status_label(i18n: &I18n, status: &str) -> String {
    match status {
        "in-progress" => i18n.t("bulk-import-status-in-progress"),
        "stopped" => i18n.t("bulk-import-status-stopped"),
        "completed" => i18n.t("bulk-import-status-completed"),
        "failed" => i18n.t("bulk-import-status-failed"),
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
    manifest_url: String,
    submitter_display: String,
    created_at: String,
    status_label: String,
    /// The persistent failed-status-change banner (#968), already localized;
    /// empty renders the bare out-of-band host the status card swaps into.
    /// Distinct from `error`, which is the edit dialog's one-shot failure.
    status_error: String,
    auth: String,
    client_id: String,
    token_url: String,
    log: Vec<LogLine>,
    /// The page paints the log in place, never out-of-band.
    log_oob: bool,
    error: Option<String>,
    edit_open: bool,
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

fn conflict(message: &str) -> Response {
    (StatusCode::CONFLICT, message.to_string()).into_response()
}

/// `GET /ui/bulk-import` — the submissions list.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    let available = state.bulk_provider.is_some();

    let mut rows = Vec::new();
    if available {
        for (id, s) in load_all(&state, &rt).await {
            rows.push(SubmissionRow {
                id,
                status_label: status_label(&i18n, &s.status),
                created_date: s.created_at.clone(),
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
    pub manifest_url: String,
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
    pub output_format: String,
    #[serde(default)]
    pub file_request_headers: String,
}

/// `POST /ui/bulk-import` — one-shot create: a submission carries exactly one
/// manifest, and creating it fires the kick-off immediately. The submission id
/// is generated (unique per submitter); the FHIR base URL derives from the
/// manifest URL's origin at kick-off time.
pub async fn create(
    State(state): State<WebState>,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    axum::Form(form): axum::Form<CreateForm>,
) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    let mut submission = Submission {
        name: form.name.trim().to_string(),
        // The recipient is always this server's HFS_BASE_URL (#689) — typing
        // it per submission is how a submission ended up pointed at something
        // that is not a Bulk Data Submit endpoint (#686).
        recipient_base_url: recipient_base_url(&state, &rt),
        auth: if form.auth == "backend-services" {
            form.auth
        } else {
            "none".to_string()
        },
        submitter_system: form.submitter_system.trim().to_string(),
        submitter_value: form.submitter_value.trim().to_string(),
        poll_url: String::new(),
        next_poll_at: String::new(),
        progress: String::new(),
        result: Value::Null,
        client_id: form.client_id.trim().to_string(),
        token_url: form.token_url.trim().to_string(),
        status: "not-started".to_string(),
        status_error: String::new(),
        created_at: now_stamp(),
        manifests: serde_json::Map::new(),
        log: Vec::new(),
    };
    let manifest = Manifest {
        manifest_url: form.manifest_url.trim().to_string(),
        fhir_base_url: String::new(),
        output_format: form.output_format.trim().to_string(),
        file_request_headers: form.file_request_headers.trim().to_string(),
        created_at: now_stamp(),
        last_submitted_at: String::new(),
        aborted_at: String::new(),
    };
    let mid = uuid::Uuid::new_v4().to_string();
    submission.manifests.insert(
        mid.clone(),
        serde_json::to_value(&manifest).unwrap_or(Value::Null),
    );
    submit_one_with_id(&mut submission, &id, &mid, &rt.id).await;
    match save(&state, &rt, &id, &submission, Some(0)).await {
        Ok(_) => Redirect::to(&format!("/ui/bulk-import/{id}")).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// `GET /ui/bulk-import/{id}` — the submission detail.
pub async fn detail(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);

    let Some((s, _sv)) = load_one(&state, &rt, &id).await else {
        return Redirect::to("/ui/bulk-import").into_response();
    };

    render_detail_page(i18n, status, id, s, None, false)
}

fn render_detail_page(
    i18n: I18n,
    status: crate::Status,
    id: String,
    s: Submission,
    error: Option<String>,
    edit_open: bool,
) -> Response {
    // One-shot model: a submission carries exactly one manifest.
    let manifest_url = s
        .manifests
        .values()
        .next()
        .and_then(|value| value.get("manifestUrl"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let log = log_lines(&s);
    let label = status_label(&i18n, &s.status);
    let status_error = status_error_message(&i18n, &s);
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
        manifest_url,
        created_at: s.created_at,
        status_label: label,
        status_error,
        auth: s.auth.clone(),
        client_id: s.client_id,
        token_url: s.token_url,
        log,
        log_oob: false,
        error,
        edit_open,
    })
}

/// `POST /ui/bulk-import/{id}/delete`.
pub async fn delete(
    State(state): State<WebState>,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(store) = &state.bulk_provider {
        if let Err(e) = store
            .delete_provider_submission(&tenant_ctx(&rt), &id)
            .await
        {
            tracing::warn!(submission = %id, error = %e, "failed to delete bulk-import submission");
        }
    }
    Redirect::to("/ui/bulk-import").into_response()
}

#[derive(Deserialize)]
pub struct EditSubmissionForm {
    pub name: String,
    #[serde(default)]
    pub auth: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub token_url: String,
}

/// `POST /ui/bulk-import/{id}/edit` updates local presentation and transport
/// settings. Protocol identity and accumulated submission state are immutable.
pub async fn edit(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
    axum::Form(form): axum::Form<EditSubmissionForm>,
) -> Response {
    let Some((mut s, sv)) = load_one(&state, &rt, &id).await else {
        return Redirect::to("/ui/bulk-import").into_response();
    };
    let auth = if form.auth == "backend-services" {
        "backend-services"
    } else {
        "none"
    };
    s.name = form.name.trim().to_string();
    s.auth = auth.to_string();
    (s.client_id, s.token_url) = if auth == "backend-services" {
        (
            form.client_id.trim().to_string(),
            form.token_url.trim().to_string(),
        )
    } else {
        (String::new(), String::new())
    };
    match save(&state, &rt, &id, &s, Some(sv)).await {
        Ok(_) => Redirect::to(&format!("/ui/bulk-import/{id}")).into_response(),
        Err(e) => {
            let i18n = I18n::new(locale);
            let mut response = render_detail_page(
                i18n,
                current_status(&state, rv.0, &rt),
                id,
                s,
                Some(e),
                true,
            );
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            response
        }
    }
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

    let response = http_client()
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

/// The URL a submission's kick-off is POSTed to — the one failure messages
/// must name (#686).
fn kickoff_target(submission: &Submission) -> String {
    public_url_with_segments(&submission.recipient_base_url, ["$bulk-submit"])
}

/// POSTs a kick-off to the recipient, returning
/// `(status, content-type, body-excerpt)`.
async fn post_kickoff(
    submission: &Submission,
    parameters: &Value,
    tenant: &str,
) -> Result<(u16, String, String), String> {
    let target = kickoff_target(submission);
    let mut request = with_tenant(
        http_client()
            .post(&target)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .timeout(std::time::Duration::from_secs(15))
            .json(parameters),
        tenant,
    );
    if submission.auth == "backend-services" {
        let token = backend_services_token(&submission.client_id, &submission.token_url).await?;
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("POST {target} failed: {e}"))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let mut body = response.text().await.unwrap_or_default();
    body.truncate(2000);
    Ok((status, content_type, body))
}

/// Summarizes an error response for the log (#686): an OperationOutcome's own
/// explanation when one came back; otherwise the content type and, for
/// non-markup bodies, a short excerpt — raw HTML is never pasted.
fn summarize_error_body(content_type: &str, body: &str) -> String {
    if let Ok(outcome) = serde_json::from_str::<Value>(body)
        && outcome.get("resourceType").and_then(Value::as_str) == Some("OperationOutcome")
        && let Some(explained) = outcome
            .get("issue")
            .and_then(Value::as_array)
            .and_then(|issues| {
                issues.iter().find_map(|i| {
                    i.get("diagnostics")
                        .and_then(Value::as_str)
                        .or_else(|| i.get("details")?.get("text")?.as_str())
                })
            })
    {
        return explained.to_string();
    }
    let kind = content_type.split(';').next().unwrap_or("").trim();
    if kind.contains("html") || body.trim_start().starts_with('<') {
        return format!(
            "the response was {} ({} bytes), not a FHIR resource — is the recipient a Bulk Data Submit endpoint?",
            if kind.is_empty() { "markup" } else { kind },
            body.len()
        );
    }
    let mut excerpt = body.trim().replace('\n', " ");
    excerpt.truncate(160);
    if excerpt.is_empty() {
        format!("empty {kind} response")
    } else {
        format!("{kind}: {excerpt}")
    }
}

/// Kicks off recipient-side status tracking: `POST $bulk-submit-status`
/// (submitter + submissionId, `Prefer: respond-async`), returning the poll
/// URL the recipient hands back in `Content-Location`.
async fn status_kickoff(submission: &Submission, id: &str, tenant: &str) -> Result<String, String> {
    let target = public_url_with_segments(&submission.recipient_base_url, ["$bulk-submit-status"]);
    // Only the identifying parameters ride the status kick-off.
    let parameters = kickoff_parameters(submission, id, "", None);
    let identifying: Vec<Value> = parameters["parameter"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["name"] == "submitter" || p["name"] == "submissionId")
        .cloned()
        .collect();
    let body = json!({ "resourceType": "Parameters", "parameter": identifying });

    let mut request = with_tenant(
        http_client()
            .post(&target)
            .header("Content-Type", "application/fhir+json")
            .header("Prefer", "respond-async")
            .timeout(std::time::Duration::from_secs(15))
            .json(&body),
        tenant,
    );
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

/// How long a status poll may take before it is abandoned. During a heavy
/// ingest the recipient's status handler contends with the writers and is
/// legitimately slow — measured at 8-13s against a local recipient under a
/// bulk load, past the previous 10s cap — so the cap is generous rather
/// than snappy: a slow status is expected under load, not an outage. A poll
/// that still times out logs its cause and backs off instead of hammering
/// (#957).
const STATUS_POLL_TIMEOUT_SECS: u64 = 30;

/// How long to hold polls after a transport failure. The error branch used to
/// leave `next_poll_at` in the past, so every subsequent 5s htmx tick fired
/// another poll — the exact hammering the recipient's rate limit (#790)
/// rejects (#957).
const STATUS_POLL_FAILURE_BACKOFF_SECS: u64 = 30;

/// Shared HTTP client. A per-request `reqwest::Client::new()` rebuilds the
/// connection pool and resolver every call, so every poll paid a fresh TCP
/// connect and nothing was ever kept alive (#957).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Stamps the selected tenant onto a self-call. The recipient is this HFS
/// process (#689): under header routing the tenant travels only here, and
/// under `both` the URL prefix already agrees with it. `Authorization` is
/// deliberately not forwarded — `/ui` sits outside the auth layer (#320)
/// and backend-services kick-offs mint their own bearer (#1006).
fn with_tenant(request: reqwest::RequestBuilder, tenant: &str) -> reqwest::RequestBuilder {
    request.header("X-Tenant-ID", tenant)
}

/// Renders a poll transport failure with its cause. `reqwest::Error`'s
/// `Display` stops at the URL and hides the reason in `source()`, which made
/// a timeout, a refused connection, and a reset log byte-identically (#957).
fn poll_failure_detail(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!(
            "timed out after {STATUS_POLL_TIMEOUT_SECS}s — the recipient's status \
             endpoint can be slow while it is ingesting"
        );
    }
    let mut detail = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(cause) = src {
        detail.push_str(": ");
        detail.push_str(&cause.to_string());
        src = cause.source();
    }
    detail
}

/// Whether the recipient asked us to hold off: a stored `next_poll_at` still
/// in the future means a poll now would only burn the rate limit (#790).
fn poll_due(submission: &Submission) -> bool {
    submission.next_poll_at.is_empty()
        || chrono::DateTime::parse_from_rfc3339(&submission.next_poll_at)
            .map(|t| Utc::now() >= t.with_timezone(&Utc))
            .unwrap_or(true)
}

/// Whether the Data Provider has closed the submission out (Abort / Mark
/// completed). Recipient polls no longer decide its status: HFS answers an
/// aborted submission's status poll with a clean `200`, which would otherwise
/// read as a finished import (#1069). `failed` is not terminal — it can still
/// be resubmitted, aborted, or completed.
fn is_terminal(status: &str) -> bool {
    matches!(status, "stopped" | "completed")
}

/// Materializes a `Retry-After` delta (seconds) into `next_poll_at`.
fn hold_polls_for(submission: &mut Submission, seconds: u64) {
    submission.next_poll_at = (Utc::now() + chrono::Duration::seconds(seconds as i64))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
}

fn retry_after_seconds(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

/// One poll of the recipient's status URL: `202` records `X-Progress`, `200`
/// records the status manifest as the submission's result, anything else is
/// logged and polling stops (the poll URL is cleared). Neither `200` nor a
/// failure changes a closed-out submission's status (#1069). `202` and `429`
/// carry `Retry-After`; both push `next_poll_at` out so the card's refresh
/// cadence never turns into a poll the recipient would reject (#790).
async fn poll_status(submission: &mut Submission, tenant: &str) {
    let poll_url = submission.poll_url.clone();
    let response = match with_tenant(
        http_client()
            .get(&poll_url)
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(STATUS_POLL_TIMEOUT_SECS)),
        tenant,
    )
    .send()
    .await
    {
        Ok(r) => r,
        Err(e) => {
            push_log(
                submission,
                format!("Status poll failed: {}", poll_failure_detail(&e)),
            );
            // A failed poll honors a backoff too — without this,
            // `next_poll_at` stays in the past and every 5s tick re-polls,
            // sailing through the recipient's rate limit (#957).
            hold_polls_for(submission, STATUS_POLL_FAILURE_BACKOFF_SECS);
            return;
        }
    };
    match response.status().as_u16() {
        202 => {
            let retry_after = retry_after_seconds(&response);
            // Decoded from the raw bytes, not through `to_str()`: that
            // accessor refuses any value outside visible US-ASCII, so a
            // recipient whose progress sentence carries a dash, an accent or a
            // non-Latin script would be reported here as a bare "in progress"
            // — losing the byte percentage and the resource count for the whole
            // ingest. `X-Progress` is free-form prose meant for a human, so
            // display whatever arrived and let the fallback mean what it says:
            // the recipient sent no progress at all.
            let progress = response
                .headers()
                .get("x-progress")
                .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
                .unwrap_or_else(|| "in progress".to_string());
            if submission.progress != progress {
                push_log(submission, format!("Status: {progress}"));
            }
            submission.progress = progress;
            hold_polls_for(submission, retry_after.unwrap_or(5));
        }
        200 => {
            let manifest: Value = response.json().await.unwrap_or(Value::Null);
            let outputs = manifest["output"].as_array().map(Vec::len).unwrap_or(0);
            // STU4 status manifests carry OperationOutcome files under
            // `outcome`; `error` is the bulk-export-manifest vocabulary some
            // recipients still use. Reading only `error` made a truncated
            // ingest look like a clean completion. An `outcome` entry counts
            // as an error unless its countSeverity says none are.
            let outcome_errors = manifest["outcome"]
                .as_array()
                .map(|files| {
                    files
                        .iter()
                        .filter(|file| {
                            file.get("countSeverity").is_none_or(|cs| {
                                cs.get("error").and_then(Value::as_u64).unwrap_or(0)
                                    + cs.get("fatal").and_then(Value::as_u64).unwrap_or(0)
                                    > 0
                            })
                        })
                        .count()
                })
                .unwrap_or(0);
            let errors = manifest["error"].as_array().map(Vec::len).unwrap_or(0) + outcome_errors;
            // A closed-out submission keeps the instant it was closed out.
            let completed_at = match submission.result["completedAt"].as_str() {
                Some(at) if is_terminal(&submission.status) && !at.is_empty() => at.to_string(),
                _ => now_stamp(),
            };
            submission.result = json!({
                "completedAt": completed_at,
                "outputs": outputs,
                "errors": errors,
            });
            submission.progress = String::new();
            submission.poll_url = String::new();
            submission.next_poll_at = String::new();
            // The recipient's status manifest is submission-scoped, so its
            // verdict is the submission's (#764, #765): errors mark it
            // failed; a clean completion completes it. Complete remains
            // available for closing out early by hand, and a later submit
            // returns a failed submission to in-progress. A submission the
            // provider already closed out keeps its status (#1069).
            if is_terminal(&submission.status) {
                let status = submission.status.clone();
                push_log(
                    submission,
                    format!(
                        "Status: got 200 OK ({outputs} outputs, {errors} error file(s)); submission stays {status}."
                    ),
                );
            } else if errors > 0 {
                submission.status = "failed".to_string();
                push_log(
                    submission,
                    format!(
                        "Status: got 200 OK — processing finished with {errors} error file(s) ({outputs} outputs); submission marked failed."
                    ),
                );
            } else {
                submission.status = "completed".to_string();
                push_log(
                    submission,
                    format!(
                        "Status: got 200 OK — processing finished cleanly ({outputs} outputs); submission completed."
                    ),
                );
            }
        }
        429 => {
            // A throttled poll is backoff bookkeeping, not a run event — it
            // never reaches the log (#790).
            let retry_after = retry_after_seconds(&response);
            hold_polls_for(submission, retry_after.unwrap_or(5).max(5));
        }
        other => {
            // Polling can never resume (the URL is dropped), so the submission
            // must not keep reading In Progress (#764). completedAt keeps the
            // status card rendered; the log carries the diagnosis. A
            // closed-out submission is not reopened as failed, and keeps the
            // result it was closed out with (#1069).
            submission.progress = String::new();
            submission.poll_url = String::new();
            submission.next_poll_at = String::new();
            if is_terminal(&submission.status) {
                let status = submission.status.clone();
                push_log(
                    submission,
                    format!(
                        "Status poll answered {other}; polling stopped; submission stays {status}."
                    ),
                );
                return;
            }
            submission.status = "failed".to_string();
            submission.result = json!({
                "completedAt": now_stamp(),
                "outputs": 0,
                "errors": 0,
            });
            push_log(
                submission,
                format!(
                    "Status poll answered {other}; polling stopped and the submission is marked failed."
                ),
            );
        }
    }
}

/// Fires the kick-off for one manifest and records the outcome on the
/// submission (status, log, poll URL).
async fn submit_one_with_id(submission: &mut Submission, id: &str, mid: &str, tenant: &str) {
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
    let parameters = kickoff_parameters(submission, id, "in-progress", Some(&m));
    match post_kickoff(submission, &parameters, tenant).await {
        Ok((status, _, _)) if (200..300).contains(&status) => {
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
                match status_kickoff(submission, id, tenant).await {
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
        Ok((status, content_type, body)) => {
            // Name the request that actually failed — the kick-off POST, not
            // the manifest URL, which HFS never called (#686).
            push_log(
                submission,
                format!(
                    "POST {} → {status}: {} (manifest {})",
                    kickoff_target(submission),
                    summarize_error_body(&content_type, &body),
                    m.manifest_url,
                ),
            );
            submission.status = "failed".to_string();
        }
        Err(e) => {
            push_log(
                submission,
                format!(
                    "Bulk Submit request failed: {e} (manifest {})",
                    m.manifest_url
                ),
            );
            submission.status = "failed".to_string();
        }
    }
}

/// `POST /ui/bulk-import/{id}/abort` — status-only kick-off, `stopped`.
pub async fn abort(
    State(state): State<WebState>,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    set_status(state, rt, _principal, id, "stopped").await
}

/// `POST /ui/bulk-import/{id}/complete` — status-only kick-off, `completed`:
/// the Data Provider's signal that no further manifests are coming. The
/// recipient keeps draining already-registered manifests and frees the
/// submission's concurrency slot (#850).
pub async fn complete(
    State(state): State<WebState>,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    set_status(state, rt, _principal, id, "completed").await
}

async fn set_status(
    state: WebState,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    id: String,
    status: &str,
) -> Response {
    let Some((mut s, sv)) = load_one(&state, &rt, &id).await else {
        return Redirect::to("/ui/bulk-import").into_response();
    };
    if !matches!(s.status.as_str(), "in-progress" | "failed") {
        return conflict("only in-progress or failed submissions can change terminal status");
    }
    push_log(&mut s, format!("Marking submission {status}..."));
    let parameters = kickoff_parameters(&s, &id, status, None);
    match post_kickoff(&s, &parameters, &rt.id).await {
        Ok((code, _, _)) if (200..300).contains(&code) => {
            push_log(&mut s, format!("Recipient acknowledged ({code})."));
            s.status = status.to_string();
            // The change landed, so any banner from an earlier attempt is
            // stale (#968).
            s.status_error = String::new();
            // A closed-out submission stops polling the recipient, which
            // answers an aborted submission's status poll with a clean `200`
            // that would otherwise flip Stopped to Completed (#1069). The
            // result stamps the close-out so the card shows its Result form,
            // keeping any counts an earlier `200` manifest recorded.
            if is_terminal(status) {
                s.poll_url = String::new();
                s.next_poll_at = String::new();
                s.progress = String::new();
                s.result = json!({
                    "completedAt": now_stamp(),
                    "outputs": s.result["outputs"].as_u64().unwrap_or(0),
                    "errors": s.result["errors"].as_u64().unwrap_or(0),
                });
            }
        }
        Ok((code, content_type, body)) => {
            let detail = format!("{code}: {}", summarize_error_body(&content_type, &body));
            push_log(
                &mut s,
                format!("Recipient rejected the status change: {detail}"),
            );
            // The submission keeps its own status, so the press left no trace
            // anywhere but the log until #968 — a saturated recipient that
            // times out or rejects the kick-off must not look like a
            // successful Abort. `can_abort` is unchanged, so the retry stays
            // one click away.
            s.status_error = detail;
        }
        Err(e) => {
            push_log(&mut s, format!("Status change failed: {e}"));
            s.status_error = e;
        }
    }
    save_or_warn(&state, &rt, &id, &s, Some(sv)).await;
    Redirect::to(&format!("/ui/bulk-import/{id}")).into_response()
}

/// The recipient-status card fragment, polled by htmx while a poll URL is
/// live and the submission is not closed out (#1069). Each fetch performs at
/// most one poll against the recipient, so the cadence is the page's
/// `every 5s` trigger — no background tasks.
#[derive(Template)]
#[template(path = "partials/bulk_import_status.html")]
struct StatusCard {
    i18n: I18n,
    id: String,
    polling: bool,
    can_abort: bool,
    /// Determinate progress when the recipient reports one; `None` renders
    /// the indeterminate bar.
    percent: Option<u8>,
    progress: String,
    outputs: u64,
    errors: u64,
    completed_at: String,
    /// Rides out-of-band into the summary card's STATUS cell.
    status_label: String,
    /// Rides out-of-band into the detail page's `#submission-error` host, the
    /// same way `status_label` does (#968): the card is re-fetched every 5s
    /// with `outerHTML`, so a banner rendered beside it would either be wiped
    /// by the next swap or duplicated by it. Empty clears the host.
    status_error: String,
    /// Rides out-of-band into the Submission Log section, whose lines this
    /// poll may have just written (#955).
    log: Vec<LogLine>,
    log_oob: bool,
}

/// `GET /ui/bulk-import/{id}/status` — at most one recipient poll, then the
/// refreshed card. The card's htmx cadence only refreshes *this server's*
/// view; the recipient is contacted when its `Retry-After` window has passed
/// (#790), so the fragment stays cheap to re-fetch and the rate limit is
/// never burned on polls that would 429.
pub async fn status_fragment(
    State(state): State<WebState>,
    locale: RequestLocale,
    rt: RequestTenant,
    _principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let Some((mut s, sv)) = load_one(&state, &rt, &id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // A closed-out submission is never polled, even if it was stored with a
    // poll URL before #1069 — the recipient's answer no longer decides it.
    if !s.poll_url.is_empty() && !is_terminal(&s.status) && poll_due(&s) {
        poll_status(&mut s, &rt.id).await;
        save_or_warn(&state, &rt, &id, &s, Some(sv)).await;
    }
    let label = status_label(&i18n, &s.status);
    let status_error = status_error_message(&i18n, &s);
    render(StatusCard {
        id,
        polling: !s.poll_url.is_empty() && !is_terminal(&s.status),
        can_abort: matches!(s.status.as_str(), "in-progress" | "failed"),
        percent: progress_percent(&s.progress),
        progress: s.progress.clone(),
        outputs: s.result["outputs"].as_u64().unwrap_or(0),
        errors: s.result["errors"].as_u64().unwrap_or(0),
        completed_at: s.result["completedAt"].as_str().unwrap_or("").to_string(),
        status_label: label,
        status_error,
        log: log_lines(&s),
        log_oob: true,
        i18n,
    })
}

/// The determinate share of the recipient's `X-Progress`, when it reports
/// one. `0%` counts: HFS reports byte-level progress, so an early zero is a
/// bar about to fill, not a percentage that never moves — the indeterminate
/// sweep is reserved for recipients that report no percentage at all.
///
/// Pre-ingest phase reports (#953) — `Queued - starting shortly`, `reading
/// manifest`, `sizing {done} of {total} files`, `downloading file {done} of
/// {total}` — deliberately fall through to `None`: there is no meaningful
/// share-of-the-whole to draw yet, so the card pairs the phase text with the
/// indeterminate sweep. #827 is the rule being honoured here — a determinate
/// reading must never be shown beside an indeterminate bar, so "phase text +
/// sweep" and "percentage + fill" are the only two shapes this card has.
///
/// The contract that keeps that true: **a phase string must never begin with
/// the literal `processing ` prefix**, or its embedded counts (`sizing 37 of
/// 412 files`) would be mis-parsed as a percentage. The recipient owns the
/// vocabulary; this parser only claims the one prefix.
fn progress_percent(progress: &str) -> Option<u8> {
    // Case-insensitive: HFS capitalizes the line ("Processing 3% -
    // …", #954), older HFS versions and foreign recipients may send lowercase
    // "processing 3% complete …". Either way the digits follow the prefix.
    let rest = progress
        .get(..11)
        .filter(|p| p.eq_ignore_ascii_case("processing "))
        .and_then(|_| progress.get(11..))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let pct: u8 = digits.parse().ok().filter(|p| *p <= 100)?;
    Some(pct)
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
/// Kept as a public compatibility endpoint for existing integrations.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_percent_reads_both_hfs_wordings_and_foreign_case() {
        // Current HFS wording (#954, ASCII-only since the sentence travels in
        // a header).
        assert_eq!(
            progress_percent("Processing 3% - 609,191 Resources written"),
            Some(3)
        );
        assert_eq!(progress_percent("Processing 0%"), Some(0));
        // A recipient that does use non-ASCII still gets its percentage read:
        // the header is decoded from bytes, so the sentence arrives intact.
        assert_eq!(
            progress_percent("Processing 3% — 609,191 Resources written"),
            Some(3)
        );
        // Pre-#954 HFS and lowercase foreign recipients.
        assert_eq!(
            progress_percent("processing 10% complete (5 entries ingested)"),
            Some(10)
        );
        // Non-matching recipients keep the indeterminate sweep.
        assert_eq!(progress_percent("halfway there"), None);
        assert_eq!(progress_percent("processing lots"), None);
    }

    /// #1069: only a provider close-out is terminal; `failed` can still be
    /// resubmitted, aborted, or completed.
    #[test]
    fn only_stopped_and_completed_are_terminal() {
        assert!(is_terminal("stopped"));
        assert!(is_terminal("completed"));
        for status in ["not-started", "in-progress", "failed", ""] {
            assert!(!is_terminal(status), "status: {status}");
        }
    }

    #[test]
    fn recipient_base_preserves_prefix_and_adds_path_tenant() {
        assert_eq!(
            recipient_base_url_value("https://public.example/fhir", true, "acme"),
            "https://public.example/fhir/acme"
        );
    }

    #[test]
    fn recipient_base_stays_unprefixed_for_header_only_routing() {
        assert_eq!(
            recipient_base_url_value("https://public.example/fhir", false, "acme"),
            "https://public.example/fhir"
        );
    }

    #[test]
    fn public_url_builder_encodes_tenant_segments() {
        assert_eq!(
            recipient_base_url_value("https://public.example/fhir", true, "north clinic"),
            "https://public.example/fhir/north%20clinic"
        );
    }

    /// #953: the pre-ingest phases carry no share-of-the-whole, so every one
    /// of them reads as `None` and the card draws the indeterminate sweep
    /// beside the phase text. The counts inside `sizing …` / `downloading
    /// file …` are the trap this guards: they must never be mistaken for a
    /// percentage (#827 — never a determinate reading on an indeterminate
    /// bar).
    #[test]
    fn pre_ingest_phases_report_no_percentage() {
        for phase in [
            "Queued - starting shortly",
            "Queued - waiting for an external worker",
            "reading manifest",
            "sizing 37 of 412 files",
            "downloading file 1 of 412",
        ] {
            assert_eq!(progress_percent(phase), None, "phase: {phase}");
            assert!(
                !phase.starts_with("processing "),
                "a phase string that starts with the parsed prefix would be \
                 mis-read as a percentage: {phase}"
            );
        }
    }

    /// The ingest-phase vocabulary is unchanged by #953: `processing N%` is
    /// still the one shape that yields a determinate bar, with or without the
    /// entry-count suffix.
    #[test]
    fn processing_percentages_still_read_as_determinate() {
        assert_eq!(progress_percent("processing 35% complete"), Some(35));
        assert_eq!(
            progress_percent("processing 35% complete (120 entries ingested)"),
            Some(35)
        );
        assert_eq!(progress_percent("processing 0% complete"), Some(0));
        assert_eq!(progress_percent("processing 100% complete"), Some(100));
    }

    /// A stalled report keeps its current behaviour: it does not carry the
    /// `processing ` prefix, so its `40%` is never lifted into the bar — the
    /// operator gets the sweep plus the full explanatory sentence.
    #[test]
    fn a_stalled_report_stays_indeterminate() {
        assert_eq!(
            progress_percent("stalled at 40% - a worker stopped without handoff; see server logs"),
            None
        );
    }

    /// Out-of-range and malformed percentages fall back to the sweep rather
    /// than clamping to something the recipient never said.
    #[test]
    fn unparseable_percentages_fall_back_to_the_sweep() {
        assert_eq!(progress_percent("processing 101% complete"), None);
        assert_eq!(progress_percent("processing complete"), None);
        assert_eq!(progress_percent(""), None);
        assert_eq!(progress_percent("in progress"), None);
    }
}
