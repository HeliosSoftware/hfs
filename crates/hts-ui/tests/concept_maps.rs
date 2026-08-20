//! ConceptMap browser + detail + translate HTTP tests (Slice D).
//!
//! Two upstream fixtures cover the whole ring:
//!
//! 1. **Closed loopback** (`127.0.0.1:1`) — used by every test that only
//!    needs to observe a UI shell / degraded / OperationOutcome partial
//!    (the reads collapse to `UpstreamError::Connect`). Matches
//!    `tests/value_sets.rs` shape.
//! 2. **In-process axum mock** — spun up per test on an ephemeral
//!    loopback port for the flows that assert HTTP-level behavior of the
//!    outgoing request: forward vs reverse Parameters bodies, the R4/R5
//!    mapping-kind column, the pre-flight validation gate (which the
//!    mock must record zero incoming calls for), and 4xx / 5xx surfaces.
//!    Captures request bodies + headers so the ring pins the wire
//!    contract without depending on a real HTS.
//!
//! Timeout envelope mirrors Slice C (§7.4.1 invariant #3): closed
//! loopback keeps `100 ms / 250 ms`, mock uses `2 s / 5 s` for the
//! spawned `axum::serve` accept headroom on Windows current-thread
//! `#[tokio::test]` runtimes. `start_mock` polls `/__mock_ready` before
//! returning so the first client request never races the accept.

use axum::{
    Router,
    body::Body,
    extract::{Path, Request, State},
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
/// timeouts so a `tokio::spawn`ed `axum::serve` has time to accept its
/// first connection on Windows (§7.4.1 mock-upstream note).
fn app_pointing_at(base_url: &str) -> Router {
    app_with_timeouts(base_url, Duration::from_secs(5), Duration::from_secs(2))
}

/// Router pointed at a closed loopback port — the "Connect" fixture for
/// tests that only need degraded / OperationOutcome shape. Timeouts
/// stay tight because the OS returns `ECONNREFUSED` immediately.
fn app() -> Router {
    app_with_timeouts(
        "http://127.0.0.1:1",
        Duration::from_millis(250),
        Duration::from_millis(100),
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ── In-process mock upstream ─────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)] // fields inspected only when tests fail; kept for triage
struct CapturedRequest {
    method: String,
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone)]
struct CannedResponse {
    status: StatusCode,
    body: Value,
}

impl CannedResponse {
    /// Standard R4/R4B `equivalence` translation success with two
    /// matches — used by the forward-direction happy-path assertions.
    fn ok_translate_equivalence() -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "result", "valueBoolean": true},
                    {"name": "match", "part": [
                        {"name": "equivalence", "valueCode": "equivalent"},
                        {"name": "concept", "valueCoding": {
                            "system": "http://example.org/target-cs",
                            "code": "T1",
                            "display": "Target One"
                        }},
                        {"name": "originMap", "valueUri": "http://example.org/cm/map#1"}
                    ]},
                    {"name": "match", "part": [
                        {"name": "equivalence", "valueCode": "wider"},
                        {"name": "concept", "valueCoding": {
                            "system": "http://example.org/target-cs",
                            "code": "T2"
                        }},
                        {"name": "originMap", "valueUri": "http://example.org/cm/map#2"}
                    ]}
                ]
            }),
        }
    }

    /// R5/R6-shaped `relationship` translation success (single match) —
    /// used to prove the mapping-kind column reads the response, not a
    /// compile-time cfg.
    fn ok_translate_relationship() -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "result", "valueBoolean": true},
                    {"name": "match", "part": [
                        {"name": "relationship", "valueCode": "related-to"},
                        {"name": "concept", "valueCoding": {
                            "system": "http://example.org/target-cs",
                            "code": "T3",
                            "display": "Related"
                        }},
                        {"name": "source", "valueUri": "http://example.org/cm/map#3"}
                    ]}
                ]
            }),
        }
    }

    /// HTTP 200 with `result=false` — the §7.5 F11-realized neutral
    /// no-matches state.
    fn no_matches() -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "result", "valueBoolean": false},
                    {"name": "message", "valueString": "no mapping found"}
                ]
            }),
        }
    }

    /// HTS-side error (500) with an OperationOutcome body — the error
    /// arm of §7.5 renders the shared `hts-outcome.html` partial.
    fn server_error() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "exception",
                    "diagnostics": "backend blew up"
                }]
            }),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "not-found",
                    "diagnostics": "unknown ConceptMap"
                }]
            }),
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

async fn mock_translate_handler(
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
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        canned: Arc::new(Mutex::new(CannedResponse::ok_translate_equivalence())),
    };
    let router: Router = Router::new()
        .route(
            "/__mock_ready",
            get(|| async { (StatusCode::OK, "ok") }),
        )
        .route("/ConceptMap", get(mock_handler_get_search))
        .route("/ConceptMap/{id}", get(mock_handler_get_id))
        .route("/ConceptMap/{id}/$translate", post(mock_translate_handler))
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
            _ => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    (base, state)
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

async fn mock_handler_get_search(
    State(state): State<MockState>,
    req: Request<Body>,
) -> axum::response::Response {
    let (parts, _body) = req.into_parts();
    state.captured.lock().await.push(CapturedRequest {
        method: "GET".to_owned(),
        path: parts.uri.to_string(),
        headers: parts.headers.clone(),
        body: String::new(),
    });
    // Empty Bundle — most tests that hit this route only care about the
    // browser shell rendering, not the row projection.
    (
        StatusCode::OK,
        axum::Json(json!({"resourceType": "Bundle", "entry": []})),
    )
        .into_response()
}

async fn mock_handler_get_id(
    State(state): State<MockState>,
    Path(id): Path<String>,
    req: Request<Body>,
) -> axum::response::Response {
    let (parts, _body) = req.into_parts();
    state.captured.lock().await.push(CapturedRequest {
        method: "GET".to_owned(),
        path: parts.uri.to_string(),
        headers: parts.headers.clone(),
        body: String::new(),
    });
    let canned = state.canned.lock().await.clone();
    if canned.status == StatusCode::NOT_FOUND {
        return (canned.status, axum::Json(canned.body)).into_response();
    }
    (
        StatusCode::OK,
        axum::Json(json!({
            "resourceType": "ConceptMap",
            "id": id,
            "url": "http://example.org/cm/example",
            "version": "1.0.0",
            "name": "ExampleMap",
            "title": "Example Concept Map",
            "status": "active",
            "sourceUri": "http://example.org/vs/source",
            "targetUri": "http://example.org/vs/target"
        })),
    )
        .into_response()
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn parameter_names(parsed: &Value) -> Vec<String> {
    parsed
        .get("parameter")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn find_parameter<'a>(parsed: &'a Value, name: &str) -> Option<&'a Value> {
    parsed
        .get("parameter")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter().find(|p| {
                p.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n == name)
                    .unwrap_or(false)
            })
        })
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn browser_renders_full_page_with_translated_heading() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps")
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
        html.contains(">ConceptMaps<"),
        "browser heading must be Fluent-resolved (en value: ConceptMaps)",
    );
    assert!(
        html.contains("id=\"hts-cm-filters\""),
        "filter form must render (stable id anchor for tests)",
    );
    for key in [
        "hts-cm-browser-title",
        "hts-cm-browser-filter-search",
        "hts-cm-browser-column-url",
        "hts-cm-browser-load-more",
        "hts-cm-translate-heading",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked (missing Fluent value?)",
        );
    }
    assert!(
        html.contains("Terminology backend not fully available"),
        "closed-loopback upstream must render the degraded banner (en)",
    );
}

#[tokio::test]
async fn browser_rows_fragment_targets_and_varies_on_htmx_request() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps/rows")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let vary: Vec<String> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "rows fragment must add HX-Request to Vary; got: {vary:?}",
    );
    let html = body_text(response).await;
    assert!(
        html.contains("hts-cm-rows"),
        "rows fragment must render its stable outer id (found: {})",
        &html[..html.len().min(300)],
    );
}

#[tokio::test]
async fn browser_over_max_count_renders_invalid_input_outcome() {
    // Slice B invariant #1 (inherited by §7.5): `_count > MAX_COUNT`
    // renders an OperationOutcome above an empty table, HTTP 200 (not a
    // hard 400). The filter form's other values stay legible.
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps?_count=200")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "over-max _count must render the outcome partial in error severity",
    );
}

#[tokio::test]
async fn detail_renders_shell_and_degraded_on_upstream_failure() {
    // Closed-loopback upstream: `read_concept_map` fails with `Connect`.
    // The handler must degrade to the banner + shell rather than a 5xx.
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps/example-cm")
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
        "detail hard nav must render a full HTML page",
    );
    assert!(
        html.contains("Terminology backend not fully available"),
        "detail must render the degraded banner when upstream is unreachable",
    );
    assert!(
        html.contains("hts-cm-detail"),
        "detail scaffold section id must be present regardless of load result",
    );
}

#[tokio::test]
async fn detail_unknown_id_renders_outcome_inside_shell() {
    // §7.5 states matrix + Slice B invariant #5: HTS returns 404 for
    // both truly-missing and soft-deleted resources; the UI renders an
    // OperationOutcome inside the shell rather than a hard page 404.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::not_found()).await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps/no-such-cm")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "detail page must never surface a page 404; the outcome partial \
         is the operator-visible signal",
    );
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "unknown CM id must render the outcome partial in error severity",
    );
}

#[tokio::test]
async fn translate_tab_htmx_returns_input_partial_only() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps/example-cm/translate")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-cm-workbench__input"),
        "htmx tab load must return only the workbench input partial",
    );
    assert!(
        !html.contains("<!doctype html>"),
        "htmx tab load must not include the full page shell",
    );
    // Default direction is forward — the source coding group must be
    // present with `code` and `system` inputs.
    assert!(
        html.contains("name=\"code\"") && html.contains("name=\"system\""),
        "forward-direction default must render `code` and `system` inputs",
    );
    // §7.5 wire contract: the direction radios MUST carry
    // `hx-params="none"` so htmx does not double the trigger radio's
    // form value onto the `hx-get` URL. Without it the wire becomes
    // `?direction=reverse&direction=reverse` and axum's
    // `Query<TranslateInputForm>` rejects the duplicate scalar field
    // with HTTP 400, silently skipping the swap. Full wire trace in
    // `edson/docs/hts-ui-cm139-diagnosis.md`.
    assert!(
        html.contains("hx-params=\"none\""),
        "direction radios must set hx-params=\"none\" to avoid duplicating direction on the URL",
    );
}

#[tokio::test]
async fn translate_input_hx_reverse_direction_renders_target_code() {
    // Wire pin for the CM:139 bug: an htmx GET carrying
    // `?direction=reverse` MUST land 200 with the reverse fieldset
    // rendered (`translate-target-code` input, no `name="code"`
    // source-side input). If a future maintainer removes
    // `hx-params="none"` from the direction radios, the browser will
    // start sending `?direction=reverse&direction=reverse` and this
    // test still passes (query duplication is a client concern), but
    // a paired e2e (`concept-maps.spec.ts:139`) will catch that. Here
    // we lock the server-side contract so a plain-URL nav (bookmark,
    // hard nav, or `hx-params="none"` in effect) always resolves.
    let response = app()
        .oneshot(
            axum::http::Request::get(
                "/ui/hts/concept-maps/example-cm/translate?direction=reverse",
            )
            .header("HX-Request", "true")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("translate-target-code"),
        "reverse direction must render the `targetCode` input in the partial",
    );
    assert!(
        !html.contains("id=\"translate-code\""),
        "reverse direction must NOT render the forward-mode `code` input",
    );
}

#[tokio::test]
async fn translate_forward_posts_code_and_system_parameters() {
    // Forward direction sends `code` (valueCode) + `system` (valueUri).
    // Slice D wire contract per §7.5 + hts-details.md §`$translate`.
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse::ok_translate_equivalence())
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "direction=forward&code=A&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let captured = state.snapshot().await;
    let translate = captured
        .iter()
        .find(|c| c.path.contains("/$translate"))
        .expect("mock must have observed the translate POST");
    let body: Value =
        serde_json::from_str(&translate.body).expect("translate body must be JSON Parameters");
    let names = parameter_names(&body);
    assert!(
        names.contains(&"code".to_owned()),
        "forward mode must emit `code` (names seen: {names:?})",
    );
    assert!(
        names.contains(&"system".to_owned()),
        "forward mode must emit `system` (names seen: {names:?})",
    );
    assert!(
        !names.contains(&"reverse".to_owned()),
        "forward mode must NOT emit `reverse=true`",
    );
    assert!(
        !names.contains(&"targetCode".to_owned()),
        "forward mode must NOT emit `targetCode`",
    );
    // Values are wired correctly.
    let code = find_parameter(&body, "code")
        .and_then(|p| p.get("valueCode"))
        .and_then(|v| v.as_str());
    assert_eq!(code, Some("A"));
    let system = find_parameter(&body, "system")
        .and_then(|p| p.get("valueUri"))
        .and_then(|v| v.as_str());
    assert_eq!(system, Some("http://example.org/cs"));
}

#[tokio::test]
async fn translate_reverse_posts_target_code_parameter() {
    // Reverse direction sends `targetCode` (valueCode) + `reverse=true`
    // (valueBoolean). Source-side `code`/`system` do not appear.
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse::ok_translate_equivalence())
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("direction=reverse&targetCode=T1"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let captured = state.snapshot().await;
    let translate = captured
        .iter()
        .find(|c| c.path.contains("/$translate"))
        .expect("mock must have observed the translate POST");
    let body: Value = serde_json::from_str(&translate.body).unwrap();
    let names = parameter_names(&body);
    assert!(
        names.contains(&"targetCode".to_owned()),
        "reverse mode must emit `targetCode` (names seen: {names:?})",
    );
    assert!(
        names.contains(&"reverse".to_owned()),
        "reverse mode must emit `reverse=true` (names seen: {names:?})",
    );
    assert!(
        !names.contains(&"code".to_owned()),
        "reverse mode must NOT emit source-side `code`",
    );
    assert!(
        !names.contains(&"system".to_owned()),
        "reverse mode must NOT emit source-side `system`",
    );
    let target = find_parameter(&body, "targetCode")
        .and_then(|p| p.get("valueCode"))
        .and_then(|v| v.as_str());
    assert_eq!(target, Some("T1"));
}

#[tokio::test]
async fn translate_reverse_without_target_code_renders_inline_validation_outcome_without_posting_to_hts()
 {
    // §7.5 states matrix: reverse without `targetCode` renders an
    // inline validation `OperationOutcome` without hitting HTS. The
    // mock captures every incoming request, so the assertion is
    // "no `$translate` request was recorded".
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::server_error()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("direction=reverse"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "missing `targetCode` in reverse mode must render the invalid-input outcome",
    );

    let captured = state.snapshot().await;
    assert!(
        !captured.iter().any(|c| c.path.contains("/$translate")),
        "reverse validation gate MUST NOT round-trip to HTS; \
         captured requests: {:?}",
        captured.iter().map(|c| &c.path).collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn translate_forward_without_code_renders_inline_validation_outcome_without_posting_to_hts() {
    // §7.5 states matrix: forward without `code` (or `system`) fires
    // the same pre-flight validation gate as reverse missing
    // `targetCode`.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::server_error()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    // system present, code missing — still invalid
                    "direction=forward&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "missing `code` in forward mode must render the invalid-input outcome",
    );

    let captured = state.snapshot().await;
    assert!(
        !captured.iter().any(|c| c.path.contains("/$translate")),
        "forward validation gate MUST NOT round-trip to HTS; \
         captured requests: {:?}",
        captured.iter().map(|c| &c.path).collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn translate_no_matches_renders_neutral_state_not_error() {
    // §7.5 F11 realized for CM: HTTP 200 with `result=false` renders
    // the neutral no-matches label, NOT the shared error partial.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::no_matches()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "direction=forward&code=A&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-cm-workbench__no-matches"),
        "result=false must render the neutral no-matches state (class marker)",
    );
    assert!(
        !html.contains("hts-outcome--error"),
        "no-matches must NOT surface as an error outcome",
    );
}

#[tokio::test]
async fn translate_r4_response_labels_column_as_equivalence() {
    // Mapping-kind column reads whichever field name HTS returned.
    // R4/R4B emits `equivalence`, so the Fluent catalog resolves to
    // "Equivalence" (the English `column-mapping` selector).
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse::ok_translate_equivalence())
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .body(Body::from(
                    "direction=forward&code=A&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // §7.5 a11y contract: the mapping column's `aria-label` reflects
    // whichever field name HTS returned. R4/R4B → "Equivalence". We
    // pin on the attribute rather than the cell's visible text because
    // Askama emits `>` and content on separate lines and a literal
    // `>Equivalence<` substring would trip on whitespace.
    assert!(
        html.contains("aria-label=\"Equivalence\""),
        "R4/R4B response must label the mapping column as 'Equivalence' \
         (aria-label was expected; body excerpt: {})",
        &html[..html.len().min(800)],
    );
    // And the visible text should still be Equivalence — check the
    // Fluent-produced label appears at least once.
    assert!(
        html.contains("Equivalence"),
        "R4/R4B response must render the label 'Equivalence' in the grid",
    );
    // Neither the default `Mapping` nor the R5-only `Relationship` label
    // should leak into an R4-shaped response.
    assert!(
        !html.contains(">Relationship<") && !html.contains("aria-label=\"Relationship\""),
        "R4/R4B response must NOT surface 'Relationship' anywhere in the grid",
    );
    // Table renders (grid is present).
    assert!(
        html.contains("hts-cm-workbench__matches"),
        "R4/R4B success response must render the match grid",
    );
}

#[tokio::test]
async fn translate_r5_response_labels_column_as_relationship() {
    // R5/R6 emit `relationship`. Same Rust build compiled for R4 must
    // still label the column "Relationship" — the label is read from
    // the response, not a cfg (§7.5).
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse::ok_translate_relationship())
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .body(Body::from(
                    "direction=forward&code=A&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Mirror `translate_r4_response_labels_column_as_equivalence`: assert
    // on the aria-label so whitespace between `>` and the cell text
    // doesn't influence the check.
    assert!(
        html.contains("aria-label=\"Relationship\""),
        "R5/R6 response must label the mapping column as 'Relationship' \
         (aria-label was expected; body excerpt: {})",
        &html[..html.len().min(800)],
    );
    assert!(
        html.contains("Relationship"),
        "R5/R6 response must render the label 'Relationship' in the grid",
    );
    assert!(
        !html.contains("aria-label=\"Equivalence\""),
        "R5/R6 response must NOT surface 'Equivalence' in the grid header",
    );
}

#[tokio::test]
async fn translate_hts_error_renders_outcome_partial() {
    // §7.5 error state: 4xx / 5xx renders the shared `hts-outcome.html`
    // partial in the result region.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::server_error()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/concept-maps/example-cm/translate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "direction=forward&code=A&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-outcome hts-outcome--error"),
        "HTS 5xx must render the shared error outcome partial",
    );
}

#[tokio::test]
async fn translate_does_not_expose_unsupported_params() {
    // §7.5 explicit list: `version` (of the ConceptMap), `dependency`,
    // and lowercase `targetsystem` must never appear in the Translate
    // input form. Grep the rendered HTML to prove they leaked no
    // control (input, select, or textarea) that would let the operator
    // send them to HTS.
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/concept-maps/example-cm/translate")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;

    // A `name="version"` attribute would ship the ConceptMap version to
    // HTS. Slice D forbids exposing it (§7.5).
    assert!(
        !html.contains("name=\"version\""),
        "Translate form must NOT expose a `version` (of the ConceptMap) input",
    );
    // `dependency` and lowercase `targetsystem` never surface either.
    assert!(
        !html.contains("name=\"dependency\""),
        "Translate form must NOT expose a `dependency` input",
    );
    assert!(
        !html.contains("name=\"targetsystem\""),
        "Translate form must NOT expose a lowercase `targetsystem` input; \
         only camelCase `targetSystem` is accepted by HTS",
    );
    // Positive shape check: camelCase target is present.
    assert!(
        html.contains("name=\"targetSystem\""),
        "camelCase `targetSystem` MUST be present as the only spelling",
    );
}
