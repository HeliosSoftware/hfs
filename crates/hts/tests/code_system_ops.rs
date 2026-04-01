//! Integration tests for CodeSystem operations:
//!   `POST /CodeSystem/$lookup`
//!   `POST /CodeSystem/$validate-code`
//!   `POST /CodeSystem/$subsumes`
//!
//! Each test imports the shared anatomy bundle (R4 format) and then exercises
//! one operation.  All assertions mirror the behaviour expected from a
//! reference FHIR terminology server such as https://tx.fhir.org/r5/.

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};

// ── $lookup ───────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_existing_code_returns_display() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/CodeSystem/$lookup", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Parameters");

    let display = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .expect("expected a display parameter");

    assert_eq!(display, "Arm");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_nested_code_returns_correct_display() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "leg"),
    ]);
    let (status, body) = app.post_fhir("/CodeSystem/$lookup", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let display = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .expect("expected display");

    assert_eq!(display, "Leg");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_unknown_code_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "xyz-does-not-exist"),
    ]);
    let (status, _body) = app.post_fhir("/CodeSystem/$lookup", req).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_unknown_system_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", "http://hts.test/no-such-system"),
        ("code", "valueCode", "arm"),
    ]);
    let (status, _body) = app.post_fhir("/CodeSystem/$lookup", req).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── $validate-code ────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn validate_existing_code_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/CodeSystem/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected a result parameter");

    assert!(result, "code 'arm' should be valid");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn validate_nonexistent_code_returns_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "notacode"),
    ]);
    let (status, body) = app.post_fhir("/CodeSystem/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected a result parameter");

    assert!(!result, "unknown code should return false");
}

// ── $subsumes ─────────────────────────────────────────────────────────────────

/// Helper: call $subsumes and return the `outcome` string.
#[cfg(feature = "sqlite")]
async fn subsumes_outcome(app: &TestApp, code_a: &str, code_b: &str) -> String {
    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("codeA", "valueCode", code_a),
        ("codeB", "valueCode", code_b),
    ]);
    let (status, body) = app.post_fhir("/CodeSystem/$subsumes", req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "subsumes({code_a},{code_b}) failed: {body}"
    );

    body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "outcome")
        .and_then(|p| p["valueCode"].as_str())
        .unwrap_or_else(|| panic!("no outcome in response: {body}"))
        .to_string()
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_self_is_equivalent() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    assert_eq!(subsumes_outcome(&app, "body", "body").await, "equivalent");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_parent_child_is_subsumes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    // body is an ancestor of limb
    assert_eq!(subsumes_outcome(&app, "body", "limb").await, "subsumes");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_grandparent_grandchild_is_subsumes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    // body is an ancestor of arm (two levels)
    assert_eq!(subsumes_outcome(&app, "body", "arm").await, "subsumes");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_child_parent_is_subsumed_by() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    // limb is a descendant of body
    assert_eq!(subsumes_outcome(&app, "limb", "body").await, "subsumed-by");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_siblings_are_not_subsumed() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    // arm and leg are siblings — no subsumption relationship
    assert_eq!(subsumes_outcome(&app, "arm", "leg").await, "not-subsumed");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn subsumes_unrelated_branches_are_not_subsumed() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;
    // head and arm are in different branches under body
    assert_eq!(subsumes_outcome(&app, "head", "arm").await, "not-subsumed");
}
