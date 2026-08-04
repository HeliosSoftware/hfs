//! Tenant context extractor.
//!
//! Extracts tenant information from multiple sources and creates
//! a TenantContext for use in handlers.
//!
//! # Sources
//!
//! Tenant can be identified from (in priority order):
//! 1. URL path prefix: `/{tenant}/Patient/123`
//! 2. X-Tenant-ID header
//! 3. JWT token claim (future)
//! 4. Default tenant from configuration
//!
//! # Configuration
//!
//! The routing mode is controlled by [`TenantRoutingMode`]:
//! - `HeaderOnly`: Only use header (default, backward compatible)
//! - `UrlPath`: Only use URL path prefix
//! - `Both`: Support both, URL takes precedence

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

use crate::state::AppState;
use crate::tenant::{ResolvedTenant, TenantResolver, TenantSource, TenantValidator};

/// Axum extractor for tenant context.
///
/// Extracts tenant information from multiple sources based on configuration
/// and creates a TenantContext with appropriate permissions.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::extractors::TenantExtractor;
///
/// async fn handler(tenant: TenantExtractor) {
///     println!("Tenant ID: {}", tenant.tenant_id());
///     println!("Source: {}", tenant.source());
///     if tenant.is_url_based() {
///         println!("Tenant was extracted from URL path");
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TenantExtractor {
    context: TenantContext,
    source: TenantSource,
    resolved: ResolvedTenant,
}

impl TenantExtractor {
    /// Creates a new TenantExtractor with the given tenant ID and source.
    pub fn new(tenant_id: &str, source: TenantSource) -> Self {
        let tenant_id_obj = TenantId::new(tenant_id);
        Self {
            context: TenantContext::new(tenant_id_obj.clone(), TenantPermissions::full_access()),
            source,
            resolved: ResolvedTenant {
                tenant_id: tenant_id_obj.clone(),
                source,
                all_sources: vec![(source, tenant_id_obj)],
            },
        }
    }

    /// Creates a TenantExtractor from a resolved tenant.
    pub fn from_resolved(resolved: ResolvedTenant) -> Self {
        Self {
            context: TenantContext::new(
                resolved.tenant_id.clone(),
                TenantPermissions::full_access(),
            ),
            source: resolved.source,
            resolved,
        }
    }

    /// Creates a TenantExtractor with the default tenant.
    pub fn default_tenant() -> Self {
        Self::new("default", TenantSource::Default)
    }

    /// Returns a reference to the tenant context.
    pub fn context(&self) -> &TenantContext {
        &self.context
    }

    /// Returns the tenant ID as a string.
    pub fn tenant_id(&self) -> &str {
        self.context.tenant_id().as_str()
    }

    /// Returns the source from which the tenant was resolved.
    pub fn source(&self) -> TenantSource {
        self.source
    }

    /// Returns true if the tenant was resolved from a URL path.
    pub fn is_url_based(&self) -> bool {
        self.source.is_url_based()
    }

    /// Returns true if the tenant is the default fallback.
    pub fn is_default(&self) -> bool {
        self.source.is_default()
    }

    /// Returns a reference to the full resolution information.
    pub fn resolved(&self) -> &ResolvedTenant {
        &self.resolved
    }

    /// Consumes the extractor and returns the tenant context.
    pub fn into_context(self) -> TenantContext {
        self.context
    }
}

impl std::fmt::Display for TenantExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.tenant_id())
    }
}

impl<S> FromRequestParts<AppState<S>> for TenantExtractor
where
    S: helios_persistence::core::ResourceStorage + Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<S>,
    ) -> Result<Self, Self::Rejection> {
        let extractor = Self::resolve(parts, state).await?;
        enforce_provisioned(state, &extractor).await?;
        Ok(extractor)
    }
}

/// Refuses a tenant id that is reserved for internal use (issue #317).
///
/// This is the **single** place the whole REST surface enforces the reservation.
/// `TenantExtractor::new` / `from_resolved` are constructed nowhere outside this
/// module and `TenantResolver` is consulted nowhere else, so every request-time
/// tenant — header, URL prefix, JWT claim, or configured default — passes
/// through [`TenantExtractor::resolve`] and therefore through here. Enforcing it
/// per-extractor instead would mean four denylists that must agree, which is the
/// shape of the bug being fixed.
///
/// `403` rather than `400`: `__system__` is a well-formed id, and the statement
/// being made is "you may not address this tenant", not "your request is
/// malformed". The same status is returned regardless of which door the id
/// arrived at, so an operator debugging a rejected client sees one behaviour.
///
/// The reason is logged at `WARN` with the offending id and its source, because
/// the most likely non-attack cause is a deployment that was reading the audit
/// trail through `X-Tenant-ID: __system__` and now needs to be repointed.
fn reject_reserved_tenant(id: &str, source: TenantSource) -> Result<(), (StatusCode, String)> {
    if !helios_persistence::tenant::TenantId::is_reserved(id) {
        return Ok(());
    }
    tracing::warn!(
        tenant = %id,
        source = ?source,
        "Rejected request naming a reserved internal tenant"
    );
    Err((
        StatusCode::FORBIDDEN,
        format!("tenant '{id}' is reserved for internal use and cannot be addressed by a request"),
    ))
}

impl TenantExtractor {
    /// Resolves the tenant for a request from JWT claim / header / URL / default,
    /// applying strict-validation consistency checks. Provisioned-tenant
    /// enforcement is applied separately by the extractor
    /// ([`enforce_provisioned`]).
    async fn resolve<S>(
        parts: &mut Parts,
        state: &AppState<S>,
    ) -> Result<Self, (StatusCode, String)>
    where
        S: helios_persistence::core::ResourceStorage + Send + Sync,
    {
        let config = state.config();

        // If auth is enabled and a Principal with a tenant_id is present,
        // use the JWT tenant authoritatively. When strict validation is enabled,
        // verify that the JWT tenant matches any header/URL tenant to prevent
        // confusion between URL context and actual data access.
        if let Some(principal) = parts.extensions.get::<helios_auth::Principal>() {
            if let Some(jwt_tenant) = principal.tenant_id().filter(|t| !t.is_empty()) {
                if config.multitenancy.strict_validation {
                    let resolver = TenantResolver::new(&config.multitenancy);
                    let resolved =
                        resolver.resolve(parts, &config.multitenancy, &config.default_tenant);
                    // If a non-default source provided a tenant, it must match the JWT
                    if !resolved.is_default() && resolved.tenant_id_str() != jwt_tenant {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            format!(
                                "Tenant mismatch: JWT tenant '{}' does not match request tenant '{}'",
                                jwt_tenant,
                                resolved.tenant_id_str()
                            ),
                        ));
                    }
                }
                // A reserved id is refused even though it came from a validated
                // token: the claim is authoritative for *which* tenant, not for
                // whether that tenant may be addressed at all. An issuer able to
                // set the tenant claim would otherwise reach the shared tenant.
                reject_reserved_tenant(jwt_tenant, TenantSource::JwtClaim)?;
                return Ok(TenantExtractor::new(
                    jwt_tenant,
                    crate::tenant::TenantSource::JwtClaim,
                ));
            }

            // A Principal is present (auth enabled + token validated) but the token
            // carries no (usable) tenant claim. We must NOT trust the client-supplied
            // X-Tenant-ID header or URL-path tenant here — doing so would let an
            // authenticated caller widen access to an arbitrary tenant by spoofing
            // the header.
            //
            // Determine what tenant the REQUEST asked for. If the client explicitly
            // selected a specific, non-default tenant via header/URL, fail loud: a
            // claimless token silently collapsing to the default would commingle
            // distinct tenants into one bucket in a multitenant-with-auth deployment.
            // If nothing (or only the default) was requested, fall back to the
            // configured default so single-tenant-with-auth deployments keep working.
            let resolver = TenantResolver::new(&config.multitenancy);
            let resolved = resolver.resolve(parts, &config.multitenancy, &config.default_tenant);
            // An empty resolved tenant means nothing real was requested — e.g. an
            // empty JWT tenant claim re-surfaced by the resolver's JwtTenantExtractor
            // (which runs even in HeaderOnly mode), with no header/URL tenant
            // supplied. Treat that as "no request" and fall back to the configured
            // default. Only a genuine, non-empty, non-default header/URL tenant
            // yields 403, preserving tenant isolation.
            if !resolved.tenant_id_str().is_empty()
                && resolved.tenant_id_str() != config.default_tenant
            {
                return Err((
                    StatusCode::FORBIDDEN,
                    "Authenticated token is missing the required tenant claim".to_string(),
                ));
            }
            // Fail closed on a misconfigured empty default tenant rather than
            // building an empty-tenant context (mirrors the resolver path below).
            if config.default_tenant.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "Invalid tenant ID".to_string()));
            }
            // `HFS_DEFAULT_TENANT` is rejected at startup by `ServerConfig::validate`,
            // so this is unreachable in the server binary. It is kept because
            // `AppState` can be built directly by an embedder or a test that
            // never calls `validate()`, and collapsing to a reserved default
            // would put every unauthenticated request in the shared tenant.
            reject_reserved_tenant(&config.default_tenant, TenantSource::Default)?;
            return Ok(TenantExtractor::new(
                &config.default_tenant,
                TenantSource::Default,
            ));
        }

        // Create resolver based on configuration
        let resolver = TenantResolver::new(&config.multitenancy);

        // Resolve tenant from request
        let resolved = resolver.resolve(parts, &config.multitenancy, &config.default_tenant);

        // Validate consistency if strict mode is enabled
        if config.multitenancy.strict_validation {
            TenantValidator::validate_consistency(&resolved).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Tenant validation error: {}", e),
                )
            })?;
        }

        // Validate tenant ID format
        if resolved.tenant_id_str().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Invalid tenant ID".to_string()));
        }

        // Covers the header, URL-path and (auth-disabled) JWT-claim doors in one
        // place, plus the configured default when nothing was requested.
        reject_reserved_tenant(resolved.tenant_id_str(), resolved.source)?;

        Ok(TenantExtractor::from_resolved(resolved))
    }
}

/// Rejects a resolved tenant that is not provisioned, when
/// `HFS_TENANT_REQUIRE_PROVISIONED` is set and the backend maintains a tenant
/// registry. Backends without a registry (mock stores, minimal deployments) are
/// never enforced, so the common single-tenant and test configurations are
/// unaffected.
///
/// This used to early-return `Ok` for the internal system tenant, which made the
/// one control an operator can switch on useless against precisely the id it
/// should have refused (issue #317). The exemption is gone:
/// [`reject_reserved_tenant`] now refuses reserved ids before this runs, and the
/// system tenant is never in the registry anyway, so a reserved id that somehow
/// reached here would correctly fail the provisioned check rather than be waved
/// through.
async fn enforce_provisioned<S>(
    state: &AppState<S>,
    extractor: &TenantExtractor,
) -> Result<(), (StatusCode, String)>
where
    S: helios_persistence::core::ResourceStorage + Send + Sync,
{
    if !state.config().multitenancy.require_provisioned_tenant {
        return Ok(());
    }
    let storage = state.storage();
    if !storage.supports_tenant_registry() {
        return Ok(());
    }
    let id = extractor.tenant_id();
    match storage.get_tenant(id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("Unknown tenant '{id}': tenants must be provisioned via the admin API"),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Tenant lookup failed: {e}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let extractor = TenantExtractor::new("test-tenant", TenantSource::Header);
        assert_eq!(extractor.tenant_id(), "test-tenant");
        assert_eq!(extractor.source(), TenantSource::Header);
        assert!(!extractor.is_url_based());
    }

    #[test]
    fn test_url_based() {
        let extractor = TenantExtractor::new("test-tenant", TenantSource::UrlPath);
        assert!(extractor.is_url_based());
    }

    #[test]
    fn test_default_tenant() {
        let extractor = TenantExtractor::default_tenant();
        assert_eq!(extractor.tenant_id(), "default");
        assert!(extractor.is_default());
    }

    #[test]
    fn test_display() {
        let extractor = TenantExtractor::new("my-tenant", TenantSource::Header);
        assert_eq!(format!("{}", extractor), "my-tenant");
    }

    // ── `FromRequestParts` resolution branches ──────────────────────────────
    //
    // These exercise the Principal-aware tenant resolution in
    // `TenantExtractor::from_request_parts`, which cannot be reached through the
    // header/URL integration tests (they run with auth disabled, so no
    // `Principal` is ever present in request extensions).

    mod from_request_parts {
        use std::sync::Arc;

        use axum::http::{HeaderValue, Request, StatusCode};
        use helios_auth::{Principal, ScopeSet};
        use helios_persistence::backends::sqlite::SqliteBackend;

        use super::super::*;
        use crate::config::{MultitenancyConfig, ServerConfig, TenantRoutingMode};

        /// Builds an in-memory-backed `AppState` with the given strict-validation
        /// flag and default tenant. Routing is `HeaderOnly` so the X-Tenant-ID
        /// header is honoured by the resolver.
        fn make_state(strict: bool, default_tenant: &str) -> AppState<SqliteBackend> {
            let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
            let config = ServerConfig {
                multitenancy: MultitenancyConfig {
                    routing_mode: TenantRoutingMode::HeaderOnly,
                    strict_validation: strict,
                    ..Default::default()
                },
                default_tenant: default_tenant.to_string(),
                ..ServerConfig::for_testing()
            };
            AppState::new(backend, config)
        }

        /// Builds request `Parts` for `/Patient/123`, optionally carrying an
        /// `X-Tenant-ID` header and/or a `Principal` extension.
        fn make_parts(tenant_header: Option<&str>, principal: Option<Principal>) -> Parts {
            let mut builder = Request::builder().uri("/Patient/123");
            if let Some(t) = tenant_header {
                builder = builder.header("x-tenant-id", HeaderValue::from_str(t).unwrap());
            }
            let mut parts = builder.body(()).unwrap().into_parts().0;
            if let Some(p) = principal {
                parts.extensions.insert(p);
            }
            parts
        }

        /// Builds a `Principal` with the given tenant claim and no scopes.
        fn make_principal(tenant_id: Option<&str>) -> Principal {
            Principal {
                subject: "test-subject".to_string(),
                issuer: "https://issuer.example".to_string(),
                tenant_id: tenant_id.map(|t| t.to_string()),
                scopes: ScopeSet::empty(),
                jti: None,
                expires_at: chrono::Utc::now(),
                custom_claims: serde_json::Map::new(),
            }
        }

        #[tokio::test]
        async fn test_jwt_tenant_claim_is_authoritative() {
            // Principal with a tenant claim → JwtClaim source, tenant from token.
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(None, Some(make_principal(Some("acme"))));

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("should resolve to JWT tenant");
            assert_eq!(extractor.tenant_id(), "acme");
            assert_eq!(extractor.source(), TenantSource::JwtClaim);
        }

        #[tokio::test]
        async fn test_jwt_claim_strict_matching_header_succeeds() {
            // strict_validation + matching header tenant → still JwtClaim.
            let state = make_state(true, "test-tenant");
            let mut parts = make_parts(Some("acme"), Some(make_principal(Some("acme"))));

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("matching JWT/header should succeed");
            assert_eq!(extractor.tenant_id(), "acme");
            assert_eq!(extractor.source(), TenantSource::JwtClaim);
        }

        #[tokio::test]
        async fn test_jwt_claim_strict_mismatching_header_is_bad_request() {
            // strict_validation + a non-default header tenant that disagrees with
            // the JWT tenant → 400.
            let state = make_state(true, "test-tenant");
            let mut parts = make_parts(Some("other"), Some(make_principal(Some("acme"))));

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("mismatch should be rejected");
            assert_eq!(err.0, StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn test_empty_claim_no_request_falls_back_to_default() {
            // An empty tenant claim is treated as "no claim"; with nothing else
            // requested, fall back to the configured default.
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(None, Some(make_principal(Some(""))));

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("empty claim should fall back to default");
            assert_eq!(extractor.tenant_id(), "test-tenant");
            assert_eq!(extractor.source(), TenantSource::Default);
        }

        #[tokio::test]
        async fn test_no_claim_no_request_falls_back_to_default() {
            // No tenant claim, no header → configured default tenant.
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(None, Some(make_principal(None)));

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("no claim + no request should use default");
            assert_eq!(extractor.tenant_id(), "test-tenant");
            assert_eq!(extractor.source(), TenantSource::Default);
        }

        #[tokio::test]
        async fn test_no_claim_nondefault_header_is_forbidden() {
            // Claimless token + client-selected non-default tenant via header → 403
            // (must not silently widen access to a spoofed tenant).
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(Some("other-tenant"), Some(make_principal(None)));

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("non-default requested tenant without claim should be forbidden");
            assert_eq!(err.0, StatusCode::FORBIDDEN);
        }

        #[tokio::test]
        async fn test_no_claim_empty_default_is_bad_request() {
            // Claimless token, nothing requested, but the configured default tenant
            // is empty (misconfiguration) → fail closed with 400.
            let state = make_state(false, "");
            let mut parts = make_parts(None, Some(make_principal(None)));

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("empty default tenant should fail closed");
            assert_eq!(err.0, StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn test_no_principal_resolves_header() {
            // No Principal at all → resolver path; header tenant is used.
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(Some("acme"), None);

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("resolver should use header tenant");
            assert_eq!(extractor.tenant_id(), "acme");
            assert_eq!(extractor.source(), TenantSource::Header);
        }

        #[tokio::test]
        async fn test_no_principal_no_header_uses_default() {
            // No Principal, no header → resolver falls back to the default tenant.
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(None, None);

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("resolver should fall back to default");
            assert_eq!(extractor.tenant_id(), "test-tenant");
            assert_eq!(extractor.source(), TenantSource::Default);
        }

        #[tokio::test]
        async fn test_no_principal_empty_default_is_bad_request() {
            // Resolver path with an empty resolved tenant (empty default) → 400.
            let state = make_state(false, "");
            let mut parts = make_parts(None, None);

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("empty resolved tenant should be rejected");
            assert_eq!(err.0, StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn test_jwt_claim_ignores_conflicting_header_non_strict() {
            // Non-strict mode: a valid JWT tenant claim is authoritative and a
            // conflicting X-Tenant-ID header is ignored (not honoured, not an error).
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(Some("other-tenant"), Some(make_principal(Some("acme"))));

            let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect("JWT claim should win over a conflicting header in non-strict mode");
            assert_eq!(extractor.tenant_id(), "acme");
            assert_eq!(extractor.source(), TenantSource::JwtClaim);
        }

        // ── #317: the internal system tenant is not addressable from a request ──

        /// Header door, auth disabled (no `Principal`) — the default deployment
        /// shape, where this needs no credentials at all.
        #[tokio::test]
        async fn test_system_tenant_via_header_is_forbidden() {
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(Some(helios_persistence::tenant::SYSTEM_TENANT), None);

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("__system__ must not be addressable via X-Tenant-ID");
            assert_eq!(err.0, StatusCode::FORBIDDEN);
        }

        /// JWT door. The claim is authoritative for *which* tenant, never for
        /// whether the shared tenant may be addressed — and `JwtTenantExtractor`
        /// validated nothing at all before this fix.
        #[tokio::test]
        async fn test_system_tenant_via_jwt_claim_is_forbidden() {
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(
                None,
                Some(make_principal(Some(
                    helios_persistence::tenant::SYSTEM_TENANT,
                ))),
            );

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("__system__ must not be addressable via a JWT tenant claim");
            assert_eq!(err.0, StatusCode::FORBIDDEN);
        }

        /// URL-path door. The router stores the extracted prefix in extensions,
        /// which is what `UrlPathTenantExtractor` reads first.
        #[tokio::test]
        async fn test_system_tenant_via_url_path_is_forbidden() {
            use crate::middleware::tenant_prefix::ExtractedTenantFromUrl;

            let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite backend"));
            let config = ServerConfig {
                multitenancy: MultitenancyConfig {
                    routing_mode: TenantRoutingMode::UrlPath,
                    ..Default::default()
                },
                default_tenant: "test-tenant".to_string(),
                ..ServerConfig::for_testing()
            };
            let state = AppState::new(backend, config);

            let mut parts = make_parts(None, None);
            parts.extensions.insert(ExtractedTenantFromUrl(
                helios_persistence::tenant::SYSTEM_TENANT.to_string(),
            ));

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("__system__ must not be addressable via a URL prefix");
            assert_eq!(err.0, StatusCode::FORBIDDEN);
        }

        /// The other reserved ids (the S3 control-plane namespaces) go through
        /// the same single authority, so they are refused identically.
        #[tokio::test]
        async fn test_other_reserved_ids_are_forbidden_too() {
            for &id in helios_persistence::tenant::RESERVED_TENANT_IDS {
                let state = make_state(false, "test-tenant");
                let mut parts = make_parts(Some(id), None);
                let err = TenantExtractor::from_request_parts(&mut parts, &state)
                    .await
                    .expect_err("reserved id must be refused");
                assert_eq!(err.0, StatusCode::FORBIDDEN, "reserved id {id}");
            }
        }

        /// An ordinary id that merely *looks* internal is still a valid tenant:
        /// the reservation is exact, not a `__` namespace ban, because tenant id
        /// is a partition key with no rename.
        #[tokio::test]
        async fn test_underscore_prefixed_tenants_still_resolve() {
            for id in ["__legacy", "__system", "_x"] {
                let state = make_state(false, "test-tenant");
                let mut parts = make_parts(Some(id), None);
                let extractor = TenantExtractor::from_request_parts(&mut parts, &state)
                    .await
                    .unwrap_or_else(|e| panic!("{id} should still resolve: {e:?}"));
                assert_eq!(extractor.tenant_id(), id);
            }
        }

        #[tokio::test]
        async fn test_empty_claim_nondefault_header_is_forbidden() {
            // An empty claim is treated as "no claim": a non-default requested
            // tenant still yields 403 (same fail-loud path as a missing claim).
            let state = make_state(false, "test-tenant");
            let mut parts = make_parts(Some("other-tenant"), Some(make_principal(Some(""))));

            let err = TenantExtractor::from_request_parts(&mut parts, &state)
                .await
                .expect_err("empty claim + non-default header should be forbidden");
            assert_eq!(err.0, StatusCode::FORBIDDEN);
        }
    }
}
