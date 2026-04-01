use axum::{Json, extract::State, response::IntoResponse};
use serde_json::{Value, json};

#[cfg(feature = "R4")]
use helios_fhir::r4::{
    TerminologyCapabilities, TerminologyCapabilitiesClosure, TerminologyCapabilitiesCodeSystem,
    TerminologyCapabilitiesExpansion, TerminologyCapabilitiesImplementation,
    TerminologyCapabilitiesSoftware, TerminologyCapabilitiesTranslation,
    TerminologyCapabilitiesValidateCode,
};
use helios_fhir::{Element, PrecisionDateTime};

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
/// Constructs a typed `TerminologyCapabilities` (FHIR R4) model and serializes it
/// to JSON. Separated from the handler so it can be tested without a running server.
#[cfg(feature = "R4")]
pub fn build_terminology_capabilities(backend: &impl TerminologyMetadata) -> Value {
    let code_systems: Vec<TerminologyCapabilitiesCodeSystem> = backend
        .supported_systems()
        .into_iter()
        .map(|url| TerminologyCapabilitiesCodeSystem {
            uri: Some(Element {
                value: Some(url),
                ..Default::default()
            }),
            subsumption: Some(Element {
                value: Some(backend.supports_subsumption()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();

    let caps = TerminologyCapabilities {
        status: Element {
            value: Some("active".to_string()),
            ..Default::default()
        },
        kind: Element {
            value: Some("terminology".to_string()),
            ..Default::default()
        },
        // Use a fixed publication date; this value identifies the capability document itself.
        date: Element {
            value: Some(PrecisionDateTime::from_date(2026, 4, 1)),
            ..Default::default()
        },
        experimental: Some(Element {
            value: Some(false),
            ..Default::default()
        }),
        software: Some(TerminologyCapabilitiesSoftware {
            name: Element {
                value: Some(HTS_NAME.to_string()),
                ..Default::default()
            },
            version: Some(Element {
                value: Some(HTS_VERSION.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        implementation: Some(TerminologyCapabilitiesImplementation {
            description: Element {
                value: Some("Helios Terminology Service SQLite backend".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }),
        code_search: Some(Element {
            value: Some("all".to_string()),
            ..Default::default()
        }),
        code_system: Some(code_systems),
        expansion: Some(TerminologyCapabilitiesExpansion {
            hierarchical: Some(Element {
                value: Some(false),
                ..Default::default()
            }),
            paging: Some(Element {
                value: Some(true),
                ..Default::default()
            }),
            incomplete: Some(Element {
                value: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }),
        validate_code: Some(TerminologyCapabilitiesValidateCode {
            translations: Element {
                value: Some(false),
                ..Default::default()
            },
            ..Default::default()
        }),
        translation: Some(TerminologyCapabilitiesTranslation {
            needs_map: Element {
                value: Some(true),
                ..Default::default()
            },
            ..Default::default()
        }),
        closure: Some(TerminologyCapabilitiesClosure {
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut value = serde_json::to_value(&caps).unwrap_or_else(|_| json!({}));
    // `resourceType` is not emitted by the FhirSerde struct serializer; add it explicitly.
    value["resourceType"] = json!("TerminologyCapabilities");
    value
}

#[cfg(not(feature = "R4"))]
pub fn build_terminology_capabilities(_backend: &impl TerminologyMetadata) -> Value {
    json!({ "resourceType": "TerminologyCapabilities", "status": "active", "kind": "terminology" })
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
    fn kind_is_terminology() {
        let caps = build_terminology_capabilities(&backend());
        assert_eq!(caps["kind"], "terminology");
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
        assert_eq!(body["status"], "active");
        assert_eq!(body["kind"], "terminology");
    }
}
