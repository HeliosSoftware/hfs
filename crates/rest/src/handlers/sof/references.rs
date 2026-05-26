//! Shared helpers for resolving FHIR references inside SQL-on-FHIR handlers.
//!
//! `$viewdefinition-run` and `$sqlquery-run` both accept references in two
//! shapes: a relative `Type/{id}` and an absolute canonical URL (optionally
//! with a `|version` suffix). This module centralises the resolution so both
//! handlers stay in sync.

use helios_persistence::core::search::SearchProvider;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};
use serde_json::Value;

use crate::error::RestError;
use crate::state::AppState;

/// Resolves a canonical-or-relative reference for the given resource type.
///
/// Accepts:
/// - `Type/{id}` — relative reference, served by `ResourceStorage::read`.
/// - absolute canonical URL (`http(s)://…`, optionally `…|version` or
///   `…@version`) — resolved via `SearchProvider::search` with `url=` (and
///   `version=` when supplied), picking the newest match by
///   `meta.lastUpdated`.
///
/// The SQL-on-FHIR spec narrative shows `@version` (e.g.
/// `http://example.org/ViewDefinition/abc@1.0.0`) while standard FHIR uses
/// `|version`. We accept both — `|` takes precedence; `@version` is
/// recognised only when no `|` is present and the segment after the last
/// `/` contains an `@`.
pub(super) async fn resolve_resource_canonical_or_relative<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let trimmed = reference.trim();
    let prefix = format!("{resource_type}/");
    if let Some(rest) = trimmed.strip_prefix(prefix.as_str()) {
        let id = rest.split('/').next().unwrap_or("");
        if id.is_empty() {
            return Err(RestError::BadRequest {
                message: format!("'{reference}' has an empty id after '{resource_type}/'"),
            });
        }
        let stored = state
            .storage()
            .read(tenant, resource_type, id)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("failed to read {resource_type}: {e}"),
            })?
            .ok_or_else(|| RestError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })?;
        return Ok(stored.content().clone());
    }
    resolve_by_canonical_url(state, tenant, resource_type, trimmed).await
}

async fn resolve_by_canonical_url<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    url: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let (canonical, version) = split_canonical_version(url);

    let mut query = SearchQuery::new(resource_type);
    query.parameters.push(SearchParameter {
        name: "url".to_string(),
        param_type: SearchParamType::Uri,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, canonical)],
        chain: Vec::new(),
        components: Vec::new(),
    });
    if let Some(v) = version {
        query.parameters.push(SearchParameter {
            name: "version".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, v)],
            chain: Vec::new(),
            components: Vec::new(),
        });
    }

    let result =
        state
            .storage()
            .search(tenant, &query)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("canonical lookup failed for {resource_type} url={url}: {e}"),
            })?;

    let candidates: Vec<_> = result.resources.items.into_iter().collect();
    if candidates.is_empty() {
        // SoF v2 spec maps "Library or ViewDefinition not found" to 404.
        // Use the full canonical URL (including any |version) as the
        // identifier in the NotFound payload so the client sees what we
        // tried to resolve.
        return Err(RestError::NotFound {
            resource_type: resource_type.to_string(),
            id: url.to_string(),
        });
    }
    let chosen = candidates
        .into_iter()
        .max_by_key(|r| r.last_modified())
        .ok_or_else(|| RestError::InternalError {
            message: "unreachable: candidates was non-empty".into(),
        })?;
    Ok(chosen.content().clone())
}

/// Splits `url|version` (preferred) or `url@version` (spec narrative form).
/// `@version` is recognised only when there is no `|` and the version
/// marker appears after the last `/`.
fn split_canonical_version(url: &str) -> (String, Option<String>) {
    if let Some((u, v)) = url.split_once('|') {
        return (u.to_string(), Some(v.to_string()));
    }
    if let Some(last_slash) = url.rfind('/') {
        if let Some(at_offset) = url[last_slash..].rfind('@') {
            let split_at = last_slash + at_offset;
            let (u, v) = url.split_at(split_at);
            // skip the '@'
            return (u.to_string(), Some(v[1..].to_string()));
        }
    }
    (url.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::split_canonical_version;

    #[test]
    fn bare_url_has_no_version() {
        let (u, v) = split_canonical_version("http://example.org/ViewDefinition/abc");
        assert_eq!(u, "http://example.org/ViewDefinition/abc");
        assert!(v.is_none());
    }

    #[test]
    fn pipe_version_takes_precedence() {
        let (u, v) = split_canonical_version("http://example.org/ViewDefinition/abc|1.0.0");
        assert_eq!(u, "http://example.org/ViewDefinition/abc");
        assert_eq!(v.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn at_version_from_spec_narrative() {
        let (u, v) = split_canonical_version("http://example.org/ViewDefinition/abc@1.0.0");
        assert_eq!(u, "http://example.org/ViewDefinition/abc");
        assert_eq!(v.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn at_before_last_slash_is_not_version() {
        // The @ is in the host segment, not in the final path segment.
        let (u, v) = split_canonical_version("http://user@host.example/ViewDefinition/abc");
        assert_eq!(u, "http://user@host.example/ViewDefinition/abc");
        assert!(v.is_none());
    }

    #[test]
    fn pipe_wins_over_at() {
        let (u, v) = split_canonical_version("http://example.org/ViewDefinition/x@y|2.0");
        assert_eq!(u, "http://example.org/ViewDefinition/x@y");
        assert_eq!(v.as_deref(), Some("2.0"));
    }
}
