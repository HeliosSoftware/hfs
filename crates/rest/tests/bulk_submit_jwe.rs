//! End-to-end tests for JWE `fileEncryptionKey` handling in the real
//! [`HttpSubmitInputFetcher`], over a live HTTP server.
//!
//! The unit tests in `bulk_submit_fetcher` and `jwe` cover the algorithms; these
//! cover the wiring — that a manifest and its NDJSON files are actually
//! decrypted on the way through `fetch_manifest` / `open_file_stream`.

use std::net::SocketAddr;
use std::sync::Arc;

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use axum::Router;
use axum::routing::get;
use base64::Engine;
use helios_persistence::core::SubmitInputFetcher;
use helios_rest::bulk_submit_fetcher::HttpSubmitInputFetcher;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

const B64URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

const NDJSON: &str = "{\"resourceType\":\"Patient\",\"id\":\"enc-1\"}\n\
                      {\"resourceType\":\"Patient\",\"id\":\"enc-2\"}\n";

/// Encrypts `plaintext` as a `dir` + A256GCM compact JWE under `key`.
fn seal(key: &[u8; 32], plaintext: &[u8]) -> String {
    let iv = [42u8; 12];
    let header = br#"{"alg":"dir","enc":"A256GCM"}"#;
    let header_b64 = B64URL.encode(header);
    let sealed = Aes256Gcm::new_from_slice(key)
        .unwrap()
        .encrypt(
            Nonce::<U12>::from_slice(&iv),
            Payload {
                msg: plaintext,
                aad: header_b64.as_bytes(),
            },
        )
        .unwrap();
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    format!(
        "{}..{}.{}.{}",
        header_b64,
        B64URL.encode(iv),
        B64URL.encode(ct),
        B64URL.encode(tag)
    )
}

/// A Bulk Export Manifest pointing at this server's single output file.
fn manifest_json(addr: &SocketAddr) -> String {
    json!({
        "transactionTime": "2026-01-01T00:00:00Z",
        "request": format!("http://{addr}/$export"),
        "requiresAccessToken": false,
        "output": [
            { "type": "Patient", "url": format!("http://{addr}/f1.ndjson") }
        ],
        "error": []
    })
    .to_string()
}

/// Binds an ephemeral port, lets `build` produce the (manifest, file) bodies for
/// that address, and serves them at `/manifest.json` and `/f1.ndjson`.
async fn serve(build: impl FnOnce(SocketAddr) -> (String, String)) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (manifest, file) = build(addr);
    let app = Router::new()
        .route("/manifest.json", get(move || async move { manifest }))
        .route("/f1.ndjson", get(move || async move { file }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn read_all(fetcher: &HttpSubmitInputFetcher, url: &str, key: Option<&Value>) -> String {
    let (mut reader, _len) = fetcher
        .open_file_stream(url, &[], false, &[], key)
        .await
        .expect("open file stream");
    let mut out = String::new();
    reader.read_to_string(&mut out).await.unwrap();
    out
}

fn fetcher() -> HttpSubmitInputFetcher {
    HttpSubmitInputFetcher::new(None, "system/*.rs".to_string())
}

#[tokio::test]
async fn encrypted_manifest_and_file_round_trip() {
    let key = [0x5cu8; 32];
    let addr = serve(|addr| {
        (
            seal(&key, manifest_json(&addr).as_bytes()),
            seal(&key, NDJSON.as_bytes()),
        )
    })
    .await;

    // The spec shape: a `fileEncryptionKey` part with `coding` and `value` parts.
    let enc_key = json!({
        "name": "fileEncryptionKey",
        "part": [
            {"name": "coding", "valueCoding": {
                "system": "http://hl7.org/fhir/uv/bulkdata/ValueSet/file-encryption-type",
                "code": "jwe"
            }},
            {"name": "value", "valueString": B64URL.encode(key)},
        ]
    });
    let fetcher = fetcher();

    let manifest = fetcher
        .fetch_manifest(
            &format!("http://{addr}/manifest.json"),
            &[],
            &[],
            Some(&enc_key),
        )
        .await
        .expect("encrypted manifest decrypts and parses");
    assert_eq!(manifest.output.len(), 1);

    let body = read_all(&fetcher, &manifest.output[0].url, Some(&enc_key)).await;
    assert_eq!(body, NDJSON);
}

#[tokio::test]
async fn plaintext_manifest_is_tolerated_but_plaintext_file_is_not() {
    let addr = serve(|addr| (manifest_json(&addr), NDJSON.to_string())).await;

    let enc_key = json!({"value": B64URL.encode([0x2du8; 32])});
    let fetcher = fetcher();

    // Manifests carry URLs rather than PHI, so a cleartext manifest is accepted.
    let manifest = fetcher
        .fetch_manifest(
            &format!("http://{addr}/manifest.json"),
            &[],
            &[],
            Some(&enc_key),
        )
        .await
        .expect("plaintext manifest is tolerated");
    assert_eq!(manifest.output.len(), 1);

    // A cleartext data file, however, is a provider conformance failure.
    let err = fetcher
        .open_file_stream(&manifest.output[0].url, &[], false, &[], Some(&enc_key))
        .await
        .err()
        .expect("plaintext file must be rejected");
    assert!(err.to_string().contains("is not a JWE"), "{err}");
}

#[tokio::test]
async fn unencrypted_submission_is_unaffected() {
    let addr = serve(|addr| (manifest_json(&addr), NDJSON.to_string())).await;

    let fetcher = fetcher();
    let manifest = fetcher
        .fetch_manifest(&format!("http://{addr}/manifest.json"), &[], &[], None)
        .await
        .unwrap();
    let body = read_all(&fetcher, &manifest.output[0].url, None).await;
    assert_eq!(body, NDJSON);
}

#[tokio::test]
async fn wrong_key_fails_the_authentication_tag() {
    let key = [0x77u8; 32];
    let addr = serve(|addr| (manifest_json(&addr), seal(&key, NDJSON.as_bytes()))).await;

    let wrong = json!({"value": B64URL.encode([0x88u8; 32])});
    let fetcher = fetcher();
    let manifest = fetcher
        .fetch_manifest(
            &format!("http://{addr}/manifest.json"),
            &[],
            &[],
            Some(&wrong),
        )
        .await
        .unwrap();
    let err = fetcher
        .open_file_stream(&manifest.output[0].url, &[], false, &[], Some(&wrong))
        .await
        .err()
        .expect("wrong key must fail");
    assert!(err.to_string().contains("decryption failed"), "{err}");
}

/// A misconfigured `fileEncryptionKey` fails before the file body is fetched.
#[tokio::test]
async fn unusable_key_material_is_rejected_without_a_fetch() {
    let enc_key = json!({"coding": {"code": "pgp"}, "value": "AAAA"});
    let err = fetcher()
        .open_file_stream(
            "http://127.0.0.1:1/never.ndjson",
            &[],
            false,
            &[],
            Some(&enc_key),
        )
        .await
        .err()
        .expect("unknown encryption scheme must be rejected");
    assert!(err.to_string().contains("'pgp'"), "{err}");
}

/// The fetcher is shareable across the worker pool.
#[tokio::test]
async fn fetcher_is_usable_behind_arc_as_a_trait_object() {
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(fetcher());
    let err = fetcher
        .fetch_manifest("http://127.0.0.1:1/nope.json", &[], &[], None)
        .await
        .expect_err("connection refused");
    assert!(err.to_string().contains("manifest GET"), "{err}");
}
