//! Integration tests for remote `Reference.resolve()` (trusted-server prefetch).
//!
//! These exercise the end-to-end remote path against a `wiremock` server:
//! references to an allowlisted server are fetched and folded into the resolution
//! pool before row generation. The mock listens on `127.0.0.1`; the SSRF guard
//! blocks loopback for *hostnames*, but a literal IP that is **explicitly
//! allowlisted** is permitted — which is the documented escape hatch and what we
//! configure here (the allowlist base is the mock's own `http://127.0.0.1:PORT`).

use std::io::Cursor;

use helios_sof::{
    ChunkConfig, ContentType, RemoteResolveConfig, RunOptions, SofBundle, SofViewDefinition,
    process_ndjson_chunked_remote, run_view_definition_with_options_remote,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bundle(resources: &[serde_json::Value]) -> SofBundle {
    let mut bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": []
    });
    let entries = bundle_json["entry"].as_array_mut().unwrap();
    for resource in resources {
        entries.push(serde_json::json!({ "resource": resource }));
    }
    SofBundle::R4(serde_json::from_value(bundle_json).expect("valid R4 bundle"))
}

fn view(view_json: serde_json::Value) -> SofViewDefinition {
    let mut v = view_json;
    let obj = v.as_object_mut().unwrap();
    obj.insert("resourceType".into(), "ViewDefinition".into());
    obj.insert("status".into(), "active".into());
    SofViewDefinition::R4(serde_json::from_value(v).expect("valid R4 ViewDefinition"))
}

fn config(base: &str, max_depth: usize, max_fetches: usize) -> RemoteResolveConfig {
    RemoteResolveConfig {
        enabled: true,
        allowed_base_urls: helios_sof::parse_allowed_base_urls(base),
        max_depth,
        max_fetches,
        ..Default::default()
    }
}

fn opts() -> RunOptions {
    RunOptions {
        since: None,
        limit: None,
        page: None,
        parquet_options: None,
    }
}

async fn run(
    v: SofViewDefinition,
    b: SofBundle,
    cfg: &RemoteResolveConfig,
) -> Vec<serde_json::Value> {
    let bytes = run_view_definition_with_options_remote(v, b, ContentType::Json, opts(), cfg)
        .await
        .expect("run remote");
    serde_json::from_slice(&bytes).expect("json rows")
}

fn encounter_referencing(id: &str, subject_ref: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Encounter",
        "id": id,
        "status": "finished",
        "subject": { "reference": subject_ref }
    })
}

fn subject_resolve_view() -> serde_json::Value {
    serde_json::json!({
        "resource": "Encounter",
        "select": [
            { "column": [{ "name": "encounter_id", "path": "id" }] },
            {
                "forEach": "subject.resolve()",
                "column": [
                    { "name": "patient_id", "path": "id" },
                    { "name": "patient_family", "path": "name.family" }
                ]
            }
        ]
    })
}

/// A reference to an allowlisted server is fetched and resolved end-to-end.
#[tokio::test]
async fn remote_resolve_fetches_from_trusted_server() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Patient/pat-remote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Patient",
            "id": "pat-remote",
            "name": [{ "family": "Remote" }]
        })))
        .mount(&server)
        .await;

    let base = server.uri();
    let reference = format!("{base}/Patient/pat-remote");
    let b = bundle(&[encounter_referencing("enc-1", &reference)]);
    let cfg = config(&base, 1, 256);

    let rows = run(view(subject_resolve_view()), b, &cfg).await;
    assert_eq!(rows.len(), 1, "rows: {rows:#?}");
    assert_eq!(rows[0]["patient_id"], serde_json::json!("pat-remote"));
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Remote"));
}

/// A reference whose host is NOT allowlisted is never fetched; the column is null.
#[tokio::test]
async fn remote_resolve_skips_non_allowlisted_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Patient/pat-remote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Patient", "id": "pat-remote", "name": [{ "family": "Remote" }]
        })))
        .mount(&server)
        .await;

    let reference = format!("{}/Patient/pat-remote", server.uri());
    // Allowlist a *different* server, so the mock reference must not be fetched.
    let cfg = config("https://trusted.example.org/fhir", 1, 256);
    let b = bundle(&[encounter_referencing("enc-1", &reference)]);

    let rows = run(view(subject_resolve_view()), b, &cfg).await;
    assert_eq!(rows.len(), 1, "rows: {rows:#?}");
    assert!(
        rows[0]["patient_family"].is_null(),
        "non-allowlisted reference must not resolve: {:#?}",
        rows[0]
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "mock server must receive no requests for a non-allowlisted host"
    );
}

/// A non-success response (404) is non-fatal: the column is null, no error.
#[tokio::test]
async fn remote_resolve_404_is_non_fatal() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Patient/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let base = server.uri();
    let reference = format!("{base}/Patient/missing");
    let cfg = config(&base, 1, 256);
    let b = bundle(&[encounter_referencing("enc-1", &reference)]);

    let rows = run(view(subject_resolve_view()), b, &cfg).await;
    assert_eq!(rows.len(), 1, "rows: {rows:#?}");
    assert!(rows[0]["patient_family"].is_null());
}

/// The fetch cap bounds the number of remote requests in a run.
#[tokio::test]
async fn remote_resolve_respects_fetch_cap() {
    let server = MockServer::start().await;
    for (id, family) in [("p1", "One"), ("p2", "Two")] {
        Mock::given(method("GET"))
            .and(path(format!("/Patient/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Patient", "id": id, "name": [{ "family": family }]
            })))
            .mount(&server)
            .await;
    }

    let base = server.uri();
    let b = bundle(&[
        encounter_referencing("enc-1", &format!("{base}/Patient/p1")),
        encounter_referencing("enc-2", &format!("{base}/Patient/p2")),
    ]);
    let cfg = config(&base, 1, 1); // cap = 1

    let _ = run(view(subject_resolve_view()), b, &cfg).await;
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "fetch cap of 1 must allow exactly one remote request"
    );
}

fn chained_view() -> serde_json::Value {
    serde_json::json!({
        "resource": "Encounter",
        "select": [{
            "forEach": "subject.resolve()",
            "column": [
                { "name": "patient_family", "path": "name.family" },
                { "name": "gp_family", "path": "generalPractitioner.resolve().name.family" }
            ]
        }]
    })
}

async fn mount_chained(server: &MockServer, base: &str) {
    Mock::given(method("GET"))
        .and(path("/Patient/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{ "family": "Root" }],
            "generalPractitioner": [{ "reference": format!("{base}/Practitioner/pr1") }]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Practitioner/pr1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Practitioner", "id": "pr1", "name": [{ "family": "Doc" }]
        })))
        .mount(server)
        .await;
}

/// With `max_depth = 1`, only the first-level reference is fetched; a chained
/// reference discovered inside the fetched resource is not.
#[tokio::test]
async fn remote_resolve_depth_one_does_not_follow_chains() {
    let server = MockServer::start().await;
    let base = server.uri();
    mount_chained(&server, &base).await;

    let b = bundle(&[encounter_referencing(
        "enc-1",
        &format!("{base}/Patient/p1"),
    )]);
    let cfg = config(&base, 1, 256);

    let rows = run(view(chained_view()), b, &cfg).await;
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Root"));
    assert!(
        rows[0]["gp_family"].is_null(),
        "depth=1 must not fetch the second-level Practitioner: {:#?}",
        rows[0]
    );
}

/// With `max_depth = 2`, the chained reference is fetched in the second round.
#[tokio::test]
async fn remote_resolve_depth_two_follows_one_chain() {
    let server = MockServer::start().await;
    let base = server.uri();
    mount_chained(&server, &base).await;

    let b = bundle(&[encounter_referencing(
        "enc-1",
        &format!("{base}/Patient/p1"),
    )]);
    let cfg = config(&base, 2, 256);

    let rows = run(view(chained_view()), b, &cfg).await;
    assert_eq!(rows[0]["patient_family"], serde_json::json!("Root"));
    assert_eq!(rows[0]["gp_family"], serde_json::json!("Doc"));
}

/// An allowlisted *hostname* that resolves to a loopback address is blocked even
/// when `allow_private_addresses` is enabled — the always-blocked tier (loopback,
/// link-local/metadata) is never bypassed by the private-address opt-in. (We use
/// `localhost`, which resolves to loopback; a real internal LB would resolve to an
/// RFC1918 address, which the opt-in *does* permit — covered by the unit tests.)
#[tokio::test]
async fn remote_resolve_allow_private_still_blocks_loopback_hostname() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Patient/pat-remote"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Patient", "id": "pat-remote", "name": [{ "family": "Remote" }]
        })))
        .mount(&server)
        .await;

    let port = server.address().port();
    let base = format!("http://localhost:{port}");
    let reference = format!("{base}/Patient/pat-remote");
    let cfg = RemoteResolveConfig {
        enabled: true,
        allowed_base_urls: helios_sof::parse_allowed_base_urls(&base),
        allow_private_addresses: true, // even so, loopback stays blocked
        ..Default::default()
    };
    let b = bundle(&[encounter_referencing("enc-1", &reference)]);

    let rows = run(view(subject_resolve_view()), b, &cfg).await;
    assert!(
        rows[0]["patient_family"].is_null(),
        "loopback hostname must be blocked even with allow_private: {:#?}",
        rows[0]
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no request must reach a loopback host"
    );
}

// ---- Streaming / NDJSON remote resolution -------------------------------------

fn ndjson(resources: &[serde_json::Value]) -> String {
    resources
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_stream(
    v: SofViewDefinition,
    input: String,
    cfg: &RemoteResolveConfig,
    chunk_size: usize,
) -> Vec<serde_json::Value> {
    let mut out: Vec<u8> = Vec::new();
    let chunk_config = ChunkConfig {
        chunk_size,
        skip_invalid_lines: false,
    };
    process_ndjson_chunked_remote(
        v,
        Cursor::new(input.into_bytes()),
        &mut out,
        ContentType::Json,
        chunk_config,
        cfg,
    )
    .await
    .expect("stream remote");
    serde_json::from_slice(&out).expect("json rows")
}

/// A reference shared by Encounters in different chunks is fetched once (the
/// cross-chunk LRU cache), and resolves in every chunk. `chunk_size = 1` forces
/// one resource per chunk, so the two Encounters land in separate chunks.
#[tokio::test]
async fn remote_resolve_streaming_caches_shared_reference_across_chunks() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Patient/shared"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resourceType": "Patient", "id": "shared", "name": [{ "family": "Shared" }]
        })))
        .mount(&server)
        .await;

    let base = server.uri();
    let reference = format!("{base}/Patient/shared");
    let input = ndjson(&[
        encounter_referencing("e1", &reference),
        encounter_referencing("e2", &reference),
    ]);
    let cfg = config(&base, 1, 256);

    let rows = run_stream(view(subject_resolve_view()), input, &cfg, 1).await;
    assert_eq!(rows.len(), 2, "expected one row per Encounter: {rows:#?}");
    for row in &rows {
        assert_eq!(row["patient_family"], serde_json::json!("Shared"));
    }
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "a reference shared across chunks must be fetched only once"
    );
}

/// `SOF_RESOLVE_MAX_FETCHES` is a per-stream cap: across chunks, only the first
/// distinct reference is fetched when the cap is 1.
#[tokio::test]
async fn remote_resolve_streaming_fetch_cap_is_per_stream() {
    let server = MockServer::start().await;
    for (id, family) in [("p1", "One"), ("p2", "Two")] {
        Mock::given(method("GET"))
            .and(path(format!("/Patient/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resourceType": "Patient", "id": id, "name": [{ "family": family }]
            })))
            .mount(&server)
            .await;
    }

    let base = server.uri();
    let input = ndjson(&[
        encounter_referencing("e1", &format!("{base}/Patient/p1")),
        encounter_referencing("e2", &format!("{base}/Patient/p2")),
    ]);
    let cfg = config(&base, 1, 1); // per-stream cap of 1

    let rows = run_stream(view(subject_resolve_view()), input, &cfg, 1).await;
    assert_eq!(rows.len(), 2, "{rows:#?}");
    // Chunks are processed in input order: e1's reference is fetched, e2's is capped.
    assert_eq!(rows[0]["patient_family"], serde_json::json!("One"));
    assert!(
        rows[1]["patient_family"].is_null(),
        "second distinct reference must be skipped by the per-stream cap: {:#?}",
        rows[1]
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "per-stream cap of 1 must allow exactly one fetch across all chunks"
    );
}
