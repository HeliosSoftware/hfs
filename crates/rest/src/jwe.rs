//! JSON Web Encryption (JWE) decryption — RFC 7516 / RFC 7518.
//!
//! Used by Bulk Data **Submit** to decrypt the manifest and NDJSON files a Data
//! Provider encrypted with the `fileEncryptionKey` supplied on `$bulk-submit`
//! (see the [submit spec](https://build.fhir.org/ig/HL7/bulk-data/en/submit.html)
//! — "the Data Provider SHALL use the key in `fileEncryptionKey.value` to encrypt
//! the manifest and each file listed in the manifest's `output` section").
//!
//! Everything here is pure Rust (RustCrypto) — no OpenSSL — and is compiled
//! unconditionally, so encrypted submissions work in a stock build.
//!
//! # Supported algorithms
//!
//! Serializations: compact (`hdr.ek.iv.ct.tag`), flattened JSON, and general
//! JSON (each recipient is tried in turn).
//!
//! | `alg` (key management) | Key source |
//! |---|---|
//! | `dir` | the shared key itself is the CEK |
//! | `A128KW`, `A192KW`, `A256KW` | AES key wrap (RFC 3394) under the shared key |
//! | `A128GCMKW`, `A192GCMKW`, `A256GCMKW` | AES-GCM key wrap under the shared key |
//! | `ECDH-ES`, `ECDH-ES+A128KW`, `+A192KW`, `+A256KW` | a configured P-256/P-384 private key |
//!
//! | `enc` (content encryption) |
//! |---|
//! | `A128GCM`, `A192GCM`, `A256GCM` |
//! | `A128CBC-HS256`, `A192CBC-HS384`, `A256CBC-HS512` |
//!
//! `zip: "DEF"` payload compression is decompressed after decryption.
//!
//! # Deliberately unsupported
//!
//! - `RSA-OAEP` / `RSA-OAEP-256`. The only pure-Rust implementation is the `rsa`
//!   crate, which carries [RUSTSEC-2023-0071] (Marvin Attack — key recovery
//!   through a decryption timing sidechannel) with no fixed release. Rather than
//!   take a knowingly vulnerable RSA implementation into a server that handles
//!   PHI, the RSA arms are rejected; use the `ECDH-ES` family to deliver a
//!   content-encryption key asymmetrically.
//! - `RSA1_5`. RFC 8017 §7.2 padding-oracle exposure; deprecated for JOSE by
//!   RFC 8725.
//! - `PBES2-*`. Password-based — the submit flow has no shared password.
//!
//! [RUSTSEC-2023-0071]: https://rustsec.org/advisories/RUSTSEC-2023-0071

use std::io::Read;

use base64::Engine;
use serde_json::{Map, Value};

type B64 = base64::engine::general_purpose::GeneralPurpose;
const B64URL: B64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Errors surfaced by JWE parsing and decryption.
///
/// Rendered into the manifest-level error recorded against a submission, so the
/// text names the offending algorithm/segment rather than just "failed".
pub type JweError = String;

type Result<T> = std::result::Result<T, JweError>;

/// Why the RSA key-management arms are absent — reused by the `alg` rejection
/// and by private-key loading so both point at the same reason.
const RSA_UNSUPPORTED: &str = "RSA key management (RSA-OAEP / RSA-OAEP-256 / RSA1_5) is not \
     supported: the only pure-Rust RSA implementation carries RUSTSEC-2023-0071 \
     (Marvin Attack timing sidechannel) with no fixed release. Use the ECDH-ES \
     family with a P-256 or P-384 key instead.";

fn b64(part: &str) -> Result<Vec<u8>> {
    // Tolerate padded input: some producers emit standard base64url with `=`.
    let trimmed = part.trim_end_matches('=');
    B64URL
        .decode(trimmed)
        .map_err(|e| format!("base64url decode failed: {e}"))
}

// ---------------------------------------------------------------------------
// Key material
// ---------------------------------------------------------------------------

/// A private key usable for asymmetric JWE key management.
#[derive(Clone)]
pub enum PrivateKey {
    /// NIST P-256 key for the `ECDH-ES` family.
    P256 {
        /// JWK `kid`, when the key was supplied as a JWK.
        kid: Option<String>,
        /// The decoded private key.
        key: p256::SecretKey,
    },
    /// NIST P-384 key for the `ECDH-ES` family.
    P384 {
        /// JWK `kid`, when the key was supplied as a JWK.
        kid: Option<String>,
        /// The decoded private key.
        key: p384::SecretKey,
    },
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render key material.
        let (kind, kid) = match self {
            PrivateKey::P256 { kid, .. } => ("P-256", kid),
            PrivateKey::P384 { kid, .. } => ("P-384", kid),
        };
        write!(f, "PrivateKey({kind}, kid={kid:?})")
    }
}

impl PrivateKey {
    fn kid(&self) -> Option<&str> {
        match self {
            PrivateKey::P256 { kid, .. } | PrivateKey::P384 { kid, .. } => kid.as_deref(),
        }
    }
}

/// Everything available to unlock a JWE: an optional shared symmetric key (the
/// `fileEncryptionKey` content-encryption key) plus any locally configured
/// private keys.
#[derive(Clone, Default)]
pub struct DecryptionKeys {
    /// Symmetric key material — the CEK for `dir`, or the KEK for `A*KW`.
    pub shared: Option<Vec<u8>>,
    /// Private keys for `ECDH-ES*` recipients.
    pub private: Vec<PrivateKey>,
}

impl std::fmt::Debug for DecryptionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the symmetric key, only whether one is present.
        f.debug_struct("DecryptionKeys")
            .field(
                "shared",
                &self.shared.as_ref().map(|k| format!("<{} bytes>", k.len())),
            )
            .field("private", &self.private)
            .finish()
    }
}

impl DecryptionKeys {
    /// Keys consisting solely of a shared symmetric key.
    pub fn shared(key: Vec<u8>) -> Self {
        Self {
            shared: Some(key),
            private: Vec::new(),
        }
    }

    /// Keys consisting solely of configured private keys.
    pub fn private(private: Vec<PrivateKey>) -> Self {
        Self {
            shared: None,
            private,
        }
    }

    /// Returns this key set with `shared` replaced.
    pub fn with_shared(mut self, key: Vec<u8>) -> Self {
        self.shared = Some(key);
        self
    }

    fn require_shared(&self, alg: &str) -> Result<&[u8]> {
        self.shared.as_deref().ok_or_else(|| {
            format!(
                "JWE alg '{alg}' needs a symmetric key but fileEncryptionKey.value supplied none"
            )
        })
    }

    /// Private keys ordered so that a `kid` match (when the header names one)
    /// comes first; keys without a `kid` always remain candidates.
    fn candidates(&self, kid: Option<&str>) -> Vec<&PrivateKey> {
        let mut out: Vec<&PrivateKey> = Vec::with_capacity(self.private.len());
        if let Some(kid) = kid {
            out.extend(self.private.iter().filter(|k| k.kid() == Some(kid)));
            out.extend(self.private.iter().filter(|k| k.kid().is_none()));
            if !out.is_empty() {
                return out;
            }
        }
        self.private.iter().collect()
    }
}

// ---------------------------------------------------------------------------
// Private-key loading
// ---------------------------------------------------------------------------

/// Parses one or more EC private keys from PEM (PKCS#8 or SEC1) or from a
/// JWK / JWK Set document.
///
/// A PEM bundle may hold several keys; every `-----BEGIN … PRIVATE KEY-----`
/// block is parsed. Keys whose type is not usable for JWE — including RSA keys,
/// since the RSA arms are unsupported — are an error rather than a silent skip,
/// so a misconfigured deployment fails at startup.
pub fn load_private_keys(material: &str) -> Result<Vec<PrivateKey>> {
    let trimmed = material.trim();
    if trimmed.is_empty() {
        return Err("no key material supplied".to_string());
    }
    if trimmed.starts_with('{') {
        let doc: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("key material is not valid JSON: {e}"))?;
        // A JWK Set ({"keys":[…]}) or a bare JWK.
        if let Some(keys) = doc.get("keys").and_then(|k| k.as_array()) {
            let mut out = Vec::new();
            for jwk in keys {
                out.push(private_key_from_jwk(jwk)?);
            }
            if out.is_empty() {
                return Err("JWK Set contains no keys".to_string());
            }
            return Ok(out);
        }
        return Ok(vec![private_key_from_jwk(&doc)?]);
    }
    load_private_keys_pem(trimmed)
}

fn load_private_keys_pem(pem: &str) -> Result<Vec<PrivateKey>> {
    use p256::pkcs8::DecodePrivateKey;

    const END: &str = "-----END ";

    let mut out = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find("-----BEGIN ") {
        let after = &rest[start..];
        // A block runs from `-----BEGIN …-----` through the closing dashes of
        // its `-----END …-----` line.
        let Some(end_label) = after.find(END) else {
            break;
        };
        let tail = end_label + END.len();
        let Some(close) = after[tail..].find("-----") else {
            break;
        };
        let block_end = tail + close + "-----".len();
        let block = &after[..block_end];
        rest = &after[block_end..];

        let key = if block.contains("BEGIN RSA PRIVATE KEY") {
            return Err(RSA_UNSUPPORTED.to_string());
        } else if block.contains("BEGIN EC PRIVATE KEY") {
            // SEC1 does not record the curve in the label; try both sizes.
            match p256::SecretKey::from_sec1_pem(block) {
                Ok(key) => PrivateKey::P256 { kid: None, key },
                Err(_) => PrivateKey::P384 {
                    kid: None,
                    key: p384::SecretKey::from_sec1_pem(block)
                        .map_err(|e| format!("invalid SEC1 EC private key: {e}"))?,
                },
            }
        } else if block.contains("BEGIN PRIVATE KEY") {
            // PKCS#8 — the algorithm OID decides.
            if let Ok(key) = p256::SecretKey::from_pkcs8_pem(block) {
                PrivateKey::P256 { kid: None, key }
            } else if let Ok(key) = p384::SecretKey::from_pkcs8_pem(block) {
                PrivateKey::P384 { kid: None, key }
            } else {
                return Err(format!(
                    "PKCS#8 private key is not P-256 or P-384. {RSA_UNSUPPORTED}"
                ));
            }
        } else {
            // A certificate or public key in the same bundle — not an error.
            continue;
        };
        out.push(key);
    }
    if out.is_empty() {
        Err("no PEM private key block found".to_string())
    } else {
        Ok(out)
    }
}

/// Re-serializes an EC JWK keeping only the members RFC 7518 §6.2 defines.
///
/// The `elliptic-curve` JWK parser rejects unknown members, so `kid`, `alg`,
/// `use`, and friends — routinely present on real keys and on the `epk` of an
/// ECDH-ES header — have to be dropped first.
fn ec_jwk_str(jwk: &Value) -> String {
    let mut out = Map::new();
    for name in ["kty", "crv", "x", "y", "d"] {
        if let Some(v) = jwk.get(name) {
            out.insert(name.to_string(), v.clone());
        }
    }
    Value::Object(out).to_string()
}

fn private_key_from_jwk(jwk: &Value) -> Result<PrivateKey> {
    let kid = jwk.get("kid").and_then(|v| v.as_str()).map(str::to_string);
    match jwk.get("kty").and_then(|v| v.as_str()) {
        Some("RSA") => Err(RSA_UNSUPPORTED.to_string()),
        Some("EC") => {
            let jwk_str = ec_jwk_str(jwk);
            match jwk.get("crv").and_then(|v| v.as_str()) {
                Some("P-256") => Ok(PrivateKey::P256 {
                    kid,
                    key: p256::SecretKey::from_jwk_str(&jwk_str)
                        .map_err(|e| format!("invalid P-256 JWK: {e}"))?,
                }),
                Some("P-384") => Ok(PrivateKey::P384 {
                    kid,
                    key: p384::SecretKey::from_jwk_str(&jwk_str)
                        .map_err(|e| format!("invalid P-384 JWK: {e}"))?,
                }),
                other => Err(format!(
                    "unsupported EC JWK crv {other:?} (expected P-256 or P-384)"
                )),
            }
        }
        other => Err(format!("unsupported JWK kty {other:?} (expected EC)")),
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// One recipient of a parsed JWE, with its headers already merged.
struct Recipient {
    header: Map<String, Value>,
    encrypted_key: Vec<u8>,
}

/// A JWE reduced to the pieces decryption needs.
struct Jwe {
    /// `BASE64URL(UTF8(Protected Header))` exactly as it appeared — the AAD input.
    protected_b64: String,
    /// Additional AAD from the JSON serialization's `aad` member, if any.
    aad_b64: Option<String>,
    recipients: Vec<Recipient>,
    iv: Vec<u8>,
    ciphertext: Vec<u8>,
    tag: Vec<u8>,
}

impl Jwe {
    /// The `Additional Authenticated Data` per RFC 7516 §5.1 step 14.
    fn aad(&self) -> Vec<u8> {
        match &self.aad_b64 {
            Some(extra) => format!("{}.{}", self.protected_b64, extra).into_bytes(),
            None => self.protected_b64.clone().into_bytes(),
        }
    }
}

/// True when `bytes` plausibly holds a JWE in compact or JSON serialization.
///
/// Used to tell an encrypted payload from a plaintext one without attempting a
/// decrypt (and to interpret `fileEncryptionKey.value`, which may itself be a
/// JWE carrying the content-encryption key).
pub fn looks_like_jwe(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim();
    if text.starts_with('{') {
        return serde_json::from_str::<Value>(text)
            .map(|v| v.get("ciphertext").is_some())
            .unwrap_or(false);
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 5 {
        return false;
    }
    // The protected header must decode to a JSON object carrying `enc`.
    b64(parts[0])
        .ok()
        .and_then(|h| serde_json::from_slice::<Value>(&h).ok())
        .map(|h| h.get("enc").is_some())
        .unwrap_or(false)
}

fn parse(bytes: &[u8]) -> Result<Jwe> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "JWE is not valid UTF-8".to_string())?
        .trim();
    if text.starts_with('{') {
        parse_json(text)
    } else {
        parse_compact(text)
    }
}

fn parse_compact(text: &str) -> Result<Jwe> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 5 {
        return Err(format!(
            "compact JWE must have 5 segments, found {}",
            parts.len()
        ));
    }
    let header = decode_header(parts[0])?;
    Ok(Jwe {
        protected_b64: parts[0].to_string(),
        aad_b64: None,
        recipients: vec![Recipient {
            header,
            encrypted_key: b64(parts[1])?,
        }],
        iv: b64(parts[2])?,
        ciphertext: b64(parts[3])?,
        tag: b64(parts[4])?,
    })
}

fn decode_header(b64_header: &str) -> Result<Map<String, Value>> {
    if b64_header.is_empty() {
        return Ok(Map::new());
    }
    let json = b64(b64_header)?;
    serde_json::from_slice::<Map<String, Value>>(&json)
        .map_err(|e| format!("invalid JWE protected header: {e}"))
}

fn parse_json(text: &str) -> Result<Jwe> {
    let doc: Map<String, Value> =
        serde_json::from_str(text).map_err(|e| format!("invalid JWE JSON serialization: {e}"))?;
    let get_b64 = |name: &str| -> Result<Vec<u8>> {
        match doc.get(name).and_then(|v| v.as_str()) {
            Some(s) => b64(s),
            None => Ok(Vec::new()),
        }
    };
    let protected_b64 = doc
        .get("protected")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let protected = decode_header(&protected_b64)?;
    let unprotected = doc
        .get("unprotected")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut recipients = Vec::new();
    let mut push = |per_recipient: Map<String, Value>, encrypted_key: Vec<u8>| {
        let mut header = protected.clone();
        header.extend(unprotected.clone());
        header.extend(per_recipient);
        recipients.push(Recipient {
            header,
            encrypted_key,
        });
    };

    if let Some(list) = doc.get("recipients").and_then(|v| v.as_array()) {
        // General JSON serialization.
        for r in list {
            let per = r
                .get("header")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let ek = match r.get("encrypted_key").and_then(|v| v.as_str()) {
                Some(s) => b64(s)?,
                None => Vec::new(),
            };
            push(per, ek);
        }
    } else {
        // Flattened JSON serialization.
        let per = doc
            .get("header")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        push(per, get_b64("encrypted_key")?);
    }
    if recipients.is_empty() {
        return Err("JWE JSON serialization has no recipients".to_string());
    }

    Ok(Jwe {
        protected_b64,
        aad_b64: doc.get("aad").and_then(|v| v.as_str()).map(str::to_string),
        recipients,
        iv: get_b64("iv")?,
        ciphertext: get_b64("ciphertext")?,
        tag: get_b64("tag")?,
    })
}

// ---------------------------------------------------------------------------
// Decryption
// ---------------------------------------------------------------------------

/// Decrypts a JWE (compact, flattened JSON, or general JSON) using `keys`.
///
/// With a general-JSON JWE every recipient is attempted; the last failure is
/// reported when none succeeds.
pub fn decrypt(jwe_bytes: &[u8], keys: &DecryptionKeys) -> Result<Vec<u8>> {
    let jwe = parse(jwe_bytes)?;
    let aad = jwe.aad();
    let mut last_err = None;
    for recipient in &jwe.recipients {
        match decrypt_recipient(&jwe, recipient, &aad, keys) {
            Ok(plaintext) => return Ok(plaintext),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "JWE has no recipients".to_string()))
}

fn decrypt_recipient(
    jwe: &Jwe,
    recipient: &Recipient,
    aad: &[u8],
    keys: &DecryptionKeys,
) -> Result<Vec<u8>> {
    let header = &recipient.header;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JWE header is missing 'alg'".to_string())?;
    let enc = header
        .get("enc")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JWE header is missing 'enc'".to_string())?;
    if let Some(crit) = header.get("crit").and_then(|v| v.as_array()) {
        // RFC 7516 §4.1.13: a recipient that does not understand a `crit`
        // extension MUST reject the JWE.
        let names: Vec<String> = crit
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        return Err(format!(
            "JWE declares unsupported critical header parameters: {}",
            names.join(", ")
        ));
    }

    let cek = derive_cek(alg, enc, &recipient.encrypted_key, header, keys)?;
    let plaintext = decrypt_content(enc, &cek, &jwe.iv, &jwe.ciphertext, &jwe.tag, aad)?;

    match header.get("zip").and_then(|v| v.as_str()) {
        None => Ok(plaintext),
        Some("DEF") => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(&plaintext[..])
                .read_to_end(&mut out)
                .map_err(|e| format!("JWE zip=DEF decompression failed: {e}"))?;
            Ok(out)
        }
        Some(other) => Err(format!("unsupported JWE zip '{other}' (expected DEF)")),
    }
}

/// Key length in bytes required by a content-encryption algorithm.
fn cek_len(enc: &str) -> Result<usize> {
    Ok(match enc {
        "A128GCM" => 16,
        "A192GCM" => 24,
        "A256GCM" => 32,
        "A128CBC-HS256" => 32,
        "A192CBC-HS384" => 48,
        "A256CBC-HS512" => 64,
        other => {
            return Err(format!(
                "unsupported JWE enc '{other}' (expected A128GCM|A192GCM|A256GCM|\
                 A128CBC-HS256|A192CBC-HS384|A256CBC-HS512)"
            ));
        }
    })
}

fn derive_cek(
    alg: &str,
    enc: &str,
    encrypted_key: &[u8],
    header: &Map<String, Value>,
    keys: &DecryptionKeys,
) -> Result<Vec<u8>> {
    match alg {
        "dir" => {
            if !encrypted_key.is_empty() {
                return Err("JWE alg=dir must not carry an encrypted key".to_string());
            }
            let cek = keys.require_shared(alg)?.to_vec();
            let want = cek_len(enc)?;
            if cek.len() != want {
                return Err(format!(
                    "JWE alg=dir with enc={enc} needs a {want}-byte key, got {}",
                    cek.len()
                ));
            }
            Ok(cek)
        }
        "A128KW" | "A192KW" | "A256KW" => {
            let kek = keys.require_shared(alg)?;
            aes_key_unwrap(alg, kek, encrypted_key)
        }
        "A128GCMKW" | "A192GCMKW" | "A256GCMKW" => {
            let kek = keys.require_shared(alg)?;
            let iv = header
                .get("iv")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("JWE alg={alg} is missing header 'iv'"))?;
            let tag = header
                .get("tag")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("JWE alg={alg} is missing header 'tag'"))?;
            let expected = match alg {
                "A128GCMKW" => 16,
                "A192GCMKW" => 24,
                _ => 32,
            };
            if kek.len() != expected {
                return Err(format!(
                    "JWE alg={alg} needs a {expected}-byte key, got {}",
                    kek.len()
                ));
            }
            let mut sealed = encrypted_key.to_vec();
            sealed.extend_from_slice(&b64(tag)?);
            aes_gcm_open(kek, &b64(iv)?, &sealed, &[])
                .map_err(|_| format!("JWE {alg} key unwrap failed"))
        }
        "ECDH-ES" | "ECDH-ES+A128KW" | "ECDH-ES+A192KW" | "ECDH-ES+A256KW" => {
            let z = ecdh_shared_secret(header, keys)?;
            let direct = alg == "ECDH-ES";
            // RFC 7518 §4.6.2: for direct agreement the derived key is the CEK
            // and AlgorithmID is `enc`; for key wrapping it is the KEK and
            // AlgorithmID is `alg`.
            let (alg_id, key_len) = if direct {
                (enc, cek_len(enc)?)
            } else {
                let kw_len = match alg {
                    "ECDH-ES+A128KW" => 16,
                    "ECDH-ES+A192KW" => 24,
                    _ => 32,
                };
                (alg, kw_len)
            };
            let apu = header
                .get("apu")
                .and_then(|v| v.as_str())
                .map(b64)
                .transpose()?
                .unwrap_or_default();
            let apv = header
                .get("apv")
                .and_then(|v| v.as_str())
                .map(b64)
                .transpose()?
                .unwrap_or_default();
            let derived = concat_kdf(&z, alg_id, &apu, &apv, key_len);
            if direct {
                if !encrypted_key.is_empty() {
                    return Err("JWE alg=ECDH-ES must not carry an encrypted key".to_string());
                }
                Ok(derived)
            } else {
                let kw = &alg["ECDH-ES+".len()..];
                aes_key_unwrap(kw, &derived, encrypted_key)
            }
        }
        "RSA-OAEP" | "RSA-OAEP-256" => Err(format!("JWE alg={alg} is rejected. {RSA_UNSUPPORTED}")),
        "RSA1_5" => Err(format!(
            "JWE alg=RSA1_5 is rejected: RSAES-PKCS1-v1_5 is deprecated for JOSE \
             (RFC 8725) because of its padding-oracle exposure. {RSA_UNSUPPORTED}"
        )),
        other if other.starts_with("PBES2-") => Err(format!(
            "unsupported JWE alg '{other}': password-based key derivation has no \
             shared secret in the bulk-submit flow"
        )),
        other => Err(format!(
            "unsupported JWE alg '{other}' (expected dir|A128KW|A192KW|A256KW|\
             A128GCMKW|A192GCMKW|A256GCMKW|ECDH-ES[+A128KW|+A192KW|+A256KW])"
        )),
    }
}

fn ecdh_shared_secret(header: &Map<String, Value>, keys: &DecryptionKeys) -> Result<Vec<u8>> {
    let epk = header
        .get("epk")
        .ok_or_else(|| "JWE ECDH-ES header is missing 'epk'".to_string())?;
    let epk_str = ec_jwk_str(epk);
    let crv = epk
        .get("crv")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "JWE 'epk' is missing 'crv'".to_string())?;
    let kid = header.get("kid").and_then(|v| v.as_str());
    let candidates = keys.candidates(kid);
    if candidates.is_empty() {
        return Err("JWE ECDH-ES requires a configured EC private key \
             (set HFS_BULK_SUBMIT_DECRYPTION_KEY)"
            .to_string());
    }
    match crv {
        "P-256" => {
            let epk = p256::PublicKey::from_jwk_str(&epk_str)
                .map_err(|e| format!("invalid P-256 'epk': {e}"))?;
            for key in candidates {
                if let PrivateKey::P256 { key, .. } = key {
                    let z = p256::ecdh::diffie_hellman(key.to_nonzero_scalar(), epk.as_affine());
                    return Ok(z.raw_secret_bytes().to_vec());
                }
            }
            Err("JWE ECDH-ES with crv=P-256 needs a configured P-256 private key".to_string())
        }
        "P-384" => {
            let epk = p384::PublicKey::from_jwk_str(&epk_str)
                .map_err(|e| format!("invalid P-384 'epk': {e}"))?;
            for key in candidates {
                if let PrivateKey::P384 { key, .. } = key {
                    let z = p384::ecdh::diffie_hellman(key.to_nonzero_scalar(), epk.as_affine());
                    return Ok(z.raw_secret_bytes().to_vec());
                }
            }
            Err("JWE ECDH-ES with crv=P-384 needs a configured P-384 private key".to_string())
        }
        other => Err(format!(
            "unsupported ECDH-ES curve '{other}' (expected P-256 or P-384)"
        )),
    }
}

/// NIST SP 800-56A Concat KDF with SHA-256, as profiled by RFC 7518 §4.6.2.
pub(crate) fn concat_kdf(
    z: &[u8],
    alg_id: &str,
    apu: &[u8],
    apv: &[u8],
    key_len: usize,
) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    fn len_prefixed(out: &mut Vec<u8>, data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
    }

    let mut suffix = Vec::new();
    len_prefixed(&mut suffix, alg_id.as_bytes());
    len_prefixed(&mut suffix, apu);
    len_prefixed(&mut suffix, apv);
    suffix.extend_from_slice(&((key_len * 8) as u32).to_be_bytes());

    let mut out = Vec::with_capacity(key_len + 32);
    let mut counter: u32 = 1;
    while out.len() < key_len {
        let mut hasher = Sha256::new();
        hasher.update(counter.to_be_bytes());
        hasher.update(z);
        hasher.update(&suffix);
        out.extend_from_slice(&hasher.finalize());
        counter += 1;
    }
    out.truncate(key_len);
    out
}

/// AES Key Wrap (RFC 3394) unwrap for the `A128KW` / `A192KW` / `A256KW` family.
fn aes_key_unwrap(alg: &str, kek: &[u8], wrapped: &[u8]) -> Result<Vec<u8>> {
    use aes::{Aes128, Aes192, Aes256};
    use aes_kw::Kek;

    let expected = match alg {
        "A128KW" => 16,
        "A192KW" => 24,
        "A256KW" => 32,
        other => return Err(format!("unsupported AES key-wrap alg '{other}'")),
    };
    if kek.len() != expected {
        return Err(format!(
            "JWE alg={alg} needs a {expected}-byte key, got {}",
            kek.len()
        ));
    }
    if wrapped.len() < 16 || !wrapped.len().is_multiple_of(8) {
        return Err(format!("JWE alg={alg} encrypted key has an invalid length"));
    }
    let mut out = vec![0u8; wrapped.len() - 8];
    let result = match expected {
        16 => Kek::<Aes128>::try_from(kek)
            .map_err(|e| format!("invalid {alg} key: {e}"))?
            .unwrap(wrapped, &mut out),
        24 => Kek::<Aes192>::try_from(kek)
            .map_err(|e| format!("invalid {alg} key: {e}"))?
            .unwrap(wrapped, &mut out),
        _ => Kek::<Aes256>::try_from(kek)
            .map_err(|e| format!("invalid {alg} key: {e}"))?
            .unwrap(wrapped, &mut out),
    };
    result.map_err(|_| format!("JWE {alg} key unwrap failed"))?;
    Ok(out)
}

fn decrypt_content(
    enc: &str,
    cek: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let want = cek_len(enc)?;
    if cek.len() != want {
        return Err(format!(
            "JWE enc={enc} needs a {want}-byte content-encryption key, got {}",
            cek.len()
        ));
    }
    match enc {
        "A128GCM" | "A192GCM" | "A256GCM" => {
            if iv.len() != 12 {
                return Err(format!(
                    "JWE enc={enc} needs a 12-byte IV, got {}",
                    iv.len()
                ));
            }
            let mut sealed = ciphertext.to_vec();
            sealed.extend_from_slice(tag);
            aes_gcm_open(cek, iv, &sealed, aad)
                .map_err(|_| format!("JWE decryption failed ({enc})"))
        }
        "A128CBC-HS256" | "A192CBC-HS384" | "A256CBC-HS512" => {
            aes_cbc_hmac_open(enc, cek, iv, ciphertext, tag, aad)
        }
        other => Err(format!("unsupported JWE enc '{other}'")),
    }
}

/// AES-GCM open over `sealed` (= ciphertext ‖ tag), keyed by length.
fn aes_gcm_open(
    key: &[u8],
    nonce: &[u8],
    sealed: &[u8],
    aad: &[u8],
) -> std::result::Result<Vec<u8>, ()> {
    use aes_gcm::aead::consts::U12;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::aes::{Aes128, Aes192, Aes256};
    use aes_gcm::{AesGcm, Nonce};

    let nonce = Nonce::<U12>::from_slice(nonce);
    let payload = Payload { msg: sealed, aad };
    match key.len() {
        16 => AesGcm::<Aes128, U12>::new_from_slice(key)
            .map_err(|_| ())?
            .decrypt(nonce, payload)
            .map_err(|_| ()),
        24 => AesGcm::<Aes192, U12>::new_from_slice(key)
            .map_err(|_| ())?
            .decrypt(nonce, payload)
            .map_err(|_| ()),
        32 => AesGcm::<Aes256, U12>::new_from_slice(key)
            .map_err(|_| ())?
            .decrypt(nonce, payload)
            .map_err(|_| ()),
        _ => Err(()),
    }
}

/// AES_CBC_HMAC_SHA2 per RFC 7518 §5.2 — MAC first, then CBC-decrypt.
fn aes_cbc_hmac_open(
    enc: &str,
    cek: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockDecryptMut, KeyIvInit};
    use hmac::{Hmac, Mac};

    if iv.len() != 16 {
        return Err(format!(
            "JWE enc={enc} needs a 16-byte IV, got {}",
            iv.len()
        ));
    }
    let half = cek.len() / 2;
    let (mac_key, enc_key) = cek.split_at(half);

    // AL = the AAD length in *bits*, as a 64-bit big-endian integer.
    let al = ((aad.len() as u64) * 8).to_be_bytes();
    let mut mac_input = Vec::with_capacity(aad.len() + iv.len() + ciphertext.len() + 8);
    mac_input.extend_from_slice(aad);
    mac_input.extend_from_slice(iv);
    mac_input.extend_from_slice(ciphertext);
    mac_input.extend_from_slice(&al);

    // The authentication tag is the leading `half` bytes of the HMAC.
    let computed: Vec<u8> = match enc {
        "A128CBC-HS256" => {
            let mut m = <Hmac<sha2::Sha256>>::new_from_slice(mac_key)
                .map_err(|e| format!("invalid {enc} MAC key: {e}"))?;
            m.update(&mac_input);
            m.finalize().into_bytes().to_vec()
        }
        "A192CBC-HS384" => {
            let mut m = <Hmac<sha2::Sha384>>::new_from_slice(mac_key)
                .map_err(|e| format!("invalid {enc} MAC key: {e}"))?;
            m.update(&mac_input);
            m.finalize().into_bytes().to_vec()
        }
        _ => {
            let mut m = <Hmac<sha2::Sha512>>::new_from_slice(mac_key)
                .map_err(|e| format!("invalid {enc} MAC key: {e}"))?;
            m.update(&mac_input);
            m.finalize().into_bytes().to_vec()
        }
    };
    if tag.len() != half || !constant_time_eq(&computed[..half], tag) {
        return Err(format!("JWE authentication tag mismatch ({enc})"));
    }

    let plaintext = match half {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(enc_key, iv)
            .map_err(|e| format!("invalid {enc} key/IV: {e}"))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| format!("JWE decryption failed ({enc}): bad padding"))?,
        24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(enc_key, iv)
            .map_err(|e| format!("invalid {enc} key/IV: {e}"))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| format!("JWE decryption failed ({enc}): bad padding"))?,
        _ => cbc::Decryptor::<aes::Aes256>::new_from_slices(enc_key, iv)
            .map_err(|e| format!("invalid {enc} key/IV: {e}"))?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .map_err(|_| format!("JWE decryption failed ({enc}): bad padding"))?,
    };
    Ok(plaintext)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds `hdr..iv.ct.tag` for an `alg=dir` compact JWE.
    fn dir_compact(header: &str, iv: &[u8], ct: &[u8], tag: &[u8]) -> String {
        format!(
            "{}..{}.{}.{}",
            B64URL.encode(header),
            B64URL.encode(iv),
            B64URL.encode(ct),
            B64URL.encode(tag)
        )
    }

    fn seal_gcm(key: &[u8], iv: &[u8], aad: &[u8], plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
        use aes_gcm::aead::consts::U12;
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::aes::{Aes128, Aes192, Aes256};
        use aes_gcm::{AesGcm, Nonce};
        let nonce = Nonce::<U12>::from_slice(iv);
        let payload = Payload {
            msg: plaintext,
            aad,
        };
        let sealed = match key.len() {
            16 => AesGcm::<Aes128, U12>::new_from_slice(key)
                .unwrap()
                .encrypt(nonce, payload)
                .unwrap(),
            24 => AesGcm::<Aes192, U12>::new_from_slice(key)
                .unwrap()
                .encrypt(nonce, payload)
                .unwrap(),
            _ => AesGcm::<Aes256, U12>::new_from_slice(key)
                .unwrap()
                .encrypt(nonce, payload)
                .unwrap(),
        };
        let (ct, tag) = sealed.split_at(sealed.len() - 16);
        (ct.to_vec(), tag.to_vec())
    }

    #[test]
    fn dir_round_trip_for_every_gcm_size() {
        for (enc, key_len) in [("A128GCM", 16), ("A192GCM", 24), ("A256GCM", 32)] {
            let key = vec![0x5au8; key_len];
            let iv = [0x11u8; 12];
            let plaintext = b"{\"resourceType\":\"Patient\",\"id\":\"p1\"}\n";
            let header = format!(r#"{{"alg":"dir","enc":"{enc}"}}"#);
            let aad = B64URL.encode(&header);
            let (ct, tag) = seal_gcm(&key, &iv, aad.as_bytes(), plaintext);
            let compact = dir_compact(&header, &iv, &ct, &tag);

            let out = decrypt(compact.as_bytes(), &DecryptionKeys::shared(key.clone()))
                .unwrap_or_else(|e| panic!("{enc}: {e}"));
            assert_eq!(out, plaintext);

            // A wrong key must fail the authentication tag.
            let wrong = DecryptionKeys::shared(vec![0x00u8; key_len]);
            assert!(decrypt(compact.as_bytes(), &wrong).is_err());
        }
    }

    #[test]
    fn dir_rejects_key_of_the_wrong_length() {
        let header = r#"{"alg":"dir","enc":"A256GCM"}"#;
        let compact = dir_compact(header, &[0u8; 12], b"ct", b"tag0123456789012");
        let err = decrypt(compact.as_bytes(), &DecryptionKeys::shared(vec![1u8; 16])).unwrap_err();
        assert!(err.contains("32-byte key"), "{err}");
    }

    /// RFC 7516 Appendix A.3 — `A128KW` + `A128CBC-HS256`, exercising both the
    /// key-wrap path and the AES-CBC-HMAC content path against a known vector.
    #[test]
    fn rfc7516_a3_a128kw_a128cbc_hs256() {
        let compact = "eyJhbGciOiJBMTI4S1ciLCJlbmMiOiJBMTI4Q0JDLUhTMjU2In0.\
                       6KB707dM9YTIgHtLvtgWQ8mKwboJW3of9locizkDTHzBC2IlrT1oOQ.\
                       AxY8DCtDaGlsbGljb3RoZQ.\
                       KDlTtXchhZTGufMYmOYGS4HffxPSUrfmqCHXaI9wOGY.\
                       U0m_YmjN04DJvceFICbCVQ";
        // The AES-128 KEK from A.3.3, as its `oct` JWK `k` value.
        let kek = B64URL.decode("GawgguFyGrWKav7AX4VKUg").unwrap();
        let out =
            decrypt(compact.as_bytes(), &DecryptionKeys::shared(kek)).expect("RFC 7516 A.3 vector");
        assert_eq!(out, b"Live long and prosper.");
    }

    /// RFC 7516 Appendix A.1 — `RSA-OAEP` + `A256GCM`. Deliberately *not*
    /// supported: both the RSA JWK and the JWE itself are rejected with an
    /// error naming RUSTSEC-2023-0071, so the reason survives in the code.
    #[test]
    fn rfc7516_a1_rsa_oaep_is_rejected_with_the_advisory() {
        // A.1.3, with the RFC's display line breaks removed.
        let jwk = serde_json::json!({
            "kty": "RSA",
            "n": "oahUIoWw0K0usKNuOR6H4wkf4oBUXHTxRvgb48E-BVvxkeDNjbC4he8rUWcJoZmds2h7M70imEVhRU5djINXtqllXI4DFqcI1DgjT9LewND8MW2Krf3Spsk_ZkoFnilakGygTwpZ3uesH-PFABNIUYpOiN15dsQRkgr0vEhxN92i2asbOenSZeyaxziK72UwxrrKoExv6kc5twXTq4h-QChLOln0_mtUZwfsRaMStPs6mS6XrgxnxbWhojf663tuEQueGC-FCMfra36C9knDFGzKsNa7LZK2djYgyD3JR_MB_4NUJW_TqOQtwHYbxevoJArm-L5StowjzGy-_bq6Gw",
            "e": "AQAB",
            "d": "kLdtIj6GbDks_ApCSTYQtelcNttlKiOyPzMrXHeI-yk1F7-kpDxY4-WY5NWV5KntaEeXS1j82E375xxhWMHXyvjYecPT9fpwR_M9gV8n9Hrh2anTpTD93Dt62ypW3yDsJzBnTnrYu1iwWRgBKrEYY46qAZIrA2xAwnm2X7uGR1hghkqDp0Vqj3kbSCz1XyfCs6_LehBwtxHIyh8Ripy40p24moOAbgxVw3rxT_vlt3UVe4WO3JkJOzlpUf-KTVI2Ptgm-dARxTEtE-id-4OJr0h-K-VFs3VSndVTIznSxfyrj8ILL6MG_Uv8YAu7VILSB3lOW085-4qE3DzgrTjgyQ",
            "p": "1r52Xk46c-LsfB5P442p7atdPUrxQSy4mti_tZI3Mgf2EuFVbUoDBvaRQ-SWxkbkmoEzL7JXroSBjSrK3YIQgYdMgyAEPTPjXv_hI2_1eTSPVZfzL0lffNn03IXqWF5MDFuoUYE0hzb2vhrlN_rKrbfDIwUbTrjjgieRbwC6Cl0",
            "q": "wLb35x7hmQWZsWJmB_vle87ihgZ19S8lBEROLIsZG4ayZVe9Hi9gDVCOBmUDdaDYVTSNx_8Fyw1YYa9XGrGnDew00J28cRUoeBB_jKI1oma0Orv1T9aXIWxKwd4gvxFImOWr3QRL9KEBRzk2RatUBnmDZJTIAfwTs0g68UZHvtc",
        });
        let err = load_private_keys(&jwk.to_string()).unwrap_err();
        assert!(err.contains("RUSTSEC-2023-0071"), "{err}");

        // A.1.7, with the RFC's display line breaks removed.
        let compact = "eyJhbGciOiJSU0EtT0FFUCIsImVuYyI6IkEyNTZHQ00ifQ.\
                       OKOawDo13gRp2ojaHV7LFpZcgV7T6DVZKTyKOMTYUmKoTCVJRgckCL9kiMT03JGeipsEdY3mx_etLbbWSrFr05kLzcSr4qKAq7YN7e9jwQRb23nfa6c9d-StnImGyFDbSv04uVuxIp5Zms1gNxKKK2Da14B8S4rzVRltdYwam_lDp5XnZAYpQdb76FdIKLaVmqgfwX7XWRxv2322i-vDxRfqNzo_tETKzpVLzfiwQyeyPGLBIO56YJ7eObdv0je81860ppamavo35UgoRdbYaBcoh9QcfylQr66oc6vFWXRcZ_ZT2LawVCWTIy3brGPi6UklfCpIMfIjf7iGdXKHzg.\
                       48V1_ALb6US04U3b.\
                       5eym8TW_c8SuK0ltJ3rpYIzOeDQz7TALvtu6UG9oMo4vpzs9tX_EFShS8iB7j6jiSdiwkIr3ajwQzaBtQD_A.\
                       XFBoMYUZodetZdvTiFvSkQ";
        let err = decrypt(compact.as_bytes(), &DecryptionKeys::default()).unwrap_err();
        assert!(err.contains("RSA-OAEP"), "{err}");
        assert!(err.contains("RUSTSEC-2023-0071"), "{err}");
        assert!(err.contains("ECDH-ES"), "{err}");
    }

    #[test]
    fn aes_kw_round_trip_for_every_size() {
        use aes::{Aes128, Aes192, Aes256};
        use aes_kw::Kek;

        for (alg, enc, kek_len, cek_len) in [
            ("A128KW", "A128GCM", 16, 16),
            ("A192KW", "A192GCM", 24, 24),
            ("A256KW", "A256CBC-HS512", 32, 64),
        ] {
            let kek = vec![0x3cu8; kek_len];
            let cek = vec![0x7eu8; cek_len];
            let mut wrapped = vec![0u8; cek_len + 8];
            match kek_len {
                16 => Kek::<Aes128>::try_from(&kek[..])
                    .unwrap()
                    .wrap(&cek, &mut wrapped)
                    .unwrap(),
                24 => Kek::<Aes192>::try_from(&kek[..])
                    .unwrap()
                    .wrap(&cek, &mut wrapped)
                    .unwrap(),
                _ => Kek::<Aes256>::try_from(&kek[..])
                    .unwrap()
                    .wrap(&cek, &mut wrapped)
                    .unwrap(),
            }
            let unwrapped = aes_key_unwrap(alg, &kek, &wrapped)
                .unwrap_or_else(|e| panic!("{alg}/{enc} unwrap: {e}"));
            assert_eq!(unwrapped, cek);
        }
    }

    #[test]
    fn cbc_hmac_round_trip() {
        use aes::cipher::block_padding::Pkcs7;
        use aes::cipher::{BlockEncryptMut, KeyIvInit};
        use hmac::{Hmac, Mac};

        let cek = (0u8..32).collect::<Vec<u8>>(); // 16-byte MAC key + 16-byte enc key
        let iv = [0x21u8; 16];
        let plaintext = b"{\"resourceType\":\"Observation\"}\n";
        let header = r#"{"alg":"dir","enc":"A128CBC-HS256"}"#;
        let aad = B64URL.encode(header);

        let ct = cbc::Encryptor::<aes::Aes128>::new_from_slices(&cek[16..], &iv)
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext);
        let al = ((aad.len() as u64) * 8).to_be_bytes();
        let mut mac_input = Vec::new();
        mac_input.extend_from_slice(aad.as_bytes());
        mac_input.extend_from_slice(&iv);
        mac_input.extend_from_slice(&ct);
        mac_input.extend_from_slice(&al);
        let mut mac = <Hmac<sha2::Sha256>>::new_from_slice(&cek[..16]).unwrap();
        mac.update(&mac_input);
        let tag = mac.finalize().into_bytes()[..16].to_vec();

        let compact = dir_compact(header, &iv, &ct, &tag);
        let out = decrypt(compact.as_bytes(), &DecryptionKeys::shared(cek.clone())).unwrap();
        assert_eq!(out, plaintext);

        // Tamper with the ciphertext → tag mismatch, and no padding oracle.
        let mut bad_ct = ct.clone();
        bad_ct[0] ^= 0xff;
        let tampered = dir_compact(header, &iv, &bad_ct, &tag);
        let err = decrypt(tampered.as_bytes(), &DecryptionKeys::shared(cek)).unwrap_err();
        assert!(err.contains("authentication tag mismatch"), "{err}");
    }

    #[test]
    fn gcmkw_round_trip() {
        let kek = vec![0x2bu8; 32];
        let cek = vec![0x6du8; 32];
        let kw_iv = [0x09u8; 12];
        let (wrapped, wrap_tag) = seal_gcm(&kek, &kw_iv, &[], &cek);
        let header = format!(
            r#"{{"alg":"A256GCMKW","enc":"A256GCM","iv":"{}","tag":"{}"}}"#,
            B64URL.encode(kw_iv),
            B64URL.encode(&wrap_tag)
        );
        let iv = [0x33u8; 12];
        let plaintext = b"payload";
        let aad = B64URL.encode(&header);
        let (ct, tag) = seal_gcm(&cek, &iv, aad.as_bytes(), plaintext);
        let compact = format!(
            "{}.{}.{}.{}.{}",
            aad,
            B64URL.encode(&wrapped),
            B64URL.encode(iv),
            B64URL.encode(&ct),
            B64URL.encode(&tag)
        );
        let out = decrypt(compact.as_bytes(), &DecryptionKeys::shared(kek)).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn zip_def_is_inflated() {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;
        use std::io::Write;

        let key = vec![0x44u8; 32];
        let iv = [0x55u8; 12];
        let plaintext = b"{\"resourceType\":\"Patient\"}\n".repeat(20);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plaintext).unwrap();
        let compressed = encoder.finish().unwrap();

        let header = r#"{"alg":"dir","enc":"A256GCM","zip":"DEF"}"#;
        let aad = B64URL.encode(header);
        let (ct, tag) = seal_gcm(&key, &iv, aad.as_bytes(), &compressed);
        let compact = dir_compact(header, &iv, &ct, &tag);
        let out = decrypt(compact.as_bytes(), &DecryptionKeys::shared(key)).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn flattened_json_serialization_round_trips() {
        let key = vec![0x66u8; 32];
        let iv = [0x77u8; 12];
        let plaintext = b"flat";
        let header = r#"{"alg":"dir","enc":"A256GCM"}"#;
        let protected = B64URL.encode(header);
        let extra_aad = B64URL.encode(b"extra");
        let aad = format!("{protected}.{extra_aad}");
        let (ct, tag) = seal_gcm(&key, &iv, aad.as_bytes(), plaintext);
        let doc = serde_json::json!({
            "protected": protected,
            "aad": extra_aad,
            "iv": B64URL.encode(iv),
            "ciphertext": B64URL.encode(&ct),
            "tag": B64URL.encode(&tag),
        });
        let bytes = serde_json::to_vec(&doc).unwrap();
        assert!(looks_like_jwe(&bytes));
        let out = decrypt(&bytes, &DecryptionKeys::shared(key)).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn general_json_tries_every_recipient() {
        let key = vec![0x88u8; 32];
        let iv = [0x99u8; 12];
        let plaintext = b"general";
        // `alg` lives per-recipient; the protected header carries only `enc`.
        let header = r#"{"enc":"A256GCM"}"#;
        let protected = B64URL.encode(header);
        let (ct, tag) = seal_gcm(&key, &iv, protected.as_bytes(), plaintext);
        let doc = serde_json::json!({
            "protected": protected,
            "recipients": [
                { "header": { "alg": "RSA-OAEP-256" }, "encrypted_key": B64URL.encode([1u8; 8]) },
                { "header": { "alg": "dir" } },
            ],
            "iv": B64URL.encode(iv),
            "ciphertext": B64URL.encode(&ct),
            "tag": B64URL.encode(&tag),
        });
        let bytes = serde_json::to_vec(&doc).unwrap();
        let out = decrypt(&bytes, &DecryptionKeys::shared(key)).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn ecdh_es_without_a_configured_key_names_the_missing_configuration() {
        use rand::rngs::OsRng;

        let ephemeral = p256::SecretKey::random(&mut OsRng);
        let epk: Value = serde_json::from_str(&ephemeral.public_key().to_jwk_string()).unwrap();
        let header = serde_json::json!({"alg": "ECDH-ES", "enc": "A256GCM", "epk": epk});
        let compact = dir_compact(
            &serde_json::to_string(&header).unwrap(),
            &[0u8; 12],
            b"ct",
            b"tag",
        );
        let err = decrypt(compact.as_bytes(), &DecryptionKeys::default()).unwrap_err();
        assert!(err.contains("HFS_BULK_SUBMIT_DECRYPTION_KEY"), "{err}");
    }

    #[test]
    fn ecdh_es_direct_round_trip() {
        use rand::rngs::OsRng;

        let recipient = p256::SecretKey::random(&mut OsRng);
        let recipient_pub = recipient.public_key();
        let ephemeral = p256::SecretKey::random(&mut OsRng);
        let epk: Value = serde_json::from_str(&ephemeral.public_key().to_jwk_string()).unwrap();

        // Sender side: Z from the ephemeral private key + recipient public key.
        let z =
            p256::ecdh::diffie_hellman(ephemeral.to_nonzero_scalar(), recipient_pub.as_affine());
        let cek = concat_kdf(z.raw_secret_bytes(), "A256GCM", b"", b"", 32);

        let header = serde_json::json!({"alg": "ECDH-ES", "enc": "A256GCM", "epk": epk});
        let header = serde_json::to_string(&header).unwrap();
        let aad = B64URL.encode(&header);
        let iv = [0xccu8; 12];
        let plaintext = b"ecdh-es";
        let (ct, tag) = seal_gcm(&cek, &iv, aad.as_bytes(), plaintext);
        let compact = format!(
            "{}..{}.{}.{}",
            aad,
            B64URL.encode(iv),
            B64URL.encode(&ct),
            B64URL.encode(&tag)
        );

        let keys = DecryptionKeys::private(vec![PrivateKey::P256 {
            kid: None,
            key: recipient,
        }]);
        let out = decrypt(compact.as_bytes(), &keys).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn ecdh_es_a128kw_round_trip() {
        use aes::Aes128;
        use aes_kw::Kek;
        use rand::rngs::OsRng;

        let recipient = p384::SecretKey::random(&mut OsRng);
        let ephemeral = p384::SecretKey::random(&mut OsRng);
        let epk: Value = serde_json::from_str(&ephemeral.public_key().to_jwk_string()).unwrap();
        let z = p384::ecdh::diffie_hellman(
            ephemeral.to_nonzero_scalar(),
            recipient.public_key().as_affine(),
        );
        let kek = concat_kdf(z.raw_secret_bytes(), "ECDH-ES+A128KW", b"", b"", 16);

        let cek = vec![0xdeu8; 32];
        let mut wrapped = vec![0u8; cek.len() + 8];
        Kek::<Aes128>::try_from(&kek[..])
            .unwrap()
            .wrap(&cek, &mut wrapped)
            .unwrap();

        let header = serde_json::json!({"alg": "ECDH-ES+A128KW", "enc": "A256GCM", "epk": epk});
        let header = serde_json::to_string(&header).unwrap();
        let aad = B64URL.encode(&header);
        let iv = [0xeeu8; 12];
        let plaintext = b"ecdh-es+kw";
        let (ct, tag) = seal_gcm(&cek, &iv, aad.as_bytes(), plaintext);
        let compact = format!(
            "{}.{}.{}.{}.{}",
            aad,
            B64URL.encode(&wrapped),
            B64URL.encode(iv),
            B64URL.encode(&ct),
            B64URL.encode(&tag)
        );

        let keys = DecryptionKeys::private(vec![PrivateKey::P384 {
            kid: None,
            key: recipient,
        }]);
        let out = decrypt(compact.as_bytes(), &keys).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn unsupported_algorithms_name_themselves() {
        for (header, needle) in [
            (r#"{"alg":"RSA-OAEP","enc":"A256GCM"}"#, "RUSTSEC-2023-0071"),
            (
                r#"{"alg":"RSA-OAEP-256","enc":"A256GCM"}"#,
                "RUSTSEC-2023-0071",
            ),
            (r#"{"alg":"RSA1_5","enc":"A128CBC-HS256"}"#, "RSA1_5"),
            (
                r#"{"alg":"PBES2-HS256+A128KW","enc":"A128GCM"}"#,
                "PBES2-HS256+A128KW",
            ),
            (r#"{"alg":"dir","enc":"A128CTR"}"#, "A128CTR"),
            (r#"{"alg":"XYZ","enc":"A128GCM"}"#, "XYZ"),
        ] {
            let compact = dir_compact(header, &[0u8; 12], b"ct", b"tag");
            let err =
                decrypt(compact.as_bytes(), &DecryptionKeys::shared(vec![0u8; 32])).unwrap_err();
            assert!(err.contains(needle), "expected {needle} in: {err}");
        }
    }

    #[test]
    fn critical_headers_are_rejected() {
        let header = r#"{"alg":"dir","enc":"A256GCM","crit":["exp"],"exp":1}"#;
        let compact = dir_compact(header, &[0u8; 12], b"ct", b"tag");
        let err = decrypt(compact.as_bytes(), &DecryptionKeys::shared(vec![0u8; 32])).unwrap_err();
        assert!(err.contains("critical header"), "{err}");
    }

    #[test]
    fn looks_like_jwe_rejects_plaintext() {
        assert!(!looks_like_jwe(b"{\"resourceType\":\"Patient\"}"));
        assert!(!looks_like_jwe(b"not.a.jwe"));
        assert!(!looks_like_jwe(&[0xff, 0xfe, 0x00]));
        // Five segments, but the header is not a JWE header.
        assert!(!looks_like_jwe(b"a.b.c.d.e"));
    }

    #[test]
    fn compact_segment_count_is_enforced() {
        let err = decrypt(b"a.b.c", &DecryptionKeys::default()).unwrap_err();
        assert!(err.contains("5 segments"), "{err}");
    }

    #[test]
    fn load_private_keys_reads_pkcs8_ec_pem() {
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        use rand::rngs::OsRng;

        let p256_pem = p256::SecretKey::random(&mut OsRng)
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string();
        let keys = load_private_keys(&p256_pem).unwrap();
        assert!(matches!(keys.as_slice(), [PrivateKey::P256 { .. }]));

        let p384_pem = p384::SecretKey::random(&mut OsRng)
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string();
        let keys = load_private_keys(&p384_pem).unwrap();
        assert!(matches!(keys.as_slice(), [PrivateKey::P384 { .. }]));

        // A bundle yields both keys.
        let bundle = format!("{p256_pem}{p384_pem}");
        assert_eq!(load_private_keys(&bundle).unwrap().len(), 2);
    }

    #[test]
    fn load_private_keys_rejects_rsa_pem_with_the_advisory() {
        // A PKCS#1 RSA block is recognised and refused by label, so the failure
        // explains itself rather than surfacing as a parse error.
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n";
        let err = load_private_keys(pem).unwrap_err();
        assert!(err.contains("RUSTSEC-2023-0071"), "{err}");
    }

    #[test]
    fn load_private_keys_reads_a_jwk_set() {
        use rand::rngs::OsRng;

        let ec = p256::SecretKey::random(&mut OsRng);
        let mut jwk: Value = serde_json::from_str(&ec.to_jwk_string()).unwrap();
        jwk["kid"] = Value::String("k1".into());
        let jwks = serde_json::json!({ "keys": [jwk] }).to_string();
        let keys = load_private_keys(&jwks).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kid(), Some("k1"));
    }

    #[test]
    fn load_private_keys_rejects_junk() {
        assert!(load_private_keys("").is_err());
        assert!(load_private_keys("not a key").is_err());
        assert!(load_private_keys(r#"{"kty":"oct","k":"AAA"}"#).is_err());
    }

    #[test]
    fn kid_selects_the_matching_key() {
        use rand::rngs::OsRng;
        let a = p256::SecretKey::random(&mut OsRng);
        let b = p256::SecretKey::random(&mut OsRng);
        let keys = DecryptionKeys::private(vec![
            PrivateKey::P256 {
                kid: Some("a".into()),
                key: a.clone(),
            },
            PrivateKey::P256 {
                kid: Some("b".into()),
                key: b,
            },
        ]);
        let picked = keys.candidates(Some("a"));
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].kid(), Some("a"));
        // An unknown kid falls back to trying everything.
        assert_eq!(keys.candidates(Some("zzz")).len(), 2);
    }
}
