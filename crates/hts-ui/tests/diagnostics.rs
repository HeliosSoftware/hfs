//! Diagnostics page HTTP tests (Slice G, design doc §7.9).
//!
//! Same in-process axum mock pattern as `tests/import.rs`. Each test
//! polls `/__mock_ready` before firing so the mock's TCP listener has
//! finished accepting on Windows (matches Slice C/D/E1/E2/F).
//!
//! The five tests exercise:
//!
//! 1. Full-page shell renders the 4-tab strip + panel container.
//! 2. Panel route with `?tab=capability` renders the property table.
//! 3. TerminologyCap tab renders the `codeSystem[]` list from the mock.
//! 4. `/metrics` tab wraps the Prometheus text-format body in `<pre>`.
//! 5. A 500 on `/health` renders `hts-outcome.html` *inside* the
//!    panel — the tab strip in the parent shell stays intact.

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower::ServiceExt;

// ── Shared fixtures ─────────────────────────────────────────────────────

fn app_pointing_at(base_url: &str) -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            base_url,
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .expect("test upstream base URL parses"),
        bundled_data_bytes: None,
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ── In-process mock upstream ─────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)] // `headers` kept for triage on failure.
struct CapturedRequest {
    method: String,
    path: String,
    headers: HeaderMap,
    body: String,
}

/// Per-endpoint canned responses. Each test seeds the shape it needs
/// before firing its request; anything not overridden falls back to
/// [`MockResponses::default`] (200 with a minimal FHIR body).
#[derive(Clone)]
struct MockResponses {
    /// `GET /metadata` — CapabilityStatement.
    capability: (StatusCode, Option<Value>),
    /// `GET /metadata?mode=terminology` — TerminologyCapabilities.
    terminology: (StatusCode, Option<Value>),
    /// `GET /health` — the JSON liveness probe.
    health: (StatusCode, Option<Value>),
    /// `GET /metrics` — Prometheus text-format body.
    metrics: (StatusCode, Option<String>),
}

impl MockResponses {
    fn default() -> Self {
        Self {
            capability: (
                StatusCode::OK,
                Some(json!({
                    "resourceType": "CapabilityStatement",
                    "url": "http://helios.test/fhir/hts/CapabilityStatement/hts",
                    "version": "9.9.9-test",
                    "name": "HeliosTerminologyServer",
                    "title": "Helios Terminology Server",
                    "status": "active",
                    "date": "2026-08-18",
                })),
            ),
            terminology: (
                StatusCode::OK,
                Some(json!({
                    "resourceType": "TerminologyCapabilities",
                    "version": "9.9.9-test",
                    "name": "HeliosTerminologyServer",
                    "title": "Helios Terminology Server",
                    "status": "active",
                    "codeSystem": [],
                })),
            ),
            health: (
                StatusCode::OK,
                Some(json!({
                    "status": "ok",
                    "service": "hts",
                    "version": "0.0.0-test",
                    "backend": "sqlite",
                    "uptime_seconds": 42,
                })),
            ),
            metrics: (
                StatusCode::OK,
                Some(String::from(
                    "# TYPE hts_up gauge\nhts_up 1\n",
                )),
            ),
        }
    }
}

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    responses: Arc<Mutex<MockResponses>>,
}

impl MockState {
    async fn snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().await.clone()
    }

    async fn set_health(&self, status: StatusCode, body: Option<Value>) {
        self.responses.lock().await.health = (status, body);
    }

    async fn set_terminology(&self, status: StatusCode, body: Option<Value>) {
        self.responses.lock().await.terminology = (status, body);
    }

    async fn set_metrics(&self, status: StatusCode, body: Option<String>) {
        self.responses.lock().await.metrics = (status, body);
    }
}

async fn capture(state: &MockState, path: &str, req: Request<Body>) -> Vec<u8> {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    state.captured.lock().await.push(CapturedRequest {
        method: parts.method.to_string(),
        path: path.to_owned(),
        headers: parts.headers.clone(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    });
    bytes.to_vec()
}

async fn mock_metadata_handler(
    State(state): State<MockState>,
    req: Request<Body>,
) -> axum::response::Response {
    let uri_path_and_query = req.uri().path_and_query().map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "/metadata".to_owned());
    let is_terminology_mode = uri_path_and_query.contains("mode=terminology");
    let _ = capture(&state, &uri_path_and_query, req).await;
    let responses = state.responses.lock().await.clone();
    let (status, body) = if is_terminology_mode {
        responses.terminology
    } else {
        responses.capability
    };
    match body {
        Some(v) => (status, axum::Json(v)).into_response(),
        None => (status, "").into_response(),
    }
}

async fn mock_health_handler(
    State(state): State<MockState>,
    req: Request<Body>,
) -> axum::response::Response {
    let _ = capture(&state, "/health", req).await;
    let responses = state.responses.lock().await.clone();
    let (status, body) = responses.health;
    match body {
        Some(v) => (status, axum::Json(v)).into_response(),
        None => (status, "").into_response(),
    }
}

async fn mock_metrics_handler(
    State(state): State<MockState>,
    req: Request<Body>,
) -> axum::response::Response {
    let _ = capture(&state, "/metrics", req).await;
    let responses = state.responses.lock().await.clone();
    let (status, body) = responses.metrics;
    match body {
        Some(text) => (status, text).into_response(),
        None => (status, "").into_response(),
    }
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
        responses: Arc::new(Mutex::new(MockResponses::default())),
    };
    let router: Router = Router::new()
        .route("/__mock_ready", get(|| async { (StatusCode::OK, "ok") }))
        .route("/metadata", get(mock_metadata_handler))
        .route("/health", get(mock_health_handler))
        .route("/metrics", get(mock_metrics_handler))
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

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn diagnostics_page_renders_all_four_tabs_in_full_shell() {
    // Full-page GET renders the shell with all 4 tab links + the
    // `#diag-panel` container. Default tab is Capability, so the
    // property table for the CapabilityStatement is pre-rendered
    // inside the panel (nojs contract).
    let (base, _state) = start_mock().await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/diagnostics")
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
    // H1 comes from `hts-diagnostics-heading` (en value: "Diagnostics").
    assert!(
        html.contains(">Diagnostics<"),
        "shell heading must be Fluent-resolved",
    );

    // All four tab id markers must be present so JS clients can target
    // them via `aria-controls` and no-JS clients still see labeled
    // anchors.
    for tab_id in [
        "hts-diagnostics-tab-capability",
        "hts-diagnostics-tab-terminology-capabilities",
        "hts-diagnostics-tab-health",
        "hts-diagnostics-tab-metrics",
    ] {
        assert!(
            html.contains(tab_id),
            "shell must expose the `{tab_id}` marker (either as id or Fluent key)",
        );
    }

    assert!(
        html.contains(r#"id="diag-panel""#),
        "shared tabpanel container `#diag-panel` must render",
    );
    assert!(
        html.contains(r#"aria-selected="true""#),
        "the active (default: capability) tab must carry aria-selected=true",
    );
    // Default tab (Capability) is pre-rendered — the property table
    // heading must appear so nojs deep-links also work.
    assert!(
        html.contains(">CapabilityStatement<"),
        "default Capability tab must be pre-rendered inside #diag-panel",
    );

    // Fluent keys that are NOT reused as element ids or aria-labelledby
    // targets must not leak. Some keys ARE deliberately reused as ids
    // (e.g. `hts-diagnostics-capability-heading` on the H2, per the
    // Slice F precedent noted in `tests/import.rs`), so those are
    // excluded from the leak check. `metrics-figcaption` / `code-
    // systems-empty` only render on non-default tabs, so they should
    // not appear in the default (capability) shell either way.
    for key in [
        "hts-diagnostics-title",
        "hts-diagnostics-terminology-code-systems-empty",
        "hts-diagnostics-metrics-figcaption",
        "hts-diagnostics-metrics-empty",
        "hts-diagnostics-health-status-label",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked (missing Fluent value?)",
        );
    }
}

#[tokio::test]
async fn capability_tab_renders_property_table() {
    let (base, _state) = start_mock().await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/diagnostics/panel?tab=capability")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Fragment response only — no full HTML document.
    assert!(
        !html.contains("<!doctype html>"),
        "htmx panel swap must return a fragment, not a full page",
    );
    // The heading + a couple of property values from the mock body
    // must be present.
    assert!(
        html.contains(">CapabilityStatement<"),
        "capability tab must render the sub-heading",
    );
    assert!(
        html.contains("http://helios.test/fhir/hts/CapabilityStatement/hts"),
        "URL property from the mock CapabilityStatement must render",
    );
    assert!(
        html.contains("9.9.9-test"),
        "version property from the mock CapabilityStatement must render",
    );
}

#[tokio::test]
async fn terminology_capabilities_tab_renders_code_system_list() {
    let (base, state) = start_mock().await;
    // Two loaded code systems, both with the flat-string `version`
    // shape the parser accepts as a fallback (HTS today emits `uri`
    // only; the spec array shape is also supported by the parser).
    state
        .set_terminology(
            StatusCode::OK,
            Some(json!({
                "resourceType": "TerminologyCapabilities",
                "url": "http://helios.test/fhir/hts/TerminologyCapabilities/hts",
                "version": "9.9.9-test",
                "name": "HeliosTerminologyServer",
                "title": "Helios Terminology Server",
                "status": "active",
                "codeSystem": [
                    { "uri": "http://snomed.info/sct", "version": "20240301" },
                    { "uri": "http://loinc.org", "version": "2.76" },
                ],
            })),
        )
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get(
                "/ui/hts/diagnostics/panel?tab=terminology-capabilities",
            )
            .header(header::ACCEPT_LANGUAGE, "en")
            .header("HX-Request", "true")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(">TerminologyCapabilities<"),
        "terminology-capabilities tab must render the sub-heading",
    );
    assert!(
        html.contains("http://snomed.info/sct"),
        "first codeSystem uri must render as a <li>",
    );
    assert!(
        html.contains("http://loinc.org"),
        "second codeSystem uri must render as a <li>",
    );
    assert!(
        html.contains("v20240301"),
        "codeSystem version tag must render next to the uri",
    );
    assert!(
        html.contains("v2.76"),
        "codeSystem version tag for LOINC must render next to the uri",
    );

    // Verify the outgoing request headed to `/metadata?mode=terminology`.
    let captured = state.snapshot().await;
    let hit = captured
        .iter()
        .find(|c| c.path.starts_with("/metadata") && c.path.contains("mode=terminology"))
        .expect("mock must have observed the /metadata?mode=terminology GET");
    assert_eq!(hit.method, "GET");
}

#[tokio::test]
async fn metrics_tab_renders_prometheus_text_verbatim() {
    let (base, state) = start_mock().await;
    state
        .set_metrics(
            StatusCode::OK,
            Some(String::from("# TYPE foo counter\nfoo 42\n")),
        )
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/diagnostics/panel?tab=metrics")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(">Prometheus metrics<"),
        "metrics tab must render the sub-heading (en value)",
    );
    assert!(
        html.contains("<pre"),
        "metrics tab must wrap the raw text in <pre>",
    );
    // The raw body must appear verbatim inside the tab. Askama's
    // default escaper turns `#` into itself and preserves whitespace
    // inside the code block, so both lines must be present.
    assert!(
        html.contains("# TYPE foo counter"),
        "raw metrics TYPE line must render verbatim inside <pre>",
    );
    assert!(
        html.contains("foo 42"),
        "raw metrics sample line must render verbatim inside <pre>",
    );
}

#[tokio::test]
async fn any_tab_5xx_renders_outcome_in_diag_panel_only() {
    // A 500 on `/health` must render the shared OperationOutcome
    // partial *inside* the panel — the tab strip in the parent shell
    // must stay intact (per-tab isolation, §7.9).
    let (base, state) = start_mock().await;
    state
        .set_health(StatusCode::INTERNAL_SERVER_ERROR, None)
        .await;

    // Hit the full-page route so we can also assert the tab links
    // survive the failure. The panel route would work just as well
    // for the outcome-render assertion but wouldn't show the shell.
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/diagnostics?tab=health")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // OperationOutcome partial marker (both severity variants use the
    // `hts-outcome hts-outcome--error` class stack).
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "500 on /health must render the shared OperationOutcome partial",
    );
    // The tab links must survive — per-tab isolation. Check the id
    // markers for the three *other* tabs are still present.
    for tab_id in [
        "hts-diagnostics-tab-capability",
        "hts-diagnostics-tab-terminology-capabilities",
        "hts-diagnostics-tab-metrics",
    ] {
        assert!(
            html.contains(tab_id),
            "tab `{tab_id}` must survive a 500 on a different tab",
        );
    }
}

// Note: dual-mode (HX-Request on the top-level page) is covered by
// `route_enum.rs::ROUTES` — the matrix walks `/ui/hts/diagnostics` and
// `/ui/hts/diagnostics/panel` with `HX-Request: true` and asserts the
// response is 200 + `Vary: HX-Request`. That keeps this file at the
// ≤ 5 `#[tokio::test]` budget (Slice G brief constraint).
