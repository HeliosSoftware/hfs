//! #2562: writes that read a resource, work on it and write it back race
//! with a metadata rewrite that keeps the version id. These tests land the
//! rewrite deterministically between the read and the write: the primary is
//! wrapped so that a `read` can be armed to change the resource right after
//! it has answered. The derived write must lose its compare-and-swap, read
//! again and redo its work, so the rewrite's label survives; one that keeps
//! losing gives up after four attempts with a 409.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{
    BackendKind, MetaChange, MetaRewrite, MetaTarget, ResourceStorage, VersionedStorage,
    WRITE_ATTEMPTS,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const TAGS: &str = "http://example.org/tags";

fn tag(code: &str) -> Value {
    json!({"system": TAGS, "code": code})
}

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

/// What another writer does right after an armed `read`.
#[derive(Clone)]
enum Interloper {
    /// `$meta-add` of this tag: same version, new revision.
    AddTag(&'static str),
    /// A new version with this content.
    Update(Value),
}

struct RacingPrimary {
    inner: SqliteBackend,
    interloper: Mutex<Option<Interloper>>,
    /// Re-arm after every read, so the caller never wins.
    always: Mutex<bool>,
    reads: AtomicUsize,
}

impl RacingPrimary {
    fn arm(&self, interloper: Interloper) {
        *self.interloper.lock().unwrap() = Some(interloper);
    }

    fn arm_always(&self, interloper: Interloper) {
        *self.always.lock().unwrap() = true;
        self.arm(interloper);
    }
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
        self.reads.fetch_add(1, Ordering::SeqCst);
        let seen = self.inner.read(tenant, resource_type, id).await?;
        let interloper = {
            let mut armed = self.interloper.lock().unwrap();
            if *self.always.lock().unwrap() {
                armed.clone()
            } else {
                armed.take()
            }
        };
        if let (Some(seen), Some(interloper)) = (seen.as_ref(), interloper) {
            match interloper {
                Interloper::AddTag(code) => {
                    self.inner
                        .rewrite_meta(
                            tenant,
                            resource_type,
                            id,
                            &MetaTarget::Current {
                                expected_version: seen.version_id().to_string(),
                                expected_meta_seq: seen.meta_seq(),
                            },
                            &MetaChange::Add(json!({"tag": [tag(code)]})),
                        )
                        .await
                        .expect("the interloper's rewrite lands");
                }
                Interloper::Update(content) => {
                    self.inner
                        .update(tenant, seen, content)
                        .await
                        .expect("the interloper's update lands");
                }
            }
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

    fn tracks_meta_revision(&self) -> bool {
        self.inner.tracks_meta_revision()
    }

    fn supports_meta_rewrite(&self) -> bool {
        self.inner.supports_meta_rewrite()
    }

    async fn rewrite_meta(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        target: &MetaTarget,
        change: &MetaChange,
    ) -> StorageResult<MetaRewrite> {
        self.inner
            .rewrite_meta(tenant, resource_type, id, target, change)
            .await
    }

    async fn restore_tombstone(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.inner
            .restore_tombstone(tenant, resource_type, id, resource)
            .await
    }
}

fn server() -> (TestServer, Arc<RacingPrimary>) {
    let primary = Arc::new(RacingPrimary {
        inner: sqlite(),
        interloper: Mutex::new(None),
        always: Mutex::new(false),
        reads: AtomicUsize::new(0),
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
        .expect("test server");
    (server, primary)
}

fn task(id: &str, tags: &[&str], status: &str) -> Value {
    let tags: Vec<Value> = tags.iter().map(|c| tag(c)).collect();
    json!({"resourceType": "Task", "id": id, "status": status, "intent": "order",
           "meta": {"tag": tags}})
}

async fn seed(server: &TestServer, id: &str) {
    server
        .put(&format!("/Task/{id}"))
        .json(&task(id, &["a"], "requested"))
        .await
        .assert_status(StatusCode::CREATED);
}

async fn current(server: &TestServer, id: &str) -> Value {
    let read = server.get(&format!("/Task/{id}")).await;
    read.assert_status_ok();
    read.json()
}

fn codes(resource: &Value) -> Vec<String> {
    resource["meta"]["tag"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .map(|t| t["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn put_keeps_a_label_added_between_its_read_and_its_write() {
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::AddTag("b"));
    server
        .put("/Task/t1")
        .json(&task("t1", &["c"], "requested"))
        .await
        .assert_status_ok();
    let body = current(&server, "t1").await;
    assert_eq!(codes(&body), ["a", "b", "c"]);
    assert_eq!(body["meta"]["versionId"], "2");
}

#[tokio::test]
async fn put_with_if_match_reads_again_and_keeps_the_label() {
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::AddTag("b"));
    server
        .put("/Task/t1")
        .add_header("If-Match", "W/\"1\"")
        .json(&task("t1", &["c"], "requested"))
        .await
        .assert_status_ok();
    let body = current(&server, "t1").await;
    assert_eq!(codes(&body), ["a", "b", "c"]);
    assert_eq!(body["meta"]["versionId"], "2", "one new version, not two");
}

#[tokio::test]
async fn an_unrelated_patch_keeps_the_label() {
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::AddTag("b"));
    server
        .patch("/Task/t1")
        .json(&json!([{"op": "replace", "path": "/status", "value": "accepted"}]))
        .content_type("application/json-patch+json")
        .await
        .assert_status_ok();
    let body = current(&server, "t1").await;
    assert_eq!(codes(&body), ["a", "b"]);
    assert_eq!(body["status"], "accepted");
}

fn bundle(entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": "batch", "entry": entries})
}

#[tokio::test]
async fn batch_puts_and_patches_keep_the_label() {
    let (server, primary) = server();
    for (id, request) in [
        ("p1", json!({"method": "PUT", "url": "Task/p1"})),
        (
            "p2",
            json!({"method": "PUT", "url": "Task/p2", "ifMatch": "W/\"1\""}),
        ),
        ("q1", json!({"method": "PATCH", "url": "Task/q1"})),
        (
            "q2",
            json!({"method": "PATCH", "url": "Task/q2", "ifMatch": "W/\"1\""}),
        ),
    ] {
        seed(&server, id).await;
        let resource = if request["method"] == "PUT" {
            task(id, &["c"], "requested")
        } else {
            json!({"resourceType": "Parameters", "parameter": [{
                "name": "operation", "part": [
                    {"name": "type", "valueCode": "replace"},
                    {"name": "path", "valueString": "Task.status"},
                    {"name": "value", "valueCode": "accepted"}
                ]
            }]})
        };
        primary.arm(Interloper::AddTag("b"));
        let response = server
            .post("/")
            .json(&bundle(vec![
                json!({"resource": resource, "request": request}),
            ]))
            .await;
        response.assert_status_ok();
        let entry = &response.json::<Value>()["entry"][0];
        assert!(
            entry["response"]["status"]
                .as_str()
                .unwrap()
                .starts_with("200"),
            "{id}: {entry}"
        );
        let body = current(&server, id).await;
        assert!(codes(&body).contains(&"b".to_string()), "{id}: {body}");
    }
}

#[tokio::test]
async fn concurrent_meta_adds_keep_both_labels() {
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::AddTag("b"));
    server
        .post("/Task/t1/$meta-add")
        .json(&json!({"resourceType": "Parameters", "parameter": [
            {"name": "meta", "valueMeta": {"tag": [tag("c")]}}
        ]}))
        .await
        .assert_status_ok();
    let body = current(&server, "t1").await;
    assert_eq!(codes(&body), ["a", "b", "c"]);
    assert_eq!(body["meta"]["versionId"], "1");
}

#[tokio::test]
async fn a_meta_add_racing_a_new_version_applies_to_that_version() {
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::Update(task("t1", &["a"], "accepted")));
    server
        .post("/Task/t1/$meta-add")
        .json(&json!({"resourceType": "Parameters", "parameter": [
            {"name": "meta", "valueMeta": {"tag": [tag("c")]}}
        ]}))
        .await
        .assert_status_ok();
    let body = current(&server, "t1").await;
    assert_eq!(body["meta"]["versionId"], "2");
    assert_eq!(body["status"], "accepted");
    assert_eq!(codes(&body), ["a", "c"]);
    let tenant = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    let v1 = primary
        .inner
        .vread(&tenant, "Task", "t1", "1")
        .await
        .unwrap()
        .expect("version 1");
    assert_eq!(
        codes(v1.content()),
        ["a"],
        "the stale target is not edited in history"
    );
}

#[tokio::test]
async fn a_write_that_keeps_losing_answers_409_after_four_attempts() {
    type Request = Box<dyn Fn(&TestServer) -> axum_test::TestRequest>;
    let cases: Vec<(&str, Request)> = vec![
        (
            "PUT If-Match",
            Box::new(|s: &TestServer| {
                s.put("/Task/t1")
                    .add_header("If-Match", "W/\"1\"")
                    .json(&task("t1", &["c"], "requested"))
            }),
        ),
        (
            "PUT",
            Box::new(|s: &TestServer| s.put("/Task/t1").json(&task("t1", &["c"], "requested"))),
        ),
        (
            "PATCH",
            Box::new(|s: &TestServer| {
                s.patch("/Task/t1")
                    .json(&json!([{"op": "replace", "path": "/status", "value": "accepted"}]))
                    .content_type("application/json-patch+json")
            }),
        ),
        (
            "$meta-add",
            Box::new(|s: &TestServer| {
                s.post("/Task/t1/$meta-add").json(&json!({
                    "resourceType": "Parameters",
                    "parameter": [{"name": "meta", "valueMeta": {"tag": [tag("c")]}}]
                }))
            }),
        ),
    ];
    for (name, request) in cases {
        let (server, primary) = server();
        seed(&server, "t1").await;
        primary.arm_always(Interloper::AddTag("b"));
        let reads_before = primary.reads.load(Ordering::SeqCst);
        let response = request(&server).await;
        assert_eq!(
            response.status_code(),
            StatusCode::CONFLICT,
            "{name}: {}",
            response.text()
        );
        assert_eq!(
            primary.reads.load(Ordering::SeqCst) - reads_before,
            WRITE_ATTEMPTS,
            "{name}: one read per attempt"
        );
    }
}

#[tokio::test]
async fn instance_delete_keeps_its_outcomes() {
    // Without If-Match an intervening metadata rewrite does not stop it.
    let (server, primary) = server();
    seed(&server, "t1").await;
    primary.arm(Interloper::AddTag("b"));
    server
        .delete("/Task/t1")
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server.get("/Task/t1").await.assert_status(StatusCode::GONE);

    // An If-Match already stale at the request's read is a 412.
    seed(&server, "t2").await;
    server
        .delete("/Task/t2")
        .add_header("If-Match", "W/\"7\"")
        .await
        .assert_status(StatusCode::PRECONDITION_FAILED);

    // Satisfied at the read, then a new version wins the storage CAS: 409.
    seed(&server, "t3").await;
    primary.arm(Interloper::Update(task("t3", &["a"], "accepted")));
    server
        .delete("/Task/t3")
        .add_header("If-Match", "W/\"1\"")
        .await
        .assert_status(StatusCode::CONFLICT);
    assert_eq!(current(&server, "t3").await["meta"]["versionId"], "2");
}
