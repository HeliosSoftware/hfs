//! The ceiling on the inflated size of a `zip: "DEF"` JWE, through the real
//! [`HttpSubmitInputFetcher`]: a manifest is held to the manifest ceiling and
//! a file to the encrypted-file ceiling. `jwe`'s unit tests cover the inflate
//! itself; these cover that each ceiling reaches it.

use std::io::Write;
use std::net::SocketAddr;

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use axum::Router;
use axum::routing::get;
use base64::Engine;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use helios_persistence::core::SubmitInputFetcher;
use helios_rest::bulk_submit_fetcher::HttpSubmitInputFetcher;
use helios_rest::bulk_submit_oauth::TrustedProviders;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

const B64URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;
const KEY: [u8; 32] = [0x5c; 32];
const IV: [u8; 12] = [42; 12];

fn deflate(plaintext: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::best());
    e.write_all(plaintext).unwrap();
    e.finish().unwrap()
}

/// (protected header b64, ciphertext, tag) for a `dir`/A256GCM/zip=DEF JWE.
fn seal_parts(plaintext: &[u8]) -> (String, Vec<u8>, Vec<u8>) {
    let header_b64 = B64URL.encode(br#"{"alg":"dir","enc":"A256GCM","zip":"DEF"}"#);
    let sealed = Aes256Gcm::new_from_slice(&KEY)
        .unwrap()
        .encrypt(
            Nonce::<U12>::from_slice(&IV),
            Payload {
                msg: &deflate(plaintext),
                aad: header_b64.as_bytes(),
            },
        )
        .unwrap();
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    (header_b64, ct.to_vec(), tag.to_vec())
}

fn seal_def(plaintext: &[u8]) -> String {
    let (h, ct, tag) = seal_parts(plaintext);
    format!(
        "{h}..{}.{}.{}",
        B64URL.encode(IV),
        B64URL.encode(ct),
        B64URL.encode(tag)
    )
}

async fn serve(manifest: String, file: String) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/manifest.json", get(move || async move { manifest }))
        .route("/f1.ndjson", get(move || async move { file }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

fn enc_key() -> Value {
    json!({"value": B64URL.encode(KEY)})
}

/// A fetcher with the given ceilings that trusts the test server, so that a
/// refusal reads with its detail rather than the fixed sentence.
fn fetcher(addr: &SocketAddr, manifest: u64, file: u64) -> HttpSubmitInputFetcher {
    HttpSubmitInputFetcher::new(None, "system/*.rs".to_string())
        .with_body_limits(manifest, file)
        .with_trusted_providers(
            TrustedProviders::parse([format!("http://{addr}")]).expect("test origin parses"),
        )
}

async fn open(f: &HttpSubmitInputFetcher, addr: SocketAddr) -> Result<usize, String> {
    match f
        .open_file_stream(
            &format!("http://{addr}/f1.ndjson"),
            &[],
            false,
            &[],
            Some(&enc_key()),
        )
        .await
    {
        Ok((mut reader, _)) => {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).await.unwrap();
            Ok(out.len())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// An 8 MiB file of one repeated byte: ~8 KiB of JWE on the wire.
#[tokio::test]
async fn file_bomb_is_refused_at_the_encrypted_file_ceiling() {
    let jwe = seal_def(&vec![b'a'; 8 * 1024 * 1024]);
    let addr = serve(String::new(), jwe).await;
    let got = open(&fetcher(&addr, 64 * 1024, 1024 * 1024), addr).await;
    let err = got.expect_err("an 8 MiB inflate must be refused at a 1 MiB ceiling");
    assert!(err.contains("ceiling"), "{err}");
}

/// A manifest that is valid JSON padded with 4 MiB of whitespace.
#[tokio::test]
async fn manifest_bomb_is_refused_at_the_manifest_ceiling() {
    let mut manifest = json!({
        "transactionTime": "2026-01-01T00:00:00Z",
        "request": "http://x/$export",
        "requiresAccessToken": false,
        "output": [],
        "error": []
    })
    .to_string();
    manifest.push_str(&" ".repeat(4 * 1024 * 1024));
    let jwe = seal_def(manifest.as_bytes());
    let addr = serve(jwe, String::new()).await;
    let got = fetcher(&addr, 64 * 1024, 1024 * 1024)
        .fetch_manifest(
            &format!("http://{addr}/manifest.json"),
            &[],
            &[],
            Some(&enc_key()),
        )
        .await
        .map(|m| m.output.len())
        .map_err(|e| e.to_string());
    let err = got.expect_err("a 4 MiB inflate must be refused at a 64 KiB ceiling");
    assert!(err.contains("ceiling"), "{err}");
}
