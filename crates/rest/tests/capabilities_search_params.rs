//! Integration tests for `GET /metadata` search-capability advertisement:
//! real per-resource search parameters, `_include`/`_revinclude` targets, and
//! per-parameter supported modifiers (carried as extensions).

mod capabilities_search_param_tests {
    use axum::http::{HeaderName, HeaderValue, StatusCode};
    use axum_test::TestServer;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_rest::ServerConfig;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::Arc;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
    const SEARCH_MODIFIER_EXTENSION_URL: &str =
        "http://heliossoftware.com/fhir/StructureDefinition/capabilitystatement-search-modifier";

    /// Builds a server whose SQLite backend loads the full spec SearchParameter
    /// registry from the repo `data/` directory (not just the minimal embedded set).
    async fn create_test_server() -> TestServer {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let backend_config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend =
            SqliteBackend::with_config(":memory:", backend_config).expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let backend = Arc::new(backend);

        let config = ServerConfig::for_testing();
        let state = helios_rest::AppState::new(backend, config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    async fn fetch_metadata() -> Value {
        let server = create_test_server().await;
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
            .await;
        response.assert_status(StatusCode::OK);
        serde_json::from_str(&response.text()).expect("metadata must be valid JSON")
    }

    /// Returns the `rest[0].resource` entry for a given resource type.
    fn resource_entry<'a>(metadata: &'a Value, resource_type: &str) -> &'a Value {
        metadata["rest"][0]["resource"]
            .as_array()
            .expect("rest[0].resource array")
            .iter()
            .find(|r| r["type"] == resource_type)
            .unwrap_or_else(|| panic!("{resource_type} resource not advertised"))
    }

    fn search_param_names(resource: &Value) -> Vec<String> {
        resource["searchParam"]
            .as_array()
            .expect("searchParam array")
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    }

    fn string_list(value: &Value, field: &str) -> Vec<String> {
        value[field]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn advertises_real_per_resource_search_params() {
        let metadata = fetch_metadata().await;

        let patient = resource_entry(&metadata, "Patient");
        let patient_params = search_param_names(patient);
        assert!(
            patient_params.contains(&"name".to_string()),
            "Patient must advertise its real `name` param, got: {patient_params:?}"
        );

        let observation = resource_entry(&metadata, "Observation");
        let observation_params = search_param_names(observation);
        assert!(
            observation_params.contains(&"code".to_string()),
            "Observation must advertise its real `code` param, got: {observation_params:?}"
        );
    }

    #[tokio::test]
    async fn advertises_real_include_and_revinclude_targets() {
        let metadata = fetch_metadata().await;
        let patient = resource_entry(&metadata, "Patient");

        // searchInclude is now derived from real reference params, not "*".
        let includes = string_list(patient, "searchInclude");
        assert!(
            !includes.contains(&"*".to_string()),
            "searchInclude must not be the wildcard, got: {includes:?}"
        );
        assert!(
            includes.iter().all(|i| i.starts_with("Patient:")),
            "every Patient searchInclude must be a `Patient:<code>` token, got: {includes:?}"
        );

        // Observation references Patient (e.g. Observation:subject / :patient), so
        // Patient's searchRevInclude must surface an `Observation:*` token.
        let revincludes = string_list(patient, "searchRevInclude");
        assert!(
            !revincludes.contains(&"*".to_string()),
            "searchRevInclude must not be the wildcard, got: {revincludes:?}"
        );
        assert!(
            revincludes.iter().any(|r| r.starts_with("Observation:")),
            "Patient searchRevInclude must include an Observation reference, got: {revincludes:?}"
        );
    }

    #[tokio::test]
    async fn reference_params_advertise_modifiers_via_extension() {
        let metadata = fetch_metadata().await;
        let patient = resource_entry(&metadata, "Patient");

        // Find a reference-typed search param and confirm it carries the modifier
        // extension with valid `valueCode` entries.
        let reference_param = patient["searchParam"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["type"] == "reference")
            .expect("Patient has at least one reference search param");

        let ext = reference_param["extension"]
            .as_array()
            .expect("reference param carries a modifier extension");
        assert!(
            ext.iter()
                .all(|e| e["url"] == SEARCH_MODIFIER_EXTENSION_URL),
            "all modifier extensions use the modifier extension URL: {ext:?}"
        );
        let codes: Vec<&str> = ext.iter().filter_map(|e| e["valueCode"].as_str()).collect();
        // SQLite advertises `below`/`above`/`identifier`/etc. for reference params.
        assert!(
            codes.contains(&"identifier"),
            "reference modifier set should include `identifier`, got: {codes:?}"
        );
    }
}
