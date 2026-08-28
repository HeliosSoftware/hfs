//! Concept information plane HTTP tests (Direction B).
//!
//! Two upstream fixtures, matching the shape `tests/concept_maps.rs`
//! established:
//!
//! 1. **Closed loopback** (`127.0.0.1:1`) for the shell / degraded / pre-flight
//!    surfaces, where every upstream read collapses to `UpstreamError::Connect`
//!    and the assertion is about the page rendering at all.
//! 2. **In-process axum mock** on an ephemeral port for the flows that pin the
//!    *wire* contract: the `url`-less cross-map `$translate`, the reverse-mode
//!    parameter shape, and the `$lookup` / `$subsumes` disagreement that the
//!    subsumption panel exists to surface. The mock captures every request so
//!    the tests can assert what was — and was not — sent.
//!
//! Timeout envelope mirrors the sibling rings: closed loopback keeps
//! `100 ms / 250 ms`; the mock gets `2 s / 5 s` of headroom for a spawned
//! `axum::serve` to accept its first connection on Windows current-thread
//! `#[tokio::test]` runtimes. `start_mock` polls `/__mock_ready` first so no
//! test races the accept.

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower::ServiceExt;

// ── Shared fixtures ─────────────────────────────────────────────────────

fn app_with_timeouts(base_url: &str, request: Duration, connect: Duration) -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(base_url, request, connect)
            .expect("test upstream base URL parses"),
        bundled_data_bytes: None,
        // The concept plane never touches the Home chart's sample ring.
        metrics_ring: Default::default(),
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

/// Router pointed at the in-process mock.
///
/// Timeouts are deliberately generous — far beyond the sibling rings' 5 s.
/// The subsumption panel fans out up to 20 upstream calls, and both the mock
/// server and the client share one current-thread `#[tokio::test]` runtime; when
/// the whole crate's eight test binaries run in parallel, a 5 s ceiling turns
/// scheduler pressure into a spurious `Timeout` and the assertion then blames
/// the wrong thing. Nothing here should ever take seconds, so a real hang
/// surfaces as a harness timeout rather than a misleading failure.
fn app_pointing_at(base_url: &str) -> Router {
    app_with_timeouts(base_url, Duration::from_secs(30), Duration::from_secs(10))
}

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

/// The concept address used throughout, already percent-encoded the way a
/// pasted permalink arrives. `%3A%2F%2F` is the whole reason this route is
/// query-shaped instead of path-shaped.
const ENCODED_ADDRESS: &str = "system=http%3A%2F%2Fexample.org%2Fcs&code=A01.0";

// ── In-process mock upstream ────────────────────────────────────────────

#[derive(Clone, Debug)]
#[allow(dead_code)] // fields inspected only when a test fails; kept for triage
struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

impl CapturedRequest {
    fn parsed(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    lookup: Arc<Mutex<Value>>,
    translate_forward: Arc<Mutex<Value>>,
    translate_reverse: Arc<Mutex<Value>>,
    subsumes_outcome: Arc<Mutex<String>>,
}

impl MockState {
    async fn snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().await.clone()
    }

    async fn calls_to(&self, needle: &str) -> Vec<CapturedRequest> {
        self.snapshot()
            .await
            .into_iter()
            .filter(|c| c.path.contains(needle))
            .collect()
    }
}

/// `$lookup` with the synthesised `parent` / `child` / `inactive` properties
/// the `property=*` wildcard produces (see `crates/hts/src/operations/lookup.rs`).
fn lookup_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "name", "valueString": "Example CS"},
            {"name": "version", "valueString": "1.0.0"},
            {"name": "display", "valueString": "Typhoid meningitis"},
            {"name": "definition", "valueString": "A meningitis caused by typhoid."},
            {"name": "system", "valueUri": "http://example.org/cs"},
            {"name": "code", "valueCode": "A01.0"},
            {"name": "property", "part": [
                {"name": "code", "valueCode": "parent"},
                {"name": "value", "valueCode": "A01"},
                {"name": "description", "valueString": "Typhoid fever"}
            ]},
            {"name": "property", "part": [
                {"name": "code", "valueCode": "child"},
                {"name": "value", "valueCode": "A01.00"}
            ]},
            {"name": "property", "part": [
                {"name": "code", "valueCode": "inactive"},
                {"name": "value", "valueBoolean": false}
            ]}
        ]
    })
}

/// Forward cross-map `$translate`: two matches from two different ConceptMaps.
/// `originMap` is emitted as `valueCanonical`, which is what `translate.rs`
/// actually sends.
fn translate_forward_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "match", "part": [
                {"name": "concept", "valueCoding": {
                    "system": "http://example.org/target-a", "code": "TA1", "display": "Target A1"
                }},
                {"name": "equivalence", "valueCode": "equivalent"},
                {"name": "originMap", "valueCanonical": "http://example.org/cm/alpha|1.0"}
            ]},
            {"name": "match", "part": [
                {"name": "concept", "valueCoding": {
                    "system": "http://example.org/target-b", "code": "TB9"
                }},
                {"name": "equivalence", "valueCode": "wider"},
                {"name": "originMap", "valueCanonical": "http://example.org/cm/beta|2.0"}
            ]},
            {"name": "result", "valueBoolean": true}
        ]
    })
}

/// Reverse-mode `$translate`. `translate.rs` suppresses `originMap` here
/// (`if !is_reverse`), so the matches arrive unattributable — the exact state
/// the panel must render honestly rather than invent an origin for.
fn translate_reverse_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "match", "part": [
                {"name": "concept", "valueCoding": {
                    "system": "http://example.org/source-a", "code": "SA1"
                }},
                {"name": "equivalence", "valueCode": "equivalent"},
                {"name": "source", "valueCoding": {
                    "system": "http://example.org/source-a", "code": "SA1"
                }}
            ]},
            {"name": "result", "valueBoolean": true}
        ]
    })
}

/// `result: false` at HTTP 200 — a neutral empty state, never an error.
fn translate_no_match_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "result", "valueBoolean": false},
            {"name": "message", "valueString": "no mapping found"}
        ]
    })
}

async fn capture(state: &MockState, method: &Method, req: Request<Body>) -> Value {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .unwrap_or_default();
    let body_str = String::from_utf8_lossy(&bytes).into_owned();
    state.captured.lock().await.push(CapturedRequest {
        method: method.to_string(),
        path: parts.uri.to_string(),
        body: body_str.clone(),
    });
    serde_json::from_str(&body_str).unwrap_or(Value::Null)
}

async fn mock_lookup(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    capture(&state, &method, req).await;
    let body = state.lookup.lock().await.clone();
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn mock_translate(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    let parsed = capture(&state, &method, req).await;
    let is_reverse = parameter_names(&parsed).iter().any(|n| n == "reverse");
    let body = if is_reverse {
        state.translate_reverse.lock().await.clone()
    } else {
        state.translate_forward.lock().await.clone()
    };
    (StatusCode::OK, axum::Json(body)).into_response()
}

async fn mock_subsumes(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    capture(&state, &method, req).await;
    let outcome = state.subsumes_outcome.lock().await.clone();
    (
        StatusCode::OK,
        axum::Json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "outcome", "valueCode": outcome}]
        })),
    )
        .into_response()
}

async fn mock_cs_search(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    capture(&state, &method, req).await;
    (
        StatusCode::OK,
        axum::Json(json!({
            "resourceType": "Bundle",
            "entry": [{"resource": {
                "resourceType": "CodeSystem",
                "id": "example",
                "url": "http://example.org/cs",
                "version": "1.0.0",
                "name": "ExampleCS",
                "title": "Example CS",
                "status": "active",
                "content": "complete"
            }}]
        })),
    )
        .into_response()
}

async fn mock_fallback(
    State(state): State<MockState>,
    method: Method,
    req: Request<Body>,
) -> axum::response::Response {
    capture(&state, &method, req).await;
    (StatusCode::NOT_FOUND, "").into_response()
}

async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        lookup: Arc::new(Mutex::new(lookup_body())),
        translate_forward: Arc::new(Mutex::new(translate_forward_body())),
        translate_reverse: Arc::new(Mutex::new(translate_reverse_body())),
        subsumes_outcome: Arc::new(Mutex::new("subsumes".to_owned())),
    };
    let router: Router = Router::new()
        .route("/__mock_ready", get(|| async { (StatusCode::OK, "ok") }))
        // The CodeSystem workbench resolves a canonical url via two search
        // hops before it calls the type-level `$lookup` (§8.2). Seeding the
        // search keeps the entry-point test on the same path production takes.
        .route("/CodeSystem", get(mock_cs_search))
        .route("/CodeSystem/$lookup", post(mock_lookup))
        .route("/CodeSystem/$subsumes", post(mock_subsumes))
        .route("/ConceptMap/$translate", post(mock_translate))
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
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
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

// ── Helpers ─────────────────────────────────────────────────────────────

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

async fn fetch(app: Router, uri: &str, htmx: bool) -> axum::response::Response {
    let mut builder = axum::http::Request::get(uri).header(header::ACCEPT_LANGUAGE, "en");
    if htmx {
        builder = builder.header("HX-Request", "true");
    }
    app.oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

// ── Shell + address contract ────────────────────────────────────────────

#[tokio::test]
async fn hard_navigation_renders_a_full_page_with_all_three_panels() {
    let (base, _state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts?{ENCODED_ADDRESS}"),
        false,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("<!doctype html>"),
        "hard nav must render a full HTML page",
    );
    for id in [
        "id=\"hts-concept-identity\"",
        "id=\"hts-concept-mappings\"",
        "id=\"hts-concept-relations\"",
    ] {
        assert!(html.contains(id), "page must carry the panel anchor {id}");
    }
    // Identity is server-rendered in the shell: first paint must be
    // meaningful, so the concept's display has to be in the initial HTML.
    assert!(
        html.contains("Typhoid meningitis"),
        "identity must render server-side in the shell",
    );
    assert!(
        html.contains("<code>http://example.org/cs</code>")
            && html.contains("<code>A01.0</code>"),
        "the address must render as the identity panel's first two facts",
    );
    // `property=*` synthesises parent / child / inactive; the identity panel
    // separates them from the plain property list.
    assert!(
        html.contains("Parent A01") && html.contains("Child A01.00"),
        "hierarchy neighbours must render as their own row, not as properties",
    );
    assert!(
        html.contains(">Active<"),
        "a reported `inactive: false` must render as an Active chip (en)",
    );
    // The two lazy panels ship as self-fetching skeletons, and each carries a
    // <noscript> escape hatch to its standalone route.
    assert!(
        html.contains("hx-trigger=\"load\""),
        "lazy panels must self-fetch on load",
    );
    assert!(
        html.contains("<noscript>"),
        "each skeleton must carry a nojs link to its standalone route",
    );
    for key in [
        "hts-concept-title",
        "hts-concept-identity-heading",
        "hts-concept-field-system",
        "hts-concept-mappings-heading",
        "hts-concept-relations-heading",
    ] {
        assert!(
            !html.contains(key),
            "raw catalog key `{key}` leaked (missing Fluent value?)",
        );
    }
}

#[tokio::test]
async fn htmx_request_to_a_panel_returns_a_bare_fragment() {
    let (base, _state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/mappings?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let vary: Vec<String> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap_or_default().to_ascii_lowercase())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("hx-request")),
        "panel fragment must add HX-Request to Vary; got {vary:?}",
    );

    let html = body_text(response).await;
    assert!(
        !html.contains("<!doctype html>") && !html.contains("<html"),
        "an HX-Request must return a fragment, not a page: {}",
        &html[..html.len().min(200)],
    );
    assert!(
        html.contains("id=\"hts-concept-mappings\""),
        "the fragment must re-emit the panel id so the outerHTML swap lands",
    );
    assert!(
        !html.contains("hx-trigger=\"load\""),
        "the loaded fragment must NOT re-emit the load trigger, or it loops",
    );
}

#[tokio::test]
async fn standalone_panel_route_renders_a_full_page_on_hard_navigation() {
    let (base, _state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}"),
        false,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("<!doctype html>"),
        "the noscript path must land on a real page",
    );
    assert!(
        html.contains("id=\"hts-concept-relations\""),
        "the focused panel must be present",
    );
}

#[tokio::test]
async fn a_half_typed_permalink_renders_an_invalid_outcome_not_a_500() {
    // The permalink is something people paste out of tickets, so a missing
    // half has to explain itself rather than 400/500 in the network tab.
    for query in ["", "code=A01.0", "system=http%3A%2F%2Fexample.org%2Fcs"] {
        let response = fetch(app(), &format!("/ui/hts/concepts?{query}"), false).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "`?{query}` must render, not error",
        );
        let html = body_text(response).await;
        assert!(
            html.contains(r#"data-severity="error""#),
            "`?{query}` must render the outcome partial in error severity",
        );
        assert!(
            html.contains("The request was rejected as invalid."),
            "`?{query}` must render the `invalid` issue code (en)",
        );
        assert!(
            html.contains("Missing required query parameter"),
            "`?{query}` must name which half of the address is missing",
        );
    }
}

#[tokio::test]
async fn upstream_failure_degrades_instead_of_erroring() {
    // Closed loopback: every panel read is `UpstreamError::Connect`. The page
    // must still be a page.
    let response = fetch(app(), &format!("/ui/hts/concepts?{ENCODED_ADDRESS}"), false).await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(html.contains("<!doctype html>"));
    // The exact reason depends on how the OS reports a closed port (refused vs
    // timed out), so assert on the banner and the reason family, not one
    // sentence.
    assert!(
        html.contains("notice notice--warn"),
        "an unreachable upstream must render the degraded banner; got:\n{html}",
    );
    assert!(
        html.contains("Could not reach the terminology server.")
            || html.contains("The terminology server did not respond in time."),
        "the banner must carry a translated degraded reason (en); got:\n{html}",
    );
}

#[tokio::test]
async fn percent_encoded_system_uri_round_trips_to_the_wire_and_back() {
    let (base, state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts?{ENCODED_ADDRESS}"),
        false,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Rendered back into the Identity panel un-mangled...
    assert!(
        html.contains("<code>http://example.org/cs</code>"),
        "the decoded canonical URI must render in the identity panel",
    );
    // ...and re-encoded (not concatenated raw) into every outgoing panel link.
    assert!(
        html.contains("system=http%3A%2F%2Fexample.org%2Fcs"),
        "panel links must percent-encode the system through form_urlencoded",
    );

    // And it reached HTS as a real URI, not a mangled path segment.
    let lookups = state.calls_to("$lookup").await;
    assert_eq!(lookups.len(), 1, "exactly one $lookup for the shell");
    let parsed = lookups[0].parsed();
    assert_eq!(
        find_parameter(&parsed, "system").and_then(|p| p.get("valueUri")),
        Some(&json!("http://example.org/cs")),
    );
    assert_eq!(
        find_parameter(&parsed, "code").and_then(|p| p.get("valueCode")),
        Some(&json!("A01.0")),
    );
    // `property=*` is what makes HTS synthesise parent / child / inactive.
    assert_eq!(
        find_parameter(&parsed, "property").and_then(|p| p.get("valueCode")),
        Some(&json!("*")),
    );
}

// ── Panel 2: mappings across every ConceptMap ───────────────────────────

#[tokio::test]
async fn cross_map_translate_omits_url_and_groups_matches_by_origin_map() {
    let (base, state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/mappings?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // Omitting `url` is what makes HTS scan every stored map instead of one.
    let translates = state.calls_to("$translate").await;
    assert_eq!(translates.len(), 1, "one cross-map $translate per render");
    let sent = parameter_names(&translates[0].parsed());
    assert!(
        !sent.iter().any(|n| n == "url"),
        "the `url` parameter must be OMITTED so the scan spans every map; sent {sent:?}",
    );
    // ValueSet scoping is parsed by the backend but never bound by the SQL, so
    // it must not be sent — a control that does nothing is worse than none.
    assert!(
        !sent.iter().any(|n| n == "source" || n == "target"),
        "source/target are unbound by the backend and must not be sent; sent {sent:?}",
    );

    // Both origin maps are rendered, so the operator can see which map each
    // mapping came from.
    for origin in [
        "http://example.org/cm/alpha|1.0",
        "http://example.org/cm/beta|2.0",
    ] {
        assert!(
            html.contains(origin),
            "matches must be grouped under their originMap; missing {origin}",
        );
    }
    assert!(html.contains("TA1") && html.contains("TB9"));
    // Forward mode attributes every match, so no unattributable footnote.
    assert!(
        !html.contains("does not attribute reverse-mode matches"),
        "forward mode attributes its matches; the caveat must stay hidden",
    );
}

#[tokio::test]
async fn reverse_mode_sends_target_code_and_flags_matches_as_unattributable() {
    let (base, state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/mappings?{ENCODED_ADDRESS}&direction=reverse"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    let translates = state.calls_to("$translate").await;
    assert_eq!(translates.len(), 1);
    let parsed = translates[0].parsed();
    // `translate_sync` keys the reverse search off targetCode + targetSystem.
    assert_eq!(
        find_parameter(&parsed, "reverse").and_then(|p| p.get("valueBoolean")),
        Some(&json!(true)),
    );
    assert_eq!(
        find_parameter(&parsed, "targetCode").and_then(|p| p.get("valueCode")),
        Some(&json!("A01.0")),
    );
    assert_eq!(
        find_parameter(&parsed, "targetSystem").and_then(|p| p.get("valueUri")),
        Some(&json!("http://example.org/cs")),
    );
    assert!(
        !parameter_names(&parsed).iter().any(|n| n == "url"),
        "reverse mode is still a cross-map scan; `url` must stay omitted",
    );

    // The backend suppresses originMap in reverse mode, so the panel must say
    // so instead of inventing an origin.
    assert!(
        html.contains("lint__item--warn"),
        "the unattributable state must raise a warn-level caveat",
    );
    assert!(
        html.contains("does not attribute reverse-mode matches"),
        "the caveat must explain why the origin column is empty (en)",
    );
    assert!(
        html.contains("&mdash;"),
        "the Origin cell must render an em-dash, never a fabricated origin",
    );
    assert!(
        !html.contains("http://example.org/cm/"),
        "no origin may be invented for a reverse-mode match",
    );
}

#[tokio::test]
async fn no_match_is_http_200_with_a_neutral_empty_state() {
    let (base, state) = start_mock().await;
    *state.translate_forward.lock().await = translate_no_match_body();

    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/mappings?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "`result: false` is a 200, not an error status",
    );
    let html = body_text(response).await;
    assert!(
        html.contains("query-empty"),
        "no-match must render the neutral empty state",
    );
    assert!(
        html.contains("No ConceptMap maps this concept."),
        "the empty state must be a translated sentence (en)",
    );
    assert!(
        !html.contains(r#"data-severity="error""#),
        "no-match must NOT render as an error outcome",
    );
}

// ── Panel 3: subsumption ────────────────────────────────────────────────

#[tokio::test]
async fn derived_comparators_always_send_the_ancestor_candidate_as_code_a() {
    let (base, state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    // One call per comparator: the parent (A01) and the child (A01.00) that
    // `$lookup` reported.
    let calls = state.calls_to("$subsumes").await;
    assert_eq!(calls.len(), 2, "one $subsumes per derived comparator");

    let mut pairs: Vec<(String, String)> = calls
        .iter()
        .map(|c| {
            let parsed = c.parsed();
            let s = |name: &str| {
                find_parameter(&parsed, name)
                    .and_then(|p| p.get("valueCode"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            // The system is pinned server-side from the concept address, which
            // is why the cross-system 400 arm is unreachable here.
            assert_eq!(
                find_parameter(&parsed, "system").and_then(|p| p.get("valueUri")),
                Some(&json!("http://example.org/cs")),
            );
            (s("codeA"), s("codeB"))
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            // parent: is A01 an ancestor of A01.0?
            ("A01".to_owned(), "A01.0".to_owned()),
            // child: is A01.0 an ancestor of A01.00?
            ("A01.0".to_owned(), "A01.00".to_owned()),
        ],
        "the ancestor candidate must always be codeA",
    );

    // The healthy answer is `subsumes` for both, so no conflict is raised.
    assert!(
        !html.contains("tag--conflict"),
        "an agreeing hierarchy must not be flagged as a conflict",
    );
    assert!(
        !html.contains("lint__item--warn"),
        "an agreeing hierarchy must not raise the caveat",
    );
    // Informational table, never a graph.
    assert!(html.contains("data-table"));
    assert!(!html.contains("<svg"));
}

#[tokio::test]
async fn lookup_and_subsumes_disagreement_is_surfaced_as_a_conflict() {
    // The reachable bug this panel exists for: re-importing a hierarchical
    // CodeSystem wipes `concept_closure` while `concept_hierarchy` survives,
    // so `$lookup` keeps reporting `parent=A01` after `$subsumes` has stopped
    // agreeing. Only a *derived* comparator can expose it.
    let (base, state) = start_mock().await;
    *state.subsumes_outcome.lock().await = "not-subsumed".to_owned();

    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains("tag--conflict"),
        "the disagreeing row must be tagged as a conflict",
    );
    assert!(
        html.contains("lint__item--warn"),
        "the disagreement must raise exactly one warn-level caveat",
    );
    assert_eq!(
        html.matches("lint__item--warn").count(),
        1,
        "one caveat for the panel, not one per row",
    );
    assert!(
        html.contains("subsumption closure was not rebuilt"),
        "the caveat must name the likely cause (en)",
    );
}

#[tokio::test]
async fn a_system_qualified_comparator_is_pre_flighted_not_round_tripped() {
    // The system is pinned server-side, so a pasted `system|code` would be
    // sent as a *code* and come back as a confusing 404. Catch the shape here.
    let (base, state) = start_mock().await;
    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}&compare=http%3A%2F%2Fexample.org%2Fcs%7CA02"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(
        html.contains(r#"data-severity="error""#),
        "a malformed comparator must render an outcome",
    );
    assert!(
        html.contains("system-qualified reference"),
        "the outcome must explain what is wrong with the input",
    );
    // Only the two derived comparators reached HTS; the bad one never did.
    let calls = state.calls_to("$subsumes").await;
    assert_eq!(
        calls.len(),
        2,
        "the rejected comparator must not produce an upstream call",
    );
}

#[tokio::test]
async fn a_valid_manual_comparator_adds_one_row_without_an_expectation() {
    let (base, state) = start_mock().await;
    *state.subsumes_outcome.lock().await = "not-subsumed".to_owned();
    // Only a manual comparator, so nothing derived can raise a conflict.
    *state.lookup.lock().await = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "system", "valueUri": "http://example.org/cs"},
            {"name": "code", "valueCode": "A01.0"}
        ]
    });

    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}&compare=A02"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    let calls = state.calls_to("$subsumes").await;
    assert_eq!(calls.len(), 1, "one call for the manual comparator");
    let parsed = calls[0].parsed();
    assert_eq!(
        find_parameter(&parsed, "codeA").and_then(|p| p.get("valueCode")),
        Some(&json!("A02")),
        "the manual comparator is asked whether it subsumes this concept",
    );

    // A manual comparator carries no expectation, so `not-subsumed` is an
    // answer, not a contradiction.
    assert!(
        !html.contains("tag--conflict"),
        "a manual comparator can never be a conflict",
    );
    assert!(
        !html.contains("lint__item--warn"),
        "a manual comparator must not raise the closure caveat",
    );
}

#[tokio::test]
async fn a_concept_with_no_neighbours_renders_a_neutral_empty_table() {
    let (base, state) = start_mock().await;
    *state.lookup.lock().await = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "system", "valueUri": "http://example.org/cs"},
            {"name": "code", "valueCode": "A01.0"}
        ]
    });

    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert!(state.calls_to("$subsumes").await.is_empty());
    assert!(
        html.contains("This concept has no parents or children to compare."),
        "an empty comparator set must render a neutral sentence (en)",
    );
}

#[tokio::test]
async fn the_comparator_cap_is_stated_rather_than_silently_truncating() {
    // A wide SNOMED concept can carry hundreds of children. The panel runs at
    // most 20 subsumption calls, and it has to say so — a table that quietly
    // stops at 20 reads as "these are all the children", which is a lie.
    let (base, state) = start_mock().await;
    let mut parameters = vec![
        json!({"name": "system", "valueUri": "http://example.org/cs"}),
        json!({"name": "code", "valueCode": "A01.0"}),
    ];
    for n in 0..25 {
        parameters.push(json!({"name": "property", "part": [
            {"name": "code", "valueCode": "child"},
            {"name": "value", "valueCode": format!("A01.{n:02}")}
        ]}));
    }
    *state.lookup.lock().await = json!({
        "resourceType": "Parameters",
        "parameter": parameters,
    });

    let response = fetch(
        app_pointing_at(&base),
        &format!("/ui/hts/concepts/relations?{ENCODED_ADDRESS}"),
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;

    assert_eq!(
        state.calls_to("$subsumes").await.len(),
        20,
        "the fan-out must stop at the cap",
    );
    assert!(
        html.contains("5 further comparators were not checked"),
        "the panel must state what it dropped (en); got:\n{html}",
    );
}

// ── Entry point ─────────────────────────────────────────────────────────

#[tokio::test]
async fn concept_permalink_is_reachable_from_a_lookup_result() {
    // The CodeSystem workbench's $lookup result links into the plane. The
    // permalink must carry the address as encoded query pairs.
    let (base, _state) = start_mock().await;
    let response = app_pointing_at(&base)
        .oneshot(
            axum::http::Request::post("/ui/hts/code-systems/example/lookup")
                .header(header::ACCEPT_LANGUAGE, "en")
                .header("HX-Request", "true")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("code=A01.0"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Askama escapes `&` as `&#38;` inside the href, so match on the
    // percent-encoded system and the code separately rather than on one
    // literal query string.
    assert!(
        html.contains("href=\"/ui/hts/concepts?system=http%3A%2F%2Fexample.org%2Fcs")
            && html.contains("code=A01.0"),
        "the lookup result must link into the concept permalink; got:\n{html}",
    );
    assert!(
        html.contains("Open concept"),
        "the link label must be Fluent-resolved (en)",
    );
}
