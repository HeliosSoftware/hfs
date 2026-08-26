//! Tenant resolution from multiple sources.
//!
//! Provides the [`TenantResolver`] which extracts tenant information from
//! requests using multiple configurable sources.

use axum::http::request::Parts;
use helios_fhir::FhirVersion;
use helios_persistence::tenant::{TenantId, TenantIdError};
use tracing::{debug, warn};

use crate::config::{MultitenancyConfig, TenantRoutingMode};
use crate::middleware::tenant::X_TENANT_ID;
use crate::middleware::tenant_prefix::{ExtractedTenantFromUrl, OriginalPath, is_reserved_path};

use super::source::TenantSource;

/// Result of resolving a tenant from a request.
#[derive(Debug, Clone)]
pub struct ResolvedTenant {
    /// The resolved tenant ID.
    pub tenant_id: TenantId,
    /// The source from which the tenant was resolved.
    pub source: TenantSource,
    /// All sources that provided a tenant ID (for validation).
    pub all_sources: Vec<(TenantSource, TenantId)>,
}

impl ResolvedTenant {
    /// Returns true if the tenant was resolved from a URL path.
    pub fn is_url_based(&self) -> bool {
        self.source.is_url_based()
    }

    /// Returns true if the tenant was the default fallback.
    pub fn is_default(&self) -> bool {
        self.source.is_default()
    }

    /// Returns the tenant ID as a string reference.
    pub fn tenant_id_str(&self) -> &str {
        self.tenant_id.as_str()
    }
}

/// Trait for extracting tenant information from a specific source.
pub trait TenantSourceExtractor: Send + Sync {
    /// Attempts to extract a tenant ID from the request.
    ///
    /// The three-way return is the whole point, and the distinction is easy to
    /// collapse by accident:
    ///
    /// - `Ok(Some(id))` — this source asserted a tenant and it is valid.
    /// - `Ok(None)` — this source asserted **nothing**. Fall through to the next
    ///   source, and ultimately to the default tenant.
    /// - `Err(_)` — this source asserted a tenant and it is **invalid**. The
    ///   request must be rejected, not silently redirected somewhere else.
    ///
    /// Before issue #385 there was no `Err` arm: an invalid `X-Tenant-ID` was
    /// filtered to `None`, which fell through to the default tenant, so a client
    /// typo silently read and wrote *another tenant's* data under a `200`.
    ///
    /// [`UrlPathTenantExtractor`] is the deliberate exception — for it, "not a
    /// valid tenant id" genuinely means "not a tenant prefix" (`/Patient/123`),
    /// so it returns `Ok(None)` and never `Err`.
    fn extract(
        &self,
        parts: &Parts,
        config: &MultitenancyConfig,
    ) -> Result<Option<TenantId>, TenantSourceError>;

    /// Returns the source type this extractor handles.
    fn source_type(&self) -> TenantSource;
}

/// A tenant id asserted by a request source that failed canonical validation.
///
/// Carries the source so the caller can pick the right status: a bad header is
/// the client's fault (`400`), a bad JWT claim is an unusable authorization
/// context (`403`).
#[derive(Debug, Clone)]
pub struct TenantSourceError {
    /// Which source carried the invalid id.
    pub source: TenantSource,
    /// Why it was rejected.
    pub error: TenantIdError,
}

impl std::fmt::Display for TenantSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for TenantSourceError {}

/// Lets a whole-id reserved tenant through the source extractors unparsed.
///
/// `TenantId::parse` refuses a reserved id (issue #385: it is a reserved
/// *segment*), but refusing it *here* would answer the wrong thing. Refusal for
/// a reserved id is `TenantExtractor`'s job and is a `403` with a specific
/// message — "you may not address this tenant" — whereas a rejection at this
/// layer is a `400`/"malformed id", or, on the URL door, silently no tenant at
/// all. Issue #317 chose the `403`, one status at one door, so an operator who
/// pointed a client at `X-Tenant-ID: __system__` reads why.
///
/// So: extract it, let it reach `reject_reserved_tenant`. Only the exact
/// reserved ids get this pass; `acme/__system__` is still a parse error, which
/// is the shape #385 added the per-segment check for.
fn reserved_id_passthrough(raw: &str) -> Option<TenantId> {
    TenantId::is_reserved(raw).then(|| TenantId::new(raw))
}

/// Extracts tenant from URL path prefix.
///
/// First checks for a tenant extracted by the middleware (stored in extensions).
/// If not found, falls back to parsing the original URL path.
#[derive(Debug, Default)]
pub struct UrlPathTenantExtractor;

impl TenantSourceExtractor for UrlPathTenantExtractor {
    /// Never returns `Err`: a first path segment that is not a valid tenant id
    /// is not a *malformed tenant*, it is an ordinary untenanted path
    /// (`/Patient/123`). Turning that into an error would 400 every non-tenanted
    /// request. See [`TenantSourceExtractor::extract`].
    fn extract(
        &self,
        parts: &Parts,
        _config: &MultitenancyConfig,
    ) -> Result<Option<TenantId>, TenantSourceError> {
        // First, check if middleware already extracted the tenant. The
        // middleware applies the same canonical check before storing it, so this
        // is trusted reconstruction.
        if let Some(ExtractedTenantFromUrl(tenant)) =
            parts.extensions.get::<ExtractedTenantFromUrl>()
        {
            return Ok(Some(TenantId::new(tenant)));
        }

        // Fall back to checking the original path (if stored) or current path
        let path = if let Some(OriginalPath(original)) = parts.extensions.get::<OriginalPath>() {
            original.as_str()
        } else {
            parts.uri.path()
        };

        // Skip leading slash and get first segment
        let path = path.strip_prefix('/').unwrap_or(path);

        // Get the first segment (before the next slash or end of path)
        let Some(tenant) = path.split('/').next() else {
            return Ok(None);
        };

        // Skip reserved paths that are not tenant identifiers
        // Use the default FHIR version for resource type checking
        let fhir_version = FhirVersion::default_enabled();
        if is_reserved_path(tenant, &fhir_version) {
            return Ok(None);
        }

        // A URL prefix is a single path segment by construction, so a
        // hierarchical id can never appear here — `acme/research` arrives as the
        // segment `acme` followed by a path. Hierarchical tenants are therefore
        // addressable by header and JWT claim, but not by URL-prefix routing.
        if let Some(reserved) = reserved_id_passthrough(tenant) {
            return Ok(Some(reserved));
        }
        Ok(TenantId::parse(tenant).ok())
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::UrlPath
    }
}

/// Extracts tenant from X-Tenant-ID header.
#[derive(Debug, Default)]
pub struct HeaderTenantExtractor;

impl TenantSourceExtractor for HeaderTenantExtractor {
    /// A present-but-invalid header is an error, not an absent tenant.
    ///
    /// This used to `filter` an invalid value away to `None`, which made the
    /// resolver fall through to the **default tenant** — so `X-Tenant-ID:
    /// acme.corp` (a `.`, which the old header charset rejected) silently read
    /// and wrote the default tenant's data and returned `200`. That is a
    /// cross-tenant read/write triggered by a typo. It is now a `400`.
    ///
    /// An absent header, and a header whose bytes are not valid UTF-8 or ASCII
    /// (`to_str()` fails), remain `Ok(None)`: neither asserts a tenant.
    fn extract(
        &self,
        parts: &Parts,
        _config: &MultitenancyConfig,
    ) -> Result<Option<TenantId>, TenantSourceError> {
        let Some(raw) = parts
            .headers
            .get(&X_TENANT_ID)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(None);
        };
        // An empty header is treated as "not sent" rather than rejected, which
        // is what it meant before and what a client emitting an unset variable
        // produces.
        if raw.is_empty() {
            return Ok(None);
        }
        if let Some(reserved) = reserved_id_passthrough(raw) {
            return Ok(Some(reserved));
        }
        TenantId::parse(raw).map(Some).map_err(|error| {
            debug!(error = %error, "Rejecting invalid X-Tenant-ID header");
            TenantSourceError {
                source: TenantSource::Header,
                error,
            }
        })
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::Header
    }
}

/// Extracts tenant from a validated JWT's [`Principal`] in request extensions.
///
/// The auth middleware inserts a [`Principal`] into request extensions after
/// successful token validation. This extractor reads the tenant ID from
/// the principal's configured claim.
#[derive(Debug, Default)]
pub struct JwtTenantExtractor;

impl TenantSourceExtractor for JwtTenantExtractor {
    /// The claim is **authoritative** — [`crate::extractors::TenantExtractor`]
    /// prefers it over header and URL — and it used to be validated by nothing
    /// at all, making it the one path that could put an arbitrary string into a
    /// storage key. It is now held to the same charset as every other source.
    ///
    /// An empty claim stays `Ok(None)`: the extractor already treats "empty" as
    /// "no claim" and has a dedicated fail-loud path for it.
    fn extract(
        &self,
        parts: &Parts,
        _config: &MultitenancyConfig,
    ) -> Result<Option<TenantId>, TenantSourceError> {
        let Some(raw) = parts
            .extensions
            .get::<helios_auth::Principal>()
            .and_then(|p| p.tenant_id())
            .filter(|t| !t.is_empty())
        else {
            return Ok(None);
        };
        if let Some(reserved) = reserved_id_passthrough(raw) {
            return Ok(Some(reserved));
        }
        TenantId::parse(raw).map(Some).map_err(|error| {
            warn!(error = %error, "Rejecting invalid tenant claim in validated token");
            TenantSourceError {
                source: TenantSource::JwtClaim,
                error,
            }
        })
    }

    fn source_type(&self) -> TenantSource {
        TenantSource::JwtClaim
    }
}

/// Resolves tenant information from multiple sources.
pub struct TenantResolver {
    extractors: Vec<Box<dyn TenantSourceExtractor>>,
}

impl TenantResolver {
    /// Creates a new TenantResolver based on the multitenancy configuration.
    pub fn new(config: &MultitenancyConfig) -> Self {
        let mut extractors: Vec<Box<dyn TenantSourceExtractor>> = Vec::new();

        // Add extractors based on routing mode (in priority order)
        match config.routing_mode {
            TenantRoutingMode::HeaderOnly => {
                extractors.push(Box::new(HeaderTenantExtractor));
            }
            TenantRoutingMode::UrlPath => {
                extractors.push(Box::new(UrlPathTenantExtractor));
            }
            TenantRoutingMode::Both => {
                // URL path has higher priority, so it's checked first
                extractors.push(Box::new(UrlPathTenantExtractor));
                extractors.push(Box::new(HeaderTenantExtractor));
            }
        }

        // Always add JWT extractor (for future use)
        extractors.push(Box::new(JwtTenantExtractor));

        Self { extractors }
    }

    /// Creates a resolver with all extractors (for testing).
    #[cfg(test)]
    pub fn with_all_extractors() -> Self {
        Self {
            extractors: vec![
                Box::new(UrlPathTenantExtractor),
                Box::new(HeaderTenantExtractor),
                Box::new(JwtTenantExtractor),
            ],
        }
    }

    /// Resolves the tenant from the request.
    ///
    /// Returns a [`ResolvedTenant`] with the tenant ID and source information,
    /// or [`TenantSourceError`] when a source **asserted** a tenant id that fails
    /// canonical validation.
    ///
    /// An invalid assertion aborts resolution rather than being skipped. Skipping
    /// it would hand the request to the next source — in practice the default
    /// tenant — which is exactly the silent cross-tenant fallback issue #385
    /// exists to remove. The first invalid source wins, in priority order, so the
    /// reported reason is the one the caller most likely meant.
    pub fn resolve(
        &self,
        parts: &Parts,
        config: &MultitenancyConfig,
        default_tenant: &str,
    ) -> Result<ResolvedTenant, TenantSourceError> {
        let mut all_sources = Vec::new();

        // Try each extractor in priority order
        for extractor in &self.extractors {
            if let Some(tenant_id) = extractor.extract(parts, config)? {
                all_sources.push((extractor.source_type(), tenant_id));
            }
        }

        // Select the highest priority source that provided a tenant
        if let Some((source, tenant_id)) = all_sources.first().cloned() {
            Ok(ResolvedTenant {
                tenant_id,
                source,
                all_sources,
            })
        } else {
            // Fall back to default tenant. Constructed unchecked: it is operator
            // configuration, not request input, and is validated once at startup
            // (see `ServerConfig::validate_default_tenant`) so a misconfiguration
            // is a boot failure rather than a per-request rejection.
            Ok(ResolvedTenant {
                tenant_id: TenantId::new(default_tenant),
                source: TenantSource::Default,
                all_sources,
            })
        }
    }
}

impl Default for TenantResolver {
    fn default() -> Self {
        Self::new(&MultitenancyConfig::default())
    }
}

// The local `is_valid_tenant_id` that used to live here is gone: it was one of
// three divergent charsets (issue #385). Validation is now `TenantId::parse`,
// the single canonical definition in `helios-persistence`.

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Request, Uri};
    use helios_persistence::tenant::MAX_TENANT_ID_LEN;

    fn make_parts(path: &str, tenant_header: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri(Uri::try_from(path).unwrap());

        if let Some(tenant) = tenant_header {
            builder = builder.header(&X_TENANT_ID, HeaderValue::from_str(tenant).unwrap());
        }

        let request = builder.body(()).unwrap();
        request.into_parts().0
    }

    /// `Ok(Some(id))` unwrapped, for the many assertions that only care about
    /// the resolved value.
    fn extracted(
        extractor: &dyn TenantSourceExtractor,
        parts: &Parts,
        config: &MultitenancyConfig,
    ) -> Option<String> {
        extractor
            .extract(parts, config)
            .expect("extractor must not reject")
            .map(|t| t.as_str().to_string())
    }

    #[test]
    fn test_url_path_extractor() {
        let extractor = UrlPathTenantExtractor;
        let config = MultitenancyConfig::default();

        // Valid tenant in URL
        let parts = make_parts("/acme/Patient/123", None);
        assert_eq!(extracted(&extractor, &parts, &config), Some("acme".into()));

        // Reserved path (should not extract)
        let parts = make_parts("/Patient/123", None);
        assert_eq!(extracted(&extractor, &parts, &config), None);

        // System endpoint (should not extract)
        let parts = make_parts("/metadata", None);
        assert_eq!(extracted(&extractor, &parts, &config), None);
    }

    /// The URL extractor must never return `Err`, or every untenanted request
    /// would 400. A first segment that is not a valid tenant id simply is not a
    /// tenant prefix.
    #[test]
    fn url_path_extractor_never_errors_on_a_non_tenant_segment() {
        let extractor = UrlPathTenantExtractor;
        let config = MultitenancyConfig::default();

        for path in [
            "/Patient/123",
            "/metadata",
            "/resources/x",  // reserved segment
            "/__system__/x", // reserved sentinel
            "/a%20b/Patient",
        ] {
            let parts = make_parts(path, None);
            assert!(
                extractor.extract(&parts, &config).is_ok(),
                "{path} must fall through, not reject"
            );
        }
    }

    #[test]
    fn test_header_extractor() {
        let extractor = HeaderTenantExtractor;
        let config = MultitenancyConfig::default();

        // Valid header
        let parts = make_parts("/Patient/123", Some("acme"));
        assert_eq!(extracted(&extractor, &parts, &config), Some("acme".into()));

        // Missing header
        let parts = make_parts("/Patient/123", None);
        assert_eq!(extracted(&extractor, &parts, &config), None);

        // Empty header — "not sent", not "malformed".
        let parts = make_parts("/Patient/123", Some(""));
        assert_eq!(extracted(&extractor, &parts, &config), None);
    }

    /// Regression for the silent cross-tenant fallback (issue #385).
    ///
    /// A present-but-invalid `X-Tenant-ID` used to be filtered away to `None`,
    /// so the resolver fell through to the **default tenant** and the request
    /// was served — a `200` against someone else's data because of a typo.
    #[test]
    fn invalid_header_is_rejected_not_silently_defaulted() {
        let extractor = HeaderTenantExtractor;
        let config = MultitenancyConfig::default();

        let too_long = "a".repeat(MAX_TENANT_ID_LEN + 1);
        for bad in [
            "acme corp",
            "acme/",
            "acme/__system__",
            "acme/resources",
            too_long.as_str(),
        ] {
            let parts = make_parts("/Patient/123", Some(bad));
            let err = extractor
                .extract(&parts, &config)
                .expect_err(&format!("{bad:?} must be rejected"));
            assert_eq!(err.source, TenantSource::Header);
        }

        // A *whole-id* reserved tenant is the one exception: it extracts here so
        // `TenantExtractor` can answer 403 "may not be addressed" rather than
        // 400 "malformed" (issue #317). See `reserved_id_passthrough`.
        for reserved in ["__system__", "resources"] {
            let parts = make_parts("/Patient/123", Some(reserved));
            assert_eq!(
                extractor
                    .extract(&parts, &config)
                    .expect("a reserved id extracts, and is refused one layer up")
                    .as_ref()
                    .map(TenantId::as_str),
                Some(reserved)
            );
        }
    }

    /// The whole-resolver view of the same regression: the invalid header must
    /// abort resolution rather than let the default tenant win.
    #[test]
    fn resolver_rejects_invalid_header_instead_of_falling_back_to_default() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        let parts = make_parts("/Patient/123", Some("acme corp"));
        let err = resolver
            .resolve(&parts, &config, "default")
            .expect_err("an invalid asserted tenant must not resolve to the default");
        assert_eq!(err.source, TenantSource::Header);
    }

    /// `.` was accepted by the admin API but by neither routing validator, so a
    /// tenant provisioned as `tenant.example` was registered and unreachable.
    /// The canonical charset is the union, which closes that gap.
    #[test]
    fn header_accepts_dotted_ids_the_admin_api_could_already_provision() {
        let extractor = HeaderTenantExtractor;
        let config = MultitenancyConfig::default();

        let parts = make_parts("/Patient/123", Some("tenant.example"));
        assert_eq!(
            extracted(&extractor, &parts, &config),
            Some("tenant.example".into())
        );
    }

    /// Hierarchical ids reach storage by header (and JWT claim), never by URL
    /// prefix — a URL prefix is one path segment by construction.
    #[test]
    fn hierarchical_ids_route_by_header_but_not_by_url_prefix() {
        let config = MultitenancyConfig::default();

        let parts = make_parts("/Patient/123", Some("acme/research"));
        assert_eq!(
            extracted(&HeaderTenantExtractor, &parts, &config),
            Some("acme/research".into())
        );

        // The URL form yields the first segment only.
        let parts = make_parts("/acme/research/Patient", None);
        assert_eq!(
            extracted(&UrlPathTenantExtractor, &parts, &config),
            Some("acme".into())
        );
    }

    #[test]
    fn test_resolver_header_only() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Header provided
        let parts = make_parts("/Patient/123", Some("acme"));
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::Header);

        // No header - falls back to default
        let parts = make_parts("/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "default");
        assert_eq!(resolved.source, TenantSource::Default);
    }

    #[test]
    fn test_resolver_url_path() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::UrlPath,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Tenant in URL
        let parts = make_parts("/acme/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::UrlPath);

        // No tenant in URL (reserved path) - falls back to default
        let parts = make_parts("/Patient/123", None);
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "default");
        assert_eq!(resolved.source, TenantSource::Default);
    }

    #[test]
    fn test_resolver_both_url_precedence() {
        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        // Both URL and header - URL wins
        let parts = make_parts("/acme/Patient/123", Some("other"));
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::UrlPath);
        assert_eq!(resolved.all_sources.len(), 2);

        // Only header (reserved URL path)
        let parts = make_parts("/Patient/123", Some("acme"));
        let resolved = resolver.resolve(&parts, &config, "default").expect("valid");
        assert_eq!(resolved.tenant_id_str(), "acme");
        assert_eq!(resolved.source, TenantSource::Header);
    }

    #[test]
    fn test_is_reserved_path() {
        let version = FhirVersion::default();

        // System endpoints
        assert!(is_reserved_path("metadata", &version));
        assert!(is_reserved_path("health", &version));

        // FHIR resource types (case insensitive)
        assert!(is_reserved_path("Patient", &version));
        assert!(is_reserved_path("PATIENT", &version));
        assert!(is_reserved_path("patient", &version));

        // Tenant IDs should NOT be reserved
        assert!(!is_reserved_path("acme", &version));
        assert!(!is_reserved_path("tenant-123", &version));
    }

    #[test]
    fn test_reserved_paths_includes_all_resource_types() {
        let version = FhirVersion::default();

        // These were missing from the old hardcoded list
        assert!(is_reserved_path("Provenance", &version));
        assert!(is_reserved_path("provenance", &version)); // case insensitive
        assert!(is_reserved_path("AuditEvent", &version));
        assert!(is_reserved_path("Binary", &version));
        assert!(is_reserved_path("OperationOutcome", &version));
        assert!(is_reserved_path("Bundle", &version));
        assert!(is_reserved_path("Parameters", &version));

        // Common resources from the old list still work
        assert!(is_reserved_path("Patient", &version));
        assert!(is_reserved_path("Observation", &version));
        assert!(is_reserved_path("Condition", &version));
        assert!(is_reserved_path("Encounter", &version));
        assert!(is_reserved_path("Medication", &version));

        // Tenant IDs should NOT be reserved
        assert!(!is_reserved_path("acme", &version));
        assert!(!is_reserved_path("tenant-123", &version));
        assert!(!is_reserved_path("my_tenant", &version));
    }

    /// The resolver deliberately still *extracts* a reserved id (#317).
    ///
    /// Refusal happens one layer up, in `TenantExtractor`, which is the single
    /// door every request-time tenant passes through. Teaching the resolver (or
    /// `is_valid_tenant_id`) to drop reserved ids instead would silently change
    /// what the authorization classifier sees for `/__system__/Patient/123`:
    /// with no tenant extracted it falls back to the raw path, declines the
    /// leading `_` segment, and skips the SMART scope check altogether. See
    /// `middleware::auth::test_url_routing_reserved_tenant_still_classifies_for_scope_check`.
    ///
    /// If you are here because you want to reject earlier: reject in
    /// `TenantExtractor`, not here.
    #[test]
    fn test_resolver_still_extracts_reserved_ids_for_consistent_classification() {
        use helios_persistence::tenant::SYSTEM_TENANT;

        let config = MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        };
        let resolver = TenantResolver::new(&config);

        let parts = make_parts("/Patient/123", Some(SYSTEM_TENANT));
        let resolved = resolver
            .resolve(&parts, &config, "default")
            .expect("a reserved id must extract, not fail validation");
        assert_eq!(resolved.tenant_id_str(), SYSTEM_TENANT);
        assert_eq!(resolved.source, TenantSource::Header);

        let parts = make_parts("/__system__/Patient/123", None);
        let resolved = resolver
            .resolve(&parts, &config, "default")
            .expect("a reserved id must extract, not fail validation");
        assert_eq!(resolved.tenant_id_str(), SYSTEM_TENANT);
        assert_eq!(resolved.source, TenantSource::UrlPath);
    }

    /// Replaces `test_is_valid_tenant_id`, which exercised this module's private
    /// copy of the charset. That copy is gone (issue #385) — validation is
    /// `TenantId::parse` — so the property worth pinning here is that the
    /// *header source* accepts exactly what the canonical validator accepts.
    ///
    /// Two of the old assertions were inverted by this change, deliberately:
    /// `tenant.com` and any dotted id are now **valid** (the admin API could
    /// always provision one, so rejecting it here made such a tenant
    /// unroutable). `tenant/path` stays invalid *for this source* only because a
    /// header carrying it is valid — see
    /// `hierarchical_ids_route_by_header_but_not_by_url_prefix` — while a URL
    /// prefix is a single segment and can never carry it.
    #[test]
    fn header_source_accepts_exactly_the_canonical_charset() {
        let extractor = HeaderTenantExtractor;
        let config = MultitenancyConfig::default();
        let too_long = "a".repeat(MAX_TENANT_ID_LEN + 1);

        for candidate in [
            "acme",
            "tenant-123",
            "my_tenant",
            "ABC123",
            "tenant.com",
            "acme/research",
            "acme/resources",
            "acme/__system__",
            "tenant path",
            too_long.as_str(),
        ] {
            let parts = make_parts("/Patient/123", Some(candidate));
            assert_eq!(
                extractor.extract(&parts, &config).is_ok(),
                TenantId::parse(candidate).is_ok(),
                "{candidate:?} classified differently from the canonical validator"
            );
        }

        // Whole-id reserved tenants are deliberately *not* held to this
        // equivalence — they extract here and are refused with a 403 in
        // `TenantExtractor` (issue #317, `reserved_id_passthrough`).
        for reserved in ["resources", "__system__"] {
            assert!(TenantId::parse(reserved).is_err());
            let parts = make_parts("/Patient/123", Some(reserved));
            assert!(extractor.extract(&parts, &config).is_ok());
        }
    }
}
