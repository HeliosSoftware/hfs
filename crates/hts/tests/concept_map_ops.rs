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

// ── GET /ConceptMap (search) ──────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_concept_maps_no_filter_returns_bundle() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ConceptMap").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Bundle");
    assert_eq!(body["type"], "searchset");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(!entries.is_empty(), "should return at least one ConceptMap");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_concept_maps_by_url_returns_match() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let url = format!("/ConceptMap?url={}", bundles::CM_URL_R4);
    let (status, body) = app.get_fhir(&url).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["url"], bundles::CM_URL_R4);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_concept_maps_unknown_url_returns_empty() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app
        .get_fhir("/ConceptMap?url=http://hts.test/no-such-cm")
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.is_empty());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_concept_maps_by_name() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ConceptMap?name=AnatomyMapR4").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["name"], "AnatomyMapR4");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_concept_maps_pagination() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ConceptMap?_count=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.len() <= 1);

    let (status2, body2) = app.get_fhir("/ConceptMap?_offset=9999").await;
    assert_eq!(status2, StatusCode::OK, "{body2}");
    assert!(body2["entry"].as_array().expect("entry array").is_empty());
}

// ── Instance-level: /ConceptMap/{id}/$translate ───────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_by_id_post_returns_same_as_system_level() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // System-level
    let req_sys = TestApp::params(&[
        ("url", "valueUri", bundles::CM_URL_R4),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (_, system_body) = app.post_fhir("/ConceptMap/$translate", req_sys).await;

    // Instance-level (the ConceptMap has id="anatomy-r4" in the r4_bundle)
    let req_id = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, by_id_body) = app
        .post_fhir("/ConceptMap/anatomy-r4/$translate", req_id)
        .await;

    assert_eq!(status, StatusCode::OK, "{by_id_body}");

    let sys_result = system_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("system result");

    let id_result = by_id_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("by-id result");

    assert_eq!(
        sys_result, id_result,
        "instance-level and system-level translate should agree"
    );
    assert!(id_result, "translation of 'arm' by id should succeed");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_by_id_get_returns_match() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app
        .get_fhir(&format!(
            "/ConceptMap/anatomy-r4/$translate?code=arm&system={}",
            bundles::ANATOMY_CS_URL
        ))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("result param");

    assert!(result, "GET by-id translate should find 'arm'");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn translate_by_id_unknown_id_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, _) = app
        .post_fhir("/ConceptMap/no-such-id/$translate", req)
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
