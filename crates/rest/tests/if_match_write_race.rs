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
//!
//! #2527: the same races through a `batch` Bundle entry carrying
//! `request.ifMatch`. The batch arm gated the entry on one read and then wrote
//! unconditionally (`create_or_update` / `delete`), so it deleted or
//! overwrote version 2 and revived a resource deleted after the check. Each
//! batch test below sits next to the single-request control it must agree
//! with.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::Barrier;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{CompositeConfig, CompositeStorage, DynStorage};
use helios_persistence::core::{BackendKind, ResourceStorage, VersionedStorage};
use helios_persistence::error::{ResourceError, StorageError, StorageResult};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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

/// What another writer does right after the next `read`.
enum Interloper {
    /// Writes a new version with this content.
    Update(Value),
    /// Deletes the resource.
    Delete,
}

/// A primary whose next `read` is followed — before the caller can act on what
/// it read — by another writer's update or delete.
struct RacingPrimary {
    inner: SqliteBackend,
    /// What the interloper does after the next `read`; taken when used.
    interloper: Mutex<Option<Interloper>>,
    /// Holds this many reads at a barrier once they have read, so that they
    /// all see the same version before any of their callers writes. Set with
    /// `concurrency > 1` to make two batch entries race on one id.
    rendezvous: Mutex<Option<(Arc<Barrier>, usize)>>,
    /// What the primary declares as `bulk_write_concurrency`, and so how many
    /// batch entries the REST loop runs at once.
    concurrency: usize,
}

impl RacingPrimary {
    fn arm(&self, interloper: Interloper) {
        *self.interloper.lock().expect("interloper lock") = Some(interloper);
    }

    /// The next `readers` reads wait for one another after reading.
    fn hold_reads_together(&self, readers: usize) {
        *self.rendezvous.lock().expect("rendezvous lock") =
            Some((Arc::new(Barrier::new(readers)), readers));
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
        let seen = self.inner.read(tenant, resource_type, id).await?;
        let interloper = self.interloper.lock().expect("interloper lock").take();
        match (seen.as_ref(), interloper) {
            (Some(seen), Some(Interloper::Update(content))) => {
                self.inner
                    .update(tenant, seen, content)
                    .await
                    .expect("the interloper's update lands");
            }
            (Some(_), Some(Interloper::Delete)) => {
                self.inner
                    .delete(tenant, resource_type, id)
                    .await
                    .expect("the interloper's delete lands");
            }
            _ => {}
        }
        let barrier = {
            let mut rendezvous = self.rendezvous.lock().expect("rendezvous lock");
            match rendezvous.as_mut() {
                Some((barrier, left)) => {
                    let barrier = Arc::clone(barrier);
                    *left -= 1;
                    if *left == 0 {
                        *rendezvous = None;
                    }
                    Some(barrier)
                }
                None => None,
            }
        };
        if let Some(barrier) = barrier {
            barrier.wait().await;
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

    fn bulk_write_concurrency(&self) -> usize {
        self.concurrency
    }

    // A PUT onto a deleted id restores it through the guarded restore
    // (#2562); a wrapper delegates it like every other write.
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
    server_with_concurrency(1)
}

fn server_with_concurrency(concurrency: usize) -> (TestServer, Arc<RacingPrimary>) {
    let primary = Arc::new(RacingPrimary {
        inner: sqlite(),
        interloper: Mutex::new(None),
        rendezvous: Mutex::new(None),
        concurrency,
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

    primary.arm(Interloper::Update(patient("del", "Interloper")));
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

    primary.arm(Interloper::Update(patient("plain", "Interloper")));
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

    primary.arm(Interloper::Update(patient("put", "Interloper")));
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

    primary.arm(Interloper::Update(patient("patch", "Interloper")));
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

// =============================================================================
// #2527: the same races through a `batch` entry's `request.ifMatch`
// =============================================================================

/// Posts a one-or-more-entry `batch` Bundle and returns the response Bundle.
/// A failed entry never fails the batch, so the Bundle itself is always `200`.
async fn post_batch(server: &TestServer, entries: Vec<Value>) -> Value {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        server
            .post("/")
            .json(&json!({"resourceType": "Bundle", "type": "batch", "entry": entries})),
    )
    .await
    .expect("the batch finishes; a rendezvous that never fills would hang it");
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    response.json()
}

fn put_entry(id: &str, family: &str, if_match: Option<&str>) -> Value {
    let mut request = json!({"method": "PUT", "url": format!("Patient/{id}")});
    if let Some(if_match) = if_match {
        request["ifMatch"] = json!(if_match);
    }
    json!({"request": request, "resource": patient(id, family)})
}

fn delete_entry(id: &str, if_match: Option<&str>) -> Value {
    let mut request = json!({"method": "DELETE", "url": format!("Patient/{id}")});
    if let Some(if_match) = if_match {
        request["ifMatch"] = json!(if_match);
    }
    json!({"request": request})
}

fn entry_response(body: &Value, index: usize) -> &Value {
    &body["entry"][index]["response"]
}

fn entry_status(body: &Value, index: usize) -> &str {
    entry_response(body, index)["status"]
        .as_str()
        .expect("entry.response.status")
}

/// Whether version `version` of `Patient/{id}` was ever written, asked of the
/// SQLite backend itself: `RacingPrimary` serves no history, so `_history`
/// over the router is a `501`. The server stores under `for_testing`'s
/// default tenant.
async fn version_exists(primary: &RacingPrimary, id: &str, version: &str) -> bool {
    let tenant = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    match primary.inner.vread(&tenant, "Patient", id, version).await {
        Ok(found) => found.is_some(),
        // A delete's history entry reads back as `Gone`; it was written.
        Err(StorageError::Resource(ResourceError::Gone { .. })) => true,
        Err(e) => panic!("vread Patient/{id}/_history/{version}: {e}"),
    }
}

/// Exactly versions `1..=last` of `Patient/{id}` exist.
async fn assert_last_version(primary: &RacingPrimary, id: &str, last: u32) {
    for version in 1..=last {
        assert!(
            version_exists(primary, id, &version.to_string()).await,
            "version {version} exists"
        );
    }
    assert!(
        !version_exists(primary, id, &(last + 1).to_string()).await,
        "nothing was written after version {last}"
    );
}

/// Single-request control for the revive race: `PUT` with `If-Match: W/"1"`
/// on a resource another writer deletes after the check is `404`, and the
/// resource stays deleted (`update.rs`, PR #747 / #1421).
#[tokio::test]
async fn put_with_if_match_does_not_revive_a_resource_deleted_after_the_check() {
    let (server, primary) = server();
    seed(&server, "rev").await;

    primary.arm(Interloper::Delete);
    let response = server
        .put("/Patient/rev")
        .add_header("If-Match", "W/\"1\"")
        .json(&patient("rev", "Reviver"))
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
    assert_eq!(
        server.get("/Patient/rev").await.status_code(),
        StatusCode::GONE
    );
}

/// Batch twin of `delete_with_if_match_does_not_delete_a_version_written_after_the_check`.
/// Before #2527 the entry answered `204` and deleted version 2.
#[tokio::test]
async fn batch_delete_with_if_match_does_not_delete_a_version_written_after_the_check() {
    let (server, primary) = server();
    seed(&server, "bdel").await;

    primary.arm(Interloper::Update(patient("bdel", "Interloper")));
    let body = post_batch(&server, vec![delete_entry("bdel", Some("W/\"1\""))]).await;

    assert_eq!(entry_status(&body, 0), "409 Conflict", "{body}");
    assert_interloper_survives(&server, "bdel").await;

    // As for the single request: the client re-reads, sees version 2, and may
    // delete that.
    let retry = post_batch(&server, vec![delete_entry("bdel", Some("W/\"2\""))]).await;
    assert_eq!(entry_status(&retry, 0), "204 No Content", "{retry}");
    assert_eq!(
        server.get("/Patient/bdel").await.status_code(),
        StatusCode::GONE
    );
}

/// Batch twin of `put_with_if_match_does_not_overwrite_a_version_written_after_the_check`.
/// Before #2527 the entry answered `200` and overwrote version 2 with a
/// version 3.
#[tokio::test]
async fn batch_put_with_if_match_does_not_overwrite_a_version_written_after_the_check() {
    let (server, primary) = server();
    seed(&server, "bput").await;

    primary.arm(Interloper::Update(patient("bput", "Interloper")));
    let body = post_batch(
        &server,
        vec![put_entry("bput", "Overwriter", Some("W/\"1\""))],
    )
    .await;

    assert_eq!(entry_status(&body, 0), "409 Conflict", "{body}");
    assert_interloper_survives(&server, "bput").await;
    assert_last_version(&primary, "bput", 2).await;
}

/// Batch twin of `put_with_if_match_does_not_revive_a_resource_deleted_after_the_check`.
/// Before #2527 the entry answered `201` and brought the resource back as
/// version 3.
#[tokio::test]
async fn batch_put_with_if_match_does_not_revive_a_resource_deleted_after_the_check() {
    let (server, primary) = server();
    seed(&server, "brev").await;

    primary.arm(Interloper::Delete);
    let body = post_batch(&server, vec![put_entry("brev", "Reviver", Some("W/\"1\""))]).await;

    assert_eq!(entry_status(&body, 0), "404 Not Found", "{body}");
    assert_eq!(
        server.get("/Patient/brev").await.status_code(),
        StatusCode::GONE,
        "the resource stays deleted"
    );
    // Version 1, and the interloper's delete as version 2; nothing after.
    assert_last_version(&primary, "brev", 2).await;
}

/// Two entries in one Bundle, both `PUT Patient/{id}` with `ifMatch: W/"1"`,
/// run concurrently and both pass the gate against version 1. Exactly one may
/// write; the other has lost the race and answers `409`, as two concurrent
/// single requests do. Before #2527 both answered `200`, writing versions 2
/// and 3, so the first write was lost.
#[tokio::test]
async fn two_batch_puts_with_the_same_if_match_on_one_id_write_once() {
    let (server, primary) = server_with_concurrency(2);
    seed(&server, "twin").await;

    primary.hold_reads_together(2);
    let body = post_batch(
        &server,
        vec![
            put_entry("twin", "First", Some("W/\"1\"")),
            put_entry("twin", "Second", Some("W/\"1\"")),
        ],
    )
    .await;

    let statuses = [entry_status(&body, 0), entry_status(&body, 1)];
    let winners: Vec<usize> = (0..2).filter(|&i| statuses[i] == "200 OK").collect();
    assert_eq!(winners.len(), 1, "exactly one entry writes: {body}");
    let winner = winners[0];
    assert_eq!(
        statuses[1 - winner],
        "409 Conflict",
        "the other lost the race: {body}"
    );
    assert_eq!(entry_response(&body, winner)["etag"], "W/\"2\"");

    let read = server.get("/Patient/twin").await;
    assert_eq!(read.status_code(), StatusCode::OK);
    let current = read.json::<Value>();
    assert_eq!(current["meta"]["versionId"], "2");
    assert_eq!(
        current["name"][0]["family"],
        ["First", "Second"][winner],
        "the winner's content is current"
    );
    assert_last_version(&primary, "twin", 2).await;
}

/// The same two entries when the batch runs one entry at a time (SQLite's
/// bound of 1): the second is checked after the first wrote version 2, so it
/// fails at the gate with `412`, never reaching a write.
#[tokio::test]
async fn two_serial_batch_puts_with_the_same_if_match_on_one_id_write_once() {
    let (server, _primary) = server();
    seed(&server, "serial").await;

    let body = post_batch(
        &server,
        vec![
            put_entry("serial", "First", Some("W/\"1\"")),
            put_entry("serial", "Second", Some("W/\"1\"")),
        ],
    )
    .await;

    assert_eq!(entry_status(&body, 0), "200 OK", "{body}");
    assert_eq!(entry_status(&body, 1), "412 Precondition Failed", "{body}");
    let current = server.get("/Patient/serial").await.json::<Value>();
    assert_eq!(current["meta"]["versionId"], "2");
    assert_eq!(current["name"][0]["family"], "First");
}

/// Single-request control: `DELETE` with `If-Match: W/"1"` on a resource
/// another writer deleted after the check is `404` — the delete was pinned to
/// version 1, which is no longer live.
#[tokio::test]
async fn delete_with_if_match_on_a_resource_deleted_after_the_check_is_not_found() {
    let (server, primary) = server();
    seed(&server, "ddel").await;

    primary.arm(Interloper::Delete);
    let response = server
        .delete("/Patient/ddel")
        .add_header("If-Match", "W/\"1\"")
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        response.text()
    );
    assert_last_version(&primary, "ddel", 2).await;
}

/// Batch twin of the control above: the pinned delete finds no live version 1
/// and answers `404`, writing nothing after the interloper's delete.
#[tokio::test]
async fn batch_delete_with_if_match_on_a_resource_deleted_after_the_check_is_not_found() {
    let (server, primary) = server();
    seed(&server, "bddel").await;

    primary.arm(Interloper::Delete);
    let body = post_batch(&server, vec![delete_entry("bddel", Some("W/\"1\""))]).await;

    assert_eq!(entry_status(&body, 0), "404 Not Found", "{body}");
    assert_last_version(&primary, "bddel", 2).await;
}

/// A satisfied `ifMatch` with nobody racing still answers what it answered
/// before #2527: `200`, the versioned `location` (#2517), the new version's
/// `etag`.
#[tokio::test]
async fn batch_put_with_a_satisfied_if_match_still_updates() {
    let (server, _primary) = server();
    seed(&server, "ok").await;

    let body = post_batch(&server, vec![put_entry("ok", "Updated", Some("W/\"1\""))]).await;

    assert_eq!(entry_status(&body, 0), "200 OK", "{body}");
    assert_eq!(
        entry_response(&body, 0)["location"],
        "Patient/ok/_history/2"
    );
    assert_eq!(entry_response(&body, 0)["etag"], "W/\"2\"");
    let read = server.get("/Patient/ok").await.json::<Value>();
    assert_eq!(read["meta"]["versionId"], "2");
    assert_eq!(read["name"][0]["family"], "Updated");
}

/// Entries without `ifMatch` are untouched by #2527: last-writer-wins, and a
/// `PUT` on a deleted resource brings it back (`201`), as `create_or_update`
/// has always done.
#[tokio::test]
async fn batch_entries_without_if_match_are_unchanged() {
    let (server, _primary) = server();
    seed(&server, "free").await;

    let put = post_batch(&server, vec![put_entry("free", "Updated", None)]).await;
    assert_eq!(entry_status(&put, 0), "200 OK", "{put}");

    let delete = post_batch(&server, vec![delete_entry("free", None)]).await;
    assert_eq!(entry_status(&delete, 0), "204 No Content", "{delete}");

    let revive = post_batch(&server, vec![put_entry("free", "Revived", None)]).await;
    assert_eq!(entry_status(&revive, 0), "201 Created", "{revive}");
    let read = server.get("/Patient/free").await.json::<Value>();
    assert_eq!(read["name"][0]["family"], "Revived");
}
