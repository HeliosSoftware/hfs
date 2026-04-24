//! Integration tests for ECL-filtered `ValueSet/$expand`.
//!
//! These tests verify that ECL expressions embedded in ValueSet `compose.filter`
//! entries are correctly evaluated during `$expand`, and that the `is-a` filter
//! shorthand is also handled.
//!
//! The anatomy CodeSystem used in all tests has this hierarchy:
//! ```text
//! body
//! ├─ limb
//! │   ├─ arm
//! │   └─ leg
//! └─ head
//! ```

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};

// ── helpers ────────────────────────────────────────────────────────────────────

/// POST `$expand` for `url`, returning the sorted list of codes in the expansion.
async fn expand_codes(app: &TestApp, url: &str) -> Vec<String> {
    let req = TestApp::params(&[("url", "valueUri", url)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "expand failed for {url}: {body}");
    let mut codes: Vec<String> = body["expansion"]["contains"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|e| e["code"].as_str().map(String::from))
        .collect();
    codes.sort();
    codes
}

// ── constraint filter (ECL) ────────────────────────────────────────────────────

/// `<< limb` — descendants including self → limb, arm, leg
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_constraint_desc_and_self() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_CONSTRAINT_VS_URL).await;
    assert_eq!(codes, ["arm", "leg", "limb"], "got: {codes:?}");
}

/// `concept is-a limb` is equivalent to `<< limb`
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_isa_filter_equivalent_to_descendants_and_self() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_ISA_VS_URL).await;
    assert_eq!(codes, ["arm", "leg", "limb"], "got: {codes:?}");
}

/// `< limb` — strict descendants (excludes limb itself) → arm, leg
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_strict_descendants_excludes_self() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_STRICT_DESC_VS_URL).await;
    assert_eq!(codes, ["arm", "leg"], "got: {codes:?}");
}

/// `>> arm` — ancestors including self → arm, limb, body
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_ancestors_including_self() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_ANCESTORS_VS_URL).await;
    assert_eq!(codes, ["arm", "body", "limb"], "got: {codes:?}");
}

/// `<< body MINUS << limb` — all body descendants except limb and its subtree
/// Expected: body, head  (arm and leg are under limb so they are excluded)
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_minus_removes_subtree() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_MINUS_VS_URL).await;
    assert_eq!(codes, ["body", "head"], "got: {codes:?}");
}

/// `<< limb OR head` — union: limb subtree plus head → arm, head, leg, limb
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_or_union_of_subtree_and_single_code() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_OR_VS_URL).await;
    assert_eq!(codes, ["arm", "head", "leg", "limb"], "got: {codes:?}");
}

/// Two filters on the same include clause are ANDed together.
/// `<< limb` AND `<< body` — intersection of limb subtree ∩ body subtree
/// limb subtree = {limb, arm, leg}; body subtree = {body, limb, arm, leg, head}
/// intersection = {limb, arm, leg}
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_two_filters_are_intersected() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes = expand_codes(&app, bundles::ECL_AND_FILTER_VS_URL).await;
    assert_eq!(codes, ["arm", "leg", "limb"], "got: {codes:?}");
}

// ── expansion total reflects ECL result count ──────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_expansion_total_matches_returned_codes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::ECL_CONSTRAINT_VS_URL)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let total = body["expansion"]["total"].as_u64().expect("expected total");
    let count = body["expansion"]["contains"]
        .as_array()
        .map(|a| a.len() as u64)
        .unwrap_or(0);
    assert_eq!(total, count, "expansion.total should match contains length");
}

// ── ECL + pagination ───────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_expansion_with_count_paginates() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url",   "valueUri": bundles::ECL_CONSTRAINT_VS_URL},
            {"name": "count", "valueInteger": 2}
        ]
    })
    .to_string();

    let (status, resp) = app.post_fhir("/ValueSet/$expand", body).await;
    assert_eq!(status, StatusCode::OK, "{resp}");

    let contains = resp["expansion"]["contains"]
        .as_array()
        .expect("expected contains");
    assert_eq!(
        contains.len(),
        2,
        "expected 2 codes with count=2, got: {resp}"
    );
    // total should still reflect the full expansion (3 codes)
    let total = resp["expansion"]["total"].as_u64().unwrap_or(0);
    assert_eq!(total, 3, "total should be 3 regardless of page size");
}

// ── ECL + $validate-code ───────────────────────────────────────────────────────

/// `arm` is a descendant of `limb`, so it should be valid inside the ECL
/// ValueSet that expands to {limb, arm, leg}.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_validate_code_member_returns_true() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", bundles::ECL_CONSTRAINT_VS_URL),
        ("code", "valueCode", "arm"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .and_then(|p| p.iter().find(|x| x["name"] == "result"))
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");
    assert!(result, "arm should be valid in << limb expansion");
}

/// `head` is NOT a descendant of `limb`, so it should be invalid.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_validate_code_non_member_returns_false() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let req = TestApp::params(&[
        ("url", "valueUri", bundles::ECL_CONSTRAINT_VS_URL),
        ("code", "valueCode", "head"),
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
    ]);
    let (status, body) = app.post_fhir("/ValueSet/$validate-code", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let result = body["parameter"]
        .as_array()
        .and_then(|p| p.iter().find(|x| x["name"] == "result"))
        .and_then(|p| p["valueBoolean"].as_bool())
        .expect("expected result parameter");
    assert!(!result, "head should not be valid in << limb expansion");
}

// ── ECL cache invalidation ─────────────────────────────────────────────────────

/// A second `$expand` call returns the same codes (cache hit).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_expansion_is_cached_and_consistent() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let codes_first = expand_codes(&app, bundles::ECL_CONSTRAINT_VS_URL).await;
    let codes_second = expand_codes(&app, bundles::ECL_CONSTRAINT_VS_URL).await;
    assert_eq!(
        codes_first, codes_second,
        "cached expansion should be identical"
    );
}

// ── wildcard ECL (*) ───────────────────────────────────────────────────────────

/// `*` in ECL means all concepts in the code system.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_wildcard_expands_all_codes() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    // Create a ValueSet with a wildcard ECL filter dynamically via import.
    let wildcard_bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [{
        "resource": {
          "resourceType": "ValueSet",
          "id": "ecl-wildcard",
          "url": "http://hts.test/vs/ecl-wildcard",
          "version": "1.0",
          "name": "EclWildcard",
          "status": "active",
          "compose": {
            "include": [{
              "system": "http://hts.test/anatomy",
              "filter": [{"property": "constraint", "op": "=", "value": "*"}]
            }]
          }
        }
      }]
    }"#;
    app.import_bundle_ok(wildcard_bundle).await;

    let codes = expand_codes(&app, "http://hts.test/vs/ecl-wildcard").await;
    assert_eq!(
        codes,
        ["arm", "body", "head", "leg", "limb"],
        "wildcard should expand all 5 codes"
    );
}

// ── unknown code system ────────────────────────────────────────────────────────

/// A ValueSet with an ECL filter referencing an unknown code system returns an
/// empty expansion (the include clause is skipped with a warning).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_unknown_system_skipped_gracefully() {
    let app = TestApp::new();

    let bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [{
        "resource": {
          "resourceType": "ValueSet",
          "id": "ecl-unknown-sys",
          "url": "http://hts.test/vs/ecl-unknown-sys",
          "version": "1.0",
          "name": "EclUnknownSys",
          "status": "active",
          "compose": {
            "include": [{
              "system": "http://hts.test/no-such-system",
              "filter": [{"property": "constraint", "op": "=", "value": "<< 404684003"}]
            }]
          }
        }
      }]
    }"#;
    app.import_bundle_ok(bundle).await;

    let req = TestApp::params(&[("url", "valueUri", "http://hts.test/vs/ecl-unknown-sys")]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let contains = body["expansion"]["contains"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(
        contains, 0,
        "unknown system should yield an empty expansion"
    );
}

// ── invalid ECL expression ─────────────────────────────────────────────────────

/// An invalid ECL expression returns a 400 Bad Request.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_invalid_expression_returns_error() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [{
        "resource": {
          "resourceType": "ValueSet",
          "id": "ecl-bad-expr",
          "url": "http://hts.test/vs/ecl-bad-expr",
          "version": "1.0",
          "name": "EclBadExpr",
          "status": "active",
          "compose": {
            "include": [{
              "system": "http://hts.test/anatomy",
              "filter": [{"property": "constraint", "op": "=", "value": "<<< broken %%%"}]
            }]
          }
        }
      }]
    }"#;
    app.import_bundle_ok(bundle).await;

    let req = TestApp::params(&[("url", "valueUri", "http://hts.test/vs/ecl-bad-expr")]);
    let (status, _body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "invalid ECL should return 400"
    );
}

// ── ECL + hierarchical mode ────────────────────────────────────────────────────

/// ECL expansion with `hierarchical=true` should return a nested tree.
/// `<< limb` yields {limb, arm, leg}; in tree mode limb is the root and
/// arm/leg are its children.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_hierarchical_expansion_returns_tree() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let body = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "url",          "valueUri":     bundles::ECL_CONSTRAINT_VS_URL},
            {"name": "hierarchical", "valueBoolean": true}
        ]
    })
    .to_string();

    let (status, resp) = app.post_fhir("/ValueSet/$expand", body).await;
    assert_eq!(status, StatusCode::OK, "{resp}");

    let roots = resp["expansion"]["contains"]
        .as_array()
        .expect("expected contains");
    // Only one root in the ECL result: limb
    assert_eq!(roots.len(), 1, "expected 1 root (limb), got: {resp}");
    assert_eq!(roots[0]["code"], "limb");

    let children = roots[0]["contains"]
        .as_array()
        .expect("expected children of limb");
    assert_eq!(children.len(), 2, "expected arm and leg as children");
    let mut child_codes: Vec<&str> = children.iter().filter_map(|c| c["code"].as_str()).collect();
    child_codes.sort();
    assert_eq!(child_codes, ["arm", "leg"]);
}

// ── ECL display names are preserved ───────────────────────────────────────────

/// Expansion entries should carry the display name from the CodeSystem.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_expansion_includes_display_names() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let req = TestApp::params(&[("url", "valueUri", bundles::ECL_CONSTRAINT_VS_URL)]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let contains = body["expansion"]["contains"]
        .as_array()
        .expect("expected contains");

    for entry in contains {
        assert!(
            entry["display"].is_string(),
            "expected display to be a string, got: {entry}"
        );
    }

    // Spot-check: arm should have display "Arm"
    let arm = contains
        .iter()
        .find(|e| e["code"] == "arm")
        .expect("arm not found");
    assert_eq!(arm["display"], "Arm");
}

// ── unrecognised filter property/op ───────────────────────────────────────────

/// A filter with an unrecognised `property`/`op` must yield an empty expansion,
/// NOT all concepts in the code system.  Silently expanding to all concepts
/// would violate the ValueSet author's intent.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn ecl_unrecognized_filter_yields_empty_expansion() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::ecl_bundle()).await;

    let bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [{
        "resource": {
          "resourceType": "ValueSet",
          "id": "ecl-unknown-filter",
          "url": "http://hts.test/vs/ecl-unknown-filter",
          "version": "1.0",
          "name": "EclUnknownFilter",
          "status": "active",
          "compose": {
            "include": [{
              "system": "http://hts.test/anatomy",
              "filter": [{"property": "someUnknownProperty", "op": "exists", "value": "true"}]
            }]
          }
        }
      }]
    }"#;
    app.import_bundle_ok(bundle).await;

    let req = TestApp::params(&[("url", "valueUri", "http://hts.test/vs/ecl-unknown-filter")]);
    let (status, body) = app.post_fhir("/ValueSet/$expand", req).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let contains = body["expansion"]["contains"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(
        contains, 0,
        "unrecognised filter must produce an empty expansion, not all concepts"
    );
}
