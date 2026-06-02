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
| `HFS_AUTH_JTI_BACKEND` | `memory` | JTI cache backend (`memory`, `redis`, or `disabled`) |
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

#### Docker Quickstart

A pre-configured Keycloak 26 instance with a `fhir` realm, two test clients, and SMART client scopes is provided in `docker/keycloak/`.

```bash
# Start Keycloak (port 8180 to avoid conflict with HFS on 8080)
docker compose -f docker/keycloak/docker-compose.yml up -d

# Wait for health check to pass (~30-40s on first run)
docker compose -f docker/keycloak/docker-compose.yml ps

# Verify the realm is available
curl -s http://localhost:8180/realms/fhir | python3 -m json.tool | grep issuer
```

#### Pre-configured Test Clients

| Client ID | Secret | Default Scopes | Use case |
|-----------|--------|----------------|----------|
| `hfs-backend-client` | `hfs-backend-secret` | `system/*.cruds` | Full access testing |
| `hfs-readonly-client` | `hfs-readonly-secret` | `system/Patient.rs` | Restricted access testing |

#### Run HFS Against Keycloak

```bash
HFS_AUTH_ENABLED=true \
  HFS_AUTH_JWKS_URL=http://localhost:8180/realms/fhir/protocol/openid-connect/certs \
  HFS_AUTH_ISSUER=http://localhost:8180/realms/fhir \
  HFS_SMART_TOKEN_ENDPOINT=http://localhost:8180/realms/fhir/protocol/openid-connect/token \
  HFS_SMART_AUTHORIZE_ENDPOINT=http://localhost:8180/realms/fhir/protocol/openid-connect/auth \
  HFS_SMART_JWKS_URL=http://localhost:8180/realms/fhir/protocol/openid-connect/certs \
  cargo run --bin hfs
```

Verify SMART discovery is populated:
```bash
curl -s http://localhost:8080/.well-known/smart-configuration | python3 -m json.tool
```

#### Get a Token

Use the provided helper script:
```bash
# Full-access token (system/*.cruds) — prints token to stdout
export TOKEN=$(./docker/keycloak/get-token.sh)

# Read-only token (system/Patient.rs)
export READONLY_TOKEN=$(./docker/keycloak/get-token.sh hfs-readonly-client)
```

Or call Keycloak directly:
```bash
export TOKEN=$(curl -s -X POST \
  http://localhost:8180/realms/fhir/protocol/openid-connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  --data-urlencode "client_id=hfs-backend-client" \
  --data-urlencode "client_secret=hfs-backend-secret" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")
```

Inspect the token claims (no library needed):
```bash
echo $TOKEN | cut -d. -f2 | base64 -d 2>/dev/null | python3 -m json.tool
```

#### End-to-End Test

```bash
# Unauthenticated request → 401
curl -sv http://localhost:8080/Patient 2>&1 | grep "< HTTP"

# Authenticated request → 200 (or 404 if no data)
curl -s -H "Authorization: Bearer $TOKEN" http://localhost:8080/Patient | python3 -m json.tool

# Create a patient (requires 'c' permission — works with hfs-backend-client)
curl -s -X POST http://localhost:8080/Patient \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Patient","name":[{"family":"Smith","given":["John"]}]}' \
  | python3 -m json.tool

# Attempt write with read-only token → 403
READONLY_TOKEN=$(./docker/keycloak/get-token.sh hfs-readonly-client)
curl -sv -X POST http://localhost:8080/Patient \
  -H "Authorization: Bearer $READONLY_TOKEN" \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Patient"}' 2>&1 | grep "< HTTP"
```

#### Manual Realm Setup (Existing Keycloak)

If you have an existing Keycloak instance, configure it via the Admin Console or CLI:

1. **Create a realm** named `fhir` (or use an existing one).

2. **Create client scopes** for each SMART scope you need:
   - Admin Console → Client Scopes → Create → Name: `system/*.cruds` → Protocol: `openid-connect`
   - Set `Include in token scope` = `On` under Settings
   - Repeat for `system/Patient.rs`, `system/Observation.r`, etc.

3. **Create a client**:
   - Admin Console → Clients → Create
   - Client ID: `hfs-backend-client`
   - Client authentication: `On`
   - Standard flow: `Off`, Service accounts roles: `On`
   - Save → Credentials tab → note the client secret

4. **Assign scopes to the client**:
   - Client → Client Scopes tab → Add client scope → select your SMART scopes → Add as Default

5. **HFS environment variables**:

```bash
HFS_AUTH_JWKS_URL=https://{keycloak-host}/realms/{realm}/protocol/openid-connect/certs
HFS_AUTH_ISSUER=https://{keycloak-host}/realms/{realm}
# Scope claim: "scope" (space-delimited string) — no additional config needed
```

#### Keycloak Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| 401 on all requests | Wrong issuer | Confirm `HFS_AUTH_ISSUER` matches the `iss` in your decoded token exactly |
| 401 — signature validation failed | JWKS not yet cached | HFS fetches JWKS on first request; retry after ~5s or check `HFS_AUTH_JWKS_URL` is reachable |
| 403 on valid token | Scope not in token | Check `scope` claim in decoded token; ensure client scopes are assigned as *Default* not *Optional* |
| Token has no `scope` claim | Mapper missing | In Keycloak Admin → Client Scopes → verify `include.in.token.scope` is `true` for each scope |
| Realm import didn't apply | File not found | Check the volume mount path; Keycloak imports from `/opt/keycloak/data/import/` |

---

### Okta

Okta uses a custom Authorization Server for client credentials. A free developer account is available at [developer.okta.com](https://developer.okta.com).

#### Okta Setup (one-time)

1. **Create a Custom Authorization Server**
   - Admin Console → Security → API → Add Authorization Server
   - Name: `FHIR`, Audience: `https://fhir.example.com` (or your FHIR base URL)
   - Note the **Issuer URI** shown on the server's Settings tab

2. **Add SMART scopes to the Authorization Server**
   - Scopes tab → Add Scope for each scope you need:
     - Name: `system/*.cruds`, Description: Full SMART access, Default scope: off
     - Name: `system/Patient.rs`, Description: Read/search Patient
     - Name: `system/Observation.r`, Description: Read Observation
   - Leave Metadata published: on

3. **Create a Machine-to-Machine application**
   - Applications → Create App Integration → API Services (machine-to-machine)
   - Note the **Client ID** and **Client Secret** from the app's General tab

4. **Assign scopes to the application**
   - App → Okta API Scopes tab — this is for Okta management APIs, not your custom server
   - Instead: go to Security → API → your Authorization Server → Access Policies
   - Add a Policy → Add Rule → Grant type: Client Credentials, Client: your app, Scopes: the SMART scopes you created

#### Run HFS Against Okta

Replace `{domain}` with your Okta domain (e.g. `dev-12345678.okta.com`) and `{auth-server-id}` with the ID from the authorization server URL (e.g. `aus1abc2defGHIJK`):

```bash
HFS_AUTH_ENABLED=true \
  HFS_AUTH_JWKS_URL=https://{domain}/oauth2/{auth-server-id}/v1/keys \
  HFS_AUTH_ISSUER=https://{domain}/oauth2/{auth-server-id} \
  HFS_AUTH_AUDIENCE=https://fhir.example.com \
  HFS_SMART_TOKEN_ENDPOINT=https://{domain}/oauth2/{auth-server-id}/v1/token \
  cargo run --bin hfs
```

#### Get a Token

```bash
export TOKEN=$(curl -s -X POST \
  https://{domain}/oauth2/{auth-server-id}/v1/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -u "{client_id}:{client_secret}" \
  --data-urlencode "grant_type=client_credentials" \
  --data-urlencode "scope=system/*.cruds" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")
```

Or use the helper script (set env vars first):

```bash
OKTA_DOMAIN=dev-12345678.okta.com \
  OKTA_AUTH_SERVER_ID=aus1abc2defGHIJK \
  OKTA_CLIENT_ID=0oa... \
  OKTA_CLIENT_SECRET=... \
  ./docker/okta/get-token.sh
```

Decode the token to confirm the `scp` claim contains your SMART scopes and test:
```bash
echo $TOKEN | cut -d. -f2 | base64 -d 2>/dev/null | python3 -m json.tool | grep -E "scp|iss|aud"
curl -H "Authorization: Bearer <token>" http://localhost:8080/Patient/123
```

#### Okta Scope Claim Notes

Okta issues scopes as `scp` (a JSON array), not `scope` (a string). HFS auto-detects both formats — no extra configuration is needed.

#### Okta Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| 401 — issuer mismatch | Wrong auth server ID | The `iss` in the token must exactly match `HFS_AUTH_ISSUER`; check with the decode one-liner above |
| 401 — audience mismatch | `HFS_AUTH_AUDIENCE` set wrong | The `aud` claim in Okta tokens is the Authorization Server's audience value (set in step 1) |
| 403 — scope not granted | Access Policy missing | Verify the Access Policy rule explicitly lists your client and the SMART scopes |
| Empty `scp` in token | Scopes not requested | Include `scope=system/*.cruds` in the token request; Okta only grants requested scopes |

### Auth0

A free developer account is available at [auth0.com](https://auth0.com).

#### Auth0 Setup (one-time)

1. **Register an API** (this is Auth0's term for the resource server HFS represents)
   - Dashboard → Applications → APIs → Create API
   - Name: `FHIR Server`, Identifier: `https://fhir.example.com` (this becomes the `aud` claim)
   - Signing Algorithm: `RS256`
   - Save — Auth0 automatically creates a test M2M application for the API

2. **Add SMART scope permissions to the API**
   - Dashboard → Applications → APIs → your API → Permissions tab
   - Add each scope you need:
     - `system/*.cruds` — Full SMART access
     - `system/Patient.rs` — Read and search Patient
     - `system/Observation.r` — Read-only Observation

3. **Create a Machine-to-Machine application** (or use the auto-created test app)
   - Dashboard → Applications → Applications → Create Application
   - Choose **Machine to Machine Applications**
   - Select your FHIR API from the dropdown and authorize it
   - Select the scopes this client may request, then Authorize

4. **Note your credentials**
   - Application → Settings tab: copy **Domain**, **Client ID**, **Client Secret**

#### Run HFS Against Auth0

```bash
HFS_AUTH_ENABLED=true \
  HFS_AUTH_JWKS_URL=https://{domain}/.well-known/jwks.json \
  HFS_AUTH_ISSUER=https://{domain}/ \
  HFS_AUTH_AUDIENCE=https://fhir.example.com \
  HFS_SMART_TOKEN_ENDPOINT=https://{domain}/oauth/token \
  cargo run --bin hfs
```

#### Get a Token

```bash
export TOKEN=$(AUTH0_DOMAIN=... AUTH0_CLIENT_ID=... AUTH0_CLIENT_SECRET=... \
  ./docker/auth0/get-token.sh)
```

Or call Auth0 directly:
```bash
export TOKEN=$(curl -s -X POST \
  https://{domain}/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  --data-urlencode "client_id={client_id}" \
  --data-urlencode "client_secret={client_secret}" \
  --data-urlencode "audience=https://fhir.example.com" \
  --data-urlencode "scope=system/*.cruds" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")
```

Decode the token to confirm claims and test:
```bash
echo $TOKEN | cut -d. -f2 | base64 -d 2>/dev/null | python3 -m json.tool | grep -E "scope|iss|aud"
curl -H "Authorization: Bearer <token>" http://localhost:8080/Patient/123
```

#### Auth0 Scope Claim Notes

Auth0 tokens include granted scopes in the `scope` claim as a space-delimited string. Scopes only appear in the token if they were both defined as API permissions *and* requested in the token request.

#### Auth0 Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| 401 — issuer mismatch | Trailing slash | Auth0 issuer is `https://{domain}/` — the trailing slash is required |
| 401 — audience mismatch | Wrong identifier | `HFS_AUTH_AUDIENCE` must exactly match the API Identifier set in step 1 |
| 403 — scope not in token | Scope not authorized | Ensure the scope is added as an API Permission *and* the M2M app is authorized for it |
| Empty `scope` in token | Scope not requested | Include `scope=system/*.cruds` in the token request body |


### Microsoft Entra ID

```bash
HFS_AUTH_JWKS_URL=https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys
HFS_AUTH_ISSUER=https://login.microsoftonline.com/{tenant}/v2.0
# Permissions are typically in the "roles" claim
```

**Setup summary:** Register an application, define App Roles with SMART scope names, grant the client application the roles, and use the client credentials flow. Entra tokens include granted permissions in the `roles` array rather than `scope`.

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
