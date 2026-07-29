//! End-to-end tests for the per-user UI settings endpoints
//! (`GET`/`PUT`/`PATCH /_user/settings`).
//!
//! These exercise the full REST stack — routing, the `UserKey` extractor, the
//! handlers, and the SQLite-backed [`SettingsStore`] — against an in-memory
//! database, with no authentication configured (so the caller resolves to the
//! auth-disabled fallback key).

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::SettingsStore;
use helios_rest::ServerConfig;
use serde_json::{Value, json};

const IF_MATCH: HeaderName = HeaderName::from_static("if-match");
const IF_NONE_MATCH: HeaderName = HeaderName::from_static("if-none-match");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");

/// Builds a test server whose SQLite backend also hosts the settings store.
fn create_test_server() -> TestServer {
    let backend = SqliteBackend::in_memory().expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };

    let settings_store: Arc<dyn SettingsStore> = backend.clone();
    let state = helios_rest::AppState::new(backend, config).with_settings_store(settings_store);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("create test server")
}

/// Reads the `ETag` response header as an owned string.
fn etag(response: &axum_test::TestResponse) -> String {
    response
        .headers()
        .get("etag")
        .expect("ETag header present")
        .to_str()
        .expect("ETag is valid UTF-8")
        .to_string()
}

#[tokio::test]
async fn get_returns_empty_document_by_default() {
    let server = create_test_server();

    let response = server.get("/_user/settings").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.json::<Value>(), json!({}));
    assert_eq!(etag(&response), "\"0\"");
}

#[tokio::test]
async fn put_then_get_round_trips_document() {
    let server = create_test_server();
    let doc = json!({
        "theme": "dark",
        "defaultTenant": "acme",
        "activeFhirVersion": "R4",
        "recentQueries": {"Patient": ["name=smith"]}
    });

    let put = server.put("/_user/settings").json(&doc).await;
    assert_eq!(put.status_code(), StatusCode::OK);
    assert_eq!(etag(&put), "\"1\"");

    let get = server.get("/_user/settings").await;
    assert_eq!(get.status_code(), StatusCode::OK);
    assert_eq!(get.json::<Value>(), doc);
    assert_eq!(etag(&get), "\"1\"");
}

#[tokio::test]
async fn patch_merges_a_single_key_and_preserves_others() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({"theme": "dark", "defaultTenant": "acme"}))
        .await;

    // Toggle just the theme via JSON merge-patch.
    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "light"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);
    assert_eq!(etag(&patch), "\"2\"");

    let get = server.get("/_user/settings").await;
    assert_eq!(
        get.json::<Value>(),
        json!({"theme": "light", "defaultTenant": "acme"})
    );
}

#[tokio::test]
async fn patch_null_deletes_a_key() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({"theme": "dark", "defaultTenant": "acme"}))
        .await;

    server
        .patch("/_user/settings")
        .json(&json!({"defaultTenant": null}))
        .await;

    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));
}

#[tokio::test]
async fn stale_if_match_is_rejected_with_412() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await; // -> version 1

    // Precondition asserts "does not exist yet", but a document now exists.
    let conflict = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"0\""))
        .json(&json!({"a": 2}))
        .await;

    assert_eq!(conflict.status_code(), StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn matching_if_match_succeeds() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await; // -> version 1

    let ok = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"1\""))
        .json(&json!({"a": 2}))
        .await;

    assert_eq!(ok.status_code(), StatusCode::OK);
    assert_eq!(etag(&ok), "\"2\"");
}

#[tokio::test]
async fn non_object_body_is_rejected_with_400() {
    let server = create_test_server();

    let response = server
        .put("/_user/settings")
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .bytes(Bytes::from_static(b"[1, 2, 3]"))
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

/// The exact sequence the web UI's theme toggle performs (#197): a merge-patch
/// of `{"theme": ...}` as the *first* write (no prior PUT — the document is
/// created on the fly), a reload's GET seeing the roamed value, and the
/// toggle back — all without touching any other settings key.
#[tokio::test]
async fn theme_toggle_round_trips_like_the_web_ui() {
    let server = create_test_server();
    // A pre-existing unrelated key, as another device may have written one.
    server
        .put("/_user/settings")
        .json(&json!({"defaultTenant": "acme"}))
        .await;

    // Toggle to dark: what theme.js sends on click.
    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);

    // Next page load: GET reconciles the cache with the server value.
    let get = server.get("/_user/settings").await;
    assert_eq!(
        get.json::<Value>(),
        json!({"theme": "dark", "defaultTenant": "acme"})
    );

    // Toggle back to light.
    server
        .patch("/_user/settings")
        .json(&json!({"theme": "light"}))
        .await;
    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>()["theme"], json!("light"));
}

/// Same round-trip when no document exists at all yet: the very first
/// interaction a brand-new user has with the toggle must not 404/412.
#[tokio::test]
async fn theme_patch_creates_the_document_when_missing() {
    let server = create_test_server();

    let patch = server
        .patch("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    assert_eq!(patch.status_code(), StatusCode::OK);

    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));
}

/// The `savedQueries` convention (#234): lists are objects keyed by query id
/// precisely so one entry can be touched by merge-patch. Bumping
/// `lastAccessedAt`/`accessCount` on one query must not clobber its siblings
/// in the same resource type, other types, or unrelated settings keys.
#[tokio::test]
async fn saved_queries_single_entry_patch_preserves_siblings() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({
            "theme": "dark",
            "savedQueries": {
                "Patient": {
                    "q1": {"name": "Smiths", "query": "name=smith",
                           "createdAt": "2026-07-01T12:00:00Z"},
                    "q2": {"name": "Bostonians", "query": "address-city=Boston",
                           "createdAt": "2026-07-02T12:00:00Z"}
                },
                "Observation": {
                    "q3": {"name": "Vitals", "query": "category=vital-signs",
                           "createdAt": "2026-07-03T12:00:00Z"}
                }
            }
        }))
        .await;

    // What the UI sends when the user runs q1.
    let touch = server
        .patch("/_user/settings")
        .json(&json!({
            "savedQueries": {"Patient": {"q1": {
                "lastAccessedAt": "2026-07-09T09:14:22Z",
                "accessCount": 1
            }}}
        }))
        .await;
    assert_eq!(touch.status_code(), StatusCode::OK);

    let doc = server.get("/_user/settings").await.json::<Value>();
    assert_eq!(
        doc["savedQueries"]["Patient"]["q1"]["lastAccessedAt"],
        json!("2026-07-09T09:14:22Z")
    );
    assert_eq!(
        doc["savedQueries"]["Patient"]["q1"]["accessCount"],
        json!(1)
    );
    // Siblings and unrelated keys are untouched.
    assert_eq!(
        doc["savedQueries"]["Patient"]["q1"]["name"],
        json!("Smiths")
    );
    assert_eq!(
        doc["savedQueries"]["Patient"]["q2"]["name"],
        json!("Bostonians")
    );
    assert_eq!(
        doc["savedQueries"]["Observation"]["q3"]["name"],
        json!("Vitals")
    );
    assert_eq!(doc["theme"], json!("dark"));
}

/// A `null` merge-patch member deletes a single saved query.
#[tokio::test]
async fn saved_queries_null_patch_deletes_one_entry() {
    let server = create_test_server();
    server
        .put("/_user/settings")
        .json(&json!({
            "savedQueries": {"Patient": {
                "q1": {"name": "Smiths", "query": "name=smith"},
                "q2": {"name": "Bostonians", "query": "address-city=Boston"}
            }}
        }))
        .await;

    server
        .patch("/_user/settings")
        .json(&json!({"savedQueries": {"Patient": {"q1": null}}}))
        .await;

    let doc = server.get("/_user/settings").await.json::<Value>();
    assert!(doc["savedQueries"]["Patient"].get("q1").is_none());
    assert_eq!(
        doc["savedQueries"]["Patient"]["q2"]["name"],
        json!("Bostonians")
    );
}

/// Builds a `savedQueries.<type>` object with `n` entries.
fn saved_queries_entries(n: usize) -> Value {
    let entries: serde_json::Map<String, Value> = (0..n)
        .map(|i| {
            (
                format!("q{i}"),
                json!({"name": format!("query {i}"), "query": "name=x"}),
            )
        })
        .collect();
    Value::Object(entries)
}

#[tokio::test]
async fn saved_queries_per_type_cap_is_enforced_on_put_and_patch() {
    let server = create_test_server();

    // 100 entries is the documented cap: accepted.
    let at_cap = server
        .put("/_user/settings")
        .json(&json!({"savedQueries": {"Patient": saved_queries_entries(100)}}))
        .await;
    assert_eq!(at_cap.status_code(), StatusCode::OK);

    // A wholesale PUT over the cap is rejected.
    let over_cap = server
        .put("/_user/settings")
        .json(&json!({"savedQueries": {"Patient": saved_queries_entries(101)}}))
        .await;
    assert_eq!(over_cap.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    // A merge-patch adding the 101st entry is rejected too — the bound applies
    // to the post-merge document, not the patch body.
    let one_more = server
        .patch("/_user/settings")
        .json(&json!({"savedQueries": {"Patient": {"q100": {"name": "one too many", "query": "name=x"}}}}))
        .await;
    assert_eq!(one_more.status_code(), StatusCode::UNPROCESSABLE_ENTITY);

    // The stored document was not modified by the rejected writes.
    let doc = server.get("/_user/settings").await.json::<Value>();
    assert_eq!(
        doc["savedQueries"]["Patient"].as_object().unwrap().len(),
        100
    );
}

#[tokio::test]
async fn saved_queries_must_be_objects_not_arrays() {
    let server = create_test_server();

    // Arrays are exactly what the keyed-by-id convention exists to avoid.
    let top_level_array = server
        .put("/_user/settings")
        .json(&json!({"savedQueries": ["name=smith"]}))
        .await;
    assert_eq!(
        top_level_array.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let per_type_array = server
        .put("/_user/settings")
        .json(&json!({"savedQueries": {"Patient": ["name=smith"]}}))
        .await;
    assert_eq!(
        per_type_array.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn oversized_document_is_rejected_with_413() {
    let server = create_test_server();

    // A single ~300 KiB string value blows the 256 KiB document cap.
    let response = server
        .put("/_user/settings")
        .json(&json!({"blob": "x".repeat(300 * 1024)}))
        .await;
    assert_eq!(response.status_code(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn patch_with_stale_if_match_is_rejected_with_412() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await; // -> version 1
    server.put("/_user/settings").json(&json!({"a": 2})).await; // -> version 2

    let conflict = server
        .patch("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"1\""))
        .json(&json!({"a": 3}))
        .await;

    assert_eq!(conflict.status_code(), StatusCode::PRECONDITION_FAILED);
    // The stale write did not land.
    let doc = server.get("/_user/settings").await.json::<Value>();
    assert_eq!(doc["a"], json!(2));
}

#[tokio::test]
async fn if_none_match_returns_304_for_unchanged_document() {
    let server = create_test_server();
    let put = server
        .put("/_user/settings")
        .json(&json!({"theme": "dark"}))
        .await;
    let current = etag(&put);

    let not_modified = server
        .get("/_user/settings")
        .add_header(
            IF_NONE_MATCH,
            HeaderValue::from_str(&current).expect("valid header"),
        )
        .await;

    assert_eq!(not_modified.status_code(), StatusCode::NOT_MODIFIED);
}

/// A malformed `If-Match` must fail the precondition rather than be discarded.
///
/// Regression for #270: this previously parsed to "no precondition", silently
/// turning a request that asked to be conditional into an unconditional
/// last-writer-wins overwrite.
#[tokio::test]
async fn malformed_if_match_is_rejected_rather_than_ignored() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await;

    for raw in ["garbage", "\"\"", "not-a-tag"] {
        let response = server
            .put("/_user/settings")
            .add_header(IF_MATCH, HeaderValue::from_str(raw).expect("valid header"))
            .json(&json!({"a": 999}))
            .await;

        assert_eq!(
            response.status_code(),
            StatusCode::PRECONDITION_FAILED,
            "If-Match: {raw} should fail the precondition"
        );
    }

    // Crucially, none of those writes landed.
    let doc = server.get("/_user/settings").await.json::<Value>();
    assert_eq!(doc["a"], json!(1));
}

/// The same rule applies to `PATCH`, which is the more dangerous of the two:
/// a discarded precondition there also used to opt the request into the
/// conflict-absorbing retry loop.
#[tokio::test]
async fn malformed_if_match_is_rejected_on_patch() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await;

    let response = server
        .patch("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("garbage"))
        .json(&json!({"a": 999}))
        .await;

    assert_eq!(response.status_code(), StatusCode::PRECONDITION_FAILED);
    let doc = server.get("/_user/settings").await.json::<Value>();
    assert_eq!(doc["a"], json!(1));
}

/// `If-Match: *` asserts the document already exists (RFC 9110 §13.1.1); it is
/// not a synonym for "no precondition". Against a fresh store it must 412
/// rather than blind-create.
#[tokio::test]
async fn wildcard_if_match_requires_an_existing_document() {
    let server = create_test_server();

    let response = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("*"))
        .json(&json!({"a": 1}))
        .await;

    assert_eq!(response.status_code(), StatusCode::PRECONDITION_FAILED);
    // Nothing was created.
    assert_eq!(
        server.get("/_user/settings").await.json::<Value>(),
        json!({})
    );
}

/// Once a document exists, `*` is satisfied.
#[tokio::test]
async fn wildcard_if_match_succeeds_against_an_existing_document() {
    let server = create_test_server();
    server.put("/_user/settings").json(&json!({"a": 1})).await;

    let response = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("*"))
        .json(&json!({"a": 2}))
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        server.get("/_user/settings").await.json::<Value>()["a"],
        json!(2)
    );
}

/// `If-Match` requires strong comparison, so the endpoint emits a strong
/// validator — but the legacy weak form is still accepted from older clients.
#[tokio::test]
async fn emits_strong_etag_but_still_accepts_a_weak_if_match() {
    let server = create_test_server();
    let put = server.put("/_user/settings").json(&json!({"a": 1})).await;

    let tag = etag(&put);
    assert_eq!(tag, "\"1\"");
    assert!(
        !tag.starts_with("W/"),
        "expected a strong validator, got {tag}"
    );

    // A client built against an earlier release echoes the weak form back.
    let ok = server
        .put("/_user/settings")
        .add_header(IF_MATCH, HeaderValue::from_static("W/\"1\""))
        .json(&json!({"a": 2}))
        .await;
    assert_eq!(ok.status_code(), StatusCode::OK);
}

/// A document written under the pre-#270 key encoding is adopted on first
/// access and removed from its old location, so upgrading does not silently
/// orphan a user's saved theme and queries.
///
/// This runs with auth disabled, so the legacy key is the old `local|default`.
#[tokio::test]
async fn legacy_document_is_migrated_on_first_read() {
    let backend = SqliteBackend::in_memory().expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    // Seed a document exactly where the previous release would have written it.
    let store: Arc<dyn SettingsStore> = backend.clone();
    store
        .put_settings("local|default", json!({"theme": "dark"}), None)
        .await
        .expect("seed legacy document");

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(backend, config).with_settings_store(store.clone());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server");

    // The caller sees their settings under the new key …
    let get = server.get("/_user/settings").await;
    assert_eq!(get.status_code(), StatusCode::OK);
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));

    // … and the legacy document is gone, not left as a duplicate copy.
    assert!(
        store
            .get_settings("local|default")
            .await
            .expect("read legacy key")
            .is_none(),
        "the legacy document should have been removed after migration"
    );
}

/// Migration must not resurrect a legacy document over settings already written
/// under the new key.
#[tokio::test]
async fn migration_does_not_clobber_an_existing_current_document() {
    let backend = SqliteBackend::in_memory().expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);
    let store: Arc<dyn SettingsStore> = backend.clone();

    store
        .put_settings("local|default", json!({"theme": "stale"}), None)
        .await
        .expect("seed legacy document");
    store
        .put_settings("l2:", json!({"theme": "current"}), None)
        .await
        .expect("seed current document");

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(backend, config).with_settings_store(store.clone());
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server");

    let get = server.get("/_user/settings").await;
    assert_eq!(get.json::<Value>(), json!({"theme": "current"}));
}

// ── Tenant scoping (issue #313) ─────────────────────────────────────────────
//
// The wire format is unchanged — a client still sends and receives a flat
// document — but PHI-bearing keys are stored under `byTenant.{tenant}` so a
// tenant purge can reach them. These exercise that through the full stack, with
// the tenant selected by `X-Tenant-ID` (the default `header_only` routing mode).

const TENANT: HeaderName = HeaderName::from_static("x-tenant-id");

/// Builds a test server, returning the backend handle too so a test can inspect
/// the *stored* document and drive `purge_tenant_data`.
fn server_with_backend() -> (TestServer, Arc<SqliteBackend>) {
    let backend = SqliteBackend::in_memory().expect("create in-memory SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        ..ServerConfig::for_testing()
    };
    let settings_store: Arc<dyn SettingsStore> = backend.clone();
    let state =
        helios_rest::AppState::new(backend.clone(), config).with_settings_store(settings_store);
    let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
        .expect("create test server");
    (server, backend)
}

/// A saved query written under one tenant is invisible under another, and each
/// tenant's user-global preferences still roam.
#[tokio::test]
async fn saved_queries_are_scoped_per_tenant_but_theme_roams() {
    let (server, _backend) = server_with_backend();

    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(
            &json!({"theme": "dark", "savedQueries": {"Patient": {"q1": {"query": "name=smith"}}}}),
        )
        .await;

    // Same user, different tenant: the global preference roams, the query does not.
    let beta = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .await;
    assert_eq!(beta.status_code(), StatusCode::OK);
    assert_eq!(beta.json::<Value>(), json!({"theme": "dark"}));

    // Back in acme it is still there.
    let acme = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(
        acme.json::<Value>()["savedQueries"]["Patient"]["q1"]["query"],
        "name=smith"
    );
}

/// The storage layout must never appear on the wire.
#[tokio::test]
async fn the_by_tenant_key_is_never_returned_to_a_client() {
    let (server, _backend) = server_with_backend();
    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"savedQueries": {"Patient": {"q1": {}}}}))
        .await;

    let get = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert!(get.json::<Value>().get("byTenant").is_none());
}

/// …and a client cannot set it either, which would otherwise let a caller in one
/// tenant plant content under another's subtree.
#[tokio::test]
async fn setting_the_reserved_key_is_rejected() {
    let (server, _backend) = server_with_backend();

    for method in ["put", "patch"] {
        let body = json!({"byTenant": {"victim": {"savedQueries": {"Patient": {"x": {}}}}}});
        let response = if method == "put" {
            server
                .put("/_user/settings")
                .add_header(TENANT, HeaderValue::from_static("attacker"))
                .json(&body)
                .await
        } else {
            server
                .patch("/_user/settings")
                .add_header(TENANT, HeaderValue::from_static("attacker"))
                .json(&body)
                .await
        };
        assert_eq!(
            response.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{method} of the reserved key must be rejected"
        );
    }

    // Nothing landed under the victim tenant.
    let victim = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("victim"))
        .await;
    assert_eq!(victim.json::<Value>(), json!({}));
}

/// A `PUT` replaces what the caller can see. Another tenant's saved queries were
/// never in that view, so they must survive it.
#[tokio::test]
async fn put_in_one_tenant_does_not_erase_another_tenants_queries() {
    let (server, _backend) = server_with_backend();

    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"savedQueries": {"Patient": {"a": {"query": "name=smith"}}}}))
        .await;
    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .json(&json!({"savedQueries": {"Patient": {"b": {"query": "name=jones"}}}}))
        .await;

    let acme = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(
        acme.json::<Value>()["savedQueries"]["Patient"]["a"]["query"],
        "name=smith"
    );
    assert!(
        acme.json::<Value>()["savedQueries"]["Patient"]
            .get("b")
            .is_none()
    );
}

/// Merge-patch semantics survive the rewrite: a `null` still deletes exactly one
/// sibling entry, in this tenant only.
#[tokio::test]
async fn patch_null_deletes_one_scoped_entry_only() {
    let (server, _backend) = server_with_backend();

    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"savedQueries": {"Patient": {"keep": {}, "drop": {}}}}))
        .await;
    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .json(&json!({"savedQueries": {"Patient": {"drop": {}}}}))
        .await;

    server
        .patch("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"savedQueries": {"Patient": {"drop": null}}}))
        .await;

    let acme = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(
        acme.json::<Value>()["savedQueries"]["Patient"],
        json!({"keep": {}})
    );
    // beta's identically-named entry is untouched.
    let beta = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .await;
    assert_eq!(
        beta.json::<Value>()["savedQueries"]["Patient"],
        json!({"drop": {}})
    );
}

/// The whole point: offboarding a tenant now reaches the PHI-derived query
/// strings in every user's settings document, while leaving the other tenants'
/// and the user's own preferences alone.
#[tokio::test]
async fn purging_a_tenant_erases_its_saved_queries_from_user_settings() {
    use helios_persistence::core::ResourceStorage;

    let (server, backend) = server_with_backend();

    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({
            "theme": "dark",
            "savedQueries": {"Patient": {"q": {"query": "name=smith&birthdate=1970-01-01"}}}
        }))
        .await;
    server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .json(&json!({"savedQueries": {"Patient": {"q": {"query": "name=jones"}}}}))
        .await;

    backend.purge_tenant_data("acme").await.expect("purge acme");

    // acme's PHI is gone …
    let acme = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(acme.json::<Value>(), json!({"theme": "dark"}));

    // … from the stored document, not merely hidden by the projection …
    let stored = SettingsStore::get_settings(backend.as_ref(), "l2:")
        .await
        .expect("read stored document")
        .expect("document exists");
    assert!(
        !serde_json::to_string(&stored.document)
            .unwrap()
            .contains("smith"),
        "purged content must not survive anywhere in the stored document"
    );

    // … while beta and the user's theme are untouched.
    let beta = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .await;
    assert_eq!(
        beta.json::<Value>()["savedQueries"]["Patient"]["q"]["query"],
        "name=jones"
    );
    assert_eq!(beta.json::<Value>()["theme"], "dark");
}

/// A document written before scoping existed keeps working: its queries are
/// still readable, and the first write files them under a tenant so a later
/// purge can reach them.
#[tokio::test]
async fn a_pre_scoping_document_is_readable_then_normalized_on_write() {
    let (server, backend) = server_with_backend();

    // Seed exactly what a previous release would have stored: flat, no `byTenant`.
    SettingsStore::put_settings(
        backend.as_ref(),
        "l2:",
        json!({"theme": "dark", "savedQueries": {"Patient": {"q": {"query": "name=smith"}}}}),
        None,
    )
    .await
    .expect("seed pre-scoping document");

    // Unattributed, so it still reads as user-global — nobody's queries vanish
    // on upgrade.
    let get = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(
        get.json::<Value>()["savedQueries"]["Patient"]["q"]["query"],
        "name=smith"
    );

    // A read must not rewrite it: GET stays side-effect-free.
    let stored = SettingsStore::get_settings(backend.as_ref(), "l2:")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.document.get("byTenant").is_none());
    assert_eq!(stored.version, 1, "GET must not bump the version");

    // The first write files it under the writing tenant …
    server
        .patch("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"theme": "light"}))
        .await;
    let stored = SettingsStore::get_settings(backend.as_ref(), "l2:")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.document["byTenant"]["acme"]["savedQueries"]["Patient"]["q"]["query"],
        "name=smith"
    );
    assert!(stored.document.get("savedQueries").is_none());
}

/// A pre-scoping document that recorded a tenant choice is attributed to *that*
/// tenant, not to whichever tenant happens to write next.
#[tokio::test]
async fn a_pre_scoping_document_is_attributed_by_its_recorded_tenant() {
    let (server, backend) = server_with_backend();

    SettingsStore::put_settings(
        backend.as_ref(),
        "l2:",
        json!({"tenantId": "acme", "savedQueries": {"Patient": {"q": {"query": "name=smith"}}}}),
        None,
    )
    .await
    .expect("seed pre-scoping document");

    // Writing from beta must not re-file acme's queries under beta.
    server
        .patch("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("beta"))
        .json(&json!({"theme": "dark"}))
        .await;

    let stored = SettingsStore::get_settings(backend.as_ref(), "l2:")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.document["byTenant"]["acme"]["savedQueries"]["Patient"]["q"]["query"],
        "name=smith"
    );
    assert!(stored.document["byTenant"].get("beta").is_none());
}

/// A pre-scoping document that no write ever normalized is still reachable by a
/// purge — the dormant-user residual that would otherwise keep the erasure gap
/// open. Unattributed content is swept by any tenant's purge, deliberately.
#[tokio::test]
async fn a_dormant_pre_scoping_document_is_still_purgeable() {
    use helios_persistence::core::ResourceStorage;

    let (server, backend) = server_with_backend();

    SettingsStore::put_settings(
        backend.as_ref(),
        "l2:",
        json!({"theme": "dark", "savedQueries": {"Patient": {"q": {"query": "name=smith"}}}}),
        None,
    )
    .await
    .expect("seed dormant document");

    backend.purge_tenant_data("acme").await.expect("purge acme");

    let stored = SettingsStore::get_settings(backend.as_ref(), "l2:")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.document, json!({"theme": "dark"}));

    let get = server
        .get("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .await;
    assert_eq!(get.json::<Value>(), json!({"theme": "dark"}));
}

/// The per-type saved-query cap applies within each tenant, so it cannot be
/// exceeded one tenant at a time.
#[tokio::test]
async fn the_saved_query_cap_applies_within_each_tenant() {
    let (server, _backend) = server_with_backend();

    let mut entries = serde_json::Map::new();
    for i in 0..=100 {
        entries.insert(format!("q{i}"), json!({}));
    }
    let response = server
        .put("/_user/settings")
        .add_header(TENANT, HeaderValue::from_static("acme"))
        .json(&json!({"savedQueries": {"Patient": entries}}))
        .await;
    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
}
