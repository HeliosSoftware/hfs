//! Values safe to put in logs, error messages, and API responses.
//!
//! Lives here, at the bottom of the dependency graph, so every server crate
//! can redact a configured URL before logging it (#2149).

/// Returns `url` with everything that can carry a credential removed, for
/// error messages, logs, and manifest `error` artifacts (#1127, #2149).
///
/// A presigned input URL carries its signature in the query string, and a URL
/// can carry `user:password@` in its authority. Both are replaced by a
/// `[redacted]` marker so the message still shows that something was there;
/// scheme, host, port, and path stay, which is what an operator needs to find
/// the file. Not a URL parser: anything without `://` keeps its path part.
pub fn redact_url(url: &str) -> String {
    let (base, tail) = match url.find(['?', '#']) {
        Some(cut) if url[cut..].starts_with('?') => (&url[..cut], "?[redacted]"),
        Some(cut) => (&url[..cut], "#[redacted]"),
        None => (url, ""),
    };
    let mut out = String::with_capacity(base.len() + tail.len());
    match base.find("://") {
        Some(scheme_end) => {
            let authority_start = scheme_end + 3;
            let authority_end = base[authority_start..]
                .find('/')
                .map_or(base.len(), |i| authority_start + i);
            match base[authority_start..authority_end].rfind('@') {
                Some(at) => {
                    out.push_str(&base[..authority_start]);
                    out.push_str("[redacted]@");
                    out.push_str(&base[authority_start + at + 1..]);
                }
                None => out.push_str(base),
            }
        }
        None => out.push_str(base),
    }
    out.push_str(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_strips_query_fragment_and_userinfo() {
        assert_eq!(
            redact_url(
                "https://bucket.s3.amazonaws.com/a/Patient.ndjson?X-Amz-Signature=abc&X-Amz-Expires=60"
            ),
            "https://bucket.s3.amazonaws.com/a/Patient.ndjson?[redacted]"
        );
        assert_eq!(
            redact_url("https://host/file.ndjson#token=abc"),
            "https://host/file.ndjson#[redacted]"
        );
        assert_eq!(
            redact_url("https://user:secret@host:8443/f.ndjson?sig=1"),
            "https://[redacted]@host:8443/f.ndjson?[redacted]"
        );
        assert_eq!(
            redact_url("http://host/path@with-at"),
            "http://host/path@with-at"
        );
        assert_eq!(
            redact_url("http://127.0.0.1:18080/Patient.ndjson"),
            "http://127.0.0.1:18080/Patient.ndjson"
        );
        assert_eq!(redact_url("not a url?x=1"), "not a url?[redacted]");
    }
}
