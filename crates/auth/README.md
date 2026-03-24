# helios-auth

Authentication and authorization for the Helios FHIR Server.

## Overview

This crate provides [SMART Backend Services](https://hl7.org/fhir/smart-app-launch/backend-services.html) authentication via JWT/JWKS validation and SMART v2 scope-based authorization. It is designed around a key architectural principle: **HFS does not act as an authorization server.** Token issuance and client registration remain external (Keycloak, Okta, Auth0, Entra ID, etc.). This crate performs local token validation only.

- **JWKS-Based JWT Validation**: Fetches and caches public keys from IdP JWKS endpoints
- **SMART v2 Scope Parsing**: Parses `system/Patient.rs`, `system/*.cruds` scope syntax
- **Scope-Based Authorization**: Maps FHIR operations to SMART permissions (CRUDS)
- **JTI Replay Prevention**: In-memory and Redis-backed caches for JWT ID tracking
- **Multi-Instance Coordination**: Redis leader election for JWKS refresh across scaled deployments
- **SMART Discovery**: Builds `/.well-known/smart-configuration` documents
- **Pluggable Audit**: Trait-based audit event sink (noop default, extensible)

## Quick Start

```rust
use std::sync::Arc;
use helios_auth::{
    AuthConfig, JwksBearerAuthProvider, JwksCache,
    InMemoryJtiCache, AuthProvider,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AuthConfig {
        enabled: true,
        jwks_url: Some("https://idp.example.com/.well-known/jwks.json".to_string()),
        expected_issuer: Some("https://idp.example.com".to_string()),
        expected_audience: Some("https://fhir.example.com".to_string()),
        ..AuthConfig::default()
    };

    // Create caches
    let jwks_cache = Arc::new(JwksCache::new(
        config.jwks_url.as_ref().unwrap(),
        config.jwks_min_refresh_interval,
    ));
    jwks_cache.initial_fetch().await?;

    let jti_cache = Arc::new(InMemoryJtiCache::new());

    // Create provider
    let provider = JwksBearerAuthProvider::new(jwks_cache, jti_cache, &config);

    // Validate a token
    match provider.authenticate("Bearer eyJhbGciOi...").await {
        Ok(principal) => println!("Authenticated: {}", principal.subject()),
        Err(e) => println!("Auth failed: {}", e),
    }

    Ok(())
}
```

## How Authentication Works

The authentication flow follows the [SMART Backend Services](https://hl7.org/fhir/smart-app-launch/backend-services.html) protocol:

1. **Client registers** with an external authorization server (Keycloak, Okta, etc.)
2. **Client obtains a token** from the authorization server using its private key
3. **Client sends request** to HFS with `Authorization: Bearer <token>`
4. **HFS validates the token locally**:
   - Decodes the JWT header to extract `kid` and `alg`
   - Rejects tokens using algorithms not in the allowed list
   - Fetches the public key from the cached JWKS keyset (refreshes on unknown `kid`)
   - Validates signature, expiration, issuer, and audience claims
   - Checks the `jti` claim against the replay prevention cache
   - Parses SMART v2 scopes from the `scope` or `scp` claim
   - Extracts the tenant ID from the configured claim
5. **HFS enforces authorization** by checking scopes against the requested FHIR operation

## SMART v2 Scopes

Scopes follow the SMART v2 syntax: `context/resourceType.permissions`

| Scope | Meaning |
|-------|---------|
| `system/Patient.rs` | Read and search Patient resources |
| `system/*.cruds` | Full CRUD + search on all resource types |
| `system/Observation.r` | Read-only access to Observation |
| `system/Condition.crud` | Create, read, update, delete Condition (no search) |
| `user/Patient.rs` | User-level read/search on Patient |

Permission characters: `c` = create, `r` = read, `u` = update, `d` = delete, `s` = search.

### Operation-to-Permission Mapping

| HTTP Request | FHIR Operation | Required Permission |
|-------------|---------------|-------------------|
| `GET /Patient/123` | read | `r` |
| `GET /Patient?name=Smith` | search | `s` |
| `POST /Patient` | create | `c` |
| `PUT /Patient/123` | update | `u` |
| `PATCH /Patient/123` | update | `u` |
| `DELETE /Patient/123` | delete | `d` |
| `GET /Patient/_history` | history | `r` |

## Configuration

All configuration is via environment variables. Auth is a runtime toggle — no feature flags needed to enable it.

### Core Settings

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_AUTH_ENABLED` | `false` | Master switch for authentication |
| `HFS_AUTH_JWKS_URL` | *(required)* | JWKS endpoint URL |
| `HFS_AUTH_ISSUER` | *(none)* | Expected JWT `iss` claim |
| `HFS_AUTH_AUDIENCE` | *(none)* | Expected JWT `aud` claim (**recommended for production** — prevents accepting tokens intended for other services) |
| `HFS_AUTH_TENANT_CLAIM` | `tenant_id` | JWT claim name for tenant ID |
| `HFS_AUTH_ALGORITHMS` | `RS256,RS384,ES256,ES384` | Allowed signing algorithms |

### Caching and Replay Prevention

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_AUTH_JTI_BACKEND` | `memory` | JTI cache backend (`memory` or `redis`) |
| `HFS_AUTH_REDIS_URL` | *(none)* | Redis URL (required for `redis` backend) |
| `HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL` | `10` | Min seconds between JWKS refreshes |

### SMART Discovery Endpoint

These populate the `GET /.well-known/smart-configuration` response:

| Variable | Description |
|----------|-------------|
| `HFS_SMART_TOKEN_ENDPOINT` | Token endpoint URL |
| `HFS_SMART_AUTHORIZE_ENDPOINT` | Authorization endpoint URL |
| `HFS_SMART_JWKS_URL` | JWKS URL (for discovery doc; falls back to `HFS_AUTH_JWKS_URL`) |
| `HFS_SMART_INTROSPECTION_ENDPOINT` | Token introspection endpoint |
| `HFS_SMART_MANAGEMENT_ENDPOINT` | Token management endpoint |
| `HFS_SMART_REGISTRATION_ENDPOINT` | Dynamic client registration endpoint |
| `HFS_SMART_REVOCATION_ENDPOINT` | Token revocation endpoint |

## Running with Authentication

```bash
# Enable auth with Keycloak
HFS_AUTH_ENABLED=true \
  HFS_AUTH_JWKS_URL=http://keycloak:8080/realms/fhir/protocol/openid-connect/certs \
  HFS_AUTH_ISSUER=http://keycloak:8080/realms/fhir \
  HFS_AUTH_AUDIENCE=https://fhir.example.com \
  HFS_SMART_TOKEN_ENDPOINT=http://keycloak:8080/realms/fhir/protocol/openid-connect/token \
  cargo run --bin hfs

# Verify SMART discovery
curl http://localhost:8080/.well-known/smart-configuration

# Unauthenticated request (expect 401)
curl -v http://localhost:8080/Patient

# Authenticated request
curl -H "Authorization: Bearer <token>" http://localhost:8080/Patient/123
```

### Exempt Paths

These endpoints are always accessible without a token:

- `/health`, `/_liveness`, `/_readiness`
- `/metadata`
- `/.well-known/smart-configuration`
- `/$versions`

## Identity Provider Integration

### Keycloak

```bash
HFS_AUTH_JWKS_URL=http://keycloak:8080/realms/{realm}/protocol/openid-connect/certs
HFS_AUTH_ISSUER=http://keycloak:8080/realms/{realm}
# Scope claim: "scope" (space-delimited string)
```

### Okta

```bash
HFS_AUTH_JWKS_URL=https://{domain}/oauth2/{auth-server}/v1/keys
HFS_AUTH_ISSUER=https://{domain}/oauth2/{auth-server}
# Scope claim: "scp" (JSON array) — both formats are auto-detected
```

### Auth0

```bash
HFS_AUTH_JWKS_URL=https://{domain}/.well-known/jwks.json
HFS_AUTH_ISSUER=https://{domain}/
HFS_AUTH_AUDIENCE=https://fhir.example.com
```

### Microsoft Entra ID

```bash
HFS_AUTH_JWKS_URL=https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys
HFS_AUTH_ISSUER=https://login.microsoftonline.com/{tenant}/v2.0
# Permissions are typically in the "roles" claim
```

## Tenant Resolution

When authentication is enabled, the tenant ID is derived **exclusively** from the JWT claim configured by `HFS_AUTH_TENANT_CLAIM` (default: `tenant_id`). The `X-Tenant-ID` header and URL-based tenant routing are ignored for authenticated requests — this prevents tenant impersonation.

If the token does not contain the tenant claim, the server falls back to the standard tenant resolution (header, URL path, or default).

## Multi-Instance Deployments

For HFS deployments with multiple instances behind a load balancer:

```bash
# Use Redis for JTI replay prevention (shared across instances)
HFS_AUTH_JTI_BACKEND=redis
HFS_AUTH_REDIS_URL=redis://redis:6379

# Build with Redis support
cargo build -p helios-hfs --features redis
```

The Redis backend also coordinates JWKS refresh across instances using leader election, so only one instance fetches from the IdP's JWKS endpoint at a time.

## Features

| Feature | Description |
|---------|-------------|
| `redis` | Enables Redis-backed JTI cache and JWKS refresh coordination |

## Testing

```bash
# Run all auth tests
cargo test -p helios-auth

# Run specific test module
cargo test -p helios-auth scope
cargo test -p helios-auth policy
cargo test -p helios-auth jti
```

## Architecture

```
src/
├── lib.rs              # Crate entry, re-exports
├── config.rs           # AuthConfig (env var parsing)
├── error.rs            # AuthError enum, FhirOperation
├── principal.rs        # Principal (authenticated identity)
├── audit.rs            # AuditEventSink trait + NoopAuditEventSink
├── discovery.rs        # SmartConfiguration builder
├── scope/
│   ├── mod.rs          # ScopeSet (collection of parsed scopes)
│   ├── smart_v2.rs     # SmartScope parser (context/type.perms)
│   └── permissions.rs  # SmartPermissions bitflags (CRUDS)
├── provider/
│   ├── mod.rs          # AuthProvider trait
│   └── jwks_bearer.rs  # JwksBearerAuthProvider (JWT validation)
├── jwks/
│   ├── mod.rs          # Module exports
│   ├── fetcher.rs      # HTTP JWKS fetcher with Cache-Control parsing
│   ├── cache.rs        # JwksCache (background refresh, rate limiting)
│   └── coordinator.rs  # Redis leader election (feature = "redis")
├── jti/
│   ├── mod.rs          # JtiCache trait
│   ├── memory.rs       # InMemoryJtiCache (moka)
│   └── redis.rs        # RedisJtiCache (feature = "redis")
└── policy/
    └── mod.rs          # SmartScopePolicy (operation → permission check)
```

### Key Types

| Type | Description |
|------|-------------|
| `Principal` | Authenticated identity from a validated JWT (subject, issuer, scopes, tenant) |
| `ScopeSet` | Parsed collection of SMART v2 scopes with permission checking |
| `SmartPermissions` | Bitflags for CRUDS permissions |
| `AuthProvider` | Trait for token validation (currently: JWKS Bearer) |
| `JwksCache` | JWKS key cache with Cache-Control awareness and background refresh |
| `JtiCache` | Trait for JWT ID replay prevention (in-memory or Redis) |
| `SmartScopePolicy` | Checks principal scopes against FHIR operations |
| `AuditEventSink` | Trait for recording auth events (noop default) |
| `AuthConfig` | Configuration from environment variables |
| `SmartConfiguration` | SMART discovery document builder |

## License

MIT
