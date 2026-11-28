//! Refusal of writes that a page on another origin sends.
//!
//! The HTS write routes (`POST /import`, resource create, update and delete,
//! `$closure`) have no authentication of their own, and a browser sends a
//! `text/plain` POST from any page without a preflight. CORS defaults to `*`,
//! so a preflighted `PUT` or `DELETE` from any page is admitted too. Any site
//! a user visits could therefore write to an HTS that user's browser can
//! reach, an intranet or `localhost` one included.
//!
//! A write is refused with `403` when the browser says it came from another
//! origin: `Sec-Fetch-Site` other than `same-origin` or `none`, or, without
//! that header, an `Origin` that does not name the request's `Host`. An
//! origin listed in `HTS_CORS_ORIGINS` (anything but `*`) is the operator's
//! choice and stays allowed. A request without `Sec-Fetch-Site` and `Origin`
//! did not come from a browser page and is not affected, nor is any read,
//! the read operations sent by `POST` included.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The origins a cross-origin write is allowed from, normalised to lowercase
/// with no trailing `/`.
#[derive(Debug, Clone, Default)]
pub(crate) struct AllowedOrigins(Arc<Vec<String>>);

impl AllowedOrigins {
    /// The explicitly listed origins of `HTS_CORS_ORIGINS`. `*`, or CORS off,
    /// lists none.
    pub(crate) fn from_config(enable_cors: bool, cors_origins: &str) -> Self {
        if !enable_cors || cors_origins.trim() == "*" {
            return Self::default();
        }
        Self(Arc::new(
            cors_origins
                .split(',')
                .map(normalise_origin)
                .filter(|o| !o.is_empty())
                .collect(),
        ))
    }

    fn contains(&self, origin: &str) -> bool {
        let origin = normalise_origin(origin);
        self.0.contains(&origin)
    }
}

fn normalise_origin(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `method` on `path` changes what HTS stores. Every `PUT`, `DELETE`
/// and `PATCH`; of the `POST`s, the import, the resource creates and
/// `$closure`. The other `POST`s are read operations.
pub(crate) fn is_write(method: &Method, path: &str) -> bool {
    match *method {
        Method::PUT | Method::DELETE | Method::PATCH => true,
        Method::POST => matches!(
            path.trim_end_matches('/'),
            "/import" | "/CodeSystem" | "/ValueSet" | "/ConceptMap" | "/ConceptMap/$closure"
        ),
        _ => false,
    }
}

/// Whether the browser says this request came from another origin that is
/// not listed.
pub(crate) fn is_foreign(headers: &HeaderMap, allowed: &AllowedOrigins) -> bool {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin.is_some_and(|o| allowed.contains(o)) {
        return false;
    }
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        let site = site.trim();
        return !(site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none"));
    }
    let Some(origin) = origin else {
        return false;
    };
    let origin_authority = origin
        .trim()
        .parse::<Uri>()
        .ok()
        .and_then(|uri| uri.authority().map(|a| a.as_str().to_ascii_lowercase()));
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.trim().to_ascii_lowercase());
    origin_authority.is_none() || origin_authority != host
}

/// Middleware: refuses a write [`is_foreign`] judges foreign.
pub(crate) async fn refuse_foreign_writes(
    allowed: AllowedOrigins,
    request: Request,
    next: Next,
) -> Response {
    if is_write(request.method(), request.uri().path()) && is_foreign(request.headers(), &allowed) {
        let body = serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "forbidden",
                "diagnostics": "a write from another origin is refused; list the origin in HTS_CORS_ORIGINS to allow it"
            }]
        });
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/fhir+json")],
            body.to_string(),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    #[test]
    fn writes_are_the_import_creates_updates_deletes_and_closure() {
        for (method, path) in [
            (Method::POST, "/import"),
            (Method::POST, "/CodeSystem"),
            (Method::POST, "/ValueSet/"),
            (Method::POST, "/ConceptMap/$closure"),
            (Method::PUT, "/CodeSystem/x"),
            (Method::DELETE, "/ValueSet/x"),
        ] {
            assert!(is_write(&method, path), "{method} {path}");
        }
        for (method, path) in [
            (Method::POST, "/ValueSet/$expand"),
            (Method::POST, "/CodeSystem/$validate-code"),
            (Method::POST, "/"),
            (Method::GET, "/CodeSystem"),
        ] {
            assert!(!is_write(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn a_foreign_origin_is_refused_unless_listed() {
        let none = AllowedOrigins::default();
        assert!(is_foreign(
            &headers(&[("sec-fetch-site", "cross-site")]),
            &none
        ));
        assert!(is_foreign(
            &headers(&[("sec-fetch-site", "same-site")]),
            &none
        ));
        assert!(!is_foreign(
            &headers(&[("sec-fetch-site", "same-origin")]),
            &none
        ));
        assert!(!is_foreign(&headers(&[("sec-fetch-site", "none")]), &none));
        assert!(is_foreign(
            &headers(&[("origin", "https://evil.example"), ("host", "hts.example")]),
            &none
        ));
        assert!(!is_foreign(
            &headers(&[("origin", "https://hts.example"), ("host", "hts.example")]),
            &none
        ));
        assert!(!is_foreign(&headers(&[]), &none));

        let listed = AllowedOrigins::from_config(true, "https://app.example/, https://b.example");
        assert!(!is_foreign(
            &headers(&[
                ("origin", "https://APP.example"),
                ("sec-fetch-site", "cross-site")
            ]),
            &listed
        ));
        assert!(AllowedOrigins::from_config(true, "*").0.is_empty());
        assert!(
            AllowedOrigins::from_config(false, "https://app.example")
                .0
                .is_empty()
        );
    }
}
