//! `/metadata` endpoint — serves the server's `CapabilityStatement` or
//! `TerminologyCapabilities` resource.
//!
//! The `mode` query parameter selects the resource type:
//!
//! - absent or `full` — returns a full FHIR `CapabilityStatement` covering
//!   CRUD plus every terminology operation wired into the router.
//! - `terminology` — returns a `TerminologyCapabilities` resource that
//!   advertises supported code systems, expansion/translation parameters,
//!   and closure support, populated from the [`TerminologyMetadata`] impl
//!   on the active backend.
//!
//! Content negotiation honours both `_format` and the `Accept` header,
//! supporting `application/fhir+json` and `application/fhir+xml`.

use axum::{
    extract::{Query, RawQuery, State},
    http::{HeaderMap, header},
    response::Response,
};
use serde::Deserialize;
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

use super::format::{fhir_respond, negotiate_format};

const HTS_VERSION: &str = env!("CARGO_PKG_VERSION");
const HTS_NAME: &str = "Helios Terminology Server";

/// Query parameters accepted by `GET /metadata`.
#[derive(Debug, Default, Deserialize)]
pub struct MetadataQuery {
    /// FHIR metadata mode:
    /// - `"terminology"` → TerminologyCapabilities
    /// - `"full"` or absent → CapabilityStatement
    pub mode: Option<String>,
}

/// GET /metadata — returns a CapabilityStatement or TerminologyCapabilities.
///
/// - No `mode` or `mode=full` → CapabilityStatement (full server capabilities)
/// - `mode=terminology`      → TerminologyCapabilities (terminology-specific)
pub async fn metadata_handler<B>(
    State(state): State<AppState<B>>,
    Query(query): Query<MetadataQuery>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Response
where
    B: TerminologyBackend + BundleImportBackend + Clone,
{
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let backend = state.backend();
    let body = match query.mode.as_deref() {
        Some("terminology") => build_terminology_capabilities(backend),
        _ => build_capability_statement(backend),
    };
    fhir_respond(body, format)
}

/// Build the `TerminologyCapabilities` JSON value from backend metadata.
///
/// Constructs a typed `TerminologyCapabilities` (FHIR R4) model and serializes it
/// to JSON. Separated from the handler so it can be tested without a running server.
///
/// # Intentional 501 for `expression` parameter
///
/// The `$lookup` operation accepts an `expression` parameter for SNOMED
/// post-coordination expressions.  This server intentionally returns
/// `HTTP 501 Not Implemented` for any request that includes that parameter —
/// post-coordination evaluation is out of scope for the SQLite MVP.
/// Callers should not pass `expression`; use `code` instead.
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
                value: Some("Helios Terminology Server SQLite backend".to_string()),
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

/// Build a FHIR R4 CapabilityStatement for the HTS server.
///
/// Describes the full set of REST interactions (CRUD + search) and FHIR
/// terminology operations supported for CodeSystem, ValueSet, and ConceptMap.
/// Includes a `capabilitystatement-supported-system` extension for each
/// code system URL currently registered in the backend.
pub fn build_capability_statement(backend: &impl TerminologyMetadata) -> Value {
    // ── capabilitystatement-supported-system extensions ───────────────────────
    let supported_system_extensions: Vec<Value> = backend
        .supported_systems()
        .into_iter()
        .map(|url| {
            json!({
                "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
                "valueUri": url
            })
        })
        .collect();

    // ── Shared search params for all three resource types ─────────────────────
    let search_params = json!([
        {"name": "url",     "type": "uri",    "documentation": "Canonical URL of the resource"},
        {"name": "version", "type": "token",  "documentation": "Business version"},
        {"name": "name",    "type": "string", "documentation": "Computer-friendly name"},
        {"name": "title",   "type": "string", "documentation": "Human-friendly title"},
        {"name": "status",  "type": "token",  "documentation": "Publication status"}
    ]);

    // ── Standard interactions supported by all three CRUD resources ───────────
    let interactions = json!([
        {"code": "read"},
        {"code": "create"},
        {"code": "update"},
        {"code": "delete"},
        {"code": "search-type"}
    ]);

    json!({
        "resourceType": "CapabilityStatement",
        "status": "active",
        "kind": "instance",
        "date": "2026-04-01",
        "fhirVersion": "4.0.1",
        "format": ["application/fhir+json", "application/fhir+xml"],
        "extension": supported_system_extensions,
        "software": {
            "name": HTS_NAME,
            "version": HTS_VERSION
        },
        "implementation": {
            "description": "Helios Terminology Server SQLite backend"
        },
        "rest": [{
            "mode": "server",
            "resource": [
                {
                    "type": "CodeSystem",
                    "interaction": interactions,
                    "searchParam": search_params
                },
                {
                    "type": "ValueSet",
                    "interaction": interactions,
                    "searchParam": search_params
                },
                {
                    "type": "ConceptMap",
                    "interaction": interactions,
                    "searchParam": search_params
                }
            ],
            "operation": [
                {
                    "name": "lookup",
                    "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"
                },
                {
                    "name": "validate-code",
                    "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-validate-code"
                },
                {
                    "name": "subsumes",
                    "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-subsumes"
                },
                {
                    "name": "expand",
                    "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-expand"
                },
                {
                    "name": "validate-code",
                    "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-validate-code"
                },
                {
                    "name": "translate",
                    "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-translate"
                },
                {
                    "name": "closure",
                    "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-closure"
                }
            ]
        }]
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    use crate::backends::sqlite::SqliteTerminologyBackend;

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

    // ── Unit tests on build_capability_statement ──────────────────────────────

    #[test]
    fn capability_statement_resource_type() {
        let cs = build_capability_statement(&backend());
        assert_eq!(cs["resourceType"], "CapabilityStatement");
    }

    #[test]
    fn capability_statement_status_and_kind() {
        let cs = build_capability_statement(&backend());
        assert_eq!(cs["status"], "active");
        assert_eq!(cs["kind"], "instance");
    }

    #[test]
    fn capability_statement_has_three_resource_types() {
        let cs = build_capability_statement(&backend());
        let resources = cs["rest"][0]["resource"].as_array().unwrap();
        let types: Vec<&str> = resources
            .iter()
            .filter_map(|r| r["type"].as_str())
            .collect();
        assert!(types.contains(&"CodeSystem"));
        assert!(types.contains(&"ValueSet"));
        assert!(types.contains(&"ConceptMap"));
    }

    #[test]
    fn capability_statement_each_resource_has_five_search_params() {
        let cs = build_capability_statement(&backend());
        for res in cs["rest"][0]["resource"].as_array().unwrap() {
            let params = res["searchParam"].as_array().unwrap();
            assert_eq!(
                params.len(),
                5,
                "expected 5 search params for {}",
                res["type"]
            );
            let names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();
            for expected in ["url", "version", "name", "title", "status"] {
                assert!(
                    names.contains(&expected),
                    "missing search param '{expected}'"
                );
            }
        }
    }

    #[test]
    fn capability_statement_lists_all_operations() {
        let cs = build_capability_statement(&backend());
        let ops = cs["rest"][0]["operation"].as_array().unwrap();
        let names: Vec<&str> = ops.iter().filter_map(|o| o["name"].as_str()).collect();
        for expected in [
            "lookup",
            "validate-code",
            "subsumes",
            "expand",
            "translate",
            "closure",
        ] {
            assert!(names.contains(&expected), "missing operation '{expected}'");
        }
    }

    #[test]
    fn capability_statement_supported_system_extensions_empty_on_fresh_backend() {
        let cs = build_capability_statement(&backend());
        let exts = cs["extension"].as_array().unwrap();
        assert!(
            exts.is_empty(),
            "fresh backend should have no supported-system extensions"
        );
    }

    #[test]
    fn capability_statement_supported_system_extension_populated() {
        let b = backend();
        let conn = b.pool().get().unwrap();
        conn.execute(
            "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', 'active', 'complete', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
        drop(conn);

        let cs = build_capability_statement(&b);
        let exts = cs["extension"].as_array().unwrap();
        assert_eq!(exts.len(), 1);
        assert_eq!(
            exts[0]["url"],
            "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system"
        );
        assert_eq!(exts[0]["valueUri"], "http://example.org/cs");
    }

    // ── Integration tests: HTTP GET /metadata mode dispatch ───────────────────

    fn make_metadata_app() -> Router {
        use crate::state::AppState;
        let b = SqliteTerminologyBackend::in_memory().unwrap();
        let state = AppState::new(b);
        Router::new()
            .route(
                "/metadata",
                get(metadata_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn get_metadata(app: Router, uri: &str) -> Value {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn get_metadata_returns_200() {
        let app = make_metadata_app();
        let body = get_metadata(app, "/metadata").await;
        assert_eq!(body["resourceType"], "CapabilityStatement");
    }

    #[tokio::test]
    async fn get_metadata_no_mode_returns_capability_statement() {
        let body = get_metadata(make_metadata_app(), "/metadata").await;
        assert_eq!(body["resourceType"], "CapabilityStatement");
        assert_eq!(body["kind"], "instance");
    }

    #[tokio::test]
    async fn get_metadata_mode_full_returns_capability_statement() {
        let body = get_metadata(make_metadata_app(), "/metadata?mode=full").await;
        assert_eq!(body["resourceType"], "CapabilityStatement");
        assert_eq!(body["kind"], "instance");
    }

    #[tokio::test]
    async fn get_metadata_mode_terminology_returns_terminology_capabilities() {
        let body = get_metadata(make_metadata_app(), "/metadata?mode=terminology").await;
        assert_eq!(body["resourceType"], "TerminologyCapabilities");
        assert_eq!(body["kind"], "terminology");
    }
}
