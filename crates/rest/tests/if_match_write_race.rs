//! #1404: `If-Match` on the instance `PUT`, `PATCH` and `DELETE` is a
//! compare-and-swap in storage, not a check in the handler followed by an
//! unconditional write.
//!
//! The handlers read the current resource to evaluate the precondition and
//! then write. These tests land a second writer *between* those two steps,
//! deterministically: the primary backend is wrapped so that a `read` can be
//! armed to update the resource right after it has produced its answer. The
//! handler therefore sees version 1, the precondition `W/"1"` is satisfied,
//! and by the time the handler writes, version 2 — which the client never saw
//! — is current. The write must be refused (409) and version 2 must survive.
//!
//! Before the fix `DELETE` answered `204` and deleted version 2.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

fn sqlite() -> SqliteBackend {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");
    backend
}

/// A primary whose next `read` is followed — before the caller can act on what
/// it read — by another writer's update.
struct RacingPrimary {
    inner: SqliteBackend,
    /// Content the interloper writes after the next `read`; taken when used.
    interloper: Mutex<Option<Value>>,
}

#[async_trait]
impl ResourceStorage for RacingPrimary {
    fn backend_name(&self) -> &'static str {
        "racing-primary"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        self.inner
            .create(tenant, resource_type, resource, fhir_version)
            .await
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        self.inner
            .create_or_update(tenant, resource_type, id, resource, fhir_version)
            .await
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let seen = self.inner.read(tenant, resource_type, id).await?;
        let interloper = self.interloper.lock().expect("interloper lock").take();
        if let (Some(seen), Some(content)) = (seen.as_ref(), interloper) {
            self.inner
                .update(tenant, seen, content)
                .await
                .expect("the interloper's update lands");
        }
        Ok(seen)
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner.update(tenant, current, resource).await
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.inner.delete(tenant, resource_type, id).await
    }

    // A wrapper around a real backend delegates; the trait's default is
    // read-compare-delete and would reopen the window.
    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        self.inner
            .delete_versioned(tenant, resource_type, id, expected_version)
            .await
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.inner.count(tenant, resource_type).await
    }
}

fn server() -> (TestServer, Arc<RacingPrimary>) {
    let primary = Arc::new(RacingPrimary {
        inner: sqlite(),
        interloper: Mutex::new(None),
    });

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));

    let state = helios_rest::AppState::new(
        composite,
        ServerConfig {
            base_url: "http://localhost:8080".to_string(),
            ..ServerConfig::for_testing()
        },
    );
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server");
    (server, primary)
}

fn patient(id: &str, family: &str) -> Value {
    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]})
}

/// Creates `Patient/{id}` and proves it is live at version 1 with `If-Match`
/// honoured at all — the positive control for every refusal below.
async fn seed(server: &TestServer, id: &str) {
    let created = server
        .put(&format!("/Patient/{id}"))
        .json(&patient(id, "Seed"))
        .await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );

    let read = server.get(&format!("/Patient/{id}")).await;
    assert_eq!(read.status_code(), StatusCode::OK);
    assert_eq!(read.json::<Value>()["meta"]["versionId"], "1");

    let stale = server
        .delete(&format!("/Patient/{id}"))
        .add_header("If-Match", "W/\"7\"")
        .await;
    assert_eq!(
        stale.status_code(),
        StatusCode::PRECONDITION_FAILED,
        "an unsatisfied If-Match is refused by the handler's own check"
    );
}

/// The interloper's version 2 is current and intact.
async fn assert_interloper_survives(server: &TestServer, id: &str) {
    let read = server.get(&format!("/Patient/{id}")).await;
    assert_eq!(
        read.status_code(),
        StatusCode::OK,
        "version 2 was never seen by the client and must not have been removed: {}",
        read.text()
    );
    let body = read.json::<Value>();
    assert_eq!(body["meta"]["versionId"], "2");
    assert_eq!(body["name"][0]["family"], "Interloper");
}

#[tokio::test]
async fn delete_with_if_match_does_not_delete_a_version_written_after_the_check() {
    let (server, primary) = server();
    seed(&server, "del").await;

    *primary.interloper.lock().unwrap() = Some(patient("del", "Interloper"));
    let response = server
        .delete("/Patient/del")
        .add_header("If-Match", "W/\"1\"")
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::CONFLICT,
        "the delete lost to a concurrent writer: {}",
        response.text()
    );
    assert_interloper_survives(&server, "del").await;

    // The client re-reads, sees version 2, and may delete that.
    let retry = server
        .delete("/Patient/del")
        .add_header("If-Match", "W/\"2\"")
        .await;
    assert_eq!(
        retry.status_code(),
        StatusCode::NO_CONTENT,
        "{}",
        retry.text()
    );
    assert_eq!(
        server.get("/Patient/del").await.status_code(),
        StatusCode::GONE
    );
}

#[tokio::test]
async fn delete_without_if_match_still_deletes_whatever_is_current() {
    let (server, primary) = server();
    seed(&server, "plain").await;

    *primary.interloper.lock().unwrap() = Some(patient("plain", "Interloper"));
    let response = server.delete("/Patient/plain").await;

    assert_eq!(
        response.status_code(),
        StatusCode::NO_CONTENT,
        "FHIR's delete carries no precondition of its own: {}",
        response.text()
    );
    assert_eq!(
        server.get("/Patient/plain").await.status_code(),
        StatusCode::GONE
    );
}

#[tokio::test]
async fn put_with_if_match_does_not_overwrite_a_version_written_after_the_check() {
    let (server, primary) = server();
    seed(&server, "put").await;

    *primary.interloper.lock().unwrap() = Some(patient("put", "Interloper"));
    let response = server
        .put("/Patient/put")
        .add_header("If-Match", "W/\"1\"")
        .json(&patient("put", "Overwriter"))
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::CONFLICT,
        "{}",
        response.text()
    );
    assert_interloper_survives(&server, "put").await;
}

#[tokio::test]
async fn patch_with_if_match_does_not_overwrite_a_version_written_after_the_check() {
    let (server, primary) = server();
    seed(&server, "patch").await;

    *primary.interloper.lock().unwrap() = Some(patient("patch", "Interloper"));
    let response = server
        .patch("/Patient/patch")
        .add_header("If-Match", "W/\"1\"")
        .text(r#"[{"op":"replace","path":"/name/0/family","value":"Patcher"}]"#)
        .content_type("application/json-patch+json")
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::CONFLICT,
        "{}",
        response.text()
    );
    assert_interloper_survives(&server, "patch").await;
}
