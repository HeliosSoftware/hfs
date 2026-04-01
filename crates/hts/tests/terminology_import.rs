//! Integration tests for `POST /import`.
//!
//! Each test imports a FHIR Bundle using a version-appropriate serialisation
//! and verifies that the returned `ImportStats` reflect the expected resource
//! counts.  A separate test checks that malformed resources produce a 207
//! Multi-Status with a non-empty `errors` array.

mod common;

use axum::http::StatusCode;
use common::{TestApp, bundles};

// ── R4 import ─────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_r4_bundle_returns_correct_counts() {
    let app = TestApp::new();
    let (status, body) = app.import_bundle(bundles::r4_bundle()).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 OK, got {status}: {body}"
    );

    assert_eq!(
        body["code_systems"].as_u64(),
        Some(1),
        "expected 1 CodeSystem"
    );
    assert_eq!(body["value_sets"].as_u64(), Some(1), "expected 1 ValueSet");
    assert_eq!(
        body["concept_maps"].as_u64(),
        Some(1),
        "expected 1 ConceptMap"
    );
    // 5 concepts: body, limb, arm, leg, head
    assert_eq!(body["concepts"].as_u64(), Some(5), "expected 5 concepts");
}

// ── R4B import ────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_r4b_bundle_returns_correct_counts() {
    let app = TestApp::new();
    let (status, body) = app.import_bundle(bundles::r4b_bundle()).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 OK, got {status}: {body}"
    );

    assert_eq!(
        body["code_systems"].as_u64(),
        Some(1),
        "expected 1 CodeSystem"
    );
    assert_eq!(body["value_sets"].as_u64(), Some(1), "expected 1 ValueSet");
    assert_eq!(
        body["concept_maps"].as_u64(),
        Some(1),
        "expected 1 ConceptMap"
    );
    assert_eq!(body["concepts"].as_u64(), Some(5), "expected 5 concepts");
}

// ── R5 import (sourceScope/targetScope + relationship) ────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_r5_bundle_returns_correct_counts() {
    let app = TestApp::new();
    let (status, body) = app.import_bundle(bundles::r5_bundle()).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 OK, got {status}: {body}"
    );

    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

// ── R6 import ─────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_r6_bundle_returns_correct_counts() {
    let app = TestApp::new();
    let (status, body) = app.import_bundle(bundles::r6_bundle()).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 OK, got {status}: {body}"
    );

    assert_eq!(body["code_systems"].as_u64(), Some(1));
    assert_eq!(body["value_sets"].as_u64(), Some(1));
    assert_eq!(body["concept_maps"].as_u64(), Some(1));
    assert_eq!(body["concepts"].as_u64(), Some(5));
}

// ── Idempotent re-import ──────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn reimport_same_bundle_is_idempotent() {
    let app = TestApp::new();

    // First import
    let (status, _) = app.import_bundle(bundles::r4_bundle()).await;
    assert_eq!(status, StatusCode::OK);

    // Second import of the same bundle must not fail
    let (status2, body2) = app.import_bundle(bundles::r4_bundle()).await;
    assert!(
        status2 == StatusCode::OK || status2 == StatusCode::MULTI_STATUS,
        "second import should succeed, got {status2}: {body2}"
    );
}

// ── Non-Bundle payload returns 400 ────────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_non_bundle_returns_400() {
    let app = TestApp::new();

    let not_a_bundle = r#"{"resourceType":"Patient","id":"p1"}"#;
    let (status, _) = app.import_bundle(not_a_bundle).await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected 400 for non-Bundle payload"
    );
}

// ── Malformed entry produces 207 with errors ──────────────────────────────────

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn import_bundle_with_malformed_cs_returns_207() {
    let app = TestApp::new();

    // A bundle where one CodeSystem is missing its `url` field (non-fatal).
    let bundle = r#"{
      "resourceType": "Bundle",
      "type": "collection",
      "entry": [
        {
          "resource": {
            "resourceType": "CodeSystem",
            "id": "no-url-cs",
            "version": "1.0",
            "name": "NoUrlCS",
            "status": "active",
            "content": "complete",
            "concept": [{"code": "X", "display": "X"}]
          }
        },
        {
          "resource": {
            "resourceType": "CodeSystem",
            "id": "good-cs",
            "url": "http://hts.test/good-cs",
            "version": "1.0",
            "name": "GoodCS",
            "status": "active",
            "content": "complete",
            "concept": [{"code": "A", "display": "A"}]
          }
        }
      ]
    }"#;

    let (status, body) = app.import_bundle(bundle).await;

    assert_eq!(
        status,
        StatusCode::MULTI_STATUS,
        "expected 207 Multi-Status when a resource has errors: {body}"
    );

    let errors = body["errors"].as_array().expect("expected errors array");
    assert!(
        !errors.is_empty(),
        "expected at least one error for the no-url CodeSystem"
    );

    // The valid CodeSystem should still have been imported.
    assert_eq!(
        body["code_systems"].as_u64(),
        Some(1),
        "the valid CodeSystem should have been imported despite the error"
    );
}
