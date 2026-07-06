# Auth verification, local Keycloak, and the UI login flow

**Status:** Verification write-up (#198)
**Scope:** open/SMART auth verified end to end against a local Keycloak 26;
local Keycloak runbook; browser login-flow design for the web UI; login-screen
customization matrix across IdPs.
**Date:** 2026-07-06

HFS is **not** an authorization server. `helios-auth` performs local
SMART-on-FHIR / OAuth2 **token validation only** (JWT signature via JWKS, plus
issuer/audience/algorithm/scope checks); token *issuance* stays with an external
IdP. This document records what was verified, how to reproduce it, and what the
interactive UI login will need.

---

## 1. What was verified (and how)

All checks ran against the committed local Keycloak (`docker/keycloak/`, realm
`fhir`, Keycloak 26.1) with HFS built from `crates/hfs` and auth enabled. The
SMART **Backend Services** shape here is `client_credentials` (the realm's two
service-account clients); see the note in §5 on `private_key_jwt`.

| # | Check | Config | Result |
|---|-------|--------|--------|
| 1 | SMART discovery is served and un-authenticated | `/.well-known/smart-configuration` | ✅ advertises issuer, `jwks_uri`, authorize/token endpoints, `code_challenge_methods_supported: ["S256"]`, `token_endpoint_auth_methods_supported: ["private_key_jwt"]` |
| 2 | No token → 401 | auth on | ✅ `401` + `OperationOutcome` (`code: login`), `WWW-Authenticate: Bearer` |
| 3 | Valid full-access token → allowed | `hfs-backend-client`, scope `system/*.cruds` | ✅ `GET /Patient` `200`, `POST /Patient` `201` |
| 4 | Read-only client cannot write | `hfs-readonly-client`, scope `system/Patient.rs` | ✅ `GET /Patient` `200`; `POST /Patient` `403` (`Forbidden: insufficient scope for create on Patient`) |
| 5 | Scope is per-resource-type | read-only client | ✅ `GET /Observation` `403` (scope only covers `Patient`) |
| 6 | Signature validation | tampered token / garbage token | ✅ both `401` (`InvalidSignature` / parse error) |
| 7 | Issuer validation | `HFS_AUTH_ISSUER` set | ✅ enforced; HFS also refuses to boot with auth on and no issuer |
| 8 | Algorithm allow-list | `HFS_AUTH_ALGORITHMS=ES384` vs RS256 token | ✅ `401` (`Unsupported algorithm: RS256`) |
| 9 | JWKS fetch/refresh | cold start, no key preload | ✅ keys fetched on first validation from `HFS_AUTH_JWKS_URL`; min refresh interval `HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL` (default 10s) |
| 10 | JTI replay cache — memory | default | ⚠️ **see #205** — rejects *legitimate* token reuse |
| 11 | JTI replay cache — redis | `--features redis`, `HFS_AUTH_REDIS_URL` | ✅ backend works and persists across HFS restarts; ⚠️ same reuse problem as memory (#205) |
| 12 | JTI disabled | `HFS_AUTH_JTI_BACKEND=disabled` | ✅ token reuse works (200/200/200) |
| 13 | Audience validation — claim present | `HFS_AUTH_AUDIENCE=hfs-api` (realm now emits `aud`) | ✅ matching `aud` `200`; wrong expected `aud` `401` (`Invalid audience`) |
| 14 | Audience validation — claim absent | `HFS_AUTH_AUDIENCE=hfs-api`, token w/o `aud` | ⚠️ **see #206** — token missing `aud` is accepted |

**Net:** the core validation path — signature, issuer, algorithm allow-list,
SMART v2 scope enforcement, discovery, JWKS — works correctly and as intended.
Two defects surfaced (below); both have workarounds and neither blocks the
Keycloak setup once configured as in §2.

### Gaps found (filed as issues)

- **#205 — JTI replay cache rejects legitimate bearer-token reuse.**
  `check_and_store` runs on every bearer validation, so the *second* request
  with the same still-valid access token is `401`ed. Single-use `jti` semantics
  belong to the `private_key_jwt` **client-assertion** JWT (checked by the IdP),
  not to resource-server access tokens. With the default `memory` backend HFS is
  unusable against Keycloak until `HFS_AUTH_JTI_BACKEND=disabled` is set.
- **#206 — `HFS_AUTH_AUDIENCE` not enforced when the token omits `aud`.**
  `jsonwebtoken` only validates `aud` when present; a token with no `aud` claim
  bypasses the restriction. `aud` should be added to `required_spec_claims` when
  an expected audience is configured.
- **Realm audience mapper — fixed in this PR.** The realm previously issued
  tokens with no `aud`, so audience validation couldn't be exercised locally.
  Both service-account clients now carry a hardcoded-audience mapper
  (`aud: hfs-api`), so `HFS_AUTH_AUDIENCE=hfs-api` validates positively (verified,
  rows 13–14). This makes #206 *observable* locally but does not fix the
  underlying missing-claim bypass.
- **`get-token.sh` python dependency — fixed in this PR.** It parsed the token
  JSON with `python3` only; on a machine without a working `python3` (e.g. this
  Windows box, where a stub alias resolves but fails) the script produced no
  token. It now probes `jq`/`node`/`python3` and uses the first that actually
  works.
- **Discovery is partly hardcoded and contradicts the committed realm.**
  `crates/auth/src/discovery.rs` always advertises
  `token_endpoint_auth_methods_supported: ["private_key_jwt"]`, signing algs
  `RS384`/`ES384`, and a fixed `scopes_supported` list. Against the committed
  realm a client that *follows* discovery would attempt an auth method the realm
  doesn't offer (its clients use `client-secret`), signed with algorithms
  Keycloak doesn't use by default (it signs RS256). These should be configurable
  — or, better, derived from the IdP's own
  `.well-known/openid-configuration` — so discovery describes the actual
  deployment. (Not filed as a bug; it is a config-fidelity gap rather than a
  validation defect.)
- **Realm has no `private_key_jwt` client.** Both committed clients use
  `client-secret`, so the true SMART Backend Services client-assertion leg can't
  be exercised locally (see §5). Adding a third client with
  `clientAuthenticatorType: client-jwt` and a committed test keypair would let
  the advertised `private_key_jwt` capability be tested end to end.

---

## 2. Local Keycloak runbook

```bash
# 1. Bring up Keycloak 26 with the fhir realm (admin/admin, port 8180)
docker compose -f docker/keycloak/docker-compose.yml up -d
# wait for health: docker inspect --format '{{.State.Health.Status}}' keycloak-keycloak-1  -> healthy

# 2. Get a Backend Services token (client_credentials)
#    full access: system/*.cruds  |  read-only: system/Patient.rs
export TOKEN=$(docker/keycloak/get-token.sh)                     # hfs-backend-client
export RO=$(docker/keycloak/get-token.sh hfs-readonly-client)    # hfs-readonly-client

# 3. Run HFS pointed at the realm. NOTE the JTI override (see #205) so a token
#    can be reused for its full lifetime, as every OAuth2 client expects.
export HFS_AUTH_ENABLED=true
export HFS_AUTH_JWKS_URL=http://localhost:8180/realms/fhir/protocol/openid-connect/certs
export HFS_AUTH_ISSUER=http://localhost:8180/realms/fhir
export HFS_AUTH_JTI_BACKEND=disabled
export HFS_SMART_TOKEN_ENDPOINT=http://localhost:8180/realms/fhir/protocol/openid-connect/token
export HFS_SMART_AUTHORIZE_ENDPOINT=http://localhost:8180/realms/fhir/protocol/openid-connect/auth
cargo run --bin hfs

# 4. Exercise accept/deny
curl -H "Authorization: Bearer $TOKEN" http://localhost:8080/Patient           # 200
curl -X POST -H "Authorization: Bearer $RO" -H 'Content-Type: application/fhir+json' \
     -d '{"resourceType":"Patient"}' http://localhost:8080/Patient             # 403
```

Committed realm summary (`docker/keycloak/realm.json`): realm `fhir`; client
scopes `system/*.cruds`, `system/Patient.rs`, `system/Observation.r`; two
confidential service-account clients (`hfs-backend-client` full,
`hfs-readonly-client` read-only). All `client_credentials`; `standardFlow`
(interactive login) is disabled on both, which is correct for backend services
but means the realm as shipped has **no** interactive-login client yet — that is
added for the UI work (§3).

---

## 3. UI login flow (browser / interactive)

The backend-services flow above is machine-to-machine. The web UI (`crates/ui`)
needs an **interactive user login**, and because HFS is not the auth server, the
login screen belongs to the IdP: the browser is redirected to Keycloak (or
Okta/Auth0/Entra) to authenticate. The appropriate grant is
**Authorization Code + PKCE** (OAuth 2.1 default for browser apps; HFS's own
discovery already advertises `code` + `S256`).

### Intended flow

```
Browser                     HFS (crates/ui + new routes)              IdP (Keycloak)
   │  GET /ui (no session)          │                                     │
   │ ─────────────────────────────▶│  no session cookie                  │
   │  302 -> /ui/login              │                                     │
   │ ─────────────────────────────▶│  build authorize URL:               │
   │                                │   client_id, redirect_uri,          │
   │                                │   scope, state, PKCE code_challenge │
   │  302 to IdP authorize ◀────────│  (state + code_verifier saved       │
   │                                │   server-side, keyed by a temp      │
   │                                │   cookie)                           │
   │ ───────────────────────────────────────────────────────────────────▶│  IdP LOGIN SCREEN
   │                                │                        user authenticates + consents
   │  302 to /ui/callback?code&state ◀───────────────────────────────────│
   │ ─────────────────────────────▶│  verify state; exchange code +      │
   │                                │  code_verifier at token endpoint ──▶│
   │                                │  ◀── access/id/refresh tokens ──────│
   │                                │  create session, set HttpOnly       │
   │  302 to /ui  + Set-Cookie ◀────│  Secure SameSite cookie             │
   │ ─────────────────────────────▶│  authenticated                      │
```

### What HFS needs to add (design, not built here)

- **Two routes** in `crates/ui` (or a small `helios-web-auth` module):
  `GET /ui/login` (redirect to the IdP authorize endpoint) and
  `GET /ui/callback` (validate `state`, exchange the code, establish a session).
  A `POST /ui/logout` clears the session and optionally hits the IdP end-session
  endpoint.
- **Server-side login state**: `state` (CSRF) and the PKCE `code_verifier`,
  stored server-side and keyed by a short-lived cookie, consumed once at
  callback.
- **A session**: an `HttpOnly; Secure; SameSite=Lax` cookie referencing
  server-side session state (holding the tokens / a derived principal). This is
  distinct from the bearer-token path used by API clients — the browser never
  sees the access token in JS. The existing `auth_middleware`
  (`crates/rest/src/middleware/auth.rs`) validates `Authorization: Bearer`; the
  UI session layer sits in front and can mint/refresh the bearer used for
  downstream FHIR calls.
- **Discovery wiring**: reuse `HFS_SMART_AUTHORIZE_ENDPOINT` /
  `HFS_SMART_TOKEN_ENDPOINT` (already config) plus a new client_id/redirect_uri
  and the OIDC end-session endpoint. Prefer fetching the IdP's
  `.well-known/openid-configuration` at startup over hardcoding paths.
- **Realm/client**: a new **public** client with `standardFlowEnabled: true`,
  PKCE required (`S256`), and `redirect_uris` including
  `http://localhost:8080/ui/callback`. Ships in `realm.json` alongside the
  backend clients.
- **Token refresh + expiry** handling, and reconciling the negotiated
  `RequestLocale` and per-user settings (#151) with the now-known user identity
  (issuer|subject), which is also the settings-store key.

This is a **design sketch for a follow-up implementation issue**, not part of
this verification.

---

## 4. Login-screen customization matrix

Because the login screen is IdP-owned, branding is done in the IdP, not in HFS.
Portability is low: each IdP has its own theming model, and a theme built for one
does not transfer to another.

| IdP | Customization mechanism | Logo / colors / CSS | Full layout / HTML control | Portable? | Notes |
|-----|------------------------|---------------------|----------------------------|-----------|-------|
| **Keycloak** | **Themes** (FreeMarker templates + CSS), configurable per realm; also v2 "declarative" simple styling | ✅ full | ✅ full (custom `.ftl` templates) | ❌ Keycloak-specific | Self-hosted → we can ship a Helios theme as a JAR/mounted dir. Most control of the four. |
| **Okta** | Brands API + custom sign-in widget (hosted or embedded); custom domain | ✅ logo/colors via console; CSS on the widget | ⚠️ widget config, not arbitrary HTML | ❌ | Embedded widget gives more control but pulls the login into our app surface. |
| **Auth0** | Universal Login + Branding (logo, colors, page templates via Liquid), custom domain | ✅ | ⚠️ page templates (Liquid), bounded | ❌ | "New" Universal Login is config-driven; "Classic" allows more HTML/JS. |
| **Entra ID** | Company Branding (logo, background, text) per tenant | ✅ limited (logo/background/text strings) | ❌ no template/CSS control | ❌ | Most locked-down; branding only, no layout control. |

### Recommendation

- **Do IdP-native theming; keep HFS out of the login-screen business.** HFS is
  not the auth server, so owning login markup would mean re-implementing an IdP.
- **Ship a Helios Keycloak theme** (logo, palette matching the Dashboard V1.1
  design tokens) as an optional, mountable theme in `docker/keycloak/` — it's the
  one IdP we bundle for dev/self-host and the only one where we control the box.
- **For Okta/Auth0/Entra, document the branding knobs** (logo + colors + custom
  domain) and stop there. Don't attempt a portable theme — nothing transfers.
- **HFS-side involvement is limited to the redirect** (§3): send users to the
  IdP and handle the callback. The only "HFS branding" the user sees pre-login is
  the `/ui/login` interstitial (if any) before the redirect — keep it minimal.

---

## 5. Notes / limitations of this pass

- **`private_key_jwt` end-to-end** was not exercised with a real asymmetric
  client assertion — the committed realm's clients use `client-secret`
  (`client_credentials`). HFS's role is identical either way (it validates the
  *resulting* access token via JWKS, and does not perform client authentication),
  and discovery correctly advertises `private_key_jwt` +
  `client-confidential-asymmetric`. Verifying a real asymmetric-client assertion
  would require adding a `private_key_jwt` client to the realm; recommended as a
  follow-up so the advertised capability is exercised.
- Interactive **Authorization Code + PKCE** is designed here (§3), not
  implemented — there is no interactive client in the realm yet and no
  authorize/callback routes in HFS.
- The two defects (#205, #206) are pre-existing in the validation path; this pass
  found and filed them but does not fix them.
