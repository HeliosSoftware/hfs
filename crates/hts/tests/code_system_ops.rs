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

// ── GET /CodeSystem (search) ──────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_no_filter_returns_bundle() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/CodeSystem").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Bundle");
    assert_eq!(body["type"], "searchset");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(!entries.is_empty(), "should return at least one CodeSystem");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_by_url_returns_match() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let url = format!("/CodeSystem?url={}", bundles::ANATOMY_CS_URL);
    let (status, body) = app.get_fhir(&url).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["url"], bundles::ANATOMY_CS_URL);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_by_url_no_match_returns_empty_bundle() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app
        .get_fhir("/CodeSystem?url=http://hts.test/nonexistent")
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Bundle");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.is_empty(), "no results for unknown URL");
    assert_eq!(body["total"], 0);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_by_name() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/CodeSystem?name=AnatomyCS").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["resource"]["name"], "AnatomyCS");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_by_status() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/CodeSystem?status=active").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let active_entries = body["entry"].as_array().expect("entry array").len();
    assert!(active_entries >= 1);

    let (status2, body2) = app.get_fhir("/CodeSystem?status=retired").await;
    assert_eq!(status2, StatusCode::OK, "{body2}");
    let retired_entries = body2["entry"].as_array().expect("entry array").len();
    assert_eq!(retired_entries, 0, "no retired systems in test bundle");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn search_code_systems_pagination_count_and_offset() {
    let app = TestApp::new();
    // Import the same bundle twice under different URLs via two separate bundles.
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // With _count=1 we get at most 1 result.
    let (status, body) = app.get_fhir("/CodeSystem?_count=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entry"].as_array().expect("entry array");
    assert!(entries.len() <= 1);

    // With _offset beyond the total we get an empty result.
    let (status2, body2) = app.get_fhir("/CodeSystem?_offset=9999").await;
    assert_eq!(status2, StatusCode::OK, "{body2}");
    let entries2 = body2["entry"].as_array().expect("entry array");
    assert!(entries2.is_empty());
}

// ── Phase 8: XML format support ───────────────────────────────────────────────

/// $lookup with `Accept: application/fhir+xml` should return valid FHIR XML.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_xml_response_contains_parameters_root() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app.post_fhir_xml("/CodeSystem/$lookup", req).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("<Parameters xmlns=\"http://hl7.org/fhir\">"),
        "expected FHIR XML root element, got: {body}"
    );
    assert!(body.contains("</Parameters>"), "got: {body}");
}

/// $lookup with `_format=xml` in the query string should also return XML.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_xml_via_format_query_param() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (status, body) = app
        .post_fhir_xml("/CodeSystem/$lookup?_format=xml", req)
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("<Parameters"), "expected XML, got: {body}");
}

/// GET /metadata with `Accept: application/fhir+xml` returns XML CapabilityStatement.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn metadata_xml_response_contains_capability_statement_root() {
    let app = TestApp::new();
    let (status, body, ct) = app.get_fhir_xml("/metadata").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        ct.contains("application/fhir+xml"),
        "expected XML content-type, got: {ct}"
    );
    assert!(
        body.contains("<CapabilityStatement xmlns=\"http://hl7.org/fhir\">"),
        "expected XML CapabilityStatement root, got: {body}"
    );
    assert!(body.contains("</CapabilityStatement>"), "got: {body}");
}

/// GET /metadata?_format=xml returns XML even without Accept header.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn metadata_xml_via_format_query_param() {
    let app = TestApp::new();
    // Use get_fhir_xml with the _format=xml query param.
    let (status, body, _ct) = app.get_fhir_xml("/metadata?_format=xml").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<CapabilityStatement"),
        "expected XML, got: {body}"
    );
}

// ── /CodeSystem/{id}/$lookup ──────────────────────────────────────────────────

/// POST /CodeSystem/{id}/$lookup returns same result as the system-level endpoint.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_by_id_post_returns_same_as_system_level() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    // System-level lookup
    let req = TestApp::params(&[
        ("system", "valueUri", bundles::ANATOMY_CS_URL),
        ("code", "valueCode", "arm"),
    ]);
    let (_, system_body) = app.post_fhir("/CodeSystem/$lookup", req).await;

    // Instance-level lookup (the CodeSystem has id="anatomy" in the r4_bundle)
    let (status, by_id_body) = app
        .post_fhir(
            "/CodeSystem/anatomy/$lookup",
            r#"{"resourceType":"Parameters","parameter":[{"name":"code","valueCode":"arm"}]}"#,
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{by_id_body}");
    assert_eq!(by_id_body["resourceType"], "Parameters");

    let system_display = system_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .expect("system-level missing display");

    let by_id_display = by_id_body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .expect("by-id missing display");

    assert_eq!(system_display, by_id_display);
}

/// GET /CodeSystem/{id}/$lookup?code=... returns the correct concept.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_by_id_get_returns_display() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, body) = app.get_fhir("/CodeSystem/anatomy/$lookup?code=leg").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["resourceType"], "Parameters");

    let display = body["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "display")
        .and_then(|p| p["valueString"].as_str())
        .expect("missing display");

    assert_eq!(display, "Leg");
}

/// POST /CodeSystem/{id}/$lookup with an unknown ID returns 404.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn lookup_by_id_unknown_id_returns_404() {
    let app = TestApp::new();
    app.import_bundle_ok(bundles::r4_bundle()).await;

    let (status, _body) = app
        .post_fhir(
            "/CodeSystem/does-not-exist/$lookup",
            r#"{"resourceType":"Parameters","parameter":[{"name":"code","valueCode":"arm"}]}"#,
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
