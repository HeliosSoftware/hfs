//! Tests for how the server advertises its SQL on FHIR support.
//!
//! SQL on FHIR 3.0.0-ballot has no `$sql-on-fhir-capabilities` endpoint. A
//! server declares which subset of an operation it supports by publishing its
//! own OperationDefinition, whose `base` names the guide's, and citing that
//! from `CapabilityStatement.rest.operation.definition`
//! (operations-capability.html#partial-operation-support). Citing the guide's
//! definition instead would assert support for every parameter it declares.

mod sof_capability_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_rest::ServerConfig;
    use serde_json::Value;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    /// Canonical URL of the guide's `$sql-run` definition. Our own definition
    /// must name this as its `base`.
    const GUIDE_SQL_RUN: &str = "http://hl7.org/fhir/uv/sql-on-fhir/OperationDefinition/SQLRun";

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

    async fn metadata(server: &TestServer) -> Value {
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status(StatusCode::OK);
        serde_json::from_str(&response.text()).expect("metadata must be valid JSON")
    }

    // =========================================================================
    // CapabilityStatement
    // =========================================================================

    /// Both data operations are invoked at the system level, so they belong in
    /// `rest.operation` rather than under a resource type.
    #[tokio::test]
    async fn test_metadata_declares_sql_run_at_system_level() {
        let server = create_test_server().await;
        let body = metadata(&server).await;

        let operations = body["rest"][0]["operation"]
            .as_array()
            .expect("rest[0].operation must be an array");
        let names: Vec<&str> = operations
            .iter()
            .filter_map(|op| op["name"].as_str())
            .collect();

        assert!(
            names.contains(&"sql-run"),
            "$sql-run must be declared: {names:?}"
        );
    }

    /// The pre-ballot operation names were consolidated away and must not be
    /// advertised: a client reading them would build requests we no longer route.
    #[tokio::test]
    async fn test_metadata_does_not_declare_pre_ballot_operations() {
        let server = create_test_server().await;
        let body = metadata(&server).await;

        let names: Vec<&str> = body["rest"][0]["operation"]
            .as_array()
            .expect("rest[0].operation must be an array")
            .iter()
            .filter_map(|op| op["name"].as_str())
            .collect();

        for gone in [
            "viewdefinition-run",
            "viewdefinition-export",
            "sqlquery-run",
            "sqlquery-export",
        ] {
            assert!(
                !names.contains(&gone),
                "{gone} was consolidated into $sql-run/$sql-export: {names:?}"
            );
        }
    }

    /// We support a subset of `$sql-run` (no `context`, no `source`), so the
    /// CapabilityStatement must cite our own definition rather than the
    /// guide's — citing the guide's asserts full support.
    #[tokio::test]
    async fn test_metadata_cites_our_own_operation_definition() {
        let server = create_test_server().await;
        let body = metadata(&server).await;

        let definition = body["rest"][0]["operation"]
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["name"] == "sql-run")
            .and_then(|op| op["definition"].as_str())
            .expect("$sql-run must carry a definition");

        assert_ne!(
            definition, GUIDE_SQL_RUN,
            "citing the guide's definition asserts support for every parameter it declares"
        );
        assert_eq!(definition, "/OperationDefinition/hfs-sql-run");
    }

    /// The pre-ballot `sof-capabilities` extension block has no counterpart in
    /// 3.0.0-ballot.
    #[tokio::test]
    async fn test_metadata_has_no_sof_capabilities_extension() {
        let server = create_test_server().await;
        let body = metadata(&server).await;

        if let Some(extensions) = body["rest"][0]["extension"].as_array() {
            for ext in extensions {
                let url = ext["url"].as_str().unwrap_or("");
                assert!(
                    !url.contains("sof-capabilities"),
                    "the sof-capabilities extension is not part of 3.0.0-ballot: {url}"
                );
            }
        }
    }

    // =========================================================================
    // The published OperationDefinition
    // =========================================================================

    #[tokio::test]
    async fn test_operation_definition_is_served() {
        let server = create_test_server().await;

        let response = server
            .get("/OperationDefinition/hfs-sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        response.assert_status(StatusCode::OK);
        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");
        assert_eq!(body["resourceType"], "OperationDefinition");
        assert_eq!(body["code"], "sql-run");
    }

    /// `base` is what tells a client this is a subset of the guide's operation
    /// rather than an unrelated one.
    #[tokio::test]
    async fn test_operation_definition_bases_on_the_guides() {
        let server = create_test_server().await;

        let response = server
            .get("/OperationDefinition/hfs-sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");

        assert_eq!(body["base"], GUIDE_SQL_RUN);
    }

    /// `system=true, type=false, instance=false` — the operation names its
    /// subject by parameter, so it needs no resource-typed path.
    #[tokio::test]
    async fn test_operation_definition_is_system_level_only() {
        let server = create_test_server().await;

        let response = server
            .get("/OperationDefinition/hfs-sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");

        assert_eq!(body["system"], true);
        assert_eq!(body["type"], false);
        assert_eq!(body["instance"], false);
    }

    /// The three subject-naming parameters are declared; the parameters we
    /// reject are omitted, which is how a client learns not to send them.
    #[tokio::test]
    async fn test_operation_definition_declares_the_supported_subset() {
        let server = create_test_server().await;

        let response = server
            .get("/OperationDefinition/hfs-sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");

        let names: Vec<&str> = body["parameter"]
            .as_array()
            .expect("parameter array")
            .iter()
            .filter_map(|p| p["name"].as_str())
            .collect();

        for supported in ["subjectCanonical", "subjectReference", "subjectResource"] {
            assert!(names.contains(&supported), "{supported} missing: {names:?}");
        }
        for unsupported in ["context", "source"] {
            assert!(
                !names.contains(&unsupported),
                "{unsupported} is rejected and must not be declared: {names:?}"
            );
        }
    }

    /// `_format` carries the guide's `OutputFormatCodes` binding, so a client
    /// can discover the format vocabulary from the definition alone.
    #[tokio::test]
    async fn test_operation_definition_binds_output_format_codes() {
        let server = create_test_server().await;

        let response = server
            .get("/OperationDefinition/hfs-sql-run")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");

        let format = body["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "_format")
            .expect("_format must be declared");

        assert_eq!(format["binding"]["strength"], "extensible");
        assert_eq!(
            format["binding"]["valueSet"],
            "http://hl7.org/fhir/uv/sql-on-fhir/ValueSet/OutputFormatCodes"
        );
    }

    /// The pre-ballot capabilities endpoint no longer serves capabilities.
    ///
    /// The path is not routed any more, so it falls through to the generic
    /// type-search handler and comes back as an empty searchset — which is
    /// pre-existing behaviour for any unrecognised path segment. What matters
    /// is that it no longer answers with a SoF capabilities `Parameters`.
    #[tokio::test]
    async fn test_pre_ballot_capabilities_endpoint_serves_no_capabilities() {
        let server = create_test_server().await;

        let response = server
            .get("/$sql-on-fhir-capabilities")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;

        let body: Value = serde_json::from_str(&response.text()).expect("valid JSON");
        assert_ne!(
            body["resourceType"], "Parameters",
            "3.0.0-ballot defines no $sql-on-fhir-capabilities endpoint: {body}"
        );
    }
}
