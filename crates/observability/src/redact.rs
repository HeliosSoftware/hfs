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
    // Userinfo first: a raw `?` or `#` in a password would otherwise be read
    // as the start of the query and leave `user:pass-prefix` in place. An `@`
    // anywhere before the first `/` ends the userinfo, so over-redaction (an
    // `@` in a path-less query) can hide the host but never shows a secret.
    let mut out = String::with_capacity(url.len());
    let rest = match url.find("://") {
        Some(scheme_end) => {
            let authority_start = scheme_end + 3;
            let authority_end = url[authority_start..]
                .find('/')
                .map_or(url.len(), |i| authority_start + i);
            out.push_str(&url[..authority_start]);
            match url[authority_start..authority_end].rfind('@') {
                Some(at) => {
                    out.push_str("[redacted]@");
                    &url[authority_start + at + 1..]
                }
                None => &url[authority_start..],
            }
        }
        None => url,
    };
    match rest.find(['?', '#']) {
        Some(cut) if rest[cut..].starts_with('?') => {
            out.push_str(&rest[..cut]);
            out.push_str("?[redacted]");
        }
        Some(cut) => {
            out.push_str(&rest[..cut]);
            out.push_str("#[redacted]");
        }
        None => out.push_str(rest),
    }
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

    #[test]
    fn redact_url_hides_passwords_containing_query_or_fragment_delimiters() {
        assert_eq!(
            redact_url("postgres://u:a?b@db:5432/x?sslmode=require"),
            "postgres://[redacted]@db:5432/x?[redacted]"
        );
        assert_eq!(
            redact_url("https://u:a#b@host/f"),
            "https://[redacted]@host/f"
        );
        assert_eq!(redact_url("https://u:a?b@host"), "https://[redacted]@host");
    }
}
