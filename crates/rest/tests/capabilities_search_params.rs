//! Integration tests for `GET /metadata` search-capability advertisement:
//! real per-resource search parameters, `_include`/`_revinclude` targets, and
//! per-parameter supported modifiers (carried as extensions), and which
//! parameters a storage serves (`SearchProvider::serves_search_param`).

#[path = "common/capability_hooks.rs"]
mod capability_hooks;

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

/// `searchParam`, `searchInclude` and `searchRevInclude` list only what the
/// storage's `serves_search_param` serves. FHIR's `searchParam` is the "Search
/// parameters for implementations to support and/or make use of"
/// (https://hl7.org/fhir/R4/capabilitystatement-definitions.html#CapabilityStatement.rest.resource.searchParam),
/// and an empty `searchInclude` means "the server does not support includes"
/// (#CapabilityStatement.rest.resource.searchInclude).
mod serves_search_param_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use helios_fhir::FhirVersion;
    use helios_persistence::types::SearchParamType;
    use serde_json::Value;

    use crate::capability_hooks::{
        HookStub, interaction_codes, metadata_server, serves_all, serves_token_date_uri, statement,
    };

    /// Each resource type's `searchParam` names, `searchInclude` and
    /// `searchRevInclude`, as sorted sets.
    type Lists = BTreeMap<String, [BTreeSet<String>; 3]>;

    fn lists(statement: &Value) -> Lists {
        let strings = |value: &Value| -> BTreeSet<String> {
            value
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        statement["rest"][0]["resource"]
            .as_array()
            .expect("resources")
            .iter()
            .map(|entry| {
                let params = entry["searchParam"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|p| p["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    entry["type"].as_str().expect("type").to_string(),
                    [
                        params,
                        strings(&entry["searchInclude"]),
                        strings(&entry["searchRevInclude"]),
                    ],
                )
            })
            .collect()
    }

    fn entry<'a>(statement: &'a Value, resource_type: &str) -> &'a Value {
        statement["rest"][0]["resource"]
            .as_array()
            .expect("resources")
            .iter()
            .find(|r| r["type"] == resource_type)
            .unwrap_or_else(|| panic!("{resource_type} not advertised"))
    }

    fn param_names(entry: &Value) -> Vec<String> {
        entry["searchParam"]
            .as_array()
            .expect("searchParam")
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    }

    /// The default path: a storage that keeps the trait default lists what
    /// SQLite's own statement lists, for every type and FHIR version, and also
    /// the `special` parameters SQLite builds no condition for and so leaves
    /// out (`near`, and from R5 `contains` and `section-text`, #2593), with the
    /// composite `section-code-text`, whose `section-text` component SQLite
    /// does not serve. Nothing else differs, the include lists included.
    #[tokio::test]
    async fn the_default_serves_every_registered_parameter() {
        let stub = metadata_server(HookStub::with_params(serves_all));
        let sqlite = metadata_server_over_sqlite();
        for version in FhirVersion::enabled_versions() {
            let from_stub = lists(&statement(&stub, *version).await);
            let from_sqlite = lists(&statement(&sqlite, *version).await);
            assert!(!from_stub.is_empty());
            assert_eq!(
                from_stub.keys().collect::<Vec<_>>(),
                from_sqlite.keys().collect::<Vec<_>>(),
                "{version:?}"
            );
            let mut only_default = BTreeSet::new();
            for (resource_type, [params, includes, revincludes]) in &from_stub {
                let [sqlite_params, sqlite_includes, sqlite_revincludes] =
                    &from_sqlite[resource_type];
                assert!(
                    sqlite_params.is_subset(params),
                    "{version:?} {resource_type}"
                );
                for name in params.difference(sqlite_params) {
                    only_default.insert(format!("{resource_type}.{name}"));
                }
                assert_eq!(includes, sqlite_includes, "{version:?} {resource_type}");
                assert_eq!(
                    revincludes, sqlite_revincludes,
                    "{version:?} {resource_type}"
                );
            }
            let mut expected: BTreeSet<String> = ["Location.near".to_string()].into();
            if from_stub["Location"][0].contains("contains") {
                expected.insert("Location.contains".to_string());
            }
            if from_stub
                .get("Composition")
                .is_some_and(|lists| lists[0].contains("section-text"))
            {
                expected.insert("Composition.section-text".to_string());
            }
            if from_stub
                .get("Composition")
                .is_some_and(|lists| lists[0].contains("section-code-text"))
            {
                expected.insert("Composition.section-code-text".to_string());
            }
            assert_eq!(only_default, expected, "{version:?}");
            let patient = &from_stub["Patient"];
            assert!(patient[0].contains("name"), "{version:?}");
            assert!(patient[1].contains("Patient:general-practitioner"));
            assert!(patient[2].contains("Observation:subject"));
        }
    }

    /// SQLite itself, through the full router, as every existing test serves it.
    fn metadata_server_over_sqlite() -> axum_test::TestServer {
        use std::sync::Arc;
        let config = helios_rest::ServerConfig {
            multitenancy: helios_rest::config::MultitenancyConfig {
                routing_mode: helios_rest::config::TenantRoutingMode::HeaderOnly,
                ..Default::default()
            },
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "test-tenant".to_string(),
            ..helios_rest::ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::new(crate::capability_hooks::sqlite()), config);
        axum_test::TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("test server")
    }

    /// A storage that serves token, date and URI parameters only: every listed
    /// parameter has one of those types, the common ones included, apart from
    /// `_list` and `_in`, which REST resolves into an `_id` filter (token), and
    /// no type lists an include, since every include names a reference
    /// parameter. `search-type` and the rest of the entry stay.
    #[tokio::test]
    async fn only_served_types_are_listed_and_no_include_without_reference_params() {
        let server = metadata_server(HookStub::with_params(serves_token_date_uri));
        for version in FhirVersion::enabled_versions() {
            let statement = statement(&server, *version).await;
            for resource in statement["rest"][0]["resource"]
                .as_array()
                .expect("resources")
            {
                let ty = &resource["type"];
                for param in resource["searchParam"].as_array().expect("searchParam") {
                    assert!(
                        matches!(param["type"].as_str(), Some("token" | "date" | "uri"))
                            || matches!(param["name"].as_str(), Some("_list" | "_in")),
                        "{version:?} {ty}: {param}"
                    );
                }
                for key in ["searchInclude", "searchRevInclude"] {
                    assert!(resource.get(key).is_none(), "{version:?} {ty}: {key}");
                }
                assert!(
                    interaction_codes(&resource["interaction"]).contains(&"search-type".into()),
                    "{version:?} {ty}"
                );
            }
            let patient = param_names(entry(&statement, "Patient"));
            for kept in [
                "_id",
                "_lastUpdated",
                "_tag",
                "_profile",
                "_security",
                "_list",
            ] {
                assert!(patient.contains(&kept.to_string()), "{version:?}: {kept}");
            }
            // `_in` exists from R5 on.
            if matches!(version.as_mime_param(), "5.0" | "6.0") {
                assert!(patient.contains(&"_in".to_string()), "{version:?}: _in");
            }
            for kept in ["identifier", "birthdate", "gender"] {
                assert!(patient.contains(&kept.to_string()), "{version:?}: {kept}");
            }
            for dropped in [
                "name",
                "family",
                "general-practitioner",
                "_text",
                "_content",
            ] {
                assert!(
                    !patient.contains(&dropped.to_string()),
                    "{version:?}: {dropped}"
                );
            }
        }
    }

    /// The hook is per parameter, not per type: refusing one `special`
    /// parameter (`near`, #2593) or one reference parameter
    /// (`Observation.subject`) drops only that one, and the reference
    /// parameter's include and reverse include with it.
    #[tokio::test]
    async fn one_parameter_is_dropped_without_its_type() {
        fn serves(resource_type: &str, code: &str, _: SearchParamType) -> bool {
            !(resource_type == "Location" && code == "near")
                && !(resource_type == "Observation" && code == "subject")
        }
        let server = metadata_server(HookStub::with_params(serves));
        let version = FhirVersion::default_enabled();
        let statement = statement(&server, version).await;

        let location = param_names(entry(&statement, "Location"));
        assert!(!location.contains(&"near".to_string()), "{location:?}");
        assert!(location.contains(&"_text".to_string()), "{location:?}");
        assert!(location.contains(&"address".to_string()), "{location:?}");

        let observation = entry(&statement, "Observation");
        let params = param_names(observation);
        assert!(!params.contains(&"subject".to_string()));
        assert!(params.contains(&"patient".to_string()));
        let all = lists(&statement);
        assert!(!all["Observation"][1].contains("Observation:subject"));
        assert!(all["Observation"][1].contains("Observation:patient"));
        assert!(!all["Patient"][2].contains("Observation:subject"));
        assert!(all["Patient"][2].contains("Observation:patient"));
        assert!(
            all.values()
                .all(|lists| !lists[2].contains("Observation:subject")),
            "no type keeps the reverse include of an unserved parameter"
        );
    }
}
