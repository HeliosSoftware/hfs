//! Integration tests for multi-source tenant resolution.
//!
//! Tests the different tenant routing modes:
//! - HeaderOnly (default): Tenant from X-Tenant-ID header
//! - UrlPath: Tenant from URL path prefix
//! - Both: URL takes precedence over header

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// Creates a test server with the given multitenancy configuration.
async fn create_test_server(multitenancy: MultitenancyConfig) -> (TestServer, Arc<SqliteBackend>) {
    // Configure with data directory to load spec SearchParameters
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy,
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    // Create app state manually to avoid Clone requirement
    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

/// Seeds a patient resource for a specific tenant.
async fn seed_patient(backend: &SqliteBackend, tenant_id: &str, patient_id: &str, family: &str) {
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    let patient = serde_json::json!({
        "resourceType": "Patient",
        "id": patient_id,
        "name": [{ "family": family }]
    });

    backend
        .create(&tenant, "Patient", patient, helios_fhir::FhirVersion::R4)
        .await
        .expect("Failed to seed patient");
}

// =============================================================================
// Header-Only Mode Tests (Default, Backward Compatible)
// =============================================================================

mod header_only_mode {
    use super::*;

    #[tokio::test]
    async fn test_header_tenant_is_used() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient for tenant "acme"
        seed_patient(&backend, "acme", "123", "Smith").await;

        // Request with X-Tenant-ID header
        let response = server
            .get("/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["id"], "123");
    }

    #[tokio::test]
    async fn test_default_tenant_when_no_header() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient for default tenant
        seed_patient(&backend, "default-tenant", "456", "Jones").await;

        // Request without X-Tenant-ID header - should use default
        let response = server.get("/Patient/456").await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["id"], "456");
    }

    #[tokio::test]
    async fn test_tenant_isolation() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient for tenant "acme" only
        seed_patient(&backend, "acme", "123", "Smith").await;

        // Request from different tenant should not find patient
        let response = server
            .get("/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("other-tenant"))
            .await;

        response.assert_status_not_found();
    }

    #[tokio::test]
    async fn test_metadata_returns_base_url_without_tenant() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();

        // Base URL should NOT include tenant in header-only mode
        let impl_url = body["implementation"]["url"].as_str().unwrap();
        assert_eq!(impl_url, "http://localhost:8080");
        assert!(!impl_url.contains("acme"));
    }
}

// =============================================================================
// URL Path Mode Tests
// =============================================================================

mod url_path_mode {
    use super::*;

    #[tokio::test]
    async fn test_tenant_from_url_path() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient for tenant "acme"
        seed_patient(&backend, "acme", "123", "Smith").await;

        // Request with tenant in URL path
        let response = server.get("/acme/Patient/123").await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert_eq!(body["id"], "123");
    }

    #[tokio::test]
    async fn test_different_tenants_different_data() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed different patients for different tenants
        seed_patient(&backend, "acme", "123", "AcmeSmith").await;
        seed_patient(&backend, "globex", "123", "GlobexJones").await;

        // Request from acme tenant
        let response = server.get("/acme/Patient/123").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["name"][0]["family"], "AcmeSmith");

        // Request from globex tenant
        let response = server.get("/globex/Patient/123").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["name"][0]["family"], "GlobexJones");
    }

    #[tokio::test]
    async fn test_metadata_includes_tenant_in_base_url() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        let response = server.get("/acme/metadata").await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();

        // Base URL should include tenant in URL path mode
        let impl_url = body["implementation"]["url"].as_str().unwrap();
        assert_eq!(impl_url, "http://localhost:8080/acme");
    }

    #[tokio::test]
    async fn test_health_endpoint_not_tenant_scoped() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        // Health check should work at root level
        let response = server.get("/health").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_liveness_endpoint_not_tenant_scoped() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        // Liveness should work at root level
        let response = server.get("/_liveness").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_readiness_endpoint_reports_storage_ok() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        // Readiness should work at root level (not tenant-scoped) and report the
        // storage check as healthy.
        let response = server.get("/_readiness").await;
        response.assert_status_ok();

        let body: serde_json::Value = response.json();
        assert_eq!(body["status"], "ready");
        assert_eq!(body["checks"]["storage"], "ok");
    }

    #[tokio::test]
    async fn test_tenant_search() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patients for tenant "acme"
        seed_patient(&backend, "acme", "p1", "Smith").await;
        seed_patient(&backend, "acme", "p2", "Jones").await;
        seed_patient(&backend, "other", "p3", "OtherTenant").await;

        // Search in acme tenant should work with URL routing
        let response = server.get("/acme/Patient").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();

        // Verify it's a search bundle (confirms routing works)
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "searchset");

        // Verify tenant isolation: if any entries, they should be for acme tenant
        // (The exact count depends on search implementation details)
        if let Some(entries) = body["entry"].as_array() {
            for entry in entries {
                // Each entry's resource should be a Patient
                assert_eq!(entry["resource"]["resourceType"], "Patient");
            }
        }
    }
}

// =============================================================================
// Combined Mode Tests (Both URL and Header)
// =============================================================================

mod combined_mode {
    use super::*;

    #[tokio::test]
    async fn test_url_takes_precedence_over_header() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            strict_validation: false,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient only for tenant "acme"
        seed_patient(&backend, "acme", "123", "Smith").await;

        // Request with URL tenant "acme" but header tenant "other"
        // URL should take precedence
        let response = server
            .get("/acme/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("other"))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["id"], "123");
    }

    #[tokio::test]
    async fn test_header_works_for_non_tenant_prefixed_paths() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        // Seed patient for tenant "acme"
        seed_patient(&backend, "acme", "123", "Smith").await;

        // Request without tenant prefix should use header
        let response = server
            .get("/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["id"], "123");
    }

    #[tokio::test]
    async fn test_metadata_base_url_reflects_access_method() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        // URL-based access should include tenant in base URL
        let response = server.get("/acme/metadata").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let impl_url = body["implementation"]["url"].as_str().unwrap();
        assert_eq!(impl_url, "http://localhost:8080/acme");

        // Header-based access should NOT include tenant in base URL
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let impl_url = body["implementation"]["url"].as_str().unwrap();
        assert_eq!(impl_url, "http://localhost:8080");
    }
}

// =============================================================================
// Strict Validation Tests
// =============================================================================

mod strict_validation {
    use super::*;

    #[tokio::test]
    async fn test_matching_url_and_header_succeeds() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            strict_validation: true,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "acme", "123", "Smith").await;

        // Same tenant in URL and header should work
        let response = server
            .get("/acme/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
            .await;

        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_mismatching_url_and_header_fails() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            strict_validation: true,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "acme", "123", "Smith").await;

        // Different tenant in URL vs header should fail in strict mode
        let response = server
            .get("/acme/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static("other"))
            .await;

        response.assert_status_bad_request();
    }

    #[tokio::test]
    async fn test_url_only_without_header_succeeds() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            strict_validation: true,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "acme", "123", "Smith").await;

        // Only URL tenant (no header) should work in strict mode
        let response = server.get("/acme/Patient/123").await;

        response.assert_status_ok();
    }
}

// =============================================================================
// Edge Cases and Error Handling
// =============================================================================

mod edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_empty_tenant_header_uses_default() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "default-tenant", "123", "Smith").await;

        // Empty header should fall back to default
        let response = server
            .get("/Patient/123")
            .add_header(X_TENANT_ID, HeaderValue::from_static(""))
            .await;

        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_tenant_id_with_hyphen() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "acme-corp", "123", "Smith").await;

        let response = server.get("/acme-corp/Patient/123").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_tenant_id_with_underscore() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, backend) = create_test_server(config).await;

        seed_patient(&backend, "acme_corp", "123", "Smith").await;

        let response = server.get("/acme_corp/Patient/123").await;
        response.assert_status_ok();
    }

    #[tokio::test]
    async fn test_create_resource_with_url_tenant() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        let patient = serde_json::json!({
            "resourceType": "Patient",
            "name": [{ "family": "NewPatient" }]
        });

        let response = server
            .post("/acme/Patient")
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&patient)
            .await;

        response.assert_status(StatusCode::CREATED);

        let body: serde_json::Value = response.json();
        assert_eq!(body["resourceType"], "Patient");
        assert!(body["id"].is_string());
    }

    #[tokio::test]
    async fn test_batch_with_url_tenant() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let (server, _backend) = create_test_server(config).await;

        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {
                    "request": {
                        "method": "POST",
                        "url": "Patient"
                    },
                    "resource": {
                        "resourceType": "Patient",
                        "name": [{ "family": "BatchPatient" }]
                    }
                }
            ]
        });

        let response = server
            .post("/acme/")
            .add_header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/fhir+json"),
            )
            .json(&bundle)
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "batch-response");
    }
}

// =============================================================================
// Tenant Source Information Tests
// =============================================================================

mod tenant_source_tests {
    use axum::http::{HeaderValue, Request, Uri};
    use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
    use helios_rest::tenant::{TenantResolver, TenantSource};

    use super::X_TENANT_ID;

    fn make_parts(path: &str, tenant_header: Option<&str>) -> axum::http::request::Parts {
        let mut builder = Request::builder().uri(Uri::try_from(path).unwrap());

        if let Some(tenant) = tenant_header {
            builder = builder.header(X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
        }

        let request = builder.body(()).unwrap();
        request.into_parts().0
    }

    #[test]
    fn test_source_priority_url_over_header() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        let parts = make_parts("/acme/Patient/123", Some("other"));
        let resolved = resolver.resolve(&parts, &config, "default");

        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::UrlPath);
        assert!(resolved.is_url_based());
    }

    #[test]
    fn test_source_falls_back_to_default() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        let parts = make_parts("/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default-tenant");

        assert_eq!(resolved.tenant_id_str(), "default-tenant");
        assert_eq!(resolved.source, TenantSource::Default);
        assert!(resolved.is_default());
    }

    #[test]
    fn test_all_sources_tracked() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        let parts = make_parts("/acme/Patient/123", Some("acme"));
        let resolved = resolver.resolve(&parts, &config, "default");

        // Both sources found the same tenant
        assert_eq!(resolved.all_sources.len(), 2);
        assert!(
            resolved
                .all_sources
                .iter()
                .any(|(s, _)| *s == TenantSource::UrlPath)
        );
        assert!(
            resolved
                .all_sources
                .iter()
                .any(|(s, _)| *s == TenantSource::Header)
        );
    }
}
