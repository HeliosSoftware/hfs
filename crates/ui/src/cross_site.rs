//! Refuses state-changing `/ui` requests sent by pages on another site.
//!
//! An HTML form post is a CORS "simple request": a page on any origin can
//! submit one to the UI through the browser of anyone who can reach it, with
//! no preflight. The UI's write routes (tenant administration, Bulk Import,
//! Bulk Export, settings, …) act for the browser's session, or for nobody when
//! authentication is off, so such a post would act with the visitor's network
//! position and session. The HTS UI closed the same hole for its import page
//! (#2776); this applies the check to every unsafe method under `/ui`.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::WebState;

/// Whether a request was sent by a page served from another origin.
///
/// `Sec-Fetch-Site` decides when the browser sends it (`same-site` counts as
/// cross-site: a sibling subdomain is a different origin). Otherwise `Origin`
/// has to match the `Host` the request arrived on, or the origin of the
/// configured public base URL (a proxy that rewrites `Host`). Both are
/// forbidden request headers, so a page cannot set them from script. A request
/// with neither header is not a browser form post from another site.
pub(crate) fn is_cross_site(headers: &HeaderMap, public_base_url: &str) -> bool {
    match headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        Some("same-origin") | Some("none") => false,
        Some(_) => true,
        None => match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            None => false,
            Some(origin) => {
                let Some(origin) = origin.trim().parse::<Uri>().ok() else {
                    return true;
                };
                let Some(authority) = origin.authority().map(|a| a.as_str().to_ascii_lowercase())
                else {
                    // `Origin: null` (sandboxed frame, data: URL, …).
                    return true;
                };
                let host = headers
                    .get(header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .map(|h| h.trim().to_ascii_lowercase());
                if host.as_deref() == Some(authority.as_str()) {
                    return false;
                }
                let public = public_base_url.parse::<Uri>().ok().and_then(|uri| {
                    Some((
                        uri.scheme_str()?.to_ascii_lowercase(),
                        uri.authority()?.as_str().to_ascii_lowercase(),
                    ))
                });
                let origin_scheme = origin.scheme_str().map(str::to_ascii_lowercase);
                public != origin_scheme.map(|scheme| (scheme, authority))
            }
        },
    }
}

/// Middleware: answers `403` to a cross-site request with an unsafe method on
/// a `/ui` path, before the session gate or any handler runs.
pub(crate) async fn refuse_cross_site_writes(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    let unsafe_method = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if unsafe_method
        && request.uri().path().starts_with("/ui")
        && is_cross_site(request.headers(), &state.public_base_url)
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "cross-site request refused",
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::is_cross_site;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    const PUBLIC: &str = "https://fhir.example/fhir";

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn sec_fetch_site_decides_when_present() {
        let same = headers(&[("sec-fetch-site", "same-origin")]);
        assert!(!is_cross_site(&same, PUBLIC));
        assert!(!is_cross_site(
            &headers(&[("sec-fetch-site", "none")]),
            PUBLIC
        ));
        assert!(is_cross_site(
            &headers(&[("sec-fetch-site", "cross-site")]),
            PUBLIC
        ));
        assert!(is_cross_site(
            &headers(&[("sec-fetch-site", "same-site")]),
            PUBLIC
        ));
        let forged = headers(&[
            ("sec-fetch-site", "cross-site"),
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080"),
        ]);
        assert!(is_cross_site(&forged, PUBLIC));
    }

    #[test]
    fn origin_must_match_host_or_public_base_when_sec_fetch_site_is_absent() {
        let ok = |pairs: &[(&str, &str)]| !is_cross_site(&headers(pairs), PUBLIC);
        assert!(ok(&[]), "no browser headers: not a cross-site form post");
        assert!(ok(&[
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080")
        ]));
        assert!(ok(&[
            ("origin", "http://LOCALHOST:8080"),
            ("host", "localhost:8080")
        ]));
        assert!(
            ok(&[
                ("origin", "https://fhir.example"),
                ("host", "10.0.0.5:8080")
            ]),
            "proxy that rewrites Host: the public base URL's origin qualifies"
        );
        assert!(!ok(&[
            ("origin", "http://evil.example"),
            ("host", "localhost:8080")
        ]));
        assert!(!ok(&[
            ("origin", "http://localhost:8081"),
            ("host", "localhost:8080")
        ]));
        assert!(!ok(&[("origin", "null"), ("host", "localhost:8080")]));
        assert!(
            !ok(&[("origin", "http://fhir.example"), ("host", "10.0.0.5:8080")]),
            "scheme is part of the public origin"
        );
    }
}
