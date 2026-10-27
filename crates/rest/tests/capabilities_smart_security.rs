//! `GET /metadata` advertises SMART App Launch in `rest.security` when auth is
//! enabled, so a client discovers it from the CapabilityStatement and not only
//! from `/.well-known/smart-configuration` (#1441). With auth off, the security
//! block stays `cors`-only.

mod capabilities_smart_security_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_auth::AuthConfig;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_rest::ServerConfig;
    use serde_json::Value;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    const AUTHORIZE: &str = "https://idp.example.com/realms/fhir/protocol/openid-connect/auth";
    const TOKEN: &str = "https://idp.example.com/realms/fhir/protocol/openid-connect/token";

    fn server_with_auth(auth_config: AuthConfig) -> TestServer {
        let backend = SqliteBackend::in_memory().expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let state = helios_rest::AppState::with_auth(
            Arc::new(backend),
            ServerConfig::for_testing(),
            auth_config,
            None,
        );
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    async fn security_block(server: &TestServer) -> Value {
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status(StatusCode::OK);
        let metadata: Value =
            serde_json::from_str(&response.text()).expect("metadata must be valid JSON");
        metadata["rest"][0]["security"].clone()
    }

    #[tokio::test]
    async fn security_advertises_smart_on_fhir_when_auth_is_enabled() {
        let auth_config = AuthConfig {
            enabled: true,
            smart_authorize_endpoint: Some(AUTHORIZE.to_string()),
            smart_token_endpoint: Some(TOKEN.to_string()),
            ..Default::default()
        };
        let server = server_with_auth(auth_config);
        let security = security_block(&server).await;

        // CORS stays as it was.
        assert!(
            security["cors"].is_boolean(),
            "cors flag is present, reflecting config"
        );

        // The SMART-on-FHIR service coding is present.
        assert_eq!(
            security["service"][0]["coding"][0]["system"],
            "http://terminology.hl7.org/CodeSystem/restful-security-service"
        );
        assert_eq!(security["service"][0]["coding"][0]["code"], "SMART-on-FHIR");

        // The oauth-uris extension carries authorize + token.
        let ext = &security["extension"][0];
        assert_eq!(
            ext["url"],
            "http://fhir-registry.smarthealthit.org/StructureDefinition/oauth-uris"
        );
        let uris = ext["extension"]
            .as_array()
            .expect("oauth-uris sub-extensions");
        let find = |name: &str| -> Option<String> {
            uris.iter()
                .find(|e| e["url"] == name)
                .and_then(|e| e["valueUri"].as_str().map(str::to_string))
        };
        assert_eq!(find("authorize").as_deref(), Some(AUTHORIZE));
        assert_eq!(find("token").as_deref(), Some(TOKEN));
    }

    #[tokio::test]
    async fn security_is_cors_only_when_auth_is_disabled() {
        let server = server_with_auth(AuthConfig::default());
        let security = security_block(&server).await;

        assert!(
            security["cors"].is_boolean(),
            "cors flag is present, reflecting config"
        );
        assert!(
            security.get("service").is_none(),
            "no SMART service coding without auth"
        );
        assert!(
            security.get("extension").is_none(),
            "no oauth-uris extension without auth"
        );
    }

    #[tokio::test]
    async fn security_is_cors_only_when_auth_on_but_smart_endpoints_absent() {
        // Auth on but no SMART endpoints configured — a plain bearer deployment,
        // not SMART-on-FHIR; the profile must not be advertised half-built.
        let auth_config = AuthConfig {
            enabled: true,
            ..Default::default()
        };
        let server = server_with_auth(auth_config);
        let security = security_block(&server).await;

        assert!(
            security["cors"].is_boolean(),
            "cors flag is present, reflecting config"
        );
        assert!(security.get("service").is_none());
        assert!(security.get("extension").is_none());
    }
}
