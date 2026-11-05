//! #1590: a backend that cannot honour a transaction's atomicity refuses the
//! bundle with the atomicity message whatever the entries carry. A transaction
//! with a conditional reference used to reach the reference resolver first,
//! whose search on such a backend answered "Feature 'search' is not
//! implemented" — the refusal the client saw instead of the actionable one.
//!
//! Driven over a composite with a SQLite primary and no bundle provider: it
//! reports no transaction support and has no search surface wired, the same
//! two properties the standalone `s3` backend has.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::BackendKind;
use helios_persistence::core::transaction::BundleProvider;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

fn server_without_transaction_support() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let primary = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    primary.init_schema().expect("init schema");
    let primary = Arc::new(primary);

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary as DynStorage);
    let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));
    assert!(
        !composite.supports_atomic_transactions(),
        "the fixture must not offer transactions"
    );

    let state = helios_rest::AppState::new(composite, ServerConfig::for_testing());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("server")
}

fn transaction_with_conditional_reference() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "fullUrl": "urn:uuid:1",
            "request": {"method": "POST", "url": "Encounter"},
            "resource": {
                "resourceType": "Encounter",
                "status": "finished",
                "class": {"code": "AMB"},
                "serviceProvider": {
                    "reference": "Organization?identifier=https://github.com/synthetichealth/synthea|756ed90d-15f4-377d-b99f-ca1de5633481"
                }
            }
        }]
    })
}

#[tokio::test]
async fn a_transaction_with_a_conditional_reference_gets_the_atomicity_refusal() {
    let server = server_without_transaction_support();
    let response = server
        .post("/")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&transaction_with_conditional_reference())
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::NOT_IMPLEMENTED,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome");
    assert_eq!(body["issue"][0]["code"], "not-supported");
    let text = body["issue"][0]["details"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.contains(
            "cannot guarantee the all-or-nothing semantics a transaction Bundle requires"
        ),
        "{text}"
    );
    assert!(
        !text.contains("'search'"),
        "the resolver's refusal must not answer first: {text}"
    );
}

#[tokio::test]
async fn a_batch_with_the_same_entry_is_still_processed() {
    let server = server_without_transaction_support();
    let mut bundle = transaction_with_conditional_reference();
    bundle["type"] = json!("batch");
    let response = server
        .post("/")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&bundle)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let body: Value = response.json();
    assert_eq!(body["type"], "batch-response", "{body}");
}
