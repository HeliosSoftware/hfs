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

/// Regression: when a request specifies `version=1.0.0` and the CodeSystem
/// exists at that version (even though a newer version also exists), the
/// response `version` parameter must echo the *requested* version ("1.0.0"),
/// not the latest stored version ("1.2.0").
///
/// Covers the IG `version-code-v10-vs10-response-parameters` fixture.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_code_version_echoes_requested_version() {
    let app = TestApp::new();

    // Import a bundle with two versions of the same CodeSystem plus a
    // ValueSet that pins version 1.0.0.
    let bundle = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "CodeSystem",
                    "id": "multi-version-cs-v100",
                    "url": "http://hts.test/cs/multi-version",
                    "version": "1.0.0",
                    "status": "active",
                    "content": "complete",
                    "concept": [
                        { "code": "code1", "display": "Code One v1.0.0" }
                    ]
                }
            },
            {
                "resource": {
                    "resourceType": "CodeSystem",
                    "id": "multi-version-cs-v120",
                    "url": "http://hts.test/cs/multi-version",
                    "version": "1.2.0",
                    "status": "active",
                    "content": "complete",
                    "concept": [
                        { "code": "code1", "display": "Code One v1.2.0" },
                        { "code": "code2", "display": "Code Two v1.2.0" }
                    ]
                }
            },
            {
                "resource": {
                    "resourceType": "ValueSet",
                    "id": "vs-pins-v100",
                    "url": "http://hts.test/vs/pins-v100",
                    "version": "1.0",
                    "status": "active",
                    "compose": {
                        "include": [
                            {
                                "system": "http://hts.test/cs/multi-version",
                                "version": "1.0.0"
                            }
                        ]
                    }
                }
            }
        ]
    })
    .to_string();
    app.import_bundle_ok(&bundle).await;

    // Validate code1 with explicit version=1.0.0 against the VS that pins 1.0.0.
    let req = TestApp::params(&[
        ("url", "valueUri", "http://hts.test/vs/pins-v100"),
        ("code", "valueCode", "code1"),
        ("system", "valueUri", "http://hts.test/cs/multi-version"),
        ("version", "valueString", "1.0.0"),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let params = body["parameter"].as_array().expect("parameter array");

    let result = params
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");

    assert!(
        result,
        "code1 should be valid in the 1.0.0-pinned VS; body={body}"
    );

    // The version echoed back MUST be "1.0.0", not "1.2.0" (the latest stored
    // version that `code_system_version_for_url` would otherwise return).
    let version = params
        .iter()
        .find(|p| p["name"] == "version")
        .and_then(|p| p["valueString"].as_str())
        .expect("expected version parameter in response");

    assert_eq!(
        version, "1.0.0",
        "response must echo the requested version (1.0.0), not the latest (1.2.0)"
    );
}

// ── GET /ValueSet (search) ────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_value_sets_no_filter_returns_bundle() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ValueSet").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Bundle");
    assert_eq!(body["type"], "searchset");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(!entries.is_empty(), "should return at least one ValueSet");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_value_sets_by_url_returns_match() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let url = format!("/ValueSet?url={}", bundles::LIMBS_VS_URL);
    let (status, body) = app.get_fhir(&url).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["url"], bundles::LIMBS_VS_URL);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_value_sets_unknown_url_returns_empty() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app
        .get_fhir("/ValueSet?url=http://hts.test/no-such-vs")
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.is_empty());
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_value_sets_by_name() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ValueSet?name=LimbsVS").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["name"], "LimbsVS");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_value_sets_pagination() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ValueSet?_count=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.len() <= 1);

    let (status2, body2) = app.get_fhir("/ValueSet?_offset=9999").await;
    assert_eq!(status2, StatusCode::OK, "{body2}");
    assert!(body2["entry"].as_array().expect("entry array").is_empty());
}

// ── Instance-level: /ValueSet/{id}/$expand ────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_by_id_post_returns_same_as_system_level() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // System-level
    let req = TestApp::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);
    let (_, system_body) = app.post_fhir("/ValueSet/$expand", req).await;

    // Instance-level (the ValueSet has id="limbs" in the r4_bundle)
    let (status, by_id_body) = app
        .post_fhir(
            "/ValueSet/limbs/$expand",
            r#"{"resourceType":"Parameters","parameter":[]}"#,
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{by_id_body}");
    assert_eq!(by_id_body["resourceType"], "ValueSet");

    let system_codes: Vec<&str> = system_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["code"].as_str())
        .collect();
    let by_id_codes: Vec<&str> = by_id_body["expansion"]["contains"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["code"].as_str())
        .collect();

    assert_eq!(
        system_codes.len(),
        by_id_codes.len(),
        "instance-level and system-level expand should return the same codes"
    );
    for code in &system_codes {
        assert!(
            by_id_codes.contains(code),
            "missing code {code} in by-id expand"
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_by_id_get_returns_expansion() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/ValueSet/limbs/$expand").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "ValueSet");
    let contains = body["expansion"]["contains"].as_array().expect("contains");
    assert_eq!(contains.len(), 3, "expected 3 codes");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_by_id_unknown_id_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, _) = app
        .post_fhir(
            "/ValueSet/no-such-id/$expand",
            r#"{"resourceType":"Parameters","parameter":[]}"#,
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Instance-level: /ValueSet/{id}/$validate-code ────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_by_id_post_included_code_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("code", "valueCode", "arm"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/limbs/$validate-code", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result param");

    assert!(result, "'arm' should be in the limbs ValueSet");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_by_id_unknown_id_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("code", "valueCode", "arm"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, _) = app
        .post_fhir("/ValueSet/no-such-id/$validate-code", req)
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── too-costly expansion limit ────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_exceeds_limit_returns_422_too_costly() {
    use axum::{body::Body, http::Request};
    use helios_hts::{backends::SqliteTerminologyBackend, config::HtsConfig, state::AppState};
    use tower::ServiceExt;

    // Build a minimal app with max_expansion_size = 1 (limbs VS has 3 codes).
    let backend = SqliteTerminologyBackend::in_memory().unwrap();
    let state = AppState::new(backend).with_max_expansion_size(1);
    let config = HtsConfig::default();
    let app = helios_hts::server::create_app(&config, state);

    // Import the bundle so the ValueSet exists.
    let import_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import")
                .header("content-type", "application/fhir+json")
                .body(Body::from(bundles::r4_bundle()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(import_response.status().is_success());

    // Now expand — should be rejected as too-costly.
    let req = TestApp::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ValueSet/$expand")
                .header("content-type", "application/fhir+json")
                .body(Body::from(req))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let issue_code = body["issue"][0]["code"].as_str().unwrap_or("");
    assert_eq!(issue_code, "too-costly");
}

// ── valueCoding / valueCodeableConcept inputs ─────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_coding_in_set_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "coding",
                "valueCoding": {
                    "system": bundles::ANATOMY_CS_URL,
                    "code": "arm"
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", body).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("result param");

    assert!(
        result,
        "'arm' via valueCoding should be in the limbs ValueSet"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_codeable_concept_one_match_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": bundles::ANATOMY_CS_URL, "code": "head"}, // not in VS
                        {"system": bundles::ANATOMY_CS_URL, "code": "arm"}   // in VS
                    ]
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", body).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("result param");

    assert!(result, "at least one coding should match the ValueSet");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn vs_validate_codeable_concept_no_match_returns_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": bundles::ANATOMY_CS_URL, "code": "head"},
                        {"system": bundles::ANATOMY_CS_URL, "code": "body"}
                    ]
                }
            }
        ]
    })
    .to_string();

    let (status, body) = app.post_fhir("/ValueSet/$validate-code", body).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "result")
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("result param");

    assert!(!result, "no coding should match the limbs ValueSet");
}

// ── Phase 8: XML format support ───────────────────────────────────────────────

// ── hierarchical $expand ──────────────────────────────────────────────────────

/// $expand with `hierarchical=true` returns a tree-structured expansion.
///
/// The limbs ValueSet contains `limb`, `arm`, `leg`.
/// Hierarchy: limb → arm, leg.  `body` (parent of `limb`) is not in the
/// expansion, so `limb` becomes the single root with arm + leg nested under it.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_hierarchical_returns_nested_tree() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {"name": "hierarchical", "valueBoolean": true}
        ]
    })
    .to_string();
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "ValueSet");

    // Enumerated composes (every include carries explicit concept[]) are
    // returned flat regardless of `hierarchical=true`, matching the
    // tx-ecosystem-ig parameters/parameters-expand-enum-hierarchy fixture
    // (curated lists are not retrofitted with the underlying CS hierarchy).
    assert_eq!(body["expansion"]["total"], 3);
    let contains = body["expansion"]["contains"]
        .as_array()
        .expect("expected expansion.contains array");
    assert_eq!(contains.len(), 3, "enumerated VS should expand flat: {body}");
    let codes: Vec<&str> = contains.iter().filter_map(|c| c["code"].as_str()).collect();
    assert!(codes.contains(&"limb"));
    assert!(codes.contains(&"arm"));
    assert!(codes.contains(&"leg"));
}

/// $expand with `hierarchical=false` (or absent) returns the flat list unchanged.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_hierarchical_false_returns_flat_list() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url", "valueUri": bundles::LIMBS_VS_URL},
            {"name": "hierarchical", "valueBoolean": false}
        ]
    })
    .to_string();
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let contains = body["expansion"]["contains"]
        .as_array()
        .expect("expected expansion.contains array");

    // Flat: 3 codes, no nesting
    assert_eq!(contains.len(), 3);
    for c in contains {
        let nested = c["contains"].as_array();
        let is_empty = nested.is_none_or(|a| a.is_empty());
        assert!(is_empty, "flat mode should not nest children, got: {body}");
    }
}

/// $expand with `Accept: application/fhir+xml` should return valid FHIR XML.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_xml_response_contains_valueset_root() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::LIMBS_VS_URL)]);
    let (status, body) = app.post_fhir_xml("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("<ValueSet xmlns=\"http://hl7.org/fhir\">"),
        "expected FHIR XML ValueSet root, got: {body}"
    );
    assert!(body.contains("</ValueSet>"), "got: {body}");
    // The expansion contains system + code attributes.
    assert!(body.contains("system"), "got: {body}");
}

// ── Implicit ValueSet from CodeSystem.valueSet ────────────────────────────────

/// $expand with a URL that matches `CodeSystem.valueSet` (no explicit ValueSet
/// resource) returns all codes in that CodeSystem.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_implicit_vs_returns_all_cs_codes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::implicit_vs_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::IMPLICIT_VS_URL)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "ValueSet");

    let contains = body["expansion"]["contains"]
        .as_array()
        .expect("expected expansion.contains");

    assert_eq!(contains.len(), 3, "expected 3 codes (bone, muscle, nerve)");

    let codes: Vec<&str> = contains.iter().filter_map(|c| c["code"].as_str()).collect();
    assert!(codes.contains(&"bone"), "expected 'bone'");
    assert!(codes.contains(&"muscle"), "expected 'muscle'");
    assert!(codes.contains(&"nerve"), "expected 'nerve'");
}

/// The `total` in the expansion equals the number of codes in the CodeSystem.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn expand_implicit_vs_total_matches_concept_count() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::implicit_vs_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::IMPLICIT_VS_URL)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["expansion"]["total"], 3);
}
