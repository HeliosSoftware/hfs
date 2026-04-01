//! Integration tests for ValueSet operations:
//!   `POST /ValueSet/$expand`
//!   `POST /ValueSet/$validate-code`

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};

// ── $expand ───────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_returns_all_included_codes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "ValueSet");

    let contains = body["expansion"]["contains"]
        .as_array()
        .expect("expected expansion.contains array");

    // The ValueSet includes limb, arm, leg — 3 codes
    assert_eq!(
        contains.len(),
        3,
        "expected 3 codes in expansion, got: {body}"
    );

    let codes: Vec<&str> = contains.iter().filter_map(|e| e["code"].as_str()).collect();

    assert!(codes.contains(&"limb"), "expected 'limb' in expansion");
    assert!(codes.contains(&"arm"), "expected 'arm' in expansion");
    assert!(codes.contains(&"leg"), "expected 'leg' in expansion");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_unknown_value_set_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", "http://hts.test/vs/no-such-vs")]);
    let (status, _body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_with_count_limits_results() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // Request only 1 result
    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url",   "valueUri": bundles::LIMBS_VS_URL},
            {"name": "count", "valueInteger": 1}
        ]
    })
    .to_string();

    let (status, resp) = app.post_fhir("/ValueSet/$expand", body).await;
    assert_eq!(status, StatusCode::OK, "{resp}");

    let contains = resp["expansion"]["contains"]
        .as_array()
        .expect("expected expansion.contains");

    assert_eq!(contains.len(), 1, "expected exactly 1 code with count=1");
}

// ── $validate-code (ValueSet) ─────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_code_included_code_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", bundles::LIMBS_VS_URL),
        ("code", "valueCode", "arm"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");

    assert!(result, "'arm' should be in the limbs ValueSet");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_code_excluded_code_returns_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // 'head' is NOT in the limbs ValueSet (which only includes limb, arm, leg)
    let req = TestApp::params(&[
        ("url", "valueUri", bundles::LIMBS_VS_URL),
        ("code", "valueCode", "head"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");

    assert!(!result, "'head' should NOT be in the limbs ValueSet");
}
