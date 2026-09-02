//! Regression test for #787: a `SearchParameter` POSTed inside a transaction
//! Bundle (exactly how Inferno's US Core setup registers `asserted-date`,
//! per `crates/hfs/tests/inferno/uscore_bundle_asserted-date.json`) must take
//! effect for the very next request — not only after the TTL cache refresh.
//!
//! `sqlite`/`postgres` transaction-bundle processing never invalidated the
//! tenant's cached SearchParameter registry on commit (unlike their own
//! non-transactional `create()`, and unlike MongoDB's transaction path), so a
//! search using the new parameter right after the transaction committed would
//! silently drop it under lenient handling and over-return, instead of
//! filtering.
//!
//! Two *separate* transaction Bundles are posted deliberately (mirroring
//! `install.sh`'s sequential file loads): the first registers the
//! `SearchParameter`, the second creates resources indexed under it. Combining
//! both writes into one Bundle would not exercise the bug, since a single
//! transaction's resource indexing uses the extractor snapshot captured at
//! its own start, before its own SearchParameter entry commits.

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

fn nickname_search_parameter() -> Value {
    json!({
        "resourceType": "SearchParameter",
        "id": "patient-nickname-tx",
        "url": "http://acme.health/fhir/SearchParameter/patient-nickname-tx",
        "name": "nickname-tx",
        "status": "active",
        "code": "nickname-tx",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use = 'nickname').given"
    })
}

fn register_search_parameter_bundle() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "resource": nickname_search_parameter(),
            "request": { "method": "POST", "url": "SearchParameter" }
        }]
    })
}

fn create_patients_bundle() -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "matches",
                    "name": [{ "use": "nickname", "given": ["Ace"] }, { "family": "Adams" }]
                },
                "request": { "method": "PUT", "url": "Patient/matches" }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "no-match",
                    "name": [{ "family": "Baker" }]
                },
                "request": { "method": "PUT", "url": "Patient/no-match" }
            }
        ]
    })
}

/// Posts both transaction Bundles, then asserts the new parameter already
/// filters correctly — under both lenient (default) and strict handling.
async fn assert_search_parameter_takes_effect_immediately(server: &TestServer) {
    let register = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&register_search_parameter_bundle())
        .await;
    register.assert_status_ok();

    let create = server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .json(&create_patients_bundle())
        .await;
    create.assert_status_ok();

    let hit = server
        .get("/Patient?nickname-tx=Ace")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await;
    assert_eq!(hit.status_code(), StatusCode::OK);
    let body: Value = hit.json();
    let entries = body["entry"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        entries.len(),
        1,
        "a SearchParameter registered in a prior transaction Bundle must filter \
         immediately, not just after the TTL refresh — got {entries:?}"
    );
    assert_eq!(entries[0]["resource"]["id"].as_str(), Some("matches"));

    // Under strict handling an unrecognized parameter 400s — confirm this one
    // is genuinely recognized, not just coincidentally passing under lenient.
    let strict = server
        .get("/Patient?nickname-tx=Ace")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("handling=strict"),
        )
        .await;
    assert_eq!(
        strict.status_code(),
        StatusCode::OK,
        "strict handling must recognize the just-registered parameter"
    );
}

mod sqlite_tests {
    use super::*;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_rest::ServerConfig;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    async fn server() -> TestServer {
        let backend = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir()),
                ..Default::default()
            },
        )
        .expect("create SQLite backend");
        backend.init_schema().expect("init schema");
        let config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::new(backend), config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    #[tokio::test]
    async fn sqlite_transaction_bundle_search_parameter_takes_effect_immediately() {
        let server = server().await;
        assert_search_parameter_takes_effect_immediately(&server).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres_tests {
    use super::*;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_rest::ServerConfig;
    use std::path::PathBuf;
    use std::sync::Arc;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn server() -> TestServer {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        // Pin the major version. testcontainers-modules defaults to
        // postgres:11, which is EOL and predates `plan_cache_mode` — a GUC the
        // backend sends as a startup option, so PG 11 rejects every connection
        // FATAL ("db error" from the pool). The rest of the repo runs 16.
        let container = Postgres::default()
            .with_tag("16-alpine")
            .with_label("github.run_id", &run_id)
            .start()
            .await
            .expect("failed to start PostgreSQL container");

        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to get host port");
        let host = container
            .get_host()
            .await
            .expect("failed to get host")
            .to_string();

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = PostgresConfig {
            host,
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        };

        let backend = PostgresBackend::new(config)
            .await
            .expect("create PostgreSQL backend");
        backend.init_schema().await.expect("init schema");
        // Container is intentionally leaked for the lifetime of the test
        // process; each test gets its own container to avoid cross-test
        // tenant-registry interference within one `SharedPg`-style pool.
        std::mem::forget(container);

        let server_config = ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            default_tenant: "default".to_string(),
            ..ServerConfig::for_testing()
        };
        let state = helios_rest::AppState::new(Arc::new(backend), server_config);
        let app = helios_rest::routing::fhir_routes::create_routes(state);
        TestServer::new(app).expect("create test server")
    }

    #[tokio::test]
    async fn postgres_transaction_bundle_search_parameter_takes_effect_immediately() {
        let server = server().await;
        assert_search_parameter_takes_effect_immediately(&server).await;
    }
}
