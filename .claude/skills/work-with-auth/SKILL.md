---
name: work-with-auth
description: Work on HFS authentication & authorization. Use for SMART-on-FHIR/OAuth2, JWT bearer validation, JWKS, scopes/permissions, the JTI replay cache, SMART discovery, and HFS_AUTH_* configuration.
---

# Authentication & Authorization

Use this when working in `helios-auth` or HFS auth behavior. The crate validates SMART-on-FHIR / OAuth2 JWT bearer tokens and enforces scopes. It is consumed by `helios-rest`, `helios-hfs`, and `helios-subscriptions`.

## Behavior

- Disabled by default. Set `HFS_AUTH_ENABLED=true` to require bearer tokens.
- Validates JWTs against a JWKS endpoint (`HFS_AUTH_JWKS_URL`), checking issuer, audience, and signing algorithm.
- Enforces SMART v2 scopes (e.g. `system/Patient.rs`) via `SmartScopePolicy` / `ScopeSet` / `SmartPermissions`.
- Derives the tenant from a JWT claim (`HFS_AUTH_TENANT_CLAIM`, default `tenant_id`).
- Replay protection via a JTI cache (in-memory or Redis).
- Serves `/.well-known/smart-configuration` (SMART discovery), populated from the `HFS_SMART_*` endpoints.

## Environment

| Variable | Default | Description |
|---|---|---|
| `HFS_AUTH_ENABLED` | `false` | Require bearer-token auth |
| `HFS_AUTH_JWKS_URL` | none | JWKS endpoint for token signature verification |
| `HFS_AUTH_ISSUER` | none | Expected `iss` |
| `HFS_AUTH_AUDIENCE` | none | Expected `aud` |
| `HFS_AUTH_ALGORITHMS` | `RS256,RS384,ES256,ES384` | Allowed signing algorithms |
| `HFS_AUTH_TENANT_CLAIM` | `tenant_id` | JWT claim used to resolve the tenant |
| `HFS_AUTH_JTI_BACKEND` | `memory` | JTI replay cache: `memory` or `redis` |
| `HFS_AUTH_REDIS_URL` | none | Redis URL when JTI backend is `redis` |
| `HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL` | `10` | Minimum seconds between JWKS refreshes |

SMART discovery passthrough (advertised in `/.well-known/smart-configuration`): `HFS_SMART_TOKEN_ENDPOINT`, `HFS_SMART_AUTHORIZE_ENDPOINT`, `HFS_SMART_JWKS_URL`, `HFS_SMART_INTROSPECTION_ENDPOINT`, `HFS_SMART_MANAGEMENT_ENDPOINT`, `HFS_SMART_REGISTRATION_ENDPOINT`, `HFS_SMART_REVOCATION_ENDPOINT`. Outbound calls can carry `HFS_OUTBOUND_BEARER_TOKEN`.

## JTI replay cache

`HFS_AUTH_JTI_BACKEND=memory` (default, per-process) or `redis`. The Redis backend needs `HFS_AUTH_REDIS_URL` and the crate built with `--features redis`, giving a replay cache shared across instances.

## Key API

`AuthConfig`, `AuthProvider` / `JwksBearerAuthProvider`, `Principal`, `ScopeSet` / `SmartPermissions`, `SmartScopePolicy`, `JtiCache` (`InMemoryJtiCache` / `RedisJtiCache`), `JwksCache`, `SmartConfiguration`.

## Code map

`config.rs`, `provider/`, `jwks/` (cache, coordinator, fetcher), `jti/` (memory, redis), `scope/` (smart_v2, permissions), `policy/`, `principal.rs`, `discovery.rs`, `outbound.rs`. Auth scopes are consumed by Bulk Data Submit (`system/bulk-submit`). Unit tests are inline in `src/`; integration tests live under `crates/auth/tests/` where present.
