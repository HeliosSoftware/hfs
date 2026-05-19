//! Tests for `GET /$sql-on-fhir-capabilities` and the SOF extensions on
//! `GET /metadata`.

mod sof_capability_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_rest::ServerConfig;
    use serde_json::Value;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    async fn create_test_server() -> TestServer {
        let backend = SqliteBackend::with_config(":memory:", Default::default())
            .expect("failed to create SQLite backend");
        backend.init_schema().expect("failed to init schema");
        let backend = Arc::new(backend);

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(Arc::clone(&backend), config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("failed to create test server")
    }

    // =========================================================================
    // GET /$sql-on-fhir-capabilities
    // =========================================================================

    #[tokio::test]
    async fn test_sof_capabilities_returns_200() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::OK);
    }

    #[tokio::test]
    async fn test_sof_capabilities_is_parameters_resource() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        assert_eq!(
            body["resourceType"], "Parameters",
            "must return a Parameters resource: {body}"
        );
    }

    #[tokio::test]
    async fn test_sof_capabilities_has_required_params() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        let params = body["parameter"]
            .as_array()
            .expect("parameter must be an array");

        let param_names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();

        for required in [
            "supportsViewDefinitionRun",
            "supportsViewDefinitionExport",
            "supportsSqlQueryRun",
            "supportsInDbRunner",
            "supportedFormat",
        ] {
            assert!(
                param_names.contains(&required),
                "missing parameter '{required}', got: {param_names:?}"
            );
        }
    }

    /// With a plain SQLite backend (no in-DB runner wired), `$viewdefinition-run`
    /// must be true and `supportsInDbRunner` must be false.
    #[tokio::test]
    async fn test_sof_capabilities_inprocess_backend_flags() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        let params = body["parameter"].as_array().unwrap();

        let get_bool = |name: &str| -> bool {
            params
                .iter()
                .find(|p| p["name"].as_str() == Some(name))
                .and_then(|p| p["valueBoolean"].as_bool())
                .unwrap_or(false)
        };

        assert!(
            get_bool("supportsViewDefinitionRun"),
            "supportsViewDefinitionRun must be true"
        );
        assert!(
            !get_bool("supportsInDbRunner"),
            "supportsInDbRunner must be false when using in-process runner"
        );
    }

    /// The `supportedFormat` parameter must appear at least three times
    /// (ndjson, json, csv) and all values must be strings.
    #[tokio::test]
    async fn test_sof_capabilities_supported_formats() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        let formats: Vec<&str> = body["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"].as_str() == Some("supportedFormat"))
            .filter_map(|p| p["valueCode"].as_str())
            .collect();

        assert!(
            formats.len() >= 3,
            "expected at least 3 supportedFormat entries, got: {formats:?}"
        );
        assert!(formats.contains(&"ndjson"), "ndjson must be supported");
        assert!(formats.contains(&"json"), "json must be supported");
        assert!(formats.contains(&"csv"), "csv must be supported");
    }

    /// Audit item #13: the spec binds `_format` to the
    /// `OutputFormatCodes` value set with `extensible` strength.
    /// `/$sql-on-fhir-capabilities` declares the binding so audit
    /// tools can discover it without dereferencing the
    /// OperationDefinition. Same shape sof-server publishes.
    #[tokio::test]
    async fn test_sof_capabilities_declares_format_binding() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");
        let params = body["parameter"].as_array().unwrap();

        let binding = params
            .iter()
            .find(|p| p["name"] == "formatBinding")
            .expect("formatBinding parameter must be present");
        let parts = binding["part"]
            .as_array()
            .expect("formatBinding must have part[]");

        let value_set = parts
            .iter()
            .find(|p| p["name"] == "valueSet")
            .and_then(|p| p["valueUri"].as_str())
            .expect("formatBinding.valueSet must be a uri");
        assert_eq!(
            value_set, "https://sql-on-fhir.org/ig/ValueSet/OutputFormatCodes",
            "binding must reference the spec's OutputFormatCodes value set"
        );

        let strength = parts
            .iter()
            .find(|p| p["name"] == "strength")
            .and_then(|p| p["valueCode"].as_str())
            .expect("formatBinding.strength must be a code");
        assert_eq!(
            strength, "extensible",
            "binding strength must be `extensible` per spec"
        );
    }

    // =========================================================================
    // GET /metadata — SOF operation extensions
    // =========================================================================

    #[tokio::test]
    async fn test_metadata_includes_sof_operations() {
        let server = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::OK);

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        let operations = body["rest"][0]["operation"]
            .as_array()
            .expect("rest[0].operation must be an array");

        let op_names: Vec<&str> = operations
            .iter()
            .filter_map(|op| op["name"].as_str())
            .collect();

        // `viewdefinition-run` and `sqlquery-run` are unconditional when SOF is
        // enabled. The spec-conforming `$sqlquery-run` implementation uses an
        // in-memory SQLite engine that any storage backend can drive via the
        // shared SofRunner, so no extra wiring is required to advertise it.
        assert!(
            op_names.contains(&"viewdefinition-run"),
            "metadata must advertise viewdefinition-run, got: {op_names:?}"
        );
        assert!(
            op_names.contains(&"sqlquery-run"),
            "metadata must advertise sqlquery-run, got: {op_names:?}"
        );
        // `viewdefinition-export` is still gated on an export controller being
        // wired. With the bare test server, none is wired.
        assert!(
            !op_names.contains(&"viewdefinition-export"),
            "viewdefinition-export must NOT be advertised without an export controller, got: {op_names:?}"
        );
    }

    #[tokio::test]
    async fn test_metadata_sof_extension_references_capabilities_endpoint() {
        let server = create_test_server().await;

        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("body must be valid JSON");

        let extensions = body["rest"][0]["extension"]
            .as_array()
            .expect("rest[0].extension must be an array when sof feature is enabled");

        assert!(
            !extensions.is_empty(),
            "rest[0].extension must not be empty"
        );

        // At least one extension must reference the $sql-on-fhir-capabilities endpoint
        let refs: Vec<&str> = extensions
            .iter()
            .filter_map(|e| e["valueReference"]["reference"].as_str())
            .collect();
        assert!(
            refs.iter().any(|r| r.contains("sql-on-fhir-capabilities")),
            "no extension references the capabilities endpoint, got: {refs:?}"
        );
    }
}
