//! #2064: every FHIR version helios-fhir is built with must be one the
//! validator, the CLI default and `$validate` can serve. helios-audit's R5/R6
//! switch helios-fhir/R4B on, so run this with
//! `--no-default-features --features R5,sqlite` to exercise a version the build
//! did not ask for. It fails there if helios-rest's own R5/R6 stop enabling
//! `R4B`; default features (R4 only) and `--all-features` cannot fail it. CI
//! runs it that way in the "FHIR version feature subsets (#1367)" step.
//!
//! The file compiles under any feature set: it only uses
//! `FhirVersion::enabled_versions()`, `default_enabled()`, `as_str()` and
//! `as_mime_param()`, plus one `#[cfg(feature = "R4")]` assertion.

use std::path::PathBuf;
use std::sync::Arc;

use axum_test::TestServer;
use clap::{CommandFactory, Parser, ValueEnum};
use helios_fhir::FhirVersion;
use helios_fhir_validator::SchemaResolver;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::config::ValidationConfig;
use helios_rest::validation::ValidationService;
use helios_rest::{MultitenancyConfig, ServerConfig, TenantRoutingMode};
use serde_json::{Value, json};

/// In-memory SQLite server, as in `validate_operation_tests.rs`.
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
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("Failed to create test server")
}

#[test]
fn validator_core_packs_cover_every_enabled_fhir_version() {
    for &v in FhirVersion::enabled_versions() {
        let registry = helios_fhir_validator::packs::core_registry(v);
        assert!(
            registry
                .resolve("http://hl7.org/fhir/StructureDefinition/Patient")
                .is_some(),
            "the {v:?} core schema pack has no Patient schema"
        );
        // Must not panic: it did for a version helios-fhir enabled but the
        // validator was not built with.
        let _ = helios_fhir_validator::core_terminology(v);
    }
}

/// The startup path for `HFS_DEFAULT_FHIR_VERSION=<v>` (state.rs builds the
/// service with the configured default version).
#[test]
fn validation_service_builds_for_every_enabled_default_version() {
    let config = ValidationConfig::default();
    assert_eq!(
        config.terminology, "embedded",
        "the default must use the embedded terminology, which loads core_terminology(version)"
    );
    for &v in FhirVersion::enabled_versions() {
        let _ = ValidationService::from_config(&config, None, v);
    }
}

#[tokio::test]
async fn validate_operation_serves_every_enabled_fhir_version() {
    let server = create_test_server().await;
    for &v in FhirVersion::enabled_versions() {
        // `.json()` resets the content type to application/json, so
        // `.content_type()` must come after it.
        let response = server
            .post("/Patient/$validate")
            .json(&json!({
                "resourceType": "Patient",
                "name": [{ "family": "Smith", "given": ["Jan"] }],
                "gender": "female",
                "birthDate": "1980-02-29"
            }))
            .content_type(&format!(
                "application/fhir+json; fhirVersion={}",
                v.as_mime_param()
            ))
            .await;

        response.assert_status_ok();
        let outcome: Value = response.json();
        assert_eq!(
            outcome["resourceType"], "OperationOutcome",
            "{v:?}: not an OperationOutcome: {outcome}"
        );
        let not_supported = outcome["issue"]
            .as_array()
            .map(|issues| issues.iter().any(|i| i["code"] == "not-supported"))
            .unwrap_or(false);
        assert!(
            !not_supported,
            "{v:?}: the core pack did not resolve Patient: {outcome}"
        );
    }
}

/// The clap default of `--default-fhir-version` must parse, and be the build's
/// default version. A literal `"R4"` default fails to parse in a build without
/// the `R4` feature.
#[test]
fn default_fhir_version_cli_default_is_an_enabled_version() {
    let cmd = ServerConfig::command();
    let arg = cmd
        .get_arguments()
        .find(|a| a.get_id().as_str() == "default_fhir_version")
        .expect("ServerConfig has a default_fhir_version argument");
    let defaults = arg.get_default_values();
    assert_eq!(
        defaults.len(),
        1,
        "default_fhir_version has exactly one default"
    );
    let raw = defaults[0]
        .to_str()
        .expect("the default is valid UTF-8")
        .to_string();
    let parsed = <FhirVersion as ValueEnum>::from_str(&raw, false).unwrap_or_else(|e| {
        panic!(
            "clap default {raw:?} for --default-fhir-version is not a valid version (#2064): {e}"
        )
    });

    assert_eq!(parsed, FhirVersion::default_enabled());
    assert_eq!(parsed, ServerConfig::default().default_fhir_version);
    assert_eq!(parsed, ServerConfig::for_testing().default_fhir_version);
    #[cfg(feature = "R4")]
    assert_eq!(parsed, FhirVersion::R4, "builds with R4 keep R4");
}

/// The `hfs` startup path (`try_from_env` -> `try_parse`) with no arguments.
#[test]
fn server_config_parses_without_arguments() {
    let config = ServerConfig::try_parse_from(["hfs"]).unwrap_or_else(|e| panic!("{e}"));
    if std::env::var_os("HFS_DEFAULT_FHIR_VERSION").is_none() {
        assert_eq!(config.default_fhir_version, FhirVersion::default_enabled());
    }
}

/// Whether a build with this feature set must refuse an implicit default: it
/// has no R4, and more than one version to choose between.
fn build_requires_explicit_default() -> bool {
    let enabled = FhirVersion::enabled_versions();
    !enabled.iter().any(|v| v.as_str() == "R4") && enabled.len() > 1
}

/// The enabled versions as the startup error lists them.
fn enabled_versions_list() -> String {
    FhirVersion::enabled_versions()
        .iter()
        .map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A server that never set `HFS_DEFAULT_FHIR_VERSION` must refuse to start when
/// it would otherwise serve a version nobody chose (R4B in an R5/R6 build, #2064).
#[test]
fn implicit_default_fhir_version_is_an_error_only_in_builds_without_r4() {
    for &v in FhirVersion::enabled_versions() {
        let config = ServerConfig {
            default_fhir_version: v,
            ..ServerConfig::for_testing()
        };
        assert!(
            config.implicit_default_fhir_version_error(false).is_none(),
            "{v:?}: an explicit default is never an error"
        );

        let error = config.implicit_default_fhir_version_error(true);
        assert_eq!(
            error.is_some(),
            build_requires_explicit_default(),
            "{v:?}: {error:?}"
        );
        if let Some(message) = error {
            assert!(
                message.contains("HFS_DEFAULT_FHIR_VERSION"),
                "the error names the variable: {message}"
            );
            assert!(
                message.contains(&enabled_versions_list()),
                "the error lists exactly the enabled versions ({}): {message}",
                enabled_versions_list()
            );
        }
    }
}

/// `hfs` startup: an unset variable is reported as implicit and, in a build
/// without R4 that enables several versions, produces the error; an explicit
/// `--default-fhir-version` does not.
#[test]
fn default_fhir_version_provenance_drives_the_startup_error() {
    let explicit = FhirVersion::default_enabled();
    let (config, implicit) = ServerConfig::try_parse_from_with_provenance([
        "hfs",
        "--default-fhir-version",
        explicit.as_str(),
    ])
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(config.default_fhir_version, explicit);
    assert!(!implicit, "an explicit flag is not implicit");
    assert!(
        config
            .implicit_default_fhir_version_error(implicit)
            .is_none()
    );

    if std::env::var_os("HFS_DEFAULT_FHIR_VERSION").is_none() {
        let (config, implicit) =
            ServerConfig::try_parse_from_with_provenance(["hfs"]).unwrap_or_else(|e| panic!("{e}"));
        assert!(implicit, "no flag and no env var is the built-in default");
        let error = config.implicit_default_fhir_version_error(implicit);
        assert_eq!(error.is_some(), build_requires_explicit_default());
        if let Some(message) = error {
            assert!(message.contains("HFS_DEFAULT_FHIR_VERSION"), "{message}");
            assert!(message.contains(&enabled_versions_list()), "{message}");
        }
    }
}
