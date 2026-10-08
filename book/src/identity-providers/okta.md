# Okta

This page sets up an **Okta custom authorization server** as the identity
provider for HFS. HFS stays a resource server: Okta issues the tokens, and HFS
validates them against the authorization server's signing keys and authorizes
each request from the SMART scopes the token carries.

It covers **people signing in to the web UI**, which signs a user in with the
authorization-code flow and PKCE, and **bearer validation** of the Okta access
tokens that API clients send, including **SMART scope** enforcement. Both were
verified with user tokens against an Okta trial tenant. Backend services that
request tokens with the `client_credentials` grant (SMART Backend Services)
could not be tried there; see [What was not verified](#what-was-not-verified).
For the HFS side of authentication in general, see
[Web UI Self-Calls and Authentication](../components/web-ui-self-calls.md).

The setup was verified with the `sqlite` storage backend and an R4 build only.

## Prerequisites

- An Okta org with **API Access Management**, so that you can create a
  custom authorization server. The org authorization server is not used.
- A running HFS built with the `ui` feature, reachable from the browser at a
  base URL you control.
- Admin access to the Okta Admin Console (the admin host, not the end-user
  dashboard).

Values you replace are written in braces:

| Placeholder | Meaning |
|-------------|---------|
| `{domain}` | Your Okta org host, for example the host of your tenant URL. |
| `{auth-server-id}` | The id Okta gives the custom authorization server. |
| `{client-id}` | The client id of the Okta application named in the step. |
| `{HFS_BASE_URL}` | The public base URL of HFS, with no trailing slash. |
| `{audience}` | The audience string you choose for the authorization server. |
| `{policy-id}` | The id of an access policy returned by the Okta API. |
| `{app-id}` | The application's id, as shown in the Admin Console URL. |
| `{client-secret}` | The client secret of the login application. |

## 1. Create the authorization server

In the Admin Console open **Security → API → Authorization Servers** and add a
server. Name it, and set an **audience**: any string that identifies your FHIR
API. HFS compares it with the `aud` claim of every token.

Set **Issuer** to **Okta URL**, not to the default "Dynamic (based on request
domain)". HFS compares the `iss` claim with `HFS_AUTH_ISSUER` exactly, so the
issuer must not change with the host a client used to reach Okta. With
"Okta URL" the values are:

```text
issuer     https://{domain}/oauth2/{auth-server-id}
JWKS       https://{domain}/oauth2/{auth-server-id}/v1/keys
audience   {audience}
```

User tokens from the verified setup carried exactly this `iss` and `aud`.

## 2. Add the SMART scopes

On the authorization server add the scopes below. Okta does not know them
until you do, and HFS enforces SMART scopes from the access token.

| Scope | Purpose |
|-------|---------|
| `system/*.cruds` | Full read and write access to every resource type. |
| `system/Patient.rs` | Read and search on Patient only; used for a read-only token. |
| `system/Observation.r` | Read on Observation only. |
| `user/*.cruds` | Grants the signed-in user access to resources. |
| `system/bulk-submit` | Lets the Import page submit `$bulk-submit` as the signed-in user. |

For each scope set **User consent** to Implicit, leave **Set as a default
scope** unticked, and tick **Include in public metadata**. Check the result in
`https://{domain}/oauth2/{auth-server-id}/.well-known/oauth-authorization-server`,
whose `scopes_supported` lists them.

`system/*.cruds` was also requested in the verified setup. `user/*.cruds`
alone was not tested as sufficient.

## 3. Register the login application

The web UI signs users in through an OIDC application.

1. Go to **Applications → Applications → Create App Integration** and use
   the **Classic experience**, not the default wizard. Choose OIDC, then Web
   Application, and select **Use Okta-generated client ID**: the form does
   not save otherwise. The form asks only for a name and the client
   definition; grant types and redirect URIs are edited after saving.
2. Grant types: **Authorization Code** and **Refresh Token**. Do not tick
   Client Credentials.
3. Sign-in redirect URI: `{HFS_BASE_URL}/ui/callback`. HFS derives the same
   value by default (see the table in step 6), and Okta rejects any other
   redirect URI.
4. Sign-out redirect URI: `{HFS_BASE_URL}/ui`.
5. Require **PKCE**. Untick **DPoP**, which is ticked by default on new
   applications. HFS does not implement DPoP and validates plain bearer
   tokens.
6. Client authentication is **Client secret**. HFS sends the secret in the
   form body of the token request (`client_secret_post`), and Okta accepted
   that method for this application.
7. Assignments: the verified setup used "Allow everyone in your
   organization". It left Federation Broker Mode on, which only hides the
   app from the end-user dashboard.

To check the registration, open the authorize endpoint
`https://{domain}/oauth2/{auth-server-id}/v1/authorize` with
`client_id={client-id}`, the registered redirect URI and a PKCE S256
challenge. A registered redirect URI returns 200 (the sign-in page); an
unregistered one returns 400.

## 4. Add the access policy

An authorization server issues a token only if an access policy rule allows
it. Create a policy assigned to the login application, with one rule that
allows the authorization-code and refresh-token grants and the scopes the
users need: `user/*.cruds`, `system/bulk-submit`, `openid`, `profile`,
`email` and `offline_access`. The verified setup later added `system/*.cruds`
and `system/Patient.rs` to the same rule, so that one client could mint both
a full-scope and a read-only token. The access policies are under **Security
→ API → Authorization Servers →** your server, on the **Access Policies** tab.

Set the **access token lifetime** in the rule. The verified setup started at
60 minutes and raised it to 1440 minutes for long-running imports. HFS
rejects an expired token. Whether a token that expires during a running
import breaks that import was **not verified**.

In the verified setup this policy and its rule were created with the Okta
Management API, not in the Admin Console. Create the API token under
**Security → API → Tokens**, and revoke it when you are done. The calls
use the token in the `Authorization` header with the `SSWS` scheme.

The exact bodies of these two calls were not recorded. The bodies below
follow the shape that was recorded for another rule on the same server, with
this policy's values filled in. They are **not verified** as sent. First
create the policy:

```json
{
  "type": "OAUTH_AUTHORIZATION_POLICY",
  "name": "ui-login",
  "status": "ACTIVE",
  "conditions": { "clients": { "include": ["{client-id}"] } }
}
```

Send it as
`POST https://{domain}/api/v1/authorizationServers/{auth-server-id}/policies`.
Then add the rule with
`POST https://{domain}/api/v1/authorizationServers/{auth-server-id}/policies/{policy-id}/rules`:

```json
{
  "type": "RESOURCE_ACCESS",
  "name": "authorization-code",
  "status": "ACTIVE",
  "conditions": {
    "people": { "groups": { "include": ["EVERYONE"] } },
    "grantTypes": { "include": ["authorization_code", "refresh_token"] },
    "scopes": {
      "include": [
        "openid", "profile", "email", "offline_access",
        "user/*.cruds", "system/bulk-submit"
      ]
    }
  },
  "actions": { "token": { "accessTokenLifetimeMinutes": 60 } }
}
```

The calls that created this policy and rule returned HTTP 201.
The `actions.token` object also accepts `refreshTokenLifetimeMinutes` and
`refreshTokenWindowMinutes`.

## 5. Check the sign-in policy

New applications get Okta's system authentication policy "Any two factors".
Its catch-all rule needs a device-bound, phishing-resistant factor such as
FastPass or a security key. An Okta Verify push or code does not satisfy it,
and the sign-in is denied: the System Log shows
`policy.evaluate_sign_on` with `DENY` and the reason `UNSATISFIABLE`.

The fix is to enrol an authenticator that satisfies the policy, or to assign
a policy your users can meet. The verified setup used a workaround that
belongs only in a disposable tenant: a separate authentication policy whose
catch-all rule requires one factor (a password), assigned to the login
application only. It was assigned with
`PUT https://{domain}/api/v1/apps/{app-id}/policies/{policy-id}` (HTTP 204).
Undo it by assigning the original policy back. A new policy's default
catch-all rule cannot be changed with a partial body: read the rule, change
only `actions.appSignOn.verificationMethod`, and send the whole object back.
Do not use a password-only policy for real users.

## 6. Configure HFS

Authentication is off unless `HFS_AUTH_ENABLED` is `true` or `1`. When it is
on, `HFS_AUTH_JWKS_URL` and `HFS_AUTH_ISSUER` are required and the server
refuses to start without them. The browser login is off unless
`HFS_UI_LOGIN_CLIENT_ID` is set and auth is enabled.

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_AUTH_ENABLED` | `false` | Enables JWT bearer validation when `true` or `1`. |
| `HFS_AUTH_JWKS_URL` | *(unset)* | JWKS endpoint used to verify token signatures. Required when auth is enabled. |
| `HFS_AUTH_ISSUER` | *(unset)* | Expected `iss` claim, compared exactly. Required when auth is enabled. |
| `HFS_AUTH_AUDIENCE` | *(unset)* | Expected `aud` claim. If unset, any audience is accepted and a warning is logged at startup. |
| `HFS_AUTH_TENANT_CLAIM` | `tenant_id` | Claim read as the tenant id. Okta tokens need not carry it. |
| `HFS_AUTH_PATIENT_CLAIM` | `patient` | Claim read as the SMART launch patient. Okta tokens need not carry it. |
| `HFS_AUTH_ENCOUNTER_CLAIM` | `encounter` | Claim read as the SMART launch encounter. Okta tokens need not carry it. |
| `HFS_AUTH_FHIR_USER_CLAIM` | `fhirUser` | Claim read as the SMART `fhirUser`. Okta tokens need not carry it. |
| `HFS_AUTH_ALGORITHMS` | `RS256,RS384,ES256,ES384` | Comma-separated signing algorithms HFS accepts. |
| `HFS_AUTH_JWKS_MIN_REFRESH_INTERVAL` | `10` | Minimum seconds between JWKS refreshes. |
| `HFS_SMART_TOKEN_ENDPOINT` | *(unset)* | Token endpoint advertised in the SMART discovery document, which omits `token_endpoint` when this is unset. The login discovers it from the issuer when unset. |
| `HFS_SMART_AUTHORIZE_ENDPOINT` | *(unset)* | Authorization endpoint. When set, the SMART discovery document advertises it plus `code` and `authorization_code`. The login discovers it from the issuer when unset. |
| `HFS_SMART_JWKS_URL` | *(unset)* | JWKS URI advertised in the SMART discovery document. Falls back to `HFS_AUTH_JWKS_URL`. |
| `HFS_SMART_INTROSPECTION_ENDPOINT` | *(unset)* | Introspection endpoint advertised in the SMART discovery document. |
| `HFS_SMART_MANAGEMENT_ENDPOINT` | *(unset)* | Token management endpoint advertised in the SMART discovery document. |
| `HFS_SMART_REGISTRATION_ENDPOINT` | *(unset)* | Registration endpoint advertised in the SMART discovery document. |
| `HFS_SMART_REVOCATION_ENDPOINT` | *(unset)* | Revocation endpoint advertised in the SMART discovery document. |
| `HFS_SMART_END_SESSION_ENDPOINT` | *(unset)* | OIDC logout endpoint used by Sign out. When unset, discovered from the issuer's `.well-known/openid-configuration`. |
| `HFS_OUTBOUND_BEARER_TOKEN` | *(unset)* | Static bearer token for HFS's own outbound calls when there is no session. Unset means no credentials. |
| `HFS_UI_LOGIN_CLIENT_ID` | *(unset)* | Client id of the login application. Setting it enables the browser login and the session gate on `/ui`. |
| `HFS_UI_LOGIN_CLIENT_SECRET` | *(unset)* | Client secret of a confidential login application. Leave unset for a public PKCE client. |
| `HFS_UI_LOGIN_REDIRECT_URI` | `{HFS_BASE_URL}/ui/callback` | Redirect URI registered at the IdP. Derived from `HFS_BASE_URL`, trailing slash trimmed. |
| `HFS_UI_LOGIN_SCOPES` | `openid profile email` | Scopes requested at login. Override it; see below. |
| `HFS_UI_LOGIN_COOKIE_SECURE` | `true` | `Secure` attribute of the session cookie. `false` or `0` is for plain-HTTP local development and logs a warning. |
| `HFS_BASE_URL` | `http://localhost:8080` | Public base URL; feeds the default redirect URI. |
| `HFS_SERVER_PORT` | `8080` | Port to listen on. |
| `HFS_UI_ENABLED` | `true` | Enables the `/ui` router. Has no effect on builds without the `ui` feature, where `/ui` is always 404. |

The default login scopes contain no SMART scope, so with them every resource
call made from the UI is a 403. Set `HFS_UI_LOGIN_SCOPES` to the scopes of
your policy rule. The setup that was verified used this block, with the
secret and the Okta values replaced by your own:

```bash
export HFS_AUTH_ENABLED=true
export HFS_AUTH_ISSUER=https://{domain}/oauth2/{auth-server-id}
export HFS_AUTH_JWKS_URL=https://{domain}/oauth2/{auth-server-id}/v1/keys
export HFS_AUTH_AUDIENCE={audience}
export HFS_SMART_TOKEN_ENDPOINT=https://{domain}/oauth2/{auth-server-id}/v1/token
export HFS_SMART_AUTHORIZE_ENDPOINT=https://{domain}/oauth2/{auth-server-id}/v1/authorize
export HFS_SMART_JWKS_URL=https://{domain}/oauth2/{auth-server-id}/v1/keys
export HFS_UI_LOGIN_CLIENT_ID={client-id}
export HFS_UI_LOGIN_CLIENT_SECRET={client-secret}
export HFS_UI_LOGIN_SCOPES="openid profile email offline_access system/*.cruds user/*.cruds system/bulk-submit"
export HFS_UI_LOGIN_COOKIE_SECURE=false
export HFS_BASE_URL=http://localhost:8080
```

`HFS_UI_LOGIN_COOKIE_SECURE=false` is for a plain-HTTP local run; leave it at
its default behind HTTPS. `offline_access` is what makes Okta return a
refresh token.

A few facts about how HFS reads Okta tokens:

- Okta sends scopes in the `scp` claim as a JSON **array**. HFS reads
  scopes from the `scope`, `scp` and `roles` claims, each as a
  space-delimited string or an array of strings, and merges them without
  duplicates.
- HFS ignores the `groups` claim. It reads a `roles` claim as scopes: a
  value that is a SMART scope, for example `system/*.cruds`, grants that
  scope, and an ordinary role name such as `admin` grants nothing. Do not
  add a custom `roles` claim to the authorization server unless its values
  are meant as scopes.
- No tenant claim is required in the token.
- No launch-context claim is required either. Okta does not send `patient`,
  `encounter` or `fhirUser` unless you add them as custom claims.
- The JWT library allows about 60 seconds of clock leeway on `exp`. It is
  not configurable in HFS.
- The SMART discovery document is built by HFS, not by Okta. Its
  `scopes_supported`, `capabilities`,
  `token_endpoint_auth_methods_supported` (`private_key_jwt`) and the
  `client_credentials` entry of `grant_types_supported` are fixed in HFS.
  `authorization_code` is advertised only when the authorize endpoint is
  configured. That `client_credentials` is advertised does not mean Okta
  issues such tokens to you.

## 7. Verify with a token

The checks below use `curl` against a running HFS. Set the base URL and two
user access tokens: one with `system/*.cruds` and one with `system/Patient.rs`
only. How to get them is described in step 8.

```bash
export HFS=http://localhost:8080
export TOKEN='<paste full-scope access token here>'
export READONLY_TOKEN='<paste read-only access token here>'
```

### No token, and the open paths

```bash
curl -s -i "$HFS/Patient"
```

Observed: `401 Unauthorized`, header `www-authenticate: Bearer`, and an
OperationOutcome with `issue[0].code` `login` and `details.text`
`Missing Authorization header`.

```bash
curl -s -o /dev/null -w '%{http_code}\n' "$HFS/health"
curl -s -o /dev/null -w '%{http_code}\n' "$HFS/metadata"
curl -s -o /dev/null -w '%{http_code}\n' "$HFS/.well-known/smart-configuration"
```

Observed: 200 for `/health`, `/metadata` and
`/.well-known/smart-configuration`. The redirect of `/ui` is checked in
step 8.

### Full-scope token

```bash
curl -s -i -H "Authorization: Bearer $TOKEN" "$HFS/Patient"

curl -s -i -X POST -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Patient","name":[{"family":"OktaA2","given":["Release"]}]}' \
  "$HFS/Patient"
```

Observed: the GET returns `200 OK` with a `searchset` Bundle. The POST
returns `201 Created` with a `location` header, `etag: W/"1"` and the new
Patient. Delete the probe Patient before any check that counts resources.

### Read-only token

```bash
curl -s -o /dev/null -w 'GET Patient: %{http_code}\n' \
  -H "Authorization: Bearer $READONLY_TOKEN" "$HFS/Patient"

curl -s -i -X POST -H "Authorization: Bearer $READONLY_TOKEN" \
  -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Patient"}' "$HFS/Patient"

curl -s -i -H "Authorization: Bearer $READONLY_TOKEN" "$HFS/Observation"
```

Observed: the GET on Patient returns 200. The POST returns `403 Forbidden`
with `details.text` `Forbidden: insufficient scope for create on Patient`.
The GET on Observation returns `403 Forbidden` with `details.text`
`Forbidden: insufficient scope for search on Observation`.

### Bad tokens

```bash
TAMPERED="${TOKEN:0:-4}AAAA"
curl -s -i -H "Authorization: Bearer $TAMPERED" "$HFS/Patient"

curl -s -i -H "Authorization: Bearer not-a-jwt" "$HFS/Patient"
```

The exact tampering used in the verified setup was not recorded; this
command is one way to break the signature and is **not verified**.

Observed: the tampered signature returns `401` with `details.text`
`Invalid signature`. The malformed token returns `401` with a `details.text`
that begins `Invalid token format`.

For an expired token, mint one from a rule with a 5-minute access token
lifetime, wait until at least 90 seconds after its `exp`, and send it:

```bash
export EXPIRED_TOKEN='<access token minted with a 5-minute lifetime>'
curl -s -i -H "Authorization: Bearer $EXPIRED_TOKEN" "$HFS/Patient"
```

Observed: `401 Unauthorized` with `details.text` `Token expired`. The
token was tested 92 seconds past `exp`, outside the clock leeway.

### Discovery document

```bash
curl -s "$HFS/.well-known/smart-configuration" | jq '{
  issuer, authorization_endpoint, token_endpoint, jwks_uri,
  grant_types_supported, capabilities, code_challenge_methods_supported }'
```

Observed:

```json
{
  "issuer": "https://{domain}/oauth2/{auth-server-id}",
  "authorization_endpoint": "https://{domain}/oauth2/{auth-server-id}/v1/authorize",
  "token_endpoint": "https://{domain}/oauth2/{auth-server-id}/v1/token",
  "jwks_uri": "https://{domain}/oauth2/{auth-server-id}/v1/keys",
  "grant_types_supported": ["client_credentials", "authorization_code"],
  "capabilities": ["permission-v2", "client-confidential-asymmetric"],
  "code_challenge_methods_supported": ["S256"]
}
```

## 8. Sign in to the web UI

Start HFS with the block from step 6. The login application from step 3 and
the variables `HFS_UI_LOGIN_CLIENT_ID`, `HFS_UI_LOGIN_CLIENT_SECRET`,
`HFS_UI_LOGIN_SCOPES` and `HFS_UI_LOGIN_COOKIE_SECURE` turn on the browser
login. Then check the gate on `/ui`:

```bash
curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' "$HFS/ui"
```

Observed: with the browser login configured, `/ui` without a session answers
`303` with a redirect to `/ui/login?next=%2Fui`, which in turn redirects to
the Okta authorize endpoint.

### How the tokens were obtained

The verification tokens in step 7 were user access tokens from
**Authorization Code with PKCE** through the login application above:

1. Build an authorize URL with `response_type=code`, `client_id={client-id}`,
   `redirect_uri={HFS_BASE_URL}/ui/callback`, a random `state`, and a PKCE
   `code_challenge` (S256 of a random `code_verifier`). For the full token
   request `openid profile email offline_access system/*.cruds user/*.cruds
   system/bulk-submit`; for the read-only token request `openid
   system/Patient.rs`.
2. Sign in as a test user. Capture the redirect to `/ui/callback` instead of
   serving it, and read the `code` from its address. No HFS instance may be
   listening on that address at that moment.
3. Exchange the code at `https://{domain}/oauth2/{auth-server-id}/v1/token`
   with `grant_type=authorization_code`, the `code`, the `redirect_uri`, the
   `code_verifier` and the client id and client secret. A code can be used once.

Decoded, the access token had `iss` equal to the issuer, `aud` equal to the
configured audience, `sub` equal to the user's login, `scp` as an array, `cid`
equal to the client id and `exp` one lifetime after issue. The read-only
request returned `openid` and `system/Patient.rs` in `scp`. There was no
tenant claim. The ID token carried `sub` (the Okta user id), `name`, `email`
and `preferred_username`.

The browser login reads the ID token's claims (`sub`, `name`, `email`,
`preferred_username`) without verifying its signature, issuer, audience or
nonce, and sends no nonce. HFS relies on the direct TLS exchange with the
token endpoint. Access tokens are still fully validated on every API
request.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Every request answers 401 although the token is genuine. | `iss` differs from `HFS_AUTH_ISSUER`, often because the authorization server uses the Dynamic issuer. | Set Issuer to "Okta URL" (step 1) and use the same string in `HFS_AUTH_ISSUER` (step 6). |
| 401 although `iss` is correct. | `aud` differs from `HFS_AUTH_AUDIENCE`, for example a token from another authorization server. | Use the audience set on the authorization server (step 1). |
| Requests are 403 although the user signed in. | The token has no SMART scope; the default `HFS_UI_LOGIN_SCOPES` has none. | Set `HFS_UI_LOGIN_SCOPES` (step 6) and allow the scopes in the access policy rule (step 4). |
| Scopes seem to be missing. | Okta sends them in the `scp` array. | None needed: HFS parses `scope`, `scp` and `roles`. Check the scopes in the rule (step 4). |
| Token requests fail or tokens do not work as bearers. | DPoP is ticked on the application. | Untick DPoP on the login application (step 3). |
| Sign-in is denied; the System Log shows `UNSATISFIABLE`. | The "Any two factors" policy needs a phishing-resistant factor. | Enrol a compliant authenticator; see step 5. |
| `401` with `Token expired`. | The access token lifetime has run out. | Get a new token, or raise the access token lifetime in the rule (step 4). |
| The authorize endpoint returns 400. | The redirect URI is not registered. | Register `{HFS_BASE_URL}/ui/callback` exactly (step 3). |
| A new Subscription fails its handshake. | By default HFS attempts the handshake once (`HFS_SUBSCRIPTION_HANDSHAKE_MAX_ATTEMPTS`, default `1`). | Start the receiver before creating the Subscription. |

## What was not verified

The setup was verified with the `sqlite` storage backend and an R4 build
only.

- **Client credentials and SMART Backend Services.** The trial tenant
  returned `invalid_grant` for `grant_type=client_credentials`, saying that
  the "NHI Authentication Tokens SKU" was not enabled, and the access policy
  rule dialog did not offer the grant. No such token was minted. The helper
  script `docker/okta/get-token.sh` was not run, and neither was the Import
  page's `private_key_jwt` variant.
- **Refresh-token renewal.** The scope is configured; no refresh call was
  made.
- **Sign-out.** The sign-out redirect URI (`post_logout_redirect_uri`) is
  registered; no sign-out was performed.
- **The exact Management API bodies** for the login policy and rule.
- **Real MFA policies.** Only a relaxed password-only policy on the login
  application was used.
- **Other storage backends and multi-version builds.**
- **`/ui/` with a trailing slash.** Only `/ui` was checked.
- **Scopes from `roles` or from a string `scp`, and the launch-context
  claims.** HFS gained them after the setup was verified. They are
  described from the code; Okta tokens were not tested against them.
- **The expiry edge inside the clock leeway.** Only a token 92 seconds past
  `exp` was tested.
