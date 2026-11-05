# Web UI Self-Calls and Authentication

The HFS web UI reads its conformance data — the SearchParameter registry and
the CompartmentDefinition set — from the server's **own FHIR API** over HTTP
(`GET /SearchParameter`, `GET /CompartmentDefinition` on the loopback
address). Primary storage is the source of truth, and the UI shows exactly
what the server serves; there is no side channel and **no auth carve-out**:
the self-call is authenticated exactly like any other client when
authentication is enabled.

## Supported modes

| Mode | Configuration | Self-call behavior |
|------|---------------|--------------------|
| **Off** (default) | `HFS_AUTH_ENABLED` unset or `false` | The self-call carries no credentials; everything works out of the box. |
| **Static bearer** | `HFS_AUTH_ENABLED=true` + `HFS_OUTBOUND_BEARER_TOKEN=<token>` | The self-call attaches the provisioned token. The token must be valid against the server's own validation config (`HFS_AUTH_JWKS_URL`, `HFS_AUTH_ISSUER`, `HFS_AUTH_AUDIENCE`) and carry the read scopes `system/SearchParameter.rs system/CompartmentDefinition.rs`. |
| **IdP-issued** | As above, with the token minted by your identity provider (e.g. a Keycloak service-account client) | Same as static bearer — the operator obtains a token via `client_credentials` and provisions it. Note the token's lifetime: when it expires the self-call starts failing and the pages degrade until a fresh token is provisioned. |

Leave the token's **tenant claim unset** for service tokens: the tenant claim
is authoritative when present, and the self-call scopes each request with
`X-Tenant-ID` so every tenant's conformance view stays reachable. A token
pinned to one tenant pins every conformance page to that tenant.

## Degraded state

With authentication enabled and **no valid outbound token**, the self-call is
rejected and the conformance pages **degrade to a warning** — they render the
page shell with a notice instead of data (never a 404 or a crash). The failed
fetch is **not cached**: the next request re-attempts it, so provisioning a
token (or fixing an expired one) heals the pages without a restart.

Both paths are exercised in CI by the browser suite's `auth` and
`auth-degraded` legs (`crates/ui/e2e`, #320), which boot the server against a
throwaway JWKS and mint the service token locally.

## Planned: self-minted service tokens

The static token puts key rotation on the operator. The planned follow-up is
a `JwtAssertionOutboundAuthProvider` (see `crates/auth/src/outbound.rs` and
the note in `crates/hfs/src/main.rs`): SMART Backend Services
`client_credentials` with `private_key_jwt`, configured from `HFS_UI_*`
client credentials, minting short-lived, auto-refreshed tokens with exactly
the two conformance read scopes. Until then, provision
`HFS_OUTBOUND_BEARER_TOKEN` with a long-lived token from your IdP.
