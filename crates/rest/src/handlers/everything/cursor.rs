use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::error::RestError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EverythingCursor {
    pub v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<String>,
    pub seg: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<String>,
    pub fp: String,
}

/// FNV-1a 64-bit, hex. Deterministic across processes and Rust versions,
/// unlike `DefaultHasher`.
pub(crate) fn fingerprint(input: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl EverythingCursor {
    pub fn new(seg: usize, inner: Option<String>, fp_input: &str) -> Self {
        Self {
            v: 1,
            pat: None,
            pid: None,
            seg,
            inner,
            fp: fingerprint(fp_input),
        }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("cursor serializes");
        URL_SAFE_NO_PAD.encode(json)
    }

    pub fn decode(token: &str, fp_input: &str) -> Result<Self, RestError> {
        let invalid = || RestError::BadRequest {
            message: "Invalid _cursor for $everything".to_string(),
        };
        let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.v != 1 {
            return Err(invalid());
        }
        if cursor.fp != fingerprint(fp_input) {
            return Err(RestError::BadRequest {
                message:
                    "_cursor was issued for a different request; repeat the original parameters"
                        .to_string(),
            });
        }
        Ok(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c = EverythingCursor {
            v: 1,
            pat: Some("p".into()),
            pid: Some("pt1".into()),
            seg: 3,
            inner: Some("in".into()),
            fp: fingerprint("x"),
        };
        let token = c.encode();
        assert!(!token.contains('='), "url-safe, no padding");
        assert_eq!(EverythingCursor::decode(&token, "x").unwrap(), c);
    }

    #[test]
    fn new_sets_version_and_fingerprint() {
        let c = EverythingCursor::new(2, None, "scope");
        assert_eq!(c.v, 1);
        assert_eq!(c.fp, fingerprint("scope"));
        assert_eq!(c.pat, None);
        assert_eq!(c.pid, None);
    }

    #[test]
    fn rejects_fingerprint_mismatch() {
        let token = EverythingCursor::new(0, None, "a").encode();
        let e = EverythingCursor::decode(&token, "b").unwrap_err();
        assert!(
            matches!(e, RestError::BadRequest { message } if message.contains("different request"))
        );
    }

    #[test]
    fn rejects_garbage_and_wrong_version() {
        assert!(matches!(
            EverythingCursor::decode("not base64!", "a"),
            Err(RestError::BadRequest { .. })
        ));
        let mut c = EverythingCursor::new(0, None, "a");
        c.v = 2;
        assert!(matches!(
            EverythingCursor::decode(&c.encode(), "a"),
            Err(RestError::BadRequest { .. })
        ));
    }

    #[test]
    fn fingerprint_is_stable_hex() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
        assert_eq!(fingerprint("").len(), 16);
    }
}
