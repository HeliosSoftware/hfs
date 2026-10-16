//! Shared helpers for resolving FHIR references inside SQL-on-FHIR handlers.
//!
//! `$sql-run` accepts references in two
//! shapes: a relative `Type/{id}` and an absolute canonical URL (optionally
//! with a `|version` suffix). This module centralises the resolution so both
//! handlers stay in sync.

use futures::StreamExt;
use helios_persistence::core::SofError;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::error::{BackendError, StorageError};
use helios_persistence::sof::in_process::ResourceStream;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};
use serde_json::Value;
use tracing::warn;

use super::run::map_sof_error_to_rest;
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
            .map_err(|e| storage_lookup_error(format!("failed to read {resource_type}/{id}"), e))?
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

    let result = match state.storage().search(tenant, &query).await {
        Ok(result) => result,
        // A backend without a search index (standalone S3) cannot answer
        // `url=`. That used to surface as a 500 for every SQL View and SQL
        // Query Library — their `depends-on` entries are canonicals — and for
        // any `subjectCanonical` (#1228). Such a backend offers a scan of the
        // type instead; definitions are few, so matching them in process is
        // cheap.
        Err(StorageError::Backend(BackendError::UnsupportedCapability { .. })) => {
            return resolve_by_scan(state, tenant, resource_type, url).await;
        }
        Err(e) => {
            return Err(storage_lookup_error(
                format!("canonical lookup failed for {resource_type} url={url}"),
                e,
            ));
        }
    };

    // The search already filters by `url=`/`version=`, but a backend could in
    // principle return an approximate match (e.g. tokenized full-text search);
    // re-checking with the same [`canonical_matches`] predicate used for the
    // `context` list keeps storage and inline lookups provably consistent
    // rather than trusting two independently-implemented notions of "matches".
    let candidates: Vec<_> = result
        .resources
        .items
        .into_iter()
        .filter(|r| canonical_matches(r.content(), url))
        .collect();
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

/// Resolves a canonical on a backend that has no search, by scanning the
/// resource type and applying [`canonical_matches`] in process.
///
/// A backend with neither search nor scan cannot resolve canonicals at all;
/// that is a capability gap and answers `501` with the reason, never a `500`.
async fn resolve_by_scan<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    url: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let Some(scan) = state.storage().resource_scan() else {
        return Err(RestError::NotImplemented {
            feature: format!(
                "resolving the canonical reference '{url}': this storage backend has no search \
                 index, so canonical references cannot be resolved on it; reference the \
                 {resource_type} by id instead"
            ),
        });
    };
    let resources = scan
        .scan_resources(tenant, resource_type)
        .await
        .map_err(|e| canonical_scan_error(resource_type, url, e))?;
    newest_canonical_match(resources, url)
        .await
        .map_err(|e| canonical_scan_error(resource_type, url, e))?
        .ok_or_else(|| RestError::NotFound {
            resource_type: resource_type.to_string(),
            id: url.to_string(),
        })
}

/// The error for a failed storage read made while resolving a definition (by
/// id, or by canonical through a search): the read's kind is classified once
/// by [`SofError::from_storage`] and answered by the one SQL-on-FHIR mapping;
/// see [`lookup_error`].
fn storage_lookup_error(context: String, e: StorageError) -> RestError {
    lookup_error(context, SofError::from_storage(e))
}

/// The error for a canonical lookup whose scan failed; see [`lookup_error`].
fn canonical_scan_error(resource_type: &str, url: &str, e: SofError) -> RestError {
    lookup_error(
        format!("canonical lookup failed for {resource_type} url={url}"),
        e,
    )
}

/// Answers a failed definition lookup through the one SQL-on-FHIR error
/// mapping ([`map_sof_error_to_rest`], #1864), so a lookup gets the status a
/// run would for the same kind: an unavailable store `503` with `Retry-After`
/// and generic wording (#2048), a storage time limit `504`, any other fault
/// `500`. A definition that does not exist never reaches here: it is the
/// caller's `404`.
///
/// `context` (what was being looked up) is logged with the error. A `500`
/// also carries it in its message, as before, since it names only the
/// client's own reference; a `503` keeps the store's reason in the log only.
fn lookup_error(context: String, e: SofError) -> RestError {
    warn!(lookup = %context, error = %e, "SQL-on-FHIR definition lookup failed");
    map_sof_error_to_rest(match e {
        SofError::Storage(message) => SofError::Storage(format!("{context}: {message}")),
        SofError::Backend { sqlstate, message } => SofError::Backend {
            sqlstate,
            message: format!("{context}: {message}"),
        },
        other => other,
    })
}

/// Picks, among the scanned resources, the one [`canonical_matches`] selects
/// for `url` — the most recently updated when several versions match, which is
/// the rule the search path applies through `last_modified`.
///
/// Folds over the scan stream keeping only the running best, so a canonical
/// lookup on a search-less backend costs one resource of memory rather than
/// the whole resource type.
async fn newest_canonical_match(
    resources: ResourceStream,
    url: &str,
) -> Result<Option<Value>, SofError> {
    let mut resources = resources;
    let mut best: Option<(Option<chrono::DateTime<chrono::FixedOffset>>, Value)> = None;
    while let Some(resource) = resources.next().await {
        let resource = resource?;
        if !canonical_matches(&resource, url) {
            continue;
        }
        let stamp = resource
            .pointer("/meta/lastUpdated")
            .and_then(Value::as_str)
            .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok());
        // `>=` mirrors `max_by_key`, which keeps the last of equal keys.
        if best.as_ref().is_none_or(|(seen, _)| stamp >= *seen) {
            best = Some((stamp, resource));
        }
    }
    Ok(best.map(|(_, resource)| resource))
}

/// The one canonical-matching rule shared by storage lookups
/// ([`resolve_by_canonical_url`]) and matching against the inline `context`
/// list ([`super::graph::fetch_dependency`], [`super::view_sources`]): a
/// resource matches `query` when its own `url` element equals `query`'s
/// canonical part, and — when `query` pins a version via `|` or `@` — its own
/// `version` element equals that pinned version. A `query` with no version
/// pin matches any version. A resource with no `url` never matches anything,
/// since there is nothing to compare.
pub(super) fn canonical_matches(resource: &Value, query: &str) -> bool {
    let (canonical, version) = split_canonical_version(query);
    let Some(resource_url) = resource.get("url").and_then(|v| v.as_str()) else {
        return false;
    };
    if resource_url != canonical {
        return false;
    }
    match version {
        Some(v) => resource.get("version").and_then(|x| x.as_str()) == Some(v.as_str()),
        None => true,
    }
}

/// Splits `url|version` (preferred) or `url@version` (spec narrative form).
/// `@version` is recognised only when there is no `|` and the version
/// marker appears after the last `/`.
pub(super) fn split_canonical_version(url: &str) -> (String, Option<String>) {
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
    use super::{
        ResourceStream, canonical_matches, newest_canonical_match, resolve_by_canonical_url,
        split_canonical_version,
    };
    use crate::config::ServerConfig;
    use crate::error::RestError;
    use crate::state::AppState;
    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::search::{SearchProvider, SearchResult};
    use helios_persistence::core::sof_runner::SofError;
    use helios_persistence::error::{BackendError, StorageError, StorageResult};
    use helios_persistence::sof::in_process::ResourceScan;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_persistence::types::{SearchQuery, StoredResource};
    use serde_json::{Value, json};
    use std::sync::Arc;

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

    /// #1228: on a backend without search the canonical is matched over a scan.
    /// The pick must follow the search path's rules — a pinned version selects
    /// that version, an unpinned canonical the most recently updated match.
    #[tokio::test]
    async fn newest_canonical_match_follows_the_search_paths_rules() {
        let scanned = vec![
            json!({"resourceType": "ViewDefinition", "id": "other", "url": "http://example.org/other",
                   "meta": {"lastUpdated": "2026-09-17T12:00:00Z"}}),
            json!({"resourceType": "ViewDefinition", "id": "old", "url": "http://example.org/vd", "version": "1.0.0",
                   "meta": {"lastUpdated": "2026-09-01T00:00:00Z"}}),
            json!({"resourceType": "ViewDefinition", "id": "new", "url": "http://example.org/vd", "version": "2.0.0",
                   "meta": {"lastUpdated": "2026-09-10T00:00:00Z"}}),
        ];
        let pick = |url: &'static str| {
            let scanned = scanned.clone();
            async move {
                let stream: ResourceStream =
                    Box::pin(futures::stream::iter(scanned.into_iter().map(Ok)));
                newest_canonical_match(stream, url)
                    .await
                    .expect("scan must not error")
                    .and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_string))
            }
        };
        assert_eq!(pick("http://example.org/vd").await.as_deref(), Some("new"));
        assert_eq!(
            pick("http://example.org/vd|1.0.0").await.as_deref(),
            Some("old")
        );
        assert_eq!(
            pick("http://example.org/vd@2.0.0").await.as_deref(),
            Some("new")
        );
        assert_eq!(pick("http://example.org/vd|9.9.9").await, None);
        assert_eq!(pick("http://example.org/missing").await, None);
    }

    #[test]
    fn canonical_matches_a_bare_url_regardless_of_the_resource_version() {
        let resource = json!({"resourceType": "ViewDefinition", "url": "http://example.org/vd"});
        assert!(canonical_matches(&resource, "http://example.org/vd"));
        let versioned = json!({"resourceType": "ViewDefinition", "url": "http://example.org/vd", "version": "1.0.0"});
        assert!(canonical_matches(&versioned, "http://example.org/vd"));
    }

    #[test]
    fn canonical_matches_requires_the_pinned_version_to_match() {
        let resource = json!({
            "resourceType": "ViewDefinition",
            "url": "http://example.org/vd",
            "version": "1.0.0"
        });
        assert!(canonical_matches(&resource, "http://example.org/vd|1.0.0"));
        assert!(!canonical_matches(&resource, "http://example.org/vd|2.0.0"));
    }

    #[test]
    fn canonical_matches_rejects_a_resource_with_no_url() {
        let resource = json!({"resourceType": "ViewDefinition"});
        assert!(!canonical_matches(&resource, "http://example.org/vd"));
    }

    #[test]
    fn canonical_matches_rejects_a_different_canonical_url() {
        let resource = json!({"resourceType": "ViewDefinition", "url": "http://example.org/other"});
        assert!(!canonical_matches(&resource, "http://example.org/vd"));
    }

    /// How the stub answers `search`, standing in for a backend with (S3)
    /// and without a search index.
    enum Search {
        Unsupported,
        Broken,
        /// The store is down: searches and reads both fail as unavailable.
        Unavailable,
        /// The store hit its own time limit: searches and reads both fail
        /// with a backend timeout.
        TimedOut,
    }

    /// A storage that cannot search — the standalone S3 shape from #1228 —
    /// optionally offering a whole-type scan instead.
    struct ScanOnly {
        search: Search,
        scan: Option<Scan>,
    }

    /// What the stub's scan does: answer with these resources, or fail.
    #[derive(Clone)]
    enum Scan {
        Of(Vec<Value>),
        Broken,
        /// The store is unavailable when the scan starts.
        Unavailable,
        /// The store becomes unavailable partway through the scan's stream.
        UnavailableInStream,
        /// The scan hits a time limit when it starts.
        TimedOut,
        /// The scan hits a time limit partway through its stream.
        TimedOutInStream,
    }

    const UNAVAILABLE_TEXT: &str = "failed to read object body after 3 attempts";

    /// What a scan reports once its store has run out of attempts at an
    /// object read.
    fn unavailable_scan() -> SofError {
        SofError::Unavailable {
            message: UNAVAILABLE_TEXT.to_string(),
        }
    }

    const TIMEOUT_TEXT: &str = "statement_timeout after 30s";

    /// What a scan reports when it hits a time limit.
    fn timed_out_scan() -> SofError {
        SofError::Timeout {
            budget: None,
            message: TIMEOUT_TEXT.to_string(),
        }
    }

    /// What a store answers when a read hits its time limit.
    fn timed_out_store() -> StorageError {
        StorageError::Backend(BackendError::Timeout {
            backend_name: "scan-only".to_string(),
            message: TIMEOUT_TEXT.to_string(),
        })
    }

    /// What a store answers once it has run out of attempts at an object read.
    fn unavailable_store() -> StorageError {
        StorageError::Backend(BackendError::Unavailable {
            backend_name: "scan-only".to_string(),
            message: UNAVAILABLE_TEXT.to_string(),
        })
    }

    #[async_trait]
    impl ResourceStorage for ScanOnly {
        fn backend_name(&self) -> &'static str {
            "scan-only"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            match self.search {
                Search::Unavailable => Err(unavailable_store()),
                Search::TimedOut => Err(timed_out_store()),
                Search::Unsupported | Search::Broken => unimplemented!(),
            }
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }

        fn resource_scan(&self) -> Option<Arc<dyn ResourceScan>> {
            self.scan
                .clone()
                .map(|scan| Arc::new(scan) as Arc<dyn ResourceScan>)
        }
    }

    #[async_trait]
    impl SearchProvider for ScanOnly {
        async fn search(
            &self,
            _tenant: &TenantContext,
            _query: &SearchQuery,
        ) -> StorageResult<SearchResult> {
            Err(match self.search {
                Search::Unsupported => StorageError::Backend(BackendError::UnsupportedCapability {
                    backend_name: "scan-only".to_string(),
                    capability: "search".to_string(),
                }),
                Search::Broken => StorageError::Backend(BackendError::Internal {
                    backend_name: "scan-only".to_string(),
                    message: "index offline".to_string(),
                    source: None,
                }),
                Search::Unavailable => unavailable_store(),
                Search::TimedOut => timed_out_store(),
            })
        }

        async fn search_count(
            &self,
            _tenant: &TenantContext,
            _query: &SearchQuery,
        ) -> StorageResult<u64> {
            unimplemented!()
        }

        fn search_param_registry(
            &self,
            _tenant: &TenantContext,
        ) -> Arc<parking_lot::RwLock<helios_persistence::search::SearchParameterRegistry>> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl ResourceScan for Scan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
        ) -> Result<ResourceStream, SofError> {
            match self {
                Scan::Of(resources) => {
                    let matched: Vec<Value> = resources
                        .iter()
                        .filter(|r| {
                            r.get("resourceType").and_then(Value::as_str) == Some(resource_type)
                        })
                        .cloned()
                        .collect();
                    Ok(Box::pin(futures::stream::iter(matched.into_iter().map(Ok))))
                }
                Scan::Broken => Err(SofError::Storage("bucket unreachable".to_string())),
                Scan::Unavailable => Err(unavailable_scan()),
                Scan::UnavailableInStream => Ok(Box::pin(futures::stream::iter(vec![
                    Ok(json!({"resourceType": resource_type, "id": "first"})),
                    Err(unavailable_scan()),
                ]))),
                Scan::TimedOut => Err(timed_out_scan()),
                Scan::TimedOutInStream => Ok(Box::pin(futures::stream::iter(vec![
                    Ok(json!({"resourceType": resource_type, "id": "first"})),
                    Err(timed_out_scan()),
                ]))),
            }
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            ids: &[String],
        ) -> Result<Vec<Value>, SofError> {
            match self {
                Scan::Of(resources) => Ok(resources
                    .iter()
                    .filter(|r| {
                        r.get("resourceType").and_then(Value::as_str) == Some(resource_type)
                            && r.get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| ids.iter().any(|want| want == id))
                    })
                    .cloned()
                    .collect()),
                Scan::Broken => Err(SofError::Storage("bucket unreachable".to_string())),
                Scan::Unavailable | Scan::UnavailableInStream => Err(unavailable_scan()),
                Scan::TimedOut | Scan::TimedOutInStream => Err(timed_out_scan()),
            }
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    fn definitions() -> Vec<Value> {
        vec![
            json!({"resourceType": "ViewDefinition", "id": "old", "url": "http://example.org/vd",
                   "version": "1.0.0", "meta": {"lastUpdated": "2026-09-01T00:00:00Z"}}),
            json!({"resourceType": "ViewDefinition", "id": "new", "url": "http://example.org/vd",
                   "version": "2.0.0", "meta": {"lastUpdated": "2026-09-10T00:00:00Z"}}),
            json!({"resourceType": "Library", "id": "lib", "url": "http://example.org/vd"}),
        ]
    }

    async fn resolve(
        storage: ScanOnly,
        resource_type: &str,
        url: &str,
    ) -> Result<Value, RestError> {
        let state = AppState::new(Arc::new(storage), ServerConfig::default());
        resolve_by_canonical_url(&state, &tenant(), resource_type, url).await
    }

    fn id(resource: Value) -> String {
        resource["id"].as_str().unwrap().to_string()
    }

    /// #1228: when the backend refuses `url=` searches the canonical is
    /// resolved over the type's scan, honouring version pins and never
    /// crossing resource types.
    #[tokio::test]
    async fn unsupported_search_resolves_the_canonical_over_the_scan() {
        let scan_only = || ScanOnly {
            search: Search::Unsupported,
            scan: Some(Scan::Of(definitions())),
        };
        let picked = resolve(scan_only(), "ViewDefinition", "http://example.org/vd").await;
        assert_eq!(id(picked.unwrap()), "new");
        let pinned = resolve(scan_only(), "ViewDefinition", "http://example.org/vd|1.0.0").await;
        assert_eq!(id(pinned.unwrap()), "old");
        let library = resolve(scan_only(), "Library", "http://example.org/vd").await;
        assert_eq!(id(library.unwrap()), "lib");
    }

    #[tokio::test]
    async fn a_scan_without_a_match_is_not_found() {
        for (storage, url) in [
            (
                ScanOnly {
                    search: Search::Unsupported,
                    scan: Some(Scan::Of(definitions())),
                },
                "http://example.org/vd|9.9.9",
            ),
            (
                ScanOnly {
                    search: Search::Unsupported,
                    scan: Some(Scan::Of(Vec::new())),
                },
                "http://example.org/vd",
            ),
        ] {
            match resolve(storage, "ViewDefinition", url).await {
                Err(RestError::NotFound { resource_type, id }) => {
                    assert_eq!(resource_type, "ViewDefinition");
                    assert_eq!(id, url);
                }
                other => panic!("expected NotFound, got {other:?}"),
            }
        }
    }

    /// A backend with neither a search index nor a scan answers 501 with a
    /// reason, not a 500 (#1228).
    #[tokio::test]
    async fn unsupported_search_without_a_scan_is_not_implemented() {
        let storage = ScanOnly {
            search: Search::Unsupported,
            scan: None,
        };
        match resolve(storage, "Library", "http://example.org/lib").await {
            Err(RestError::NotImplemented { feature }) => {
                assert!(feature.contains("http://example.org/lib"), "{feature}");
                assert!(feature.contains("reference the Library by id"), "{feature}");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    /// A scan that fails is reported as the internal error it is, naming the
    /// canonical it was resolving.
    #[tokio::test]
    async fn a_failing_scan_is_an_internal_error() {
        let storage = ScanOnly {
            search: Search::Unsupported,
            scan: Some(Scan::Broken),
        };
        match resolve(storage, "ViewDefinition", "http://example.org/vd").await {
            Err(RestError::InternalError { message }) => {
                assert!(message.contains("bucket unreachable"), "{message}");
                assert!(message.contains("url=http://example.org/vd"), "{message}");
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }

    /// Asserts that `result` is the 503 the one SoF mapping gives an
    /// unavailable store: `Retry-After`, and the store's reason kept out of
    /// the client's diagnostics (#1864, A4).
    fn assert_unavailable_503(what: &str, result: Result<Value, RestError>) {
        match result {
            Err(rest @ RestError::ServiceUnavailable { .. }) => {
                let RestError::ServiceUnavailable { message } = &rest else {
                    unreachable!()
                };
                assert!(!message.contains(UNAVAILABLE_TEXT), "{what}: {message}");
                let response = axum::response::IntoResponse::into_response(rest);
                assert_eq!(
                    response.status(),
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "{what}"
                );
                assert!(
                    response
                        .headers()
                        .contains_key(axum::http::header::RETRY_AFTER),
                    "{what}: a 503 must carry Retry-After"
                );
            }
            other => panic!("{what}: expected ServiceUnavailable, got {other:?}"),
        }
    }

    /// Asserts that `result` is the 504 the one SoF mapping gives a storage
    /// time limit, never a 500.
    fn assert_timeout_504(what: &str, result: Result<Value, RestError>) {
        match result {
            Err(rest @ RestError::SofTimeout { .. }) => {
                let response = axum::response::IntoResponse::into_response(rest);
                assert_eq!(
                    response.status(),
                    axum::http::StatusCode::GATEWAY_TIMEOUT,
                    "{what}"
                );
            }
            other => panic!("{what}: expected SofTimeout, got {other:?}"),
        }
    }

    /// #2048: a scan whose store is unavailable, at its start or partway
    /// through its stream, is a retryable 503, answered by the one SoF mapping
    /// (#1864): `Retry-After`, the store's reason logged rather than sent.
    #[tokio::test]
    async fn an_unavailable_scan_is_a_503() {
        for scan in [Scan::Unavailable, Scan::UnavailableInStream] {
            let storage = ScanOnly {
                search: Search::Unsupported,
                scan: Some(scan),
            };
            let result = resolve(storage, "ViewDefinition", "http://example.org/vd").await;
            assert_unavailable_503("scan", result);
        }
    }

    /// #2048: a definition read, by id or by canonical, from a store that is
    /// unavailable is a retryable 503, not a 500, answered by the one SoF
    /// mapping (#1864).
    #[tokio::test]
    async fn an_unavailable_store_is_a_503_for_a_definition_lookup() {
        for reference in ["ViewDefinition/abc", "http://example.org/vd"] {
            let storage = ScanOnly {
                search: Search::Unavailable,
                scan: None,
            };
            let result = resolve(storage, "ViewDefinition", reference).await;
            assert_unavailable_503(reference, result);
        }
    }

    /// #1864: a definition read, by id or by canonical, that hits the store's
    /// time limit is the 504 a run gets for the same kind, not a 500.
    #[tokio::test]
    async fn a_timed_out_store_is_a_504_for_a_definition_lookup() {
        for reference in ["ViewDefinition/abc", "http://example.org/vd"] {
            let storage = ScanOnly {
                search: Search::TimedOut,
                scan: None,
            };
            let result = resolve(storage, "ViewDefinition", reference).await;
            assert_timeout_504(reference, result);
        }
    }

    /// #1864: a canonical scan that hits a time limit, at its start or partway
    /// through its stream, is a 504, not a 500.
    #[tokio::test]
    async fn a_timed_out_scan_is_a_504() {
        for scan in [Scan::TimedOut, Scan::TimedOutInStream] {
            let storage = ScanOnly {
                search: Search::Unsupported,
                scan: Some(scan),
            };
            let result = resolve(storage, "ViewDefinition", "http://example.org/vd").await;
            assert_timeout_504("scan", result);
        }
    }

    /// Only the missing-capability error falls back; any other search failure
    /// is still reported as the internal error it is.
    #[tokio::test]
    async fn other_search_failures_stay_internal_errors() {
        let storage = ScanOnly {
            search: Search::Broken,
            scan: Some(Scan::Of(definitions())),
        };
        match resolve(storage, "ViewDefinition", "http://example.org/vd").await {
            Err(RestError::InternalError { message }) => {
                assert!(message.contains("index offline"), "{message}");
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }
}
