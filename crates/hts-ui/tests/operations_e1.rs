//! Integration ring for Slice E1 — standalone Operations workbench
//! shell (design doc §7.6). Slice E1 ships the shell, the 7-op
//! selector, the resource-family tab strip, the closure banner
//! visibility rule, the widened per-op input surfaces, and the
//! four E2 stubs. E2 will replace the stub handler bodies without
//! editing the route table or the tests below.
//!
//! The suite mixes two upstream fixtures:
//!
//! 1. **Closed loopback** (`127.0.0.1:1`) — used by every shell /
//!    fragment / pre-flight / stub test. The upstream client with
//!    `100 ms / 250 ms` timeouts fails deterministically (Connect),
//!    so the shell renders without an outbound round-trip.
//! 2. **In-process axum mock** — spun up per happy-path test
//!    (`run_lookup_free_scope_*`, `run_expand_free_scope_*`) on an
//!    ephemeral loopback port so the assertions can inspect the
//!    outgoing `Parameters` body without depending on a real HTS
//!    (mirrors the Slice C / D `start_mock` + `/__mock_ready`
//!    pattern; §7.5.1 mock-upstream ready probe).

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

async fn get(path: &str) -> (StatusCode, String) {
    let response = app_closed_loopback()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = body_text(response).await;
    (status, body)
}

async fn post_form(path: &str, form: &str) -> (StatusCode, String) {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form.to_owned()))
        .unwrap();
    let response = app_closed_loopback().oneshot(request).await.unwrap();
    let status = response.status();
    let body = body_text(response).await;
    (status, body)
}

// ── In-process mock upstream (mirrors Slice C / D pattern) ──────────────

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
    lookup: Arc<Mutex<CannedResponse>>,
    expand: Arc<Mutex<CannedResponse>>,
}

impl MockState {
    async fn snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().await.clone()
    }
}

fn ok_lookup_body() -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "name", "valueString": "Example CS"},
            {"name": "display", "valueString": "Fever"},
            {"name": "version", "valueString": "1.2.3"}
        ]
    })
}

fn ok_expand_body() -> Value {
    json!({
        "resourceType": "ValueSet",
        "expansion": {
            "identifier": "urn:uuid:0000",
            "total": 1,
            "offset": 0,
            "contains": [{
                "system": "http://example.org/cs",
                "code": "abc",
                "display": "Alpha Beta Gamma"
            }]
        }
    })
}

async fn mock_lookup_type(
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
        path: "/CodeSystem/$lookup".to_owned(),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.lookup.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn mock_lookup_instance(
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
        path: format!("/CodeSystem/{}/$lookup", id),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.lookup.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn mock_expand(
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
        path: format!("/ValueSet/{}/$expand", id),
        headers: parts.headers.clone(),
        body: body_str,
    });
    let canned = state.expand.lock().await.clone();
    (canned.status, axum::Json(canned.body)).into_response()
}

async fn start_mock() -> (String, MockState) {
    let state = MockState {
        captured: Arc::new(Mutex::new(Vec::new())),
        lookup: Arc::new(Mutex::new(CannedResponse {
            status: StatusCode::OK,
            body: ok_lookup_body(),
        })),
        expand: Arc::new(Mutex::new(CannedResponse {
            status: StatusCode::OK,
            body: ok_expand_body(),
        })),
    };
    let router: Router = Router::new()
        .route(
            "/__mock_ready",
            axum::routing::get(|| async { (StatusCode::OK, "ok") }),
        )
        .route("/CodeSystem/$lookup", post(mock_lookup_type))
        .route("/CodeSystem/{id}/$lookup", post(mock_lookup_instance))
        .route("/ValueSet/{id}/$expand", post(mock_expand))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{addr}");

    // Ready-probe: poll until the spawned axum::serve is accepting.
    // On Windows current-thread `#[tokio::test]` runtimes the server
    // task can trail the first client request by several milliseconds
    // otherwise. `no_proxy()` is inherited from `UpstreamClient`; the
    // probe uses its own reqwest client so we set it explicitly here.
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

// ── Slice E consolidated walker ─────────────────────────────────────────
//
// Merged into a single `#[tokio::test]` so we do not create/tear-down a
// tokio runtime + reqwest client per assertion — that pattern has bitten
// the CS/VS/CM rings on Windows (STATUS_INVALID_HANDLE across drops).
// Every assertion reuses the same runtime.

#[tokio::test]
async fn slice_e_shell_input_and_stubs_hold_together() {
    // ── Shell renders the seven-op selector with the current op active.
    let (status, body) =
        get("/ui/hts/operations?op=lookup&resource=CodeSystem").await;
    assert_eq!(status, StatusCode::OK, "lookup shell must return 200");
    // Op-selector emits one link per op; the seven slugs must all
    // appear as `?op={slug}` href fragments (F5 selector).
    for slug in [
        "op=lookup",
        "op=validate-code",
        "op=subsumes",
        "op=expand",
        "op=translate",
        "op=closure",
        "op=batch-validate",
    ] {
        assert!(
            body.contains(slug),
            "op selector must expose slug `{slug}` (got body prefix: {})",
            body.chars().take(200).collect::<String>()
        );
    }

    // Active op gets aria-current="page".
    assert!(
        body.contains("aria-current=\"page\""),
        "the active op must be marked with aria-current=\"page\""
    );

    // ── Closure banner: rendered ONLY when op=closure (§7.6 F6).
    let (_, body_closure) = get("/ui/hts/operations?op=closure&resource=").await;
    assert!(
        body_closure.contains("hts-op-banner"),
        "closure op must render the stateless banner"
    );
    let (_, body_lookup) =
        get("/ui/hts/operations?op=lookup&resource=CodeSystem").await;
    assert!(
        !body_lookup.contains("hts-op-banner"),
        "non-closure op must NOT render the stateless banner"
    );

    // ── Threshold Advanced panel: rendered ONLY on ?op=expand (§7.6 F12).
    let (_, body_expand_input) =
        get("/ui/hts/operations/input?op=expand&resource=ValueSet").await;
    assert!(
        body_expand_input.contains("name=\"threshold\""),
        "expand op input must expose the threshold input"
    );
    let (_, body_lookup_input) =
        get("/ui/hts/operations/input?op=lookup&resource=CodeSystem").await;
    assert!(
        !body_lookup_input.contains("name=\"threshold\""),
        "lookup op input must NOT expose the threshold input (F12 visibility rule)"
    );

    // ── Resource-family tab strip: rendered for validate-code + batch.
    let (_, body_vc) =
        get("/ui/hts/operations?op=validate-code&resource=CodeSystem").await;
    assert!(
        body_vc.contains("hts-op-tabs"),
        "validate-code shell must expose the resource-family tab strip"
    );
    let (_, body_batch) =
        get("/ui/hts/operations?op=batch-validate&resource=CodeSystem").await;
    assert!(
        body_batch.contains("hts-op-tabs"),
        "batch-validate shell must expose the resource-family tab strip"
    );
    assert!(
        !body_lookup.contains("hts-op-tabs"),
        "lookup shell must NOT expose the resource-family tab strip"
    );

    // ── Widened input surfaces (§7.6 F4):
    //     - CS lookup exposes `useSupplement` (Slice B deferred).
    let (_, body_lookup_i) =
        get("/ui/hts/operations/input?op=lookup&resource=CodeSystem").await;
    assert!(
        body_lookup_i.contains("name=\"useSupplement\""),
        "CS lookup standalone input must expose useSupplement (widening confirmed)"
    );
    //     - CS validate exposes a `CodeableConcept` mode radio.
    let (_, body_val_cs_i) =
        get("/ui/hts/operations/input?op=validate-code&resource=CodeSystem")
            .await;
    assert!(
        body_val_cs_i.contains("value=\"CodeableConcept\""),
        "CS validate standalone input must expose the CodeableConcept mode"
    );
    //     - VS expand exposes a `designation[]` chip input.
    let (_, body_expand_i) =
        get("/ui/hts/operations/input?op=expand&resource=ValueSet").await;
    assert!(
        body_expand_i.contains("name=\"designation\""),
        "VS expand standalone input must expose the designation chip"
    );

    // ── VS validate-code against an unreachable upstream renders the
    // shared degraded banner (Slice E2 wires the real handler; the
    // closed-loopback fixture surfaces UpstreamError::Connect).
    let (status_vs, body_vs) = post_form(
        "/ui/hts/operations/validate-code",
        "resource=ValueSet&sourceMode=canonical&sourceCanonical=https://example.org/vs&code=x",
    )
    .await;
    assert_eq!(status_vs, StatusCode::OK);
    assert!(
        body_vs.contains("hts-degraded")
            || body_vs.contains("hts-outcome"),
        "VS validate-code must render the shared degraded/outcome partial \
         against an unreachable upstream (Slice E2)"
    );

    // ── Closure POST with valid `name` reaches upstream; against the
    // closed loopback the handler surfaces the shared degraded banner.
    let (status_cl, body_cl) =
        post_form("/ui/hts/operations/closure", "name=test").await;
    assert_eq!(status_cl, StatusCode::OK);
    assert!(
        body_cl.contains("hts-degraded") || body_cl.contains("hts-outcome"),
        "closure must render the shared degraded/outcome partial \
         against an unreachable upstream (Slice E2)"
    );

    // ── batch-validate seed with a valid target + one row seeds the
    // job store and returns the skeleton results table (the polling
    // targets and progress region).
    let (status_bs, body_bs) = post_form(
        "/ui/hts/operations/batch-validate",
        "target=https://example.org/vs&row.code=abc",
    )
    .await;
    assert_eq!(status_bs, StatusCode::OK);
    assert!(
        body_bs.contains("hts-batch-progress")
            || body_bs.contains("aria-busy")
            || body_bs.contains("hts-workbench-result"),
        "batch-validate seed must return the skeleton table + progress region \
         (Slice E2; got body prefix: {})",
        body_bs.chars().take(200).collect::<String>()
    );

    // ── batch-validate row endpoint with a missing batch_id renders a
    // row-scoped not-found OperationOutcome.
    let (status_br, body_br) =
        get("/ui/hts/operations/batch-validate/row/0?batch_id=missing").await;
    assert_eq!(status_br, StatusCode::OK);
    assert!(
        body_br.contains("hts-batch-row-0"),
        "batch-validate row endpoint must render the per-row `<tr>` shell (Slice E2)"
    );

    // ── batch-validate progress endpoint renders the counter region.
    let (status_bp, body_bp) =
        get("/ui/hts/operations/batch-validate/progress?batch_id=missing").await;
    assert_eq!(status_bp, StatusCode::OK);
    assert!(
        body_bp.contains("hts-batch-progress"),
        "batch-validate progress region must render its id (got: {})",
        body_bp.chars().take(200).collect::<String>()
    );

    // ── HX-Request Vary header is emitted by the AutoVaryLayer for HTMX
    // requests (mirrors the route_enum matrix invariant).
    let response = app_closed_loopback()
        .oneshot(
            Request::get("/ui/hts/operations?op=lookup&resource=CodeSystem")
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
        "operations shell must Vary on HX-Request for htmx requests (got: {vary:?})"
    );
}

#[tokio::test]
async fn every_pre_flight_validation_gates_short_circuit_without_upstream() {
    // Slice E1: pre-flight validation on each real runner avoids
    // burning a round-trip when required inputs are missing. All
    // return an OpResultView::invalid_input outcome + 200 OK so
    // the workbench keeps rendering.

    for (path, body_form) in [
        ("/ui/hts/operations/lookup", "system=&code=x"),
        ("/ui/hts/operations/lookup", "system=https://x&code="),
        (
            "/ui/hts/operations/validate-code",
            "resource=CodeSystem&system=&code=x",
        ),
        (
            "/ui/hts/operations/subsumes",
            "system=&codeA=x&codeB=y",
        ),
        (
            "/ui/hts/operations/subsumes",
            "system=https://x&codeA=&codeB=y",
        ),
        ("/ui/hts/operations/expand", "sourceInstance="),
        ("/ui/hts/operations/translate", "sourceInstance="),
        (
            "/ui/hts/operations/translate",
            "sourceInstance=cm-1&direction=forward&code=&system=",
        ),
        (
            "/ui/hts/operations/translate",
            "sourceInstance=cm-1&direction=reverse&targetCode=",
        ),
    ] {
        let (status, body) = post_form(path, body_form).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "pre-flight validation must render 200 OK for {path} ({body_form})"
        );
        assert!(
            body.contains("hts-outcome") || body.contains("invalid"),
            "pre-flight validation must render an outcome partial for {path} ({body_form}), got body starting with: {}",
            body.chars().take(200).collect::<String>()
        );
    }
}

// ── Mock-backed happy-path tests ────────────────────────────────────────
//
// These two tests exercise the full round-trip: the operations POST
// handler forms a FHIR `Parameters` body and sends it to the upstream
// HTS surface. The in-process mock captures the outgoing request so
// the assertions can pin the wire contract (proxy verb rule: standalone
// `$lookup` / `$expand` POST to `/{Resource}/{id}/$op`).

#[tokio::test]
async fn run_lookup_free_scope_posts_to_hts_and_swaps_result_region() {
    let (base, state) = start_mock().await;

    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/lookup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=CodeSystem&sourceInstance=example-cs&\
                     system=http%3A%2F%2Fexample.org%2Fcs&code=abc",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    // Result region rendered — the workbench-result wrapper is the
    // stable oracle across shell and fragment renders (F15).
    assert!(
        html.contains("hts-workbench-result"),
        "run_lookup response must include the shared workbench-result region"
    );

    let captured = state.snapshot().await;
    let lookup = captured
        .iter()
        .find(|c| c.path.contains("/$lookup"))
        .expect("mock must observe the $lookup POST");
    assert_eq!(lookup.method, "POST", "proxy verb rule: standalone $lookup is POST");
    let body: Value = serde_json::from_str(&lookup.body)
        .expect("$lookup body must be JSON Parameters");
    // Wire-contract check: `code` + `system` parameters made it through
    // the multi-map form parser + Parameters emitter (§7.6.1 F2).
    let parameters = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .expect("Parameters resource must carry a parameter[] array");
    let names: Vec<&str> = parameters
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(
        names.iter().any(|n| *n == "code"),
        "$lookup body must carry a `code` parameter (names seen: {names:?})"
    );
    assert!(
        names.iter().any(|n| *n == "system"),
        "$lookup body must carry a `system` parameter (names seen: {names:?})"
    );
}

#[tokio::test]
async fn run_expand_free_scope_pins_instance_id_and_forwards_expand_params() {
    let (base, state) = start_mock().await;

    let response = app_pointing_at(&base)
        .oneshot(
            Request::post("/ui/hts/operations/expand")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("HX-Request", "true")
                .body(Body::from(
                    "resource=ValueSet&sourceInstance=example-vs&\
                     filter=fev&count=25&mode=tree",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("hts-workbench-result"),
        "run_expand response must include the shared workbench-result region"
    );

    let captured = state.snapshot().await;
    let expand = captured
        .iter()
        .find(|c| c.path.contains("/$expand"))
        .expect("mock must observe the $expand POST");
    assert!(
        expand.path.contains("example-vs"),
        "instance-scoped $expand must pin `{{id}}` to the operator's input (path: {})",
        expand.path
    );
    let body: Value = serde_json::from_str(&expand.body)
        .expect("$expand body must be JSON Parameters");
    let parameters = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .expect("Parameters resource must carry a parameter[] array");
    let names: Vec<&str> = parameters
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();
    // `filter` + `count` must be forwarded verbatim; the tree-mode
    // toggle maps to `hierarchical=true` (§7.4.1 F7).
    assert!(
        names.iter().any(|n| *n == "filter"),
        "$expand body must carry a `filter` parameter (names seen: {names:?})"
    );
    assert!(
        names.iter().any(|n| *n == "count"),
        "$expand body must carry a `count` parameter (names seen: {names:?})"
    );
    assert!(
        names.iter().any(|n| *n == "hierarchical"),
        "$expand body must carry a `hierarchical` parameter for tree mode (names seen: {names:?})"
    );
}
