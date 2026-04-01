use axum::{Json, extract::State, response::IntoResponse};
use serde_json::{Value, json};

use crate::import::BundleImportBackend;
use crate::state::AppState;
use crate::traits::{TerminologyBackend, TerminologyMetadata};

const HTS_VERSION: &str = env!("CARGO_PKG_VERSION");
const HTS_NAME: &str = "Helios Terminology Service";

/// GET /metadata — returns a FHIR TerminologyCapabilities resource.
///
/// Lists all supported operations, known code systems, and backend metadata.
/// Conforms to the FHIR R4 TerminologyCapabilities resource structure.
pub async fn metadata_handler<B>(State(state): State<AppState<B>>) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend + Clone,
{
    let backend = state.backend();
    let capabilities = build_terminology_capabilities(backend);
    Json(capabilities)
}

/// Build the `TerminologyCapabilities` JSON value from backend metadata.
///
/// Separated from the handler so it can be tested independently without
/// needing a running Axum server.
pub fn build_terminology_capabilities(backend: &impl TerminologyMetadata) -> Value {
    let code_systems: Vec<Value> = backend
        .supported_systems()
        .into_iter()
        .map(|url| {
            json!({
                "uri": url,
                "subsumption": backend.supports_subsumption()
            })
        })
        .collect();

    json!({
        "resourceType": "TerminologyCapabilities",
        "status": "active",
        "experimental": false,
        "kind": "terminology",
        "software": {
            "name": HTS_NAME,
            "version": HTS_VERSION
        },
        "implementation": {
            "description": "Helios Terminology Service SQLite backend"
        },
        "codeSearch": "all",
        "codeSystem": code_systems,
        "expansion": {
            "hierarchical": false,
            "paging": true,
            "incomplete": false
        },
        "validateCode": {
            "translations": false
        },
        "translation": {
            "needsMap": true
        },
        "closure": {
            "translation": false
        },
        // List all six supported FHIR terminology operations.
        // Each entry names an operation that the service will respond to.
        "operationSupported": [
            { "name": "$lookup",        "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup" },
            { "name": "$validate-code", "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-validate-code" },
            { "name": "$subsumes",      "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-subsumes" },
            { "name": "$expand",        "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-expand" },
            { "name": "$translate",     "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-translate" },
            { "name": "$closure",       "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-closure" }
        ]
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    use crate::backend::sqlite::SqliteTerminologyBackend;

    // Helper: build a fresh in-memory backend.
    fn backend() -> SqliteTerminologyBackend {
        SqliteTerminologyBackend::in_memory().expect("in-memory backend must initialise")
    }

    // ── Unit tests on build_terminology_capabilities ───────────────────────────

    #[test]
    fn resource_type_is_terminology_capabilities() {
        let caps = build_terminology_capabilities(&backend());
        assert_eq!(caps["resourceType"], "TerminologyCapabilities");
    }

    #[test]
    fn status_is_active() {
        let caps = build_terminology_capabilities(&backend());
        assert_eq!(caps["status"], "active");
    }

    #[test]
    fn software_name_and_version_present() {
        let caps = build_terminology_capabilities(&backend());
        assert_eq!(caps["software"]["name"], HTS_NAME);
        // version is the crate version from CARGO_PKG_VERSION — just check it's a non-empty string.
        let ver = caps["software"]["version"].as_str().unwrap_or("");
        assert!(!ver.is_empty(), "software.version must not be empty");
    }

    #[test]
    fn all_six_operations_listed() {
        let caps = build_terminology_capabilities(&backend());
        let ops = caps["operationSupported"]
            .as_array()
            .expect("operationSupported must be an array");
        let op_names: Vec<&str> = ops.iter().filter_map(|o| o["name"].as_str()).collect();

        let expected = [
            "$lookup",
            "$validate-code",
            "$subsumes",
            "$expand",
            "$translate",
            "$closure",
        ];
        for name in &expected {
            assert!(
                op_names.contains(name),
                "operation {name} missing from operationSupported"
            );
        }
        assert_eq!(op_names.len(), 6, "expected exactly 6 operations");
    }

    #[test]
    fn code_system_array_empty_on_fresh_backend() {
        let caps = build_terminology_capabilities(&backend());
        let arr = caps["codeSystem"]
            .as_array()
            .expect("codeSystem must be an array");
        assert!(arr.is_empty(), "fresh backend should have no code systems");
    }

    #[test]
    fn code_system_entry_includes_subsumption_flag() {
        let b = backend();
        // Seed a code system directly into the DB.
        let conn = b.pool().get().unwrap();
        conn.execute(
            "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', 'active', 'complete', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        drop(conn);

        let caps = build_terminology_capabilities(&b);
        let arr = caps["codeSystem"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["uri"], "http://example.org/cs");
        assert_eq!(arr[0]["subsumption"], true);
    }

    #[test]
    fn multiple_code_systems_all_listed() {
        let b = backend();
        let conn = b.pool().get().unwrap();
        for (id, url) in [("cs1", "http://a.org"), ("cs2", "http://b.org")] {
            conn.execute(
                "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
                 VALUES (?1, ?2, 'active', 'complete', '2024-01-01', '2024-01-01')",
                rusqlite::params![id, url],
            )
            .unwrap();
        }
        drop(conn);

        let caps = build_terminology_capabilities(&b);
        let arr = caps["codeSystem"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let urls: Vec<&str> = arr.iter().filter_map(|e| e["uri"].as_str()).collect();
        assert!(urls.contains(&"http://a.org"));
        assert!(urls.contains(&"http://b.org"));
    }

    // ── Integration test: HTTP GET /metadata returns 200 ──────────────────────

    #[tokio::test]
    async fn get_metadata_returns_200() {
        use crate::state::AppState;

        let b = SqliteTerminologyBackend::in_memory().unwrap();
        let state = AppState::new(b);

        let app = Router::new()
            .route(
                "/metadata",
                get(metadata_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn get_metadata_body_is_terminology_capabilities() {
        use crate::state::AppState;

        let b = SqliteTerminologyBackend::in_memory().unwrap();
        let state = AppState::new(b);

        let app = Router::new()
            .route(
                "/metadata",
                get(metadata_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metadata")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["resourceType"], "TerminologyCapabilities");
        let ops = body["operationSupported"].as_array().unwrap();
        assert_eq!(ops.len(), 6);
    }
}
