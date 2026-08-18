//! Integration ring for Slice E2 — the closure workbench, VS
//! `$validate-code`, and the batch fan-out (design doc §7.6.1 F1 = D,
//! F6/F7, F11, F16).
//!
//! Follows Slice E1's fixture pattern (`start_mock` + `/__mock_ready`)
//! and shares its Windows-safe execution model: mock-backed happy paths
//! each get their own `#[tokio::test]`, everything else piggybacks on a
//! consolidated closed-loopback walker. The dispatch matrix hook
//! (§7.6.1 F16 #1) intentionally lives on `tests/route_enum.rs` so
//! it stays inside the single merged `#[tokio::test]` walker per
//! §7.3.1 invariant #6.

use axum::{
    Router,
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tower::ServiceExt;

// ── Closed-loopback shell fixture ───────────────────────────────────────

fn app_closed_loopback() -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            "http://127.0.0.1:1",
            Duration::from_millis(250),
            Duration::from_millis(100),
        )
        .expect("closed loopback URL always parses"),
        bundled_data_bytes: None,
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

fn app_pointing_at(base_url: &str) -> Router {
    let state = Arc::new(helios_hts_ui::HtsUiState {
        fhir_version: "R4",
        version: "9.9.9-test",
        upstream: helios_hts_ui::UpstreamClient::new_with_timeouts(
            base_url,
            Duration::from_secs(5),
            Duration::from_secs(2),
        )
        .expect("mock base URL parses"),
        bundled_data_bytes: None,
    });
    Router::new().nest("/ui", helios_hts_ui::router(state))
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn get_closed(path: &str) -> (StatusCode, String) {
    let response = app_closed_loopback()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = body_text(response).await;
    (status, body)
}

// ── In-process mock upstream (mirrors operations_e1's pattern) ──────────

#[derive(Clone, Debug)]
#[allow(dead_code)]
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

#[derive(Clone)]
struct MockState {
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    closure: Arc<Mutex<CannedResponse>>,
    vs_validate: Arc<Mutex<CannedResponse>>,
}

impl MockState {
    async fn snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().await.clone()
    }
}

fn empty_closure_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "return",
            "resource": {
                "resourceType": "ConceptMap",
                "name": "example-closure"
            }
        }]
    })
}

fn ok_vs_validate_body(result: bool) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "result", "valueBoolean": result},
            {"name": "display", "valueString": "Fever"}
        ]
    })
}

async fn mock_closure(
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
        path: "/ConceptMap/$closure".to_owned(),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.closure.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn mock_vs_validate_type(
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
        path: "/ValueSet/$validate-code".to_owned(),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.vs_validate.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn mock_vs_validate_instance(
    State(state): State<MockState>,
    Path(id): Path<String>,
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
        path: format!("/ValueSet/{}/$validate-code", id),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.vs_validate.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        closure: Arc::new(Mutex::new(CannedResponse {
            status: StatusCode::OK,
            body: empty_closure_body(),
        })),
        vs_validate: Arc::new(Mutex::new(CannedResponse {
            status: StatusCode::OK,
            body: ok_vs_validate_body(true),
        })),
    };
    let router: Router = Router::new()
        .route(
            "/__mock_ready",
            axum::routing::get(|| async { (StatusCode::OK, "ok") }),
        )
        .route("/ConceptMap/$closure", post(mock_closure))
        .route("/ValueSet/$validate-code", post(mock_vs_validate_type))
        .route(
            "/ValueSet/{id}/$validate-code",
            post(mock_vs_validate_instance),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{addr}");
    let probe = reqwest::Client::builder()
        .no_proxy()
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

// ── F16 hook #3: closure banner renders only on op=closure ──────────────

#[tokio::test]
async fn closure_banner_renders_only_on_closure_op() {
    // The op-conditional `hts-op-banner` (§7.6 F6) with `role="status"`
    // is the stateless-warning surface for `$closure` and must not
    // leak into any other op. Walk the seven ops through the shell
    // and assert the banner presence rule against each.
    for (op, resource, expected) in [
        ("lookup", "CodeSystem", false),
        ("validate-code", "CodeSystem", false),
        ("validate-code", "ValueSet", false),
        ("subsumes", "CodeSystem", false),
        ("expand", "ValueSet", false),
        ("translate", "ConceptMap", false),
        ("closure", "", true),
        ("batch-validate", "ValueSet", false),
    ] {
        let path = if resource.is_empty() {
            format!("/ui/hts/operations?op={op}")
        } else {
            format!("/ui/hts/operations?op={op}&resource={resource}")
        };
        let (status, body) = get_closed(&path).await;
        assert_eq!(status, StatusCode::OK, "shell for op={op} must render 200");
        let has_banner = body.contains("hts-op-banner")
            && body.contains(r#"role="status""#);
        assert_eq!(
            has_banner, expected,
            "closure banner visibility for op={op} must be {expected} \
             (got body prefix: {})",
            body.chars().take(160).collect::<String>()
        );
    }
}

// ── F16 hook #4: every op POSTs to HTS (verb rule §7.6) ─────────────────

#[tokio::test]
async fn verb_rule_all_ops_post_to_hts() {
    // §7.6 verb rule: every UI-side POST /ui/hts/operations/{op} must
    // fan out a POST upstream, regardless of the source form's verb.
    // Slice E2 owns the two new upstream POSTs (closure + VS validate);
    // Slice E1 already covered lookup + expand via the E1 ring, so
    // this hook focuses on the E2-shipped ones.
    let (base, state) = start_mock().await;

    // 1) Closure POST reaches HTS as POST /ConceptMap/$closure.
    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/closure")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("name=test-graph"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = body_text(response).await;

    // 2) VS validate canonical POST reaches HTS as POST /ValueSet/$validate-code.
    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/validate-code")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=ValueSet&sourceMode=canonical&\
                     sourceCanonical=https%3A%2F%2Fexample.org%2Fvs&\
                     mode=code&code=abc&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = body_text(response).await;

    let captured = state.snapshot().await;
    let closure = captured
        .iter()
        .find(|c| c.path.contains("/$closure"))
        .expect("mock must observe the $closure POST");
    assert_eq!(
        closure.method, "POST",
        "closure must POST to HTS (verb rule §7.6)"
    );
    let vs = captured
        .iter()
        .find(|c| c.path == "/ValueSet/$validate-code")
        .expect("mock must observe the $validate-code POST");
    assert_eq!(
        vs.method, "POST",
        "VS validate-code must POST to HTS (verb rule §7.6)"
    );
}

// ── F16 hook #2: batch seed emits N aria-busy skeleton rows ─────────────

#[tokio::test]
async fn batch_seed_returns_n_skeleton_rows() {
    // §7.6.1 F1 = D: the batch seed handler must return the skeleton
    // results table with one `<tr aria-busy="true">` per submitted
    // row, each carrying its per-row `hx-get` polling target so the
    // client fans out from there. No OOB attributes.
    let (base, _state) = start_mock().await;
    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/batch-validate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=ValueSet&target=https%3A%2F%2Fexample.org%2Fvs&\
                     row.code=a&row.system=http%3A%2F%2Fexample.org%2Fcs&row.display=A&\
                     row.code=b&row.system=http%3A%2F%2Fexample.org%2Fcs&row.display=B&\
                     row.code=c&row.system=http%3A%2F%2Fexample.org%2Fcs&row.display=C",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Three skeleton rows: `hts-batch-row-0..2`, each aria-busy.
    for i in 0..3 {
        assert!(
            html.contains(&format!("hts-batch-row-{i}")),
            "seed table must include skeleton row {i} (body prefix: {})",
            html.chars().take(200).collect::<String>()
        );
        assert!(
            html.contains(&format!(
                "/ui/hts/operations/batch-validate/row/{i}?batch_id="
            )),
            "row {i} must carry its per-row hx-get target"
        );
    }
    assert!(
        html.contains(r#"aria-busy="true""#),
        "skeleton rows must carry aria-busy=\"true\""
    );
    // Progress region wired.
    assert!(
        html.contains("hts-batch-progress"),
        "seed table must include the progress region"
    );
    assert!(
        html.contains("/ui/hts/operations/batch-validate/progress?batch_id="),
        "progress region must poll the batch-validate/progress endpoint"
    );
    // Pre-flight rule: no OOB swap attributes on the initial response
    // (§7.6.1 F1 bullet).
    assert!(
        !html.contains("hx-swap-oob"),
        "seed response must not use OOB swaps"
    );
}

// ── Additional Slice E2 neutral-state hooks (F11 analogs) ───────────────

#[tokio::test]
async fn closure_empty_graph_renders_neutral_state_not_outcome() {
    // §7.6 F6/F7: HTS returns an empty ConceptMap when only `name` is
    // seeded on first submit. The workbench must render the neutral
    // `hts-operations-closure-empty-graph` copy inside the shared
    // workbench-result region, NOT the shared error partial.
    let (base, _state) = start_mock().await;
    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/closure")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from("name=example-closure"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Positive: neutral empty state marker (English source).
    assert!(
        html.contains("No closure edges yet"),
        "closure empty-graph must render the neutral empty-state copy \
         (body prefix: {})",
        html.chars().take(240).collect::<String>()
    );
    // Negative: it must NOT surface as the shared error partial —
    // no `hts-outcome__code` / `hts-degraded__title`. The wrapping
    // region uses the workbench result id.
    assert!(
        !html.contains("hts-outcome__code"),
        "closure empty-graph must NOT render the shared error partial"
    );
    assert!(
        !html.contains("hts-degraded__title"),
        "closure empty-graph must NOT render the degraded banner"
    );
    assert!(
        html.contains("hts-workbench-result"),
        "closure empty-graph must still swap into #hts-workbench-result"
    );
}

#[tokio::test]
async fn vs_validate_false_result_renders_neutral_badge_not_outcome() {
    // §7.6 F11 companion (§7.4.1 F11): a `result=false` response on
    // HTTP 200 is the no-membership neutral state, NOT an error surface.
    let (base, state) = start_mock().await;
    // Override the canned VS validate body to result=false.
    {
        let mut guard = state.vs_validate.lock().await;
        *guard = CannedResponse {
            status: StatusCode::OK,
            body: ok_vs_validate_body(false),
        };
    }
    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/validate-code")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=ValueSet&sourceMode=canonical&\
                     sourceCanonical=https%3A%2F%2Fexample.org%2Fvs&\
                     mode=code&code=abc&system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-op-workbench__badge--false"),
        "result=false must render the neutral false-badge (body prefix: {})",
        html.chars().take(240).collect::<String>()
    );
    assert!(
        !html.contains("hts-outcome__code"),
        "result=false must NOT render the shared error partial"
    );
    assert!(
        !html.contains("hts-degraded__title"),
        "result=false must NOT render the degraded banner"
    );
}

#[tokio::test]
async fn batch_progress_terminal_state_stops_polling() {
    // §7.6.1 F1 = D bullet on terminal-state polling: once every row
    // has completed, the progress endpoint must render the final
    // variant that OMITS the hx-trigger polling attribute so htmx
    // naturally stops. First, seed a small batch and let the workers
    // drain against the mock; then poll `/progress` and observe the
    // done arm.
    let (base, _state) = start_mock().await;
    let seed = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/batch-validate")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=ValueSet&target=https%3A%2F%2Fexample.org%2Fvs&\
                     row.code=a&row.system=http%3A%2F%2Fexample.org%2Fcs",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(seed.status(), StatusCode::OK);
    let seed_html = body_text(seed).await;
    // Extract the batch id embedded in the row hx-get targets.
    let marker = "batch-validate/row/0?batch_id=";
    let idx = seed_html
        .find(marker)
        .expect("seed must include the batch id");
    let start = idx + marker.len();
    let rest = &seed_html[start..];
    let end = rest
        .find(|c: char| c == '"' || c == '&' || c == ' ' || c == '\n')
        .unwrap_or(rest.len());
    let batch_id = &rest[..end];
    assert!(!batch_id.is_empty(), "batch id must be non-empty");

    // Give the spawned worker a small window to drain the row.
    let mut done_html = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let poll = app_pointing_at(&base)
            .oneshot(
                Request::get(format!(
                    "/ui/hts/operations/batch-validate/progress?batch_id={batch_id}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(poll.status(), StatusCode::OK);
        let html = body_text(poll).await;
        if !html.contains("hx-trigger") {
            done_html = html;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !done_html.is_empty(),
        "progress endpoint must reach the terminal state within the deadline"
    );
    // Terminal state omits the polling trigger.
    assert!(
        !done_html.contains("hx-trigger"),
        "terminal-state progress must OMIT the hx-trigger attribute \
         (got body: {})",
        done_html.chars().take(240).collect::<String>()
    );
    // Terminal state still emits the region id so future selectors keep working.
    assert!(
        done_html.contains("hts-batch-progress"),
        "terminal-state progress must still emit the region id"
    );
}
