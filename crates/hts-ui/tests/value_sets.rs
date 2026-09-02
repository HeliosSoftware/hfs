//! ValueSet browser + detail + expand HTTP tests (Slice C).
//!
//! Two upstream fixtures cover the whole ring:
//!
//! 1. **Closed loopback** (`127.0.0.1:1`) — used by every test that only
//!    needs to observe a UI shell / degraded / OperationOutcome partial
//!    (the reads collapse to `UpstreamError::Connect`). This matches the
//!    Slice B `tests/code_systems.rs` shape and keeps most of the ring
//!    dependency-free.
//! 2. **In-process axum mock** — spun up per test on an ephemeral loopback
//!    port for the flows that assert HTTP-level behavior of the outgoing
//!    request: the tree/flat parameter mapping (§7.4.1 F7), the
//!    too-costly banner (§7.4 wireframe), and the pager (§7.4.1 F6). The
//!    mock captures headers + body per call and returns a canned
//!    response, so the ring pins the wire contract without depending on
//!    a real HTS.
//!
//! Closed-loopback tests keep the tight `100 ms / 250 ms` timeout envelope
//! from design doc §7.4.1 invariant #3 (matches `tests/code_systems.rs`);
//! mock-based tests use a more generous `2 s / 5 s` envelope so the
//! `tokio::spawn`ed `axum::serve` on a Windows current-thread `#[tokio::
//! test]` runtime has enough headroom to poll its accept before the client
//! side times out. `start_mock` pings a `/__mock_ready` probe to guarantee
//! the server is actually accepting before the base URL is returned.

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

/// Build a router pointed at `base_url` with the given upstream timeouts.
///
/// Split into two helpers below so mock-based tests (which need an actual
/// round-trip to succeed) can use generous timeouts while closed-loopback
/// tests keep tight ones — the latter *want* `Connect` to fire fast so the
/// whole matrix finishes in seconds.
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
        metrics_ring: Default::default(),
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

/// Router pointed at a real upstream (in-process mock). Generous timeouts
/// so a `tokio::spawn`ed `axum::serve` has time to accept its first
/// connection on Windows — the current-thread runtime that `#[tokio::test]`
/// defaults to gives no guarantees about when the server task first polls,
/// and 100 ms connect + 250 ms request is not enough headroom in practice.
fn app_pointing_at(base_url: &str) -> Router {
    app_with_timeouts(base_url, Duration::from_secs(5), Duration::from_secs(2))
}

/// Router pointed at a closed loopback port — the "Connect" fixture for
/// tests that only need to assert degraded / OperationOutcome shape.
/// Timeouts stay tight because the OS returns `ECONNREFUSED` immediately.
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

/// One captured request, in the order the mock saw it.
#[derive(Clone, Debug)]
#[allow(dead_code)] // `method` and `body` are inspected only when tests fail; kept for triage.
struct CapturedRequest {
    method: String,
    path: String,
    headers: HeaderMap,
    body: String,
}

/// Canned mock response. Keeps `Value` for JSON bodies + a status so
/// tests can dial in success + 422 too-costly + 404 unknown from the
/// same fixture.
#[derive(Clone)]
struct CannedResponse {
    status: StatusCode,
    body: Value,
}

impl CannedResponse {
    fn ok_expansion_flat() -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "ValueSet",
                "url": "http://example.org/vs/limbs",
                "expansion": {
                    "identifier": "test-expansion",
                    "total": 3,
                    "offset": 0,
                    "contains": [
                        {"code": "A", "system": "http://example.org/cs", "display": "Alpha"},
                        {"code": "B", "system": "http://example.org/cs", "display": "Bravo"},
                        {"code": "C", "system": "http://example.org/cs", "display": "Charlie"}
                    ]
                }
            }),
        }
    }

    fn ok_expansion_tree() -> Self {
        Self {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "ValueSet",
                "expansion": {
                    "contains": [{
                        "code": "root", "system": "http://example.org/cs", "display": "Root",
                        "contains": [
                            {"code": "leaf1", "system": "http://example.org/cs", "display": "Leaf 1"},
                            {"code": "leaf2", "system": "http://example.org/cs", "display": "Leaf 2"}
                        ]
                    }]
                }
            }),
        }
    }

    fn too_costly() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "too-costly",
                    "diagnostics": "expansion exceeds threshold"
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
                    "diagnostics": "unknown ValueSet"
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

async fn mock_handler(
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

/// Spin up an in-process mock HTS upstream on an ephemeral loopback
/// port. Returns the base URL (no trailing slash) and the shared
/// `MockState` so tests can peek at captured requests + swap the canned
/// response between calls.
///
/// The spawned server is bound to the current tokio runtime and cleans
/// up when the test's runtime shuts down (no explicit teardown needed).
///
/// A `__mock_ready` probe route is included so this helper can block
/// until `axum::serve` is actually accepting connections — on Windows
/// current-thread `#[tokio::test]` runtimes the spawned server task can
/// otherwise trail the first client request by several milliseconds.
async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        canned: Arc::new(Mutex::new(CannedResponse::ok_expansion_flat())),
    };
    let router: Router = Router::new()
        .route("/__mock_ready", get(|| async { (StatusCode::OK, "ok") }))
        .route("/ValueSet", get(mock_handler_get_search))
        .route("/ValueSet/{id}", get(mock_handler_get_id))
        .route("/ValueSet/{id}/$expand", post(mock_handler))
        // §8.2 canonical-url contract: expand_run prefers the type-level
        // endpoint when the resource read resolves a canonical url. The
        // seeded search Bundle already provides one, so the type-level
        // route is what tests actually exercise now — keep the instance
        // route above as a fallback that mirrors the pre-§8.2 path.
        .route("/ValueSet/$expand", post(mock_handler))
        // Fallback catches routing misses so an unexpected URL becomes a
        // captured request (with method GET-through-{method} inferred by
        // axum) rather than a silent 404 the test can't attribute.
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
    // Poll the ready probe until axum::serve is actually accepting. Two
    // seconds of headroom is plenty on Windows; the loop returns on the
    // first 200. If the probe never answers the tests will still fail
    // downstream — this helper just prevents a phantom timeout in the
    // very first client request.
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

/// Fallback handler for the mock — captures method + path so tests can
/// surface which URLs were sent but did not match a registered route.
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
    // Seed the mock with the `example-vs` entry so `read_value_set`'s
    // Alt-E two-hop (`resolve_canonical_url` + `fetch_by_url`) resolves
    // successfully — the detail page can render the workbench and the
    // §8.2 canonical-url fallback in `expand_run` picks the type-level
    // path instead of the instance-level fallback.
    (
        StatusCode::OK,
        axum::Json(json!({
            "resourceType": "Bundle",
            "entry": [
                {
                    "resource": {
                        "resourceType": "ValueSet",
                        "id": "example-vs",
                        "url": "http://example.org/vs/example",
                        "version": "1.0.0",
                        "name": "ExampleVS",
                        "title": "Example ValueSet",
                        "status": "active"
                    }
                }
            ]
        })),
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
    // For 404 tests, the canned response's status can override the read;
    // otherwise we return a minimal ValueSet keyed on the id.
    if canned.status == StatusCode::NOT_FOUND {
        return (canned.status, axum::Json(canned.body)).into_response();
    }
    (
        StatusCode::OK,
        axum::Json(json!({
            "resourceType": "ValueSet",
            "id": id,
            "url": "http://example.org/vs/limbs",
            "version": "1.0.0",
            "name": "Limbs",
            "title": "Limbs of the Body",
            "status": "active"
        })),
    )
        .into_response()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn browser_renders_full_page_with_translated_heading() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets")
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
        html.contains(">ValueSets<"),
        "browser heading must be Fluent-resolved (en value: ValueSets)",
    );
    assert!(
        html.contains("id=\"hts-vs-filters\""),
        "filter form must render (stable id anchor for tests)",
    );
    for key in [
        "hts-vs-browser-title",
        "hts-vs-browser-filter-reset",
        "hts-vs-browser-column-url",
        "hts-vs-browser-load-more",
        "hts-vs-expand-heading",
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
            axum::http::Request::get("/ui/hts/value-sets/rows")
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
        html.contains("hts-vs-rows"),
        "rows fragment must render its stable outer id (found: {})",
        &html[..html.len().min(300)],
    );
}

#[tokio::test]
async fn browser_over_max_count_renders_invalid_input_outcome() {
    // Mirror of the CS `_count > MAX` clamp (§7.4.1 invariant #1).
    // A pre-flight OperationOutcome is preferred over an HTTP 400 so the
    // filter form and its other values stay legible.
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets?_count=200")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains(r#"data-severity="error""#),
        "over-max _count must render the outcome partial in error severity",
    );
}

#[tokio::test]
async fn detail_renders_shell_and_degraded_on_upstream_failure() {
    // Closed-loopback upstream: `read_value_set` fails with `Connect`.
    // The handler must degrade to the banner + shell rather than a 5xx.
    //
    // §8.3: the naked `/{id}` URL now 308-redirects to `/{id}/expand`,
    // so this test hits the effective landing directly. The redirect
    // is covered by `detail_base_url_redirects_to_expand` below.
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/example-vs/expand")
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
        html.contains("hts-vs-detail"),
        "detail scaffold section id must be present regardless of load result",
    );
}

#[tokio::test]
async fn detail_base_url_redirects_to_expand() {
    // §8.3 operation-first landing: the naked `/ui/hts/value-sets/{id}`
    // URL 308-redirects to the default operation tab (`/{id}/expand`).
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/example-vs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/ui/hts/value-sets/example-vs/expand"),
    );
}

#[tokio::test]
async fn detail_unknown_id_renders_outcome_inside_shell() {
    // §7.4.1 invariant #5: HTS returns 404 for both truly-missing and
    // soft-deleted resources; the UI cannot tell them apart at the HTTP
    // layer. The detail handler renders an OperationOutcome inside the
    // shell rather than a hard page 404. This test uses the mock so we
    // can dial the upstream response to 404 (closed-loopback would
    // surface Connect + degraded, not the outcome path).
    //
    // §8.3: request targets `/{id}/expand` directly (naked `/{id}` now
    // 308-redirects; see `detail_base_url_redirects_to_expand`).
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::not_found()).await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/no-such-vs/expand")
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
        html.contains(r#"data-severity="error""#),
        "unknown VS id must render the outcome partial in error severity",
    );
}

#[tokio::test]
async fn expand_tab_htmx_returns_full_page_for_region_swap() {
    // Region-wrap contract (design doc §8.1): a tab-click GET returns
    // the full detail page; htmx uses `hx-select="#hts-vs-detail-region"`
    // to pick the tabs+workbench region out. Against the closed-loopback
    // upstream read_value_set fails so the outcome banner renders in
    // place of the workbench; the assertion is that the response is a
    // full HTML page (previously the input partial only).
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/example-vs/expand")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<!doctype html>") || html.contains("<!DOCTYPE html>"),
        "htmx tab load now returns the full page so htmx can hx-select the region",
    );
    assert!(
        html.contains("notice--warn"),
        "closed-loopback upstream must surface the degraded banner (Connect) or outcome banner (NotFound)",
    );
}

#[tokio::test]
async fn expand_input_shows_advanced_details_and_threshold_field() {
    // The Expand tab renders the always-visible controls + Advanced
    // <details> panel with the threshold input (§7.4 / §7.4.1 F1/F4).
    // Needs a mock upstream so read_value_set succeeds (§8.1 region-wrap
    // contract: the workbench renders only when detail is Ok).
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_flat()).await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/example-vs/expand")
                .header("HX-Request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains("name=\"filter\""),
        "filter input must be present"
    );
    assert!(
        html.contains("name=\"count\""),
        "count input must be present"
    );
    assert!(
        html.contains("name=\"threshold\""),
        "Advanced panel must expose a `threshold` numeric input",
    );
    // The V3 layout pass dropped every `hts-*` class hook (none of them had
    // a rule in app.css); the Advanced disclosure is now addressed by id.
    assert!(
        html.contains(r#"<details id="expand-advanced">"#),
        "Advanced <details> must render with its stable id",
    );
    assert!(
        html.contains("value=\"tree\"") && html.contains("value=\"flat\""),
        "tree/flat toggle must render both radio options",
    );
}

#[tokio::test]
async fn expand_tree_mode_sends_hierarchical_true_and_no_exclude_nested() {
    // §7.4.1 F7: tree ⇒ `hierarchical=true`, flat ⇒ `excludeNested=true`.
    // Never both — this test asserts the wire body directly.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_tree()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=tree&count=25&offset=0"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let captured = state.snapshot().await;
    let expand = captured
        .iter()
        .find(|c| c.path.contains("/$expand"))
        .expect("mock must have observed the expand POST");
    let body: Value =
        serde_json::from_str(&expand.body).expect("expand body must be JSON Parameters");
    let names: Vec<&str> = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .expect("parameter array")
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(
        names.contains(&"hierarchical"),
        "tree mode must emit `hierarchical` (names seen: {names:?})",
    );
    assert!(
        !names.contains(&"excludeNested"),
        "tree mode must NOT emit `excludeNested` (names seen: {names:?})",
    );
}

#[tokio::test]
async fn expand_flat_mode_sends_exclude_nested_true_and_no_hierarchical() {
    // Companion to the tree-mode test: same wire assertion, other direction.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_flat()).await;

    let _response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=25&offset=0"))
                .unwrap(),
        )
        .await
        .unwrap();

    let captured = state.snapshot().await;
    let expand = captured
        .iter()
        .find(|c| c.path.contains("/$expand"))
        .expect("mock must have observed the expand POST");
    let body: Value = serde_json::from_str(&expand.body).unwrap();
    let names: Vec<&str> = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(
        names.contains(&"excludeNested"),
        "flat mode must emit `excludeNested` (names seen: {names:?})",
    );
    assert!(
        !names.contains(&"hierarchical"),
        "flat mode must NOT emit `hierarchical` (names seen: {names:?})",
    );
}

#[tokio::test]
async fn expand_flat_renders_load_more_when_total_exceeds_page() {
    // §7.4.1 F6 pager rule: `remaining = expansion.total - expansion.offset
    //  - contains.len()`. Fixture returns 3 rows out of 3 total but with
    // count=2, so the terminal-page fallback also fires; using the
    // total-based path we set count=2 & total=5.
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "ValueSet",
                "expansion": {
                    "total": 5,
                    "offset": 0,
                    "contains": [
                        {"code": "A", "display": "A"},
                        {"code": "B", "display": "B"}
                    ]
                }
            }),
        })
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=2&offset=0"))
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains("hts-vs-expand-load-more") || html.contains(">Load more<"),
        "flat mode with expansion.total > offset+len must render [Load more]",
    );
    // Tree and flat now share one `.data-table`; depth is the only
    // difference and it is expressed as an inline `padding-left` on the
    // Code cell. A flat window has no depth, so no row may be indented.
    // Matched on the cell rather than the bare property: since #803 the raw
    // fold below the table renders a JSON view, whose gutter indents its own
    // lines with `padding-left` too.
    assert!(
        !html.contains(r#"<td style="padding-left"#),
        "flat mode result must not indent any row",
    );
}

#[tokio::test]
async fn expand_tree_hides_pager_and_labels_total_leaves() {
    // §7.4.1 F10: tree mode hides the pager and renders `showing full
    // tree {N}` — HTS ignores count / offset in tree mode.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_tree()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=tree&count=10&offset=0"))
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    // §7.4.1 F10 realized as an indented `.data-table` (the agreed
    // zero-CSS shape) rather than a `<ul role="tree">`: the flattened
    // rows carry their depth as an inline `padding-left` on the Code
    // cell. The tree fixture is one root with two children, so the
    // depth-1 indent must appear.
    assert!(
        html.contains(r#"style="padding-left: calc(14px + 1 * 20px)""#),
        "tree mode must indent child rows by depth in the shared data table",
    );
    assert!(
        html.contains("showing full tree"),
        "tree mode must render the `showing full tree {{N}}` label",
    );
    // The Load more button belongs strictly to flat mode.
    assert!(
        !html.contains(">Load more<"),
        "tree mode must NOT render [Load more]",
    );
}

#[tokio::test]
async fn expand_422_renders_too_costly_banner_with_raise_form() {
    // §7.4 wireframe: 422 too-costly renders a status banner containing
    // a compact "Raise threshold" form. The raise form's hidden field is
    // the same `threshold` key the Advanced panel writes to, so the
    // value survives a re-submit (§7.4.1 F1/F4).
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::too_costly()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=25&offset=0&threshold=1000"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains(r#"id="expand-too-costly""#),
        "422 must render the too-costly banner",
    );
    assert!(
        html.contains(r#"id="expand-too-costly-form""#),
        "banner must render the Raise-threshold form",
    );
    assert!(
        html.contains("value=\"1000\""),
        "the submitted threshold (1000) must echo back into the Raise form input",
    );
}

#[tokio::test]
async fn expand_threshold_below_ceiling_attaches_x_too_costly_header() {
    // §7.4 / §7.4.1 F1/F4: values at or below HTS_UI_MAX_EXPANSION_SIZE
    // _HINT are attached as `X-TOO-COSTLY-THRESHOLD`; values above are
    // dropped. This test hits the below-ceiling path.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_flat()).await;

    let _response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=25&offset=0&threshold=42"))
                .unwrap(),
        )
        .await
        .unwrap();

    let captured = state.snapshot().await;
    let expand = captured
        .iter()
        .find(|c| c.path.contains("/$expand"))
        .expect("mock must have observed the expand POST");
    let hdr = expand
        .headers
        .get("x-too-costly-threshold")
        .expect("threshold header must be attached for values below the ceiling");
    assert_eq!(hdr.to_str().unwrap(), "42");
}

#[tokio::test]
async fn expand_threshold_above_ceiling_drops_header_and_warns() {
    // §7.4 / §7.4.1 F1/F4 ceiling rule: the operator can enter values
    // above `HTS_UI_MAX_EXPANSION_SIZE_HINT`, but the UI drops them
    // from the outgoing request and renders a warning so the operator
    // sees why HTS did not honour the value.
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_flat()).await;

    let over = helios_hts_ui::HTS_UI_MAX_EXPANSION_SIZE_HINT + 1;
    let form_body = format!("mode=flat&count=25&offset=0&threshold={over}");

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(form_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let captured = state.snapshot().await;
    let expand = captured
        .iter()
        .find(|c| c.path.contains("/$expand"))
        .expect("mock must have observed the expand POST");
    assert!(
        expand.headers.get("x-too-costly-threshold").is_none(),
        "threshold above the ceiling must NOT be attached as a header",
    );

    let html = body_text(response).await;
    assert!(
        html.contains(r#"id="expand-ceiling-warning""#),
        "requests above the ceiling must render the ceiling-warning banner",
    );
}

#[tokio::test]
async fn expand_no_members_renders_neutral_state() {
    // §7.4.1 F11 companion: empty `contains` without a filter renders
    // the `no-members` neutral state, not an error.
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "ValueSet",
                "expansion": { "total": 0, "offset": 0, "contains": [] }
            }),
        })
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=25&offset=0"))
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains(r#"id="expand-no-members""#),
        "empty expansion without filter must render the no-members neutral state",
    );
    assert!(
        !html.contains(r#"data-severity="error""#),
        "no-members must NOT surface as an error outcome",
    );
}

#[tokio::test]
async fn expand_filter_no_match_renders_neutral_state_with_filter() {
    // §7.4 states matrix: empty `contains` WITH a filter renders the
    // filter-no-match neutral state (still not an error) and echoes the
    // filter string in the label.
    let (base, state) = start_mock().await;
    state
        .set_canned(CannedResponse {
            status: StatusCode::OK,
            body: json!({
                "resourceType": "ValueSet",
                "expansion": { "total": 0, "offset": 0, "contains": [] }
            }),
        })
        .await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri("/ui/hts/value-sets/example-vs/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("mode=flat&count=25&offset=0&filter=xyzzy"))
                .unwrap(),
        )
        .await
        .unwrap();
    let html = body_text(response).await;
    assert!(
        html.contains(r#"id="expand-filter-no-match""#),
        "empty expansion with filter must render the filter-no-match neutral state",
    );
    assert!(
        html.contains("xyzzy"),
        "filter-no-match label must echo the submitted filter",
    );
}

// Compile-time sanity check that the exported constant exists — a link
// error here means Slice C removed the shared threshold ceiling.
#[allow(dead_code)]
const _: u64 = helios_hts_ui::HTS_UI_MAX_EXPANSION_SIZE_HINT;

// ── V2 "top strip" browser layout (#551 browser redesign) ───────────────
//
// Mirror of `code_systems.rs`: the two-column `.filter-layout--two` rail
// is gone, replaced by a `.toolbar` filter strip + `.facets.facets--bare`
// status chips over a full-width table.

#[tokio::test]
async fn browser_renders_v2_top_strip_shell() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    for hook in [
        r#"<body class="app-shell">"#,
        r#"class="content content--app""#,
        r#"class="card table-card""#,
        r#"class="toolbar__search""#,
        r#"class="facets facets--bare""#,
        r#"class="chip""#,
    ] {
        assert!(
            html.contains(hook),
            "V2 top-strip layout must render `{hook}`",
        );
    }
    for dead in [
        "filter-rail",
        "filter-layout",
        "content--wide",
        "btn--ghost",
        "btn--secondary",
        r#"class="hts-degraded""#,
    ] {
        assert!(
            !html.contains(dead),
            "`{dead}` has no rule in app.css and must not be rendered",
        );
    }
    assert!(
        html.contains(r#"<aside class="notice notice--warn""#),
        "the degraded banner must use the shared `.notice.notice--warn` skin",
    );
}

#[tokio::test]
async fn status_chips_mark_the_active_facet_and_carry_text_filters() {
    let response = app()
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets?title=gender&status=active")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(
            r#"href="/ui/hts/value-sets?title=gender&#38;status=active" aria-current="true""#
        ),
        "the active status chip must carry aria-current=\"true\"",
    );
    assert_eq!(
        html.matches(r#"class="chip" href"#).count(),
        5,
        "five chips: any + draft/active/retired/unknown",
    );
    assert!(
        html.contains(r#"href="/ui/hts/value-sets?title=gender""#),
        "the `any status` chip must drop only `status`, keeping `title`",
    );
    assert!(
        html.contains(r#"<input type="hidden" name="status" value="active">"#),
        "the active status must ride along with the filter form",
    );
}

// ── Static guard: the VS detail surface introduces no CSS of its own ────

/// Strip Askama comments (`{# … #}`) from a template source, so class
/// names quoted in prose (including ones the template deliberately does
/// NOT use, like `.addbox`) are not mistaken for rendered markup.
fn strip_template_comments(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{#") {
        out.push_str(&rest[..start]);
        match rest[start..].find("#}") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Every class the ValueSet detail templates use must already have a rule
/// in the shared `crates/ui/assets/app.css`. The V3 layout pass ships zero
/// new CSS, so an unmatched class is a typo or a reintroduced HTS-only hook.
#[test]
fn vs_detail_templates_only_use_classes_that_exist_in_app_css() {
    const APP_CSS: &str = include_str!("../../ui/assets/app.css");
    let templates = [
        (
            "pages/vs-detail.html",
            include_str!("../templates/pages/vs-detail.html"),
        ),
        (
            "partials/hts-vs-expand-input.html",
            include_str!("../templates/partials/hts-vs-expand-input.html"),
        ),
        (
            "partials/hts-vs-expand-result.html",
            include_str!("../templates/partials/hts-vs-expand-result.html"),
        ),
    ];

    let mut checked = 0usize;
    for (name, template) in templates {
        let body = strip_template_comments(template);
        for chunk in body.split(r#"class=""#).skip(1) {
            let value = chunk.split('"').next().unwrap_or_default();
            if value.contains('{') {
                continue;
            }
            for class in value.split_whitespace() {
                assert!(
                    APP_CSS.contains(&format!(".{class}")),
                    "class `{class}` used by {name} has no rule in crates/ui/assets/app.css",
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 30,
        "expected the scan to reach the templates' class attributes, only saw {checked}",
    );
}

/// The V3 compact header on the ValueSet detail page. Since #801 it also
/// takes the HFS back-link idiom: a `.page-head--back-link` modifier, a
/// leading `.back-link`, and the rest of the head wrapped in
/// `.page-head__copy`.
#[test]
fn vs_detail_page_uses_the_v3_compact_header_shape() {
    const PAGE: &str = include_str!("../templates/pages/vs-detail.html");
    let body = strip_template_comments(PAGE);

    for hook in [
        r#"<header class="page-head page-head--back-link">"#,
        r#"class="back-link""#,
        r#"class="page-head__copy""#,
        r#"class="page-head__title""#,
        r#"class="facets facets--bare""#,
        r#"class="detail__field detail__field--wide""#,
        r#"<summary class="field__label">"#,
    ] {
        assert!(
            body.contains(hook),
            "V3 compact header must render `{hook}`"
        );
    }
    for dead in [
        "page-header",
        "addbox",
        "hts-vs-detail__",
        "backlink",
        "row-link",
        "<dl",
    ] {
        assert!(
            !body.contains(dead),
            "`{dead}` belongs to the pre-V3 stacked layout and must be gone",
        );
    }
    assert!(
        !body.contains("<details open"),
        "the facts disclosure must render collapsed",
    );
}

/// Runtime companion to the source-shape guard: with a real summary in
/// hand, the V3 compact header must actually render — chip row, canonical
/// URL field, and the collapsed disclosure — and the workbench card must
/// follow it directly, with no full-width facts card in between.
#[tokio::test]
async fn vs_detail_renders_the_compact_header_from_a_live_summary() {
    let (base, state) = start_mock().await;
    state.set_canned(CannedResponse::ok_expansion_flat()).await;

    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::get("/ui/hts/value-sets/example-vs/expand")
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    for hook in [
        r#"<header class="page-head page-head--back-link">"#,
        r#"class="facets facets--bare""#,
        ">Facts<",
        "http://example.org/vs/example",
        r#"<summary class="field__label">All ValueSet facts</summary>"#,
    ] {
        assert!(
            html.contains(hook),
            "the rendered detail page must carry `{hook}`",
        );
    }
    // Version and status arrive as chips in the head, not as a stacked
    // definition list below it.
    assert!(
        html.contains(r#"<span class="count">1.0.0</span>"#),
        "the version chip must carry the live version",
    );
    // The workbench form must come before the raw-facts disclosure is
    // ever expanded — i.e. it is high on the page, not buried under a
    // full-width facts card.
    let head = html
        .find(r#"<header class="page-head page-head--back-link">"#)
        .expect("page head present");
    let workbench = html
        .find(r#"id="hts-workbench-input""#)
        .expect("workbench form present");
    let disclosure = html
        .find(r#"<summary class="field__label">All ValueSet facts</summary>"#)
        .expect("facts disclosure present");
    assert!(head < disclosure && disclosure < workbench);
}
