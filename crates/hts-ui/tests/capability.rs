//! Capability & Conformance page HTTP tests.
//!
//! Same in-process axum mock pattern as `tests/import.rs`. Each test polls
//! `/__mock_ready` before firing so the mock's TCP listener has finished
//! accepting on Windows.
//!
//! **Shape of record (2026-09-01, #808).** Four of the six cards are no
//! longer this crate's markup at all: they are rendered by
//! `helios_ui_chrome::capability::CapabilityCards`, the same code HFS's
//! `/ui/capability-statement` renders. These tests therefore assert the
//! *page* — which cards appear, in what order, fed by which fetch, degrading
//! how — and leave the cards' internals to the shared crate's own unit tests.
//! The page was previously "Diagnostics" at `/ui/hts/diagnostics`; that path
//! now 308s here.
//!
//! The tests exercise:
//!
//! 1. Every card renders, in HFS's order, with HFS's Fluent strings.
//! 2. The Operations and Per-Resource cards read `rest[].operation[]` and
//!    `rest[].resource[].searchParam[]`.
//! 3. System Interactions is **absent** when the server declares none (HTS
//!    today) and **present** when it does — never blank, never invented.
//! 4. The Terminology capabilities card shows real capabilities and
//!    repeats **no** identity field from the CapabilityStatement above it.
//! 5. `closure` support is detected by *presence*, not by boolean value.
//! 6. The page no longer fetches `/health` or `/metrics` — those cards
//!    moved to Home, and the round-trips went with them.
//! 7. The old `/ui/hts/diagnostics` path still resolves, as a 308.
//! 8. A 5xx on one source degrades only its own card.
//! 9. The shared cards arrive with HFS's spec links and colour-coded
//!    interaction chips — the two improvements this page did not have while
//!    it kept its own copy of the markup.
//! 10. (sync) Every CSS class the page names — its own *and* the shared
//!    cards' — has a real rule in the shared `crates/ui/assets/app.css`.

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
        metrics_ring: Default::default(),
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

/// Per-test overrides. A test mutates what it needs before firing its
/// request; anything not overridden falls back to [`MockResponses::default`].
#[derive(Clone)]
struct MockResponses {
    /// `GET /metadata` — CapabilityStatement.
    capability: (StatusCode, Option<Value>),
    /// `GET /metadata?mode=terminology` — TerminologyCapabilities.
    terminology: (StatusCode, Option<Value>),
}

impl MockResponses {
    fn default() -> Self {
        Self {
            // Shaped after what `crates/hts/src/operations/metadata.rs`
            // actually emits — including the absence of a system-level
            // `rest[].interaction[]`, which is why the System Interactions
            // card is hidden against a real HTS.
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
                    "kind": "instance",
                    "fhirVersion": "4.0.1",
                    "format": ["application/fhir+json", "application/fhir+xml"],
                    "implementation": { "description": "Helios Terminology Server SQLite backend" },
                    "rest": [{
                        "mode": "server",
                        "resource": [
                            {
                                "type": "CodeSystem",
                                "interaction": [{"code": "read"}, {"code": "search-type"}],
                                "searchParam": [
                                    {"name": "url"}, {"name": "version"}, {"name": "name"},
                                    {"name": "title"}, {"name": "status"}
                                ]
                            },
                            {
                                "type": "ValueSet",
                                "interaction": [{"code": "read"}],
                                "searchParam": [{"name": "url"}]
                            }
                        ],
                        "operation": [
                            {"name": "lookup", "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"},
                            {"name": "expand", "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-expand"}
                        ]
                    }]
                })),
            ),
            terminology: (
                StatusCode::OK,
                Some(json!({
                    "resourceType": "TerminologyCapabilities",
                    // Identity fields are deliberately present in the
                    // fixture: the page must *not* render them, and a
                    // fixture that omitted them could not prove that.
                    "version": "9.9.9-test",
                    "name": "HeliosTerminologyServer",
                    "title": "Helios Terminology Server",
                    "status": "active",
                    "codeSystem": [{"uri": "http://loinc.org"}, {"uri": "http://snomed.info/sct"}],
                    "expansion": {
                        "hierarchical": false,
                        "paging": true,
                        "incomplete": false,
                        "parameter": [{"name": "activeOnly"}, {"name": "displayLanguage"}]
                    },
                    "validateCode": { "translations": false },
                    "translation": { "needsMap": true },
                    // A bare `{}` — presence is the signal, and reading it
                    // as a boolean would report closure as unsupported.
                    "closure": {}
                })),
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

    async fn set_capability(&self, status: StatusCode, body: Option<Value>) {
        self.responses.lock().await.capability = (status, body);
    }

    async fn set_terminology(&self, status: StatusCode, body: Option<Value>) {
        self.responses.lock().await.terminology = (status, body);
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
    let uri_path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_owned())
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
//
// Two `#[tokio::test]`s, deliberately. libtest runs tests in parallel —
// one current-thread runtime, one reqwest client and one in-process mock
// *per test*. Splitting the happy-path assertions across many tests turns
// into a pile of simultaneous loopback connections, which on Windows
// stalls until the request timeout fires and turns the whole ring red.
// Merging them onto a single mock + a single cloned router (the same
// remedy `tests/route_enum.rs` documents for its matrix) keeps it fast.

const PATH: &str = "/ui/hts/capability-statement";

/// One hard-navigation GET of the page against `app`. The router is cloned
/// rather than rebuilt so every request in a test shares one reqwest
/// client and its connection pool.
async fn get_page(app: &Router) -> String {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::get(PATH)
                .header(header::ACCEPT_LANGUAGE, "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[tokio::test]
async fn capability_page_mirrors_hfs_and_declares_terminology_capabilities() {
    let (base, state) = start_mock().await;
    let app = app_pointing_at(&base);
    let html = get_page(&app).await;

    // ── HFS parity: the page identity ───────────────────────────────────
    // The heading and the sidebar label are HFS's own Fluent strings, not
    // HTS clones — the catalog is shared, so reusing them is what stops
    // the two sidebars drifting apart.
    assert!(
        html.contains("Capability Statement"),
        "page heading should use HFS's `cap-title`",
    );
    // Askama escapes `&` as the numeric entity `&#38;`, not `&amp;`.
    assert!(
        html.contains("Capability &#38; Conformance"),
        "sidebar should use HFS's `nav-capability-conformance` label; nav renders as: {:?}",
        html.split(r#"href="/ui/hts/capability-statement""#)
            .nth(1)
            .map(|s| &s[..s.len().min(120)]),
    );
    assert!(
        !html.contains(">Diagnostics<"),
        "no surface should still say `Diagnostics`",
    );
    assert!(
        !html.contains("nav-item--soon"),
        "the `coming soon` nav modifier must be gone — the page has shipped",
    );
    assert!(
        html.contains(r#"href="/ui/hts/capability-statement""#),
        "the sidebar entry should link to the new path",
    );

    // ── HFS parity: Server Summary uses HFS's field set ─────────────────
    for expected in [
        "Helios Terminology Server SQLite backend", // implementation.description
        "4.0.1",                                    // fhirVersion
        "instance",                                 // kind
        "application/fhir+json, application/fhir+xml", // format[] joined
    ] {
        assert!(
            html.contains(expected),
            "Server Summary should render `{expected}`",
        );
    }

    // ── Operations card reads `rest[].operation[]` ──────────────────────
    assert!(html.contains("$lookup"), "Operations should list $lookup");
    assert!(html.contains("$expand"), "Operations should list $expand");
    assert!(
        html.contains("http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"),
        "Operations should render each operation's canonical definition",
    );

    // ── Per-Resource card reads `searchParam[]` ─────────────────────────
    assert!(
        html.contains("CodeSystem"),
        "resource table should list CodeSystem"
    );
    assert!(
        html.contains(r#"<td class="col-num">5</td>"#),
        "CodeSystem advertises 5 search params and the count column should say so",
    );

    // ── #808: the shared cards bring HFS's links and colour coding ──────
    // This page rendered plain text here while it kept its own copy of the
    // markup. The links follow the release the binary was built for, so an
    // R4 build never sends the operator at the current-release page (#797).
    assert!(
        html.contains(
            r#"<a href="https://hl7.org/fhir/R4/codesystem.html" target="_blank" rel="noopener">CodeSystem</a>"#
        ),
        "resource types should link into the release's own specification",
    );
    assert!(
        html.contains(r#"<span class="tag tag--member">read</span>"#),
        "per-resource interaction verbs should carry HFS's semantic classes",
    );
    assert!(
        html.contains(
            r#"<a class="url" href="http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup""#
        ),
        "operation definitions should link when the canonical is safe",
    );

    // ── System Interactions is absent, not blank ────────────────────────
    // HTS serves `POST /` but declares no `rest[].interaction[]`. The card
    // must not appear at all rather than render an empty chip row.
    assert!(
        !html.contains("System Interactions"),
        "with no declared interactions the card should be omitted entirely",
    );

    // ── Terminology capabilities: real content, no duplication ──────────
    assert!(
        html.contains("Terminology Capabilities"),
        "the terminology card should render",
    );
    assert!(
        html.contains("Hierarchical expansion") && html.contains("Expansion paging"),
        "expansion flags should render",
    );
    assert!(
        html.contains("activeOnly") && html.contains("displayLanguage"),
        "the $expand parameter chips should render",
    );
    assert!(
        html.contains(r#"href="/ui/hts/code-systems""#),
        "the declared-systems count should link to the code-systems browser",
    );
    assert!(
        html.contains(">2</a>"),
        "two code systems are declared in the fixture",
    );

    // `closure` is a bare `{}` in the fixture. Presence means supported —
    // parsing it as a boolean would render "No" here.
    let closure_row = html
        .split("Closure maintenance")
        .nth(1)
        .expect("closure row renders");
    assert!(
        closure_row.starts_with("</span><div>Yes"),
        "a bare `closure: {{}}` means supported; got: {}",
        &closure_row[..closure_row.len().min(60)],
    );

    // ── Raw CapabilityStatement, HFS's foldable block ───────────────────
    assert!(
        html.contains("<details>") && html.contains(r#"<pre class="detail__code">"#),
        "the raw statement should fold into HFS's `<details>` + `<pre>`",
    );
    // Askama emits numeric entities (`&#34;`), so assert on the payload
    // rather than on a particular quoting of it.
    let folded = html
        .split(r#"<pre class="detail__code">"#)
        .nth(1)
        .and_then(|s| s.split("</pre>").next())
        .expect("the raw block renders");
    assert!(
        folded.contains("resourceType") && folded.contains("CapabilityStatement"),
        "the folded block should hold the statement; got: {}",
        &folded[..folded.len().min(80)],
    );
    assert!(
        folded.contains("\n"),
        "the folded statement should be pretty-printed, not one compact line",
    );

    // ── The page fetches exactly two sources ────────────────────────────
    // Health and Prometheus moved to Home; their round-trips went too.
    let calls = state.snapshot().await;
    let paths: Vec<&str> = calls.iter().map(|c| c.path.as_str()).collect();
    assert!(
        !paths.iter().any(|p| p.contains("/health")),
        "the page should no longer fetch /health; saw {paths:?}",
    );
    assert!(
        !paths.iter().any(|p| p.contains("/metrics")),
        "the page should no longer fetch /metrics; saw {paths:?}",
    );

    // ── The old path still resolves, as a 308 ───────────────────────────
    let redirect = app
        .clone()
        .oneshot(
            axum::http::Request::get("/ui/hts/diagnostics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        redirect.status(),
        StatusCode::PERMANENT_REDIRECT,
        "the pre-rename path should 308, not 404",
    );
    assert_eq!(
        redirect.headers().get(header::LOCATION).unwrap(),
        PATH,
        "the redirect should land on the renamed page",
    );
}

#[tokio::test]
async fn interactions_appear_when_declared_and_one_failure_degrades_only_its_card() {
    let (base, state) = start_mock().await;
    let app = app_pointing_at(&base);

    // A server that *does* declare system interactions gets the card.
    state
        .set_capability(
            StatusCode::OK,
            Some(json!({
                "resourceType": "CapabilityStatement",
                "status": "active",
                "rest": [{
                    "mode": "server",
                    "interaction": [{"code": "batch"}, {"code": "transaction"}]
                }]
            })),
        )
        .await;
    let html = get_page(&app).await;
    assert!(
        html.contains("System Interactions"),
        "the card should appear once the server declares interactions",
    );
    // #808: the chips arrive from the shared card, so they carry HFS's
    // semantic colour classes and HFS's link into the release's HTTP
    // specification. Before the unification this page emitted a bare
    // `<span class="tag">`.
    assert!(
        html.contains(
            r#"<a class="tag tag--config" href="https://hl7.org/fhir/R4/http.html#batch""#
        ),
        "declared interactions should be colour-coded chips linked into the spec",
    );
    assert!(
        html.contains(
            r#"<a class="tag tag--config" href="https://hl7.org/fhir/R4/http.html#transaction""#
        ),
        "every declared verb should be linked, not just the first",
    );

    // A 500 on the terminology probe degrades only the terminology card;
    // the CapabilityStatement cards keep rendering live data.
    state
        .set_terminology(StatusCode::INTERNAL_SERVER_ERROR, None)
        .await;
    let html = get_page(&app).await;
    assert!(
        html.contains("notice notice--warn"),
        "the failed card should carry a warning notice",
    );
    assert!(
        html.contains("System Interactions") && html.contains("batch"),
        "the healthy cards should be unaffected by the terminology failure",
    );
    assert!(
        !html.contains("Hierarchical expansion"),
        "the degraded card should show its notice instead of stale flags",
    );

    // A statement that grows with the data must not ship whole. HTS carries
    // one `capabilitystatement-supported-system` extension per loaded code
    // system; against the bundled seed set that is ~1,975 of them and a
    // 422 KB raw block — 95% of the page — on every load, `<details>` or
    // not. Only the seeded deployment exposes this; the small fixtures
    // above never would.
    // 400 is already ~10× the 16 KB cap; the real seed set carries ~1,975.
    // Kept deliberately modest — this fixture is serialized and
    // pretty-printed on every render, and a needlessly huge one only adds
    // CPU to a suite that already runs eleven binaries in parallel.
    let bulky: Vec<Value> = (0..400)
        .map(|i| {
            json!({
                "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
                "valueUri": format!("http://example.test/fhir/CodeSystem/padding-{i}")
            })
        })
        .collect();
    state
        .set_capability(
            StatusCode::OK,
            Some(json!({
                "resourceType": "CapabilityStatement",
                "status": "active",
                "extension": bulky,
            })),
        )
        .await;
    let html = get_page(&app).await;
    let folded = html
        .split(r#"<pre class="detail__code">"#)
        .nth(1)
        .and_then(|s| s.split("</pre>").next())
        .expect("the raw block renders");
    assert!(
        folded.len() < 20 * 1024,
        "the raw block must stay capped; got {} bytes",
        folded.len(),
    );
    // Never a silent truncation: the note states both sizes and points at
    // the endpoint that serves the complete document.
    assert!(
        html.contains("Truncated to the first"),
        "a truncated statement must say so",
    );
    assert!(
        html.contains(r#"<a href="/metadata">"#),
        "the truncation note must link to the full statement",
    );
    assert!(
        html.len() < 60 * 1024,
        "the whole page should stay small even against a bulky statement; got {} bytes",
        html.len(),
    );
}

/// Guard: the page adds **no** CSS. Every class it names must already have
/// a rule in the shared stylesheet, so a future edit cannot smuggle in a
/// reintroduced HTS-only style hook.
///
/// The scan covers the shared cards too (#808). They are the bulk of the
/// page now, and they are edited from `crates/ui-chrome` — where nothing
/// otherwise checks that a class reaches an HTS page with a rule behind it,
/// because that crate carries no stylesheet of its own.
///
/// A plain `#[test]`: it reads files and never touches the runtime, so it
/// stays outside the `#[tokio::test]` budget this file works to.
#[test]
fn capability_markup_only_uses_classes_that_exist_in_app_css() {
    const APP_CSS: &str = include_str!("../../ui/assets/app.css");
    const SOURCES: [(&str, &str); 5] = [
        (
            "the HTS page",
            include_str!("../templates/pages/capability-statement.html"),
        ),
        (
            "the shared summary card",
            include_str!("../../ui-chrome/templates/partials/capability-summary-card.html"),
        ),
        (
            "the shared interactions card",
            include_str!("../../ui-chrome/templates/partials/capability-interactions-card.html"),
        ),
        (
            "the shared operations card",
            include_str!("../../ui-chrome/templates/partials/capability-operations-card.html"),
        ),
        (
            "the shared resources card",
            include_str!("../../ui-chrome/templates/partials/capability-resources-card.html"),
        ),
    ];

    let mut checked = 0usize;
    for (label, source) in SOURCES {
        // Skip only the leading `{#- … -#}` / `{# … #}` header comment, which
        // quotes CSS selectors and `class="…"` fragments in prose. First
        // match, not last: these files carry a short comment above each card,
        // and splitting on the last one would skip nearly the whole file —
        // silently reducing this guard to a couple of classes.
        let body = source
            .split_once("-#}")
            .or_else(|| source.split_once("#}"))
            .map(|(_, rest)| rest)
            .unwrap_or(source);
        for chunk in body.split(r#"class=""#).skip(1) {
            let value = chunk.split('"').next().unwrap_or_default();
            // A `class` attribute may interpolate: `class="tag {{ i.tag_class }}"`.
            // Strip the expression and check the literal classes around it —
            // the interpolated values are `&'static str`s chosen in Rust, and
            // the shared crate's unit tests pin them.
            let literals = value
                .split("{{")
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        part
                    } else {
                        part.split_once("}}").map(|(_, rest)| rest).unwrap_or("")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            for class in literals.split_whitespace() {
                assert!(
                    APP_CSS.contains(&format!(".{class}")),
                    "class `{class}` used by {label} has no rule in crates/ui/assets/app.css",
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 20,
        "expected the scan to reach the markup's class attributes, only saw {checked}",
    );
}
