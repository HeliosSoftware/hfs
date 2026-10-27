//! `GET /metadata` identifies the build: `CapabilityStatement.software`
//! carries the product name and crate version for every FHIR version the
//! server can describe (#992).
//!
//! Before this, the only surface reporting the version was `/health`, which
//! FHIR clients do not consult, and a tester recording "which build did I
//! exercise" had nothing to read off the artifact under test.

mod capabilities_software_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_rest::ServerConfig;
    use helios_rest::build_info::{GIT_SHA_EXTENSION_URL, PKG_VERSION, SOFTWARE_NAME, git_sha};
    use serde_json::Value;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    fn create_test_server() -> TestServer {
        let backend = SqliteBackend::in_memory().expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let state = helios_rest::AppState::new(Arc::new(backend), ServerConfig::for_testing());
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    async fn fetch_metadata(server: &TestServer, version: FhirVersion) -> Value {
        let accept = format!(
            "application/fhir+json; fhirVersion={}",
            version.as_mime_param()
        );
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .add_header(
                axum::http::header::ACCEPT,
                HeaderValue::from_str(&accept).expect("accept header"),
            )
            .await;
        response.assert_status(StatusCode::OK);
        serde_json::from_str(&response.text()).expect("metadata must be valid JSON")
    }

    #[tokio::test]
    async fn metadata_software_names_the_build_for_every_enabled_fhir_version() {
        let server = create_test_server();
        let versions = FhirVersion::enabled_versions();
        assert!(
            !versions.is_empty(),
            "at least one FHIR version is compiled in"
        );

        for version in versions {
            let metadata = fetch_metadata(&server, *version).await;
            assert_eq!(
                metadata["fhirVersion"],
                version.full_version(),
                "describing {version:?}"
            );

            let software = &metadata["software"];
            assert_eq!(
                software["name"], SOFTWARE_NAME,
                "software.name for {version:?}"
            );
            assert_eq!(
                software["version"],
                env!("CARGO_PKG_VERSION"),
                "software.version is the crate version for {version:?}"
            );
            assert_eq!(software["version"], PKG_VERSION);

            // The commit, when the build knows it, rides as an extension so
            // `software.version` stays comparable to a release number.
            match git_sha() {
                Some(sha) => {
                    let ext = &software["extension"][0];
                    assert_eq!(ext["url"], GIT_SHA_EXTENSION_URL);
                    assert_eq!(ext["valueString"], sha);
                }
                None => assert!(software.get("extension").is_none()),
            }

            // `implementation` still describes this deployment.
            assert_eq!(metadata["implementation"]["description"], SOFTWARE_NAME);
            assert!(metadata["implementation"]["url"].is_string());
        }
    }
}
