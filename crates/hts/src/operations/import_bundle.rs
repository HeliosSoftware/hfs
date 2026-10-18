//! `POST /import` — FHIR Bundle import endpoint.
//!
//! Accepts a FHIR Bundle as `application/fhir+json` and imports all contained
//! `CodeSystem`, `ValueSet`, and `ConceptMap` resources into the terminology store.
//! Returns an [`ImportResponse`] JSON object with counts of imported resources
//! and any non-fatal errors.

use axum::{
    Json,
    body::Bytes,
    extract::{RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::tenant::TenantContext;
use serde::Serialize;

use crate::import::BundleImportBackend;
use crate::operations::params::parse_query_string;
use crate::state::AppState;
use crate::traits::TerminologyBackend;

/// Whether the request asked for a closure rebuild via `?finalize=true`.
///
/// Chunked importers set this on their **final** chunk only. Without it, a
/// bulk load into a running server leaves the system with hierarchy edges but
/// no closure rows until the next restart, which silently disables `$subsumes`
/// for that system — see [`BundleImportBackend::rebuild_missing_closures`].
///
/// Parsed from the raw query string like every other HTS handler, so an
/// unrecognised value is ignored rather than turned into a framework 400.
fn wants_finalize(raw_query: Option<&str>) -> bool {
    parse_query_string(raw_query.unwrap_or(""))
        .iter()
        .any(|(k, v)| k == "finalize" && v.eq_ignore_ascii_case("true"))
}

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
    /// Number of code systems whose concept closure was rebuilt. Only ever
    /// non-zero when the request passed `finalize=true`.
    #[serde(skip_serializing_if = "is_zero")]
    pub closures_rebuilt: usize,
    /// Non-fatal errors encountered during import (malformed resources, missing
    /// required fields).  An HTTP 200 is still returned even when this is
    /// non-empty; the caller should inspect this field to detect partial failures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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
pub async fn import_handler<B>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    body: Bytes,
) -> Response
where
    B: TerminologyBackend + BundleImportBackend,
{
    let ctx = TenantContext::system();
    let finalize = wants_finalize(raw.as_deref());

    match state.backend.import_bundle(&ctx, &body).await {
        Ok(stats) => {
            // Invalidate cached expansions — newly imported terminology may
            // change which codes belong to a ValueSet.
            state.clear_expand_cache();

            let mut errors = stats.errors;
            let mut closures_rebuilt = 0usize;
            if finalize {
                match state.backend.rebuild_missing_closures().await {
                    Ok(n) => closures_rebuilt = n,
                    // Non-fatal: the concepts are stored either way, and the
                    // startup migration will still rebuild. Surfaced in
                    // `errors` so the caller does not assume it succeeded.
                    Err(e) => errors.push(format!("Closure rebuild failed: {e}")),
                }
            }

            // Return 207 Multi-Status when non-fatal errors were encountered so
            // callers can distinguish a clean import from a partial one.
            let status = if errors.is_empty() {
                StatusCode::OK
            } else {
                StatusCode::MULTI_STATUS
            };
            let response = ImportResponse {
                code_systems: stats.code_systems,
                value_sets: stats.value_sets,
                concept_maps: stats.concept_maps,
                concepts: stats.concepts,
                closures_rebuilt,
                errors,
            };
            (status, Json(response)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use crate::backends::sqlite::SqliteTerminologyBackend;
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

    /// A chunked import leaves `concept_closure` empty, and `finalize=true`
    /// rebuilds it.
    ///
    /// Every batch deletes the system's closure rows, and only a batch that
    /// found the system with *zero* concepts rebuilds them. So chunk 2 builds
    /// the closure and chunk 3 deletes it again without rebuilding, leaving a
    /// system with hierarchy edges but no closure — which silently answers
    /// `not-subsumed` for a genuine parent/child pair. Reproduced against
    /// `$subsumes` rather than the table so the assertion is about behaviour.
    #[tokio::test]
    async fn finalize_rebuilds_closure_dropped_by_a_chunked_import() {
        const SYS: &str = "http://example.org/chunked-cs";

        async fn post(app: &axum::Router, uri: &str, body: String) -> StatusCode {
            let req = Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/fhir+json")
                .body(Body::from(body))
                .unwrap();
            app.clone().oneshot(req).await.unwrap().status()
        }

        async fn subsumes(app: &axum::Router, a: &str, b: &str) -> String {
            let uri = format!("/CodeSystem/$subsumes?system={SYS}&codeA={a}&codeB={b}",);
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            json["parameter"]
                .as_array()
                .and_then(|ps| {
                    ps.iter()
                        .find(|p| p["name"] == "outcome")
                        .and_then(|p| p["valueCode"].as_str())
                })
                .unwrap_or("<none>")
                .to_string()
        }

        fn chunk(concepts: &str) -> String {
            format!(
                r#"{{"resourceType":"Bundle","type":"collection","entry":[{{"resource":{{
                     "resourceType":"CodeSystem","id":"chunked-cs","url":"{SYS}",
                     "version":"1.0","name":"ChunkedCS","status":"active",
                     "content":"complete","hierarchyMeaning":"is-a",
                     "property":[{{"code":"parent","uri":"http://hl7.org/fhir/concept-properties#parent","type":"code"}}],
                     "concept":[{concepts}]
                   }}}}]}}"#
            )
        }

        let app = app();

        // Chunk 1: metadata only, no concepts and so no hierarchy to close over.
        assert_eq!(post(&app, "/import", chunk("")).await, StatusCode::OK);
        // Chunk 2: the parent plus one child. This batch does build the closure.
        assert_eq!(
            post(
                &app,
                "/import",
                chunk(
                    r#"{"code":"A","display":"A"},
                       {"code":"A.0","display":"A zero","property":[{"code":"parent","valueCode":"A"}]}"#
                )
            )
            .await,
            StatusCode::OK
        );
        // Chunk 3: another child. The system is no longer empty, so this batch
        // deletes the closure and skips the rebuild.
        assert_eq!(
            post(
                &app,
                "/import",
                chunk(
                    r#"{"code":"A.1","display":"A one","property":[{"code":"parent","valueCode":"A"}]}"#
                )
            )
            .await,
            StatusCode::OK
        );

        assert_eq!(
            subsumes(&app, "A", "A.0").await,
            "not-subsumed",
            "chunked import should have left the closure empty"
        );

        // Finalizing rebuilds it, and reports that it did.
        let req = Request::builder()
            .method("POST")
            .uri("/import?finalize=true")
            .header("content-type", "application/fhir+json")
            .body(Body::from(chunk("")))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["closures_rebuilt"], 1);

        assert_eq!(subsumes(&app, "A", "A.0").await, "subsumes");
        assert_eq!(subsumes(&app, "A", "A.1").await, "subsumes");
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
