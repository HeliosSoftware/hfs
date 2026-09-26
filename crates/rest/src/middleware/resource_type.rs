//! Resource-type gate for the FHIR router (#989).
//!
//! Every type- and instance-level route is registered under a `{resource_type}`
//! (or `{compartment_type}`) path parameter, so any first path segment that no
//! static route claims reaches a FHIR handler as a resource type: `GET
//! /Patinet` ran an empty search and answered `200`, and a missing `/ui`
//! router presented as a healthy server for the same reason (#975).
//!
//! This layer is the single place that decides whether such a segment names a
//! resource type the server serves for the request's effective FHIR version.
//! It runs after routing (so it sees the matched template and never touches
//! `/metadata`, `/$export`, `/export-status/…` and the other static routes)
//! and before any handler, so read, search, history, write and operation paths
//! all refuse an unknown type the same way: `404 Not Found` with an
//! `OperationOutcome` whose issue code is `not-supported`, which is what
//! `http.html` prescribes for "resource type not supported" on every
//! interaction.
//!
//! The check is exact and case-sensitive (`observation` is not `Observation`),
//! and version-aware: `ActorDefinition` is refused under R4 and admitted under
//! R5, using the same effective version the handlers use — the `fhirVersion`
//! MIME parameter of `Content-Type` for writes and of `Accept` otherwise,
//! falling back to the server default.

use axum::{
    extract::{MatchedPath, Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;

use crate::error::RestError;
use crate::fhir_types::is_valid_resource_type_for_version;
use crate::middleware::content_type::{get_accept_fhir_version, get_content_type_fhir_version};
use crate::state::AppState;

/// Path parameters under which the FHIR router registers type-scoped routes.
const TYPE_PARAMETERS: &[&str] = &["{resource_type}", "{compartment_type}"];

/// Refuses a request whose route is type-scoped but whose type segment is not
/// a resource type for the request's effective FHIR version.
///
/// Installed with [`axum::middleware::from_fn_with_state`] on the FHIR router
/// (see `routing::fhir_routes`), so the matched route template is available in
/// the request extensions. Requests that matched no route, or a route whose
/// first segment is static, pass through untouched.
pub async fn reject_unknown_resource_type<S>(
    State(state): State<AppState<S>>,
    request: Request,
    next: Next,
) -> Response
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let type_scoped = request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|matched| is_type_scoped_route(matched.as_str()));
    if !type_scoped {
        return next.run(request).await;
    }

    let Some(resource_type) = first_segment(request.uri().path()) else {
        return next.run(request).await;
    };

    let version = effective_version(
        request.method(),
        request.headers(),
        state.config().default_fhir_version,
    );
    if is_valid_resource_type_for_version(&resource_type, version) {
        return next.run(request).await;
    }

    RestError::UnknownResourceType {
        resource_type,
        version,
    }
    .into_response()
}

/// Whether a matched route template starts with a resource-type parameter.
fn is_type_scoped_route(template: &str) -> bool {
    template
        .strip_prefix('/')
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|first| TYPE_PARAMETERS.contains(&first))
}

/// The first path segment, percent-decoded the way axum's `Path` extractor
/// will decode it for the handler, so `/Pat%69ent` is judged as `Patient`.
fn first_segment(path: &str) -> Option<String> {
    let raw = path.strip_prefix('/').unwrap_or(path).split('/').next()?;
    if raw.is_empty() {
        return None;
    }
    Some(percent_decode(raw))
}

/// Decodes `%XX` escapes; a malformed escape or non-UTF-8 result leaves the
/// segment as it was, which no resource type name will match anyway.
fn percent_decode(segment: &str) -> String {
    if !segment.contains('%') {
        return segment.to_string();
    }
    let hex = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            decoded.push(high << 4 | low);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| segment.to_string())
}

/// The FHIR version the request is resolved against: the `fhirVersion` MIME
/// parameter of `Content-Type` for writes and of `Accept` otherwise, mirroring
/// `FhirVersionExtractor::storage_version_or` / `accept_version_or`.
fn effective_version(
    method: &Method,
    headers: &axum::http::HeaderMap,
    default: FhirVersion,
) -> FhirVersion {
    let negotiated = if matches!(*method, Method::POST | Method::PUT | Method::PATCH) {
        get_content_type_fhir_version(headers).or_else(|| get_accept_fhir_version(headers))
    } else {
        get_accept_fhir_version(headers)
    };
    negotiated.unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    #[test]
    fn only_type_scoped_templates_are_gated() {
        for template in [
            "/{resource_type}",
            "/{resource_type}/{id}",
            "/{resource_type}/{id}/_history/{version_id}",
            "/{resource_type}/_search",
            "/{resource_type}/$validate",
            "/{compartment_type}/{compartment_id}/{target_type}",
        ] {
            assert!(is_type_scoped_route(template), "{template}");
        }
        for template in [
            "/metadata",
            "/_history",
            "/$export",
            "/Patient/$export",
            "/Group/{id}/$export",
            "/export-status/{job_id}",
            "/export/{job_id}/{filename}",
            "/OperationDefinition/{id}",
            "/ws/subscriptions/bind",
            "/.well-known/smart-configuration",
            "/",
        ] {
            assert!(!is_type_scoped_route(template), "{template}");
        }
    }

    #[test]
    fn first_segment_is_decoded_like_the_path_extractor() {
        assert_eq!(first_segment("/Patient/123").as_deref(), Some("Patient"));
        assert_eq!(first_segment("/Patient").as_deref(), Some("Patient"));
        assert_eq!(first_segment("/Pat%69ent").as_deref(), Some("Patient"));
        assert_eq!(first_segment("/Pat%ZZent").as_deref(), Some("Pat%ZZent"));
        assert_eq!(first_segment("/Patient%").as_deref(), Some("Patient%"));
        assert_eq!(first_segment("/"), None);
        assert_eq!(first_segment(""), None);
    }

    #[test]
    fn writes_negotiate_from_content_type_and_reads_from_accept() {
        let default = FhirVersion::default_enabled();
        let mut headers = HeaderMap::new();
        assert_eq!(effective_version(&Method::GET, &headers, default), default);
        assert_eq!(effective_version(&Method::POST, &headers, default), default);

        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.0"),
        );
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.0"),
        );
        assert_eq!(
            effective_version(&Method::GET, &headers, default),
            FhirVersion::R4
        );
        assert_eq!(
            effective_version(&Method::PUT, &headers, default),
            FhirVersion::R4
        );
    }
}
