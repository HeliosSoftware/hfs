//! Shared test helpers for HTS integration tests.
#![allow(dead_code)] // items are shared across test binaries; not all are used in each

pub mod bundles;

#[cfg(feature = "sqlite")]
use helios_hts::{backends::SqliteTerminologyBackend, config::HtsConfig, state::AppState};

#[cfg(feature = "postgres")]
use helios_hts::{config::HtsConfig as PgHtsConfig, state::AppState as PgAppState};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

// ── PostgreSQL shared container for HTTP tests ─────────────────────────────────

#[cfg(feature = "postgres")]
static PG_HTTP_CONTAINER: std::sync::OnceLock<
    testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
> = std::sync::OnceLock::new();

#[cfg(feature = "postgres")]
static PG_HTTP_URL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// Returns the PostgreSQL URL for the shared HTTP-test container, starting it
/// on the first call. Shared across all `TestAppPg` instances in a test binary.
#[cfg(feature = "postgres")]
pub async fn pg_http_url() -> &'static str {
    PG_HTTP_URL
        .get_or_init(|| async {
            use testcontainers::runners::AsyncRunner;
            use testcontainers_modules::postgres::Postgres;

            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start Postgres container for HTTP tests");

            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get postgres port");

            let host = container
                .get_host()
                .await
                .expect("Failed to get postgres host");

            let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
            let _ = PG_HTTP_CONTAINER.set(container);
            url
        })
        .await
}

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

    /// POST a FHIR JSON body to `path` requesting XML, returning `(status, body_string)`.
    pub async fn post_fhir_xml(&self, path: &str, body: impl Into<String>) -> (StatusCode, String) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/fhir+json")
                    .header("accept", "application/fhir+xml")
                    .body(Body::from(body.into()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// GET `path` requesting XML, returning `(status, body_string, content_type)`.
    pub async fn get_fhir_xml(&self, path: &str) -> (StatusCode, String, String) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .header("accept", "application/fhir+xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned(), ct)
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
        build_params(entries)
    }
}

// ── Shared Parameters builder ──────────────────────────────────────────────────

/// Build a FHIR Parameters JSON body from a slice of `(name, value_key, value)` triples.
pub fn build_params(entries: &[(&str, &str, &str)]) -> String {
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

// ── TestAppPg — PostgreSQL HTTP test harness ───────────────────────────────────

/// A test application that wraps the full HTS Axum router backed by a live
/// PostgreSQL instance (via testcontainers).
///
/// Uses the same `pg_http_url()` shared container so all `TestAppPg` instances
/// within a single test binary share one Docker container.
///
/// Because each `TestAppPg` connects to the *same* database, tests that write
/// data should use unique resource URLs to avoid cross-test interference.
#[cfg(feature = "postgres")]
pub struct TestAppPg {
    router: Router,
}

#[cfg(feature = "postgres")]
impl TestAppPg {
    /// Create a new `TestAppPg` wired to the shared PostgreSQL container.
    pub async fn new() -> Self {
        use helios_hts::backends::PostgresTerminologyBackend;
        use helios_persistence::backends::postgres::PostgresBackend;
        use std::sync::Arc;

        let url = pg_http_url().await;

        let backend = PostgresTerminologyBackend::new(url)
            .await
            .expect("Failed to create PostgresTerminologyBackend");

        let resource_store = PostgresBackend::from_connection_string(url)
            .await
            .expect("Failed to open PostgreSQL resource store");
        resource_store
            .init_schema()
            .await
            .expect("Failed to initialize PostgreSQL resource store schema");

        let state = PgAppState::new(backend.clone())
            .with_resource_store_pg(resource_store)
            .with_terminology_importer(Arc::new(backend));

        let config = PgHtsConfig::default();
        let router = helios_hts::server::create_app(&config, state);
        TestAppPg { router }
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

    /// PUT a FHIR JSON body to `path`, with optional `If-Match` header.
    pub async fn put_fhir(
        &self,
        path: &str,
        body: impl Into<String>,
        if_match: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("PUT")
            .uri(path)
            .header("content-type", "application/fhir+json");
        if let Some(etag) = if_match {
            builder = builder.header("if-match", etag);
        }
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::from(body.into())).unwrap())
            .await
            .unwrap();

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// DELETE `path`, returning status code.
    pub async fn delete_fhir(&self, path: &str) -> StatusCode {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
    }

    /// POST a FHIR JSON body to `path` requesting XML, returning `(status, body_string)`.
    pub async fn post_fhir_xml(&self, path: &str, body: impl Into<String>) -> (StatusCode, String) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/fhir+json")
                    .header("accept", "application/fhir+xml")
                    .body(Body::from(body.into()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// GET `path` requesting XML, returning `(status, body_string, content_type)`.
    pub async fn get_fhir_xml(&self, path: &str) -> (StatusCode, String, String) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .header("accept", "application/fhir+xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned(), ct)
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
        build_params(entries)
    }
}
