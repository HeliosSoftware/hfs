//! Shared test helpers for HTS integration tests.
#![allow(dead_code)] // items are shared across test binaries; not all are used in each

pub mod bundles;

#[cfg(feature = "sqlite")]
use helios_hts::{backend::SqliteTerminologyBackend, config::HtsConfig, state::AppState};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

/// A lightweight test application that wraps the full HTS Axum router backed
/// by an isolated in-memory SQLite database.
pub struct TestApp {
    router: Router,
}

impl TestApp {
    /// Create a new `TestApp` with a fresh, empty in-memory SQLite store.
    #[cfg(feature = "sqlite")]
    pub fn new() -> Self {
        let backend =
            SqliteTerminologyBackend::in_memory().expect("failed to create in-memory HTS backend");
        let state = AppState::new(backend);
        let config = HtsConfig::default();
        let router = helios_hts::server::create_app(&config, state);
        TestApp { router }
    }

    /// POST a FHIR JSON body to `path`, returning `(status, response_body)`.
    pub async fn post_fhir(&self, path: &str, body: impl Into<String>) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/fhir+json")
                    .body(Body::from(body.into()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// GET `path`, returning `(status, response_body)`.
    pub async fn get_fhir(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// Import a FHIR Bundle JSON string, returning `(status, response_body)`.
    pub async fn import_bundle(&self, bundle: &str) -> (StatusCode, Value) {
        self.post_fhir("/import", bundle).await
    }

    /// Import a FHIR Bundle and assert success (200 or 207 with no errors).
    pub async fn import_bundle_ok(&self, bundle: &str) -> Value {
        let (status, body) = self.import_bundle(bundle).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::MULTI_STATUS,
            "import failed with {status}: {body}"
        );
        if status == StatusCode::MULTI_STATUS {
            let errors = &body["errors"];
            assert!(
                errors.is_null() || errors.as_array().is_none_or(|a| a.is_empty()),
                "import returned errors: {errors}"
            );
        }
        body
    }

    /// Build a FHIR Parameters JSON body from a slice of `(name, value_key, value)` triples.
    pub fn params(entries: &[(&str, &str, &str)]) -> String {
        let params: Vec<Value> = entries
            .iter()
            .map(|(name, key, val)| {
                let mut obj = serde_json::Map::new();
                obj.insert("name".into(), Value::String(name.to_string()));
                obj.insert(key.to_string(), Value::String(val.to_string()));
                Value::Object(obj)
            })
            .collect();
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": params
        })
        .to_string()
    }
}
