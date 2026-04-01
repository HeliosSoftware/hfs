//! Integration tests for ConceptMap operations:
//!   `POST /ConceptMap/$translate`
//!
//! Tests cover both R4/R4B bundles (using `equivalence`) and R5/R6 bundles
//! (using `relationship`), verifying that the import normalises both
//! serialisations into a queryable form.

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};

// ── Shared helper ─────────────────────────────────────────────────────────────

/// Call $translate and return `(result_bool, target_code_or_empty)`.
#[cfg(feature = "sqlite")]
async fn translate(app: &TestApp, cm_url: &str, source_system: &str, code: &str) -> (bool, String) {
    let req = TestApp::params(&[
        ("url", "valueUri", cm_url),
        ("system", "valueUri", source_system),
        ("code", "valueCode", code),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", req).await;
    assert_eq!(status, StatusCode::OK, "translate failed: {body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("no result param");

    // Extract target code from the first `match` parameter, if present.
    let target_code = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "match")
        .and_then(|m| m["part"].as_array())
        .and_then(|parts| {
            parts
                .iter()
                .find(|part| part["name"] == "concept")
                .and_then(|part| part["valueCoding"]["code"].as_str().map(|s| s.to_string()))
        })
        .unwrap_or_default();

    (result, target_code)
}

// ── R4 ConceptMap translation ─────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r4_arm_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R4, bundles::ANATOMY_CS_URL, "arm").await;

    assert!(result, "translation of 'arm' should succeed");
    assert_eq!(code, "40983000", "expected SNOMED code for 'arm'");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r4_leg_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R4, bundles::ANATOMY_CS_URL, "leg").await;

    assert!(result, "translation of 'leg' should succeed");
    assert_eq!(code, "61685007", "expected SNOMED code for 'leg'");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r4_unmapped_code_returns_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // 'head' has no mapping in the R4 ConceptMap
    let (result, _) = translate(&app, bundles::CM_URL_R4, bundles::ANATOMY_CS_URL, "head").await;

    assert!(!result, "unmapped code should return result=false");
}

// ── R4B ConceptMap translation ────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r4b_arm_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4b_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R4B, bundles::ANATOMY_CS_URL, "arm").await;

    assert!(result);
    assert_eq!(code, "40983000");
}

// ── R5 ConceptMap translation (relationship field) ────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r5_arm_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r5_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R5, bundles::ANATOMY_CS_URL, "arm").await;

    assert!(result, "R5 translation of 'arm' should succeed");
    assert_eq!(
        code, "40983000",
        "expected SNOMED code for 'arm' via R5 ConceptMap"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r5_leg_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r5_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R5, bundles::ANATOMY_CS_URL, "leg").await;

    assert!(result);
    assert_eq!(code, "61685007");
}

// ── R6 ConceptMap translation ─────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r6_arm_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r6_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R6, bundles::ANATOMY_CS_URL, "arm").await;

    assert!(result, "R6 translation of 'arm' should succeed");
    assert_eq!(code, "40983000");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_r6_leg_returns_snomed_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r6_bundle()).await;

    let (result, code) = translate(&app, bundles::CM_URL_R6, bundles::ANATOMY_CS_URL, "leg").await;

    assert!(result);
    assert_eq!(code, "61685007");
}

// ── Unknown ConceptMap returns result=false ───────────────────────────────────
//
// Per FHIR spec §ConceptMap/$translate, a missing or unmatched mapping returns
// HTTP 200 with `result=false`, not 404.

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_unknown_concept_map_returns_result_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", "http://hts.test/cm/no-such-map"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir("/ConceptMap/$translate", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");

    assert!(!result, "unknown ConceptMap should return result=false");
}
