//! `POST /import` — FHIR Bundle import endpoint.
//!
//! Accepts a FHIR Bundle as `application/fhir+json` and imports all contained
//! `CodeSystem`, `ValueSet`, and `ConceptMap` resources into the terminology store.
//! Returns an [`ImportResponse`] JSON object with counts of imported resources
//! and any non-fatal errors.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::tenant::TenantContext;
use serde::Serialize;

use crate::import::BundleImportBackend;
use crate::state::AppState;
use crate::traits::TerminologyBackend;

/// Response body returned by `POST /import`.
#[derive(Debug, Serialize)]
pub struct ImportResponse {
    /// Number of `CodeSystem` resources successfully imported.
    pub code_systems: u32,
    /// Number of `ValueSet` resources successfully imported.
    pub value_sets: u32,
    /// Number of `ConceptMap` resources successfully imported.
    pub concept_maps: u32,
    /// Total number of concept rows inserted.
    pub concepts: u32,
    /// Non-fatal errors encountered during import (malformed resources, missing
    /// required fields).  An HTTP 200 is still returned even when this is
    /// non-empty; the caller should inspect this field to detect partial failures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Handler for `POST /import`.
///
/// Reads the raw request body as a FHIR Bundle (JSON) and delegates to the
/// backend's [`BundleImportBackend::import_bundle`] implementation.
///
/// # Responses
///
/// | Status | Condition |
/// |--------|-----------|
/// | 200 OK | Import ran; check `errors` field for non-fatal issues |
/// | 400 Bad Request | Body is not valid JSON or not a FHIR Bundle |
/// | 500 Internal Server Error | Storage or task failure |
pub async fn import_handler<B>(State(state): State<AppState<B>>, body: Bytes) -> Response
where
    B: TerminologyBackend + BundleImportBackend,
{
    let ctx = TenantContext::system();

    match state.backend.import_bundle(&ctx, &body).await {
        Ok(stats) => {
            // Return 207 Multi-Status when non-fatal errors were encountered so
            // callers can distinguish a clean import from a partial one.
            let status = if stats.has_errors() {
                StatusCode::MULTI_STATUS
            } else {
                StatusCode::OK
            };
            let response = ImportResponse {
                code_systems: stats.code_systems,
                value_sets: stats.value_sets,
                concept_maps: stats.concept_maps,
                concepts: stats.concepts,
                errors: stats.errors,
            };
            (status, Json(response)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    use crate::backend::sqlite::SqliteTerminologyBackend;
    use crate::config::HtsConfig;
    use crate::server::create_app;
    use crate::state::AppState;

    fn app() -> axum::Router {
        let backend =
            SqliteTerminologyBackend::in_memory().expect("in-memory backend should initialise");
        let state = AppState::new(backend);
        create_app(&HtsConfig::default(), state)
    }

    fn minimal_bundle() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-handler",
                "url": "http://example.org/handler-cs",
                "version": "1.0",
                "name": "HandlerCS",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "X", "display": "Concept X" }
                ]
              }
            }
          ]
        }"#
    }

    #[tokio::test]
    async fn import_returns_200_with_stats() {
        let app = app();

        let req = Request::builder()
            .method("POST")
            .uri("/import")
            .header("content-type", "application/fhir+json")
            .body(Body::from(minimal_bundle()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["code_systems"], 1);
        assert_eq!(json["concepts"], 1);
    }

    #[tokio::test]
    async fn import_non_bundle_returns_400() {
        let app = app();

        let req = Request::builder()
            .method("POST")
            .uri("/import")
            .header("content-type", "application/fhir+json")
            .body(Body::from(r#"{"resourceType":"Patient","id":"p1"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn import_invalid_json_returns_400() {
        let app = app();

        let req = Request::builder()
            .method("POST")
            .uri("/import")
            .header("content-type", "application/fhir+json")
            .body(Body::from("not json"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 400);
    }
}
