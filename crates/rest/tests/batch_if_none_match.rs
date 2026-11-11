//! `ifNoneMatch` on a Bundle `GET` entry (#869).
//!
//! `Bundle.entry.request.ifNoneMatch` is the Bundle spelling of the
//! `If-None-Match` header, and on a read it is the conditional-read validator
//! (FHIR R4 http.html#cread). The bundle entry parser filled
//! `BundleEntry.if_none_match` and nothing read it, so a `GET [type]/[id]`
//! entry whose tag matched the current version still answered `200` with the
//! whole resource where `GET [type]/[id]` answers `304`.
//!
//! These tests drive both Bundle arms over the wire, against a real backend:
//! the REST batch loop, and the transaction, whose instance reads run inside
//! the storage transaction (a search, vread or history `GET` is deferred to
//! the REST layer, which is why those are pinned here as unaffected).
//!
//! The reference behaviour is the direct read handler's, so each case checks
//! the entry against what `GET [type]/[id]` answers for the same tag.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const IF_NONE_MATCH: HeaderName = HeaderName::from_static("if-none-match");

async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
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
    let server = TestServer::new(app).expect("Failed to create test server");

    (server, backend)
}

fn test_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Seeds a Patient and returns its version id (`"1"` for a fresh create).
async fn seed_patient(backend: &SqliteBackend, id: &str, family: &str) -> String {
    let patient = json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"family": family}],
        "active": true
    });
    backend
        .create(&test_tenant(), "Patient", patient, FhirVersion::R4)
        .await
        .expect("Failed to seed patient")
        .version_id()
        .to_string()
}

/// Posts a Bundle and returns the raw response, so a test can assert on a
/// whole-bundle failure as well as on a per-entry outcome.
async fn post_bundle_raw(server: &TestServer, bundle: &Value) -> axum_test::TestResponse {
    server
        .post("/")
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(bundle)
        .await
}

async fn post_bundle(server: &TestServer, bundle: &Value) -> Value {
    let response = post_bundle_raw(server, bundle).await;
    response.assert_status_ok();
    response.json()
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": kind,
        "entry": entries,
    })
}

/// A `GET` entry, optionally carrying an `ifNoneMatch`.
fn get_entry(url: &str, if_none_match: Option<&str>) -> Value {
    let mut request = json!({ "method": "GET", "url": url });
    if let Some(tag) = if_none_match {
        request["ifNoneMatch"] = json!(tag);
    }
    json!({ "request": request })
}

fn put_entry(id: &str, family: &str) -> Value {
    json!({
        "request": { "method": "PUT", "url": format!("Patient/{id}") },
        "resource": {
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": family}],
            "active": true
        }
    })
}

fn entry_status(body: &Value, index: usize) -> String {
    body["entry"][index]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// What `GET /Patient/{id}` answers for `If-None-Match: {tag}`.
async fn direct_read_status(server: &TestServer, id: &str, tag: &str) -> StatusCode {
    server
        .get(&format!("/Patient/{id}"))
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .add_header(IF_NONE_MATCH, HeaderValue::from_str(tag).unwrap())
        .await
        .status_code()
}

/// Both Bundle arms: the REST batch loop and the storage transaction.
const BUNDLE_KINDS: [&str; 2] = ["batch", "transaction"];

/// Asserts entry `index` is a `304`: no resource, and no outcome either.
fn assert_not_modified(body: &Value, index: usize, context: &str) {
    assert_eq!(
        entry_status(body, index),
        "304 Not Modified",
        "{context}: {body}"
    );
    let entry = &body["entry"][index];
    assert!(
        entry.get("resource").is_none(),
        "{context}: a 304 entry carries no resource body: {entry}"
    );
    assert!(
        entry["response"].get("outcome").is_none(),
        "{context}: a 304 entry is not a failure: {entry}"
    );
}

/// Asserts entry `index` is a full `200` read of `family`.
fn assert_full_read(body: &Value, index: usize, family: &str, context: &str) {
    assert_eq!(entry_status(body, index), "200 OK", "{context}: {body}");
    assert_eq!(
        body["entry"][index]["resource"]["name"][0]["family"].as_str(),
        Some(family),
        "{context}: {body}"
    );
}

// =============================================================================
// A matching tag answers 304
// =============================================================================

#[tokio::test]
async fn a_weak_tag_matching_the_current_version_answers_304() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        let version = seed_patient(&backend, "p1", "Smith").await;
        let tag = format!("W/\"{version}\"");

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some(&tag))]),
        )
        .await;

        assert_not_modified(&body, 0, kind);
        // The entry agrees with the direct read for the same tag.
        assert_eq!(
            direct_read_status(&server, "p1", &tag).await,
            StatusCode::NOT_MODIFIED
        );
    }
}

/// `If-None-Match` compares weakly (RFC 9110 section 8.8.3.2), so the strong
/// spelling of the current version's tag matches too, as it does on a direct
/// read.
#[tokio::test]
async fn a_strong_tag_matching_the_current_version_answers_304() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        let version = seed_patient(&backend, "p1", "Smith").await;
        let tag = format!("\"{version}\"");

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some(&tag))]),
        )
        .await;

        assert_not_modified(&body, 0, kind);
        assert_eq!(
            direct_read_status(&server, "p1", &tag).await,
            StatusCode::NOT_MODIFIED
        );
    }
}

/// `*` matches any current representation, so a read of an existing resource
/// is `304`, exactly as `GET [type]/[id]` with `If-None-Match: *`.
#[tokio::test]
async fn star_answers_304_for_an_existing_resource() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some("*"))]),
        )
        .await;

        assert_not_modified(&body, 0, kind);
        assert_eq!(
            direct_read_status(&server, "p1", "*").await,
            StatusCode::NOT_MODIFIED
        );
    }
}

/// `If-None-Match` is a list; one member matching is enough.
#[tokio::test]
async fn a_list_containing_the_current_version_answers_304() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        let version = seed_patient(&backend, "p1", "Smith").await;
        let tags = format!("W/\"7\", W/\"{version}\", W/\"9\"");

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some(&tags))]),
        )
        .await;

        assert_not_modified(&body, 0, kind);
    }
}

// =============================================================================
// A tag that does not match answers 200 with the body
// =============================================================================

#[tokio::test]
async fn a_tag_naming_another_version_answers_200_with_the_body() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some("W/\"99\""))]),
        )
        .await;

        assert_full_read(&body, 0, "Smith", kind);
        assert_eq!(
            body["entry"][0]["response"]["etag"].as_str(),
            Some("W/\"1\""),
            "{kind}: the full read keeps its etag"
        );
        assert_eq!(
            direct_read_status(&server, "p1", "W/\"99\"").await,
            StatusCode::OK
        );
    }
}

/// A client holding version 1 of a resource that has since moved to version 2
/// gets the new body, not a `304`.
#[tokio::test]
async fn a_stale_tag_after_an_update_answers_the_new_version() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        let v1 = seed_patient(&backend, "p1", "Smith").await;
        let stored = backend
            .read(&test_tenant(), "Patient", "p1")
            .await
            .unwrap()
            .unwrap();
        backend
            .update(
                &test_tenant(),
                &stored,
                json!({
                    "resourceType": "Patient",
                    "id": "p1",
                    "name": [{"family": "Jones"}],
                    "active": true
                }),
            )
            .await
            .expect("Failed to update patient");

        let stale = format!("W/\"{v1}\"");
        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some(&stale))]),
        )
        .await;
        assert_full_read(&body, 0, "Jones", kind);
        assert_eq!(
            body["entry"][0]["response"]["etag"].as_str(),
            Some("W/\"2\""),
            "{kind}"
        );

        let current = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some("W/\"2\""))]),
        )
        .await;
        assert_not_modified(&current, 0, kind);
    }
}

/// A value that is not a list of entity-tags cannot be satisfied; the direct
/// read treats it as "does not match" and returns the full representation,
/// and so does the entry.
#[tokio::test]
async fn an_unparseable_tag_answers_200_with_the_body() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let body = post_bundle(
            &server,
            &bundle(kind, vec![get_entry("Patient/p1", Some("not-a-tag"))]),
        )
        .await;

        assert_full_read(&body, 0, "Smith", kind);
        assert_eq!(
            direct_read_status(&server, "p1", "not-a-tag").await,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn a_read_without_if_none_match_is_unchanged() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let body = post_bundle(&server, &bundle(kind, vec![get_entry("Patient/p1", None)])).await;

        assert_full_read(&body, 0, "Smith", kind);
    }
}

// =============================================================================
// A missing resource keeps its behaviour
// =============================================================================

/// `ifNoneMatch` names a version of a resource that is not there; the read is
/// the same failure it is without the tag. In a batch that is the entry's
/// `404`; in a transaction it fails the whole bundle, as it always has.
#[tokio::test]
async fn a_read_of_a_missing_resource_is_unchanged_by_if_none_match() {
    for kind in BUNDLE_KINDS {
        let (server, _backend) = create_test_server().await;

        let plain = post_bundle_raw(
            &server,
            &bundle(kind, vec![get_entry("Patient/ghost", None)]),
        )
        .await;
        for tag in ["W/\"1\"", "*"] {
            let tagged = post_bundle_raw(
                &server,
                &bundle(kind, vec![get_entry("Patient/ghost", Some(tag))]),
            )
            .await;

            assert_eq!(
                tagged.status_code(),
                plain.status_code(),
                "{kind} {tag}: bundle status"
            );
            if kind == "batch" {
                let body: Value = tagged.json();
                assert_eq!(entry_status(&body, 0), "404 Not Found", "{kind} {tag}");
                assert_eq!(
                    body["entry"][0]["response"]["outcome"]["issue"][0]["code"].as_str(),
                    Some("not-found"),
                    "{kind} {tag}"
                );
            } else {
                assert!(
                    !tagged.status_code().is_success(),
                    "{kind} {tag}: a missing read still fails the transaction"
                );
            }
        }
    }
}

// =============================================================================
// Scope: only an instance read honours it
// =============================================================================

/// A search entry is not a conditional read. The direct search handler does
/// not consult `If-None-Match`, and neither does the entry.
#[tokio::test]
async fn a_search_entry_ignores_if_none_match() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        for tag in ["*", "W/\"1\""] {
            let body = post_bundle(
                &server,
                &bundle(kind, vec![get_entry("Patient?family=Smith", Some(tag))]),
            )
            .await;

            assert_eq!(entry_status(&body, 0), "200 OK", "{kind} {tag}: {body}");
            assert_eq!(
                body["entry"][0]["resource"]["resourceType"].as_str(),
                Some("Bundle"),
                "{kind} {tag}"
            );
        }
    }
}

/// The direct vread handler does not consult `If-None-Match` (a version is
/// immutable, so the read is not conditional), and the entry matches it.
#[tokio::test]
async fn a_vread_entry_ignores_if_none_match() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;

        let body = post_bundle(
            &server,
            &bundle(
                kind,
                vec![get_entry("Patient/p1/_history/1", Some("W/\"1\""))],
            ),
        )
        .await;

        assert_full_read(&body, 0, "Smith", kind);
    }
}

// =============================================================================
// Order within a Bundle
// =============================================================================

/// A GET entry is answered after the writes of its Bundle (R4 3.1.0.11.2), so
/// its tag is compared with the version the Bundle's own `PUT` produced.
#[tokio::test]
async fn a_read_is_compared_with_the_version_a_sibling_write_produced() {
    let (server, backend) = create_test_server().await;
    seed_patient(&backend, "p1", "Smith").await;

    let body = post_bundle(
        &server,
        &bundle(
            "transaction",
            vec![
                // Listed before the write, executed after it.
                get_entry("Patient/p1", Some("W/\"2\"")),
                get_entry("Patient/p1", Some("W/\"1\"")),
                put_entry("p1", "Jones"),
            ],
        ),
    )
    .await;

    assert_not_modified(&body, 0, "tag of the new version");
    assert_full_read(&body, 1, "Jones", "tag of the replaced version");
    assert_eq!(entry_status(&body, 2), "200 OK");
}

/// One Bundle, several reads, each judged on its own tag.
#[tokio::test]
async fn each_read_in_a_bundle_is_judged_on_its_own_tag() {
    for kind in BUNDLE_KINDS {
        let (server, backend) = create_test_server().await;
        seed_patient(&backend, "p1", "Smith").await;
        seed_patient(&backend, "p2", "Jones").await;

        let body = post_bundle(
            &server,
            &bundle(
                kind,
                vec![
                    get_entry("Patient/p1", Some("W/\"1\"")),
                    get_entry("Patient/p2", Some("W/\"5\"")),
                    get_entry("Patient/p2", None),
                ],
            ),
        )
        .await;

        assert_not_modified(&body, 0, kind);
        assert_full_read(&body, 1, "Jones", kind);
        assert_full_read(&body, 2, "Jones", kind);
    }
}
