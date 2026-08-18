//! Import page HTTP tests (Slice F, design doc §7.7).
//!
//! Same two-fixture pattern as Slice C / D / E1 / E2:
//!
//! 1. **Closed loopback** (`127.0.0.1:1`) — the shell + degraded /
//!    OperationOutcome tests. `UpstreamClient::new_with_timeouts` at
//!    100 ms / 250 ms so `Connect` fires fast and the matrix stays
//!    under wallclock.
//! 2. **In-process axum mock** — spun up per test with a canned
//!    `POST /import` response so the ring asserts the 200 / 207 / 400
//!    / 413 wire contract without depending on a real HTS. Mock uses
//!    generous 2 s / 5 s timeouts because `tokio::spawn`ed
//!    `axum::serve` on a Windows current-thread `#[tokio::test]`
//!    runtime needs headroom to accept the first connection.
//!
//! Every mock-backed test polls `/__mock_ready` before firing the real
//! request — matches the pattern from `tests/value_sets.rs` and
//! `tests/concept_maps.rs`.

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower::ServiceExt;

// ── Shared fixtures ─────────────────────────────────────────────────────

fn app_with_timeouts(
    base_url: &str,
    request_timeout: Duration,
    connect_timeout: Duration,
) -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            base_url,
            request_timeout,
            connect_timeout,
        )
        .expect("test upstream base URL parses"),
        bundled_data_bytes: None,
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

/// Router pointed at a real upstream (in-process mock). Generous
/// timeouts so a `tokio::spawn`ed `axum::serve` has time to accept
/// its first connection on Windows. Every Slice F test lives on the
/// mock leg (the pre-flight-gate test still needs the mock to
/// observe the "no `/import` request was recorded" assertion), so
/// the closed-loopback `app()` fixture from the sibling test files
/// is not needed here.
fn app_pointing_at(base_url: &str) -> Router {
    app_with_timeouts(base_url, Duration::from_secs(5), Duration::from_secs(2))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ── In-process mock upstream ─────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)] // `headers` inspected only when tests fail; kept for triage.
struct CapturedRequest {
    method: String,
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct CannedResponse {
    status: StatusCode,
    /// Response body. `None` produces an empty body (used to model
    /// the 413 case where HTS often returns nothing).
    body: Option<Value>,
}

impl CannedResponse {
    fn import_ok() -> Self {
        Self {
            status: StatusCode::OK,
            body: Some(json!({
                "code_systems": 2,
                "value_sets": 3,
                "concept_maps": 1,
                "concepts": 42,
            })),
        }
    }

    fn import_partial() -> Self {
        Self {
            status: StatusCode::MULTI_STATUS,
            body: Some(json!({
                "code_systems": 1,
                "value_sets": 0,
                "concept_maps": 0,
                "concepts": 5,
                "errors": [
                    "ValueSet.entry[3]: missing required field `url`",
                    "ConceptMap.entry[1]: reference to unknown CodeSystem",
                ],
            })),
        }
    }

    fn import_rejected() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: Some(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "invalid",
                    "diagnostics": "Body is not a FHIR Bundle: missing resourceType"
                }]
            })),
        }
    }

    fn import_too_large() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            body: None,
        }
    }
}

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    canned: Arc<Mutex<CannedResponse>>,
}

impl MockState {
    async fn snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().await.clone()
    }

    async fn set_canned(&self, response: CannedResponse) {
        *self.canned.lock().await = response;
    }
}

async fn mock_import_handler(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    let body_str = String::from_utf8_lossy(&bytes).into_owned();
    state.captured.lock().await.push(CapturedRequest {
        method: method.to_string(),
        path: parts.uri.to_string(),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.canned.lock().await.clone();
    match canned.body {
        Some(v) => (canned.status, axum::Json(v)).into_response(),
        None => (canned.status, "").into_response(),
    }
}

/// `GET /health` — mock's default answer so the Import page's
/// degraded probe returns a non-degraded shell. Tests that need a
/// specific import-response shape set the canned body via
/// `state.set_canned(...)`.
async fn mock_health_handler() -> axum::response::Response {
    (
        StatusCode::OK,
        axum::Json(json!({
            "status": "ok",
            "service": "hts",
            "version": "0.0.0-test",
            "backend": "sqlite",
            "uptime_seconds": 42,
        })),
    )
        .into_response()
}

async fn mock_fallback(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    state.captured.lock().await.push(CapturedRequest {
        method: method.to_string(),
        path: format!("<fallback>{}", parts.uri),
        headers: parts.headers.clone(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    });
    (StatusCode::NOT_FOUND, "").into_response()
}

async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        canned: Arc::new(Mutex::new(CannedResponse::import_ok())),
    };
    let router: Router = Router::new()
        .route("/__mock_ready", get(|| async { (StatusCode::OK, "ok") }))
        .route("/health", get(mock_health_handler))
        .route("/import", post(mock_import_handler))
        .fallback(mock_fallback)
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream listener");
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{addr}");
    let probe = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("build ready-probe client");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let ready_url = format!("{base}/__mock_ready");
    loop {
        if std::time::Instant::now() >= deadline {
            break;
        }
        match probe.get(&ready_url).send().await {
            Ok(r) if r.status().is_success() => break,
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    (base, state)
}

/// Minimal well-formed Bundle body used by every test that expects to
/// pass the pre-flight JSON gate. HTS only inspects it here through
/// the mock, so the shape is only "parseable JSON".
const VALID_BUNDLE: &str =
    r#"{"resourceType":"Bundle","type":"collection","entry":[]}"#;

fn encode_form(pairs: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        serializer.append_pair(k, v);
    }
    serializer.finish()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn import_page_renders_full_shell_with_upload_form() {
    // Mock upstream so `/health` returns 200 and the degraded banner
    // stays off. Asserts the shell H1 + the paste-mode textarea id
    // (the only element other tests key off of) + that Fluent keys
    // don't leak raw.
    let (base, _state) = start_mock().await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/import")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("<!doctype html>"),
        "hard nav must render a full HTML page",
    );
    assert!(
        html.contains(">Import terminology<"),
        "shell heading must be Fluent-resolved (en value: Import terminology)",
    );
    assert!(
        html.contains(r#"id="hts-import-bundle""#),
        "paste-mode textarea must render on the shell",
    );
    assert!(
        html.contains(r#"id="hts-import-submit""#),
        "submit button must render",
    );
    assert!(
        !html.contains("Terminology backend not fully available"),
        "healthy upstream must NOT render the degraded banner",
    );
    // Some keys collide with element ids by design (`hts-import-heading`
    // on the H1, `hts-import-submit` on the button, `hts-import-bundle`
    // on the textarea, `hts-import-status` on the status region) —
    // matches the naming used by the CS / VS / CM shells. The keys
    // below are safe to leak-check because none of them are used as
    // element ids or class names anywhere in the shell.
    for key in [
        "hts-import-title",
        "hts-import-source-paste",
        "hts-import-status-success",
        "hts-import-status-rejected",
        "hts-import-empty-bundle-error",
        "hts-import-invalid-json-error",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked (missing Fluent value?)",
        );
    }
}

#[tokio::test]
async fn import_post_200_renders_success_summary() {
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::import_ok()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(encode_form(&[("bundle", VALID_BUNDLE)])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("hts-import-status--ok"),
        "200 must render the ok status class marker; got: {}",
        &html[..html.len().min(400)],
    );
    assert!(
        html.contains("Import complete"),
        "200 must render the success title (en value)",
    );
    // Counts table columns render numeric values, not "—".
    assert!(
        html.contains(">2<") && html.contains(">42<"),
        "counts table must render the values from the mock body",
    );

    // Verify the outgoing request headed to `/import` with the right
    // content-type.
    let captured = state.snapshot().await;
    let import = captured
        .iter()
        .find(|c| c.path == "/import")
        .expect("mock must have observed the /import POST");
    assert_eq!(import.method, "POST");
    let ct = import
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        ct.contains("application/fhir+json"),
        "outgoing Content-Type must be application/fhir+json; got: {ct}",
    );
}

#[tokio::test]
async fn import_post_207_renders_partial_success_with_issue_list() {
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::import_partial()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(encode_form(&[("bundle", VALID_BUNDLE)])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("hts-import-status--warn"),
        "207 must render the amber (warn) status class marker",
    );
    assert!(
        html.contains("Import partially succeeded"),
        "207 must render the partial-success title (en value)",
    );
    assert!(
        html.contains("<details"),
        "issue list must be inside a <details> expander",
    );
    assert!(
        html.contains("2 issues"),
        "issues heading must be plural-selected (en 'other' arm)",
    );
    assert!(
        html.contains("missing required field"),
        "individual issue strings must be rendered inside the list",
    );
}

#[tokio::test]
async fn import_post_400_renders_outcome_partial() {
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::import_rejected()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(encode_form(&[("bundle", VALID_BUNDLE)])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("hts-import-status--error"),
        "400 must render the error status class marker",
    );
    assert!(
        html.contains("Import rejected"),
        "400 must render the rejected title (en value)",
    );
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "400 must render the shared OperationOutcome partial in error severity",
    );
    assert!(
        html.contains("Body is not a FHIR Bundle"),
        "the outcome diagnostics must reach the rendered body",
    );
}

#[tokio::test]
async fn import_post_413_renders_too_large_guidance() {
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::import_too_large()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(encode_form(&[("bundle", VALID_BUNDLE)])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("hts-import-status--warn"),
        "413 must render the warn status class marker",
    );
    assert!(
        html.contains("Bundle too large"),
        "413 must render the too-large title (en value)",
    );
    assert!(
        html.contains("Split the Bundle"),
        "413 must render the split-the-Bundle hint from `hts-import-too-large-hint`",
    );
}

#[tokio::test]
async fn import_pre_flight_empty_bundle_returns_outcome_without_calling_hts() {
    // The pre-flight gate must catch an empty paste and render an
    // OperationOutcome without round-tripping to HTS. Mock captures
    // every incoming request, so the assertion is "no `/import`
    // request was recorded".
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::import_ok()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/import")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(encode_form(&[("bundle", "")])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "empty bundle must render the invalid-input outcome",
    );

    let captured = state.snapshot().await;
    assert!(
        !captured.iter().any(|c| c.path == "/import"),
        "empty-bundle gate MUST NOT round-trip to HTS; captured: {:?}",
        captured.iter().map(|c| &c.path).collect::<Vec<_>>(),
    );
}

// Note: dual-mode (htmx GET returns just the upload-form partial)
// is covered by `route_enum.rs::ROUTES` — the matrix walks
// `/ui/hts/import` with `HX-Request: true` and asserts the response
// is 200 + `Vary: HX-Request`, and the shell-marker walk asserts the
// paste-mode textarea id is present on the full-page cell. That
// keeps the `import.rs` ring at the ≤ 6 `#[tokio::test]` budget
// (matches the constraint from the Slice F task brief).
