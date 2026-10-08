# Microsoft Entra ID

This page sets up Microsoft Entra ID (formerly Azure AD) as the identity
provider for HFS. HFS stays a resource server: Entra ID issues the tokens, and
HFS validates them against Entra ID's signing keys and authorizes each request
from the SMART scopes the token carries.

Two kinds of caller are covered:

- **Backend services** use the client-credentials flow and get SMART
  `system/` scopes.
- **People signing in to the web UI** use the authorization-code flow and get
  SMART `user/` scopes.

## Prerequisites

- An Entra ID tenant, and an account that can register applications.
- An administrator role that can grant admin consent and assign users to
  applications, for example **Cloud Application Administrator**.

Nothing in this setup requires weakening the tenant's sign-in security.
Keep security defaults, multi-factor authentication and Conditional Access
as they are: Entra ID applies them when the user signs in, before HFS
receives a token.

## 1. Register the application

1. Go to **Entra ID → App registrations → New registration**.
2. Fill in the form:
   - **Name:** `HFS`.
   - **Supported account types:** *Single tenant only*.
   - **Redirect URI:** leave it empty for now.
3. Click **Register**.
4. From **Overview**, copy the **Application (client) ID** and the
   **Directory (tenant) ID**. Below they are `{client-id}` and `{tenant-id}`.

## 2. Request v2 access tokens

Open **Manifest**, set `"requestedAccessTokenVersion": 2` inside `"api"`, and
click **Save**.

Without this step, Entra ID issues v1 tokens, whose issuer is
`https://sts.windows.net/{tenant-id}/` and whose audience is the
`api://` URI. Neither matches the configuration in step 6.

## 3. Expose the API

Under **Expose an API**, click **Add** next to *Application ID URI* and keep
the proposed `api://{client-id}`.

## 4. Define the SMART scopes as App Roles

Entra ID carries application permissions, and roles assigned to users, in the
token's `roles` claim. HFS reads SMART scopes from `roles` as well as from
`scope` and `scp`. Define one App Role per SMART scope under **App roles →
Create app role**:

| Display name | Allowed member types | Value |
|---|---|---|
| FHIR system full access | Applications | `system/*.cruds` |
| FHIR system read-only | Applications | `system/*.rs` |
| FHIR user full access | Users/Groups | `user/*.cruds` |

Use any SMART v2 scope as the value, for example `system/Patient.rs`.
App Role values accept `/` and `*`.

> **Use App Roles, not delegated scopes.** Entra ID does not allow `/` in the
> name of a scope under *Expose an API*, so a SMART scope cannot be a
> delegated permission. Grant SMART scopes to people as App Roles too.

A user who imports data from the web UI's Import page also needs the
`system/bulk-submit` scope. Add it as a user App Role and assign it only to
the people who may import.

## 5. Create a client secret

Under **Certificates & secrets**, click **New client secret**. Copy the
**Value** column right away: the portal shows it only once. The *Secret ID*
column is not the secret.

## 6. Grant the backend client its roles

1. Under **API permissions**, click **Add a permission → My APIs → HFS →
   Application permissions**.
2. Select `system/*.cruds` and add it.
3. Click **Grant admin consent**. The status must read *Granted*. Until then,
   tokens carry no `roles`, and every request returns `403`.

A separate client follows the same pattern. For example, a read-only client
is its own app registration, with its own secret and the
`requestedAccessTokenVersion` setting, granted only `system/*.rs` of the HFS
API.

## 7. Configure HFS

```bash
export HFS_AUTH_ENABLED=true
export HFS_AUTH_JWKS_URL=https://login.microsoftonline.com/{tenant-id}/discovery/v2.0/keys
export HFS_AUTH_ISSUER=https://login.microsoftonline.com/{tenant-id}/v2.0
export HFS_AUTH_AUDIENCE={client-id}

# Advertised in /.well-known/smart-configuration
export HFS_SMART_TOKEN_ENDPOINT=https://login.microsoftonline.com/{tenant-id}/oauth2/v2.0/token
export HFS_SMART_AUTHORIZE_ENDPOINT=https://login.microsoftonline.com/{tenant-id}/oauth2/v2.0/authorize
export HFS_SMART_JWKS_URL=https://login.microsoftonline.com/{tenant-id}/discovery/v2.0/keys
```

Leave `HFS_AUTH_TENANT_CLAIM` at its default. Entra ID's `tid` claim is the
Entra tenant, not an HFS tenant.

## 8. Verify with a backend token

```bash
TOKEN=$(curl -s -X POST \
  https://login.microsoftonline.com/{tenant-id}/oauth2/v2.0/token \
  -d grant_type=client_credentials \
  -d client_id={client-id} \
  --data-urlencode client_secret={client-secret} \
  --data-urlencode "scope=api://{client-id}/.default" \
  | jq -r .access_token)

echo "$TOKEN" | cut -d. -f2 | base64 -d 2>/dev/null | jq '{iss, aud, roles}'
curl -H "Authorization: Bearer $TOKEN" http://localhost:8080/Patient
```

The decoded token should show:
- `iss`: `https://login.microsoftonline.com/{tenant-id}/v2.0`
- `aud`: `{client-id}`
- `roles`: `["system/*.cruds"]`

## 9. Sign in to the web UI

1. Under **Authentication → Add a platform → Web**, add the redirect URI
   `{HFS_BASE_URL}/ui/callback`, for example
   `http://localhost:8080/ui/callback`. Entra ID accepts plain `http` only for
   `localhost`.
2. Under **Enterprise applications → HFS → Users and groups**, assign each
   user the `user/*.cruds` role. Assigning groups requires Entra ID P1 or P2.
   An assignment holds **one** role. To give a user a second role, such as
   `system/bulk-submit`, use **Add user/group** again rather than editing the
   existing assignment, which would replace its role.
3. Start HFS with:

```bash
export HFS_UI_LOGIN_CLIENT_ID={client-id}
export HFS_UI_LOGIN_CLIENT_SECRET={client-secret}
export HFS_UI_LOGIN_SCOPES="openid profile email offline_access {client-id}/.default"
export HFS_SMART_END_SESSION_ENDPOINT=https://login.microsoftonline.com/{tenant-id}/oauth2/v2.0/logout
# Only for plain-HTTP local development:
export HFS_UI_LOGIN_COOKIE_SECURE=false
```

`{client-id}/.default` asks for an access token for the HFS API, which
carries the user's assigned roles. Without it, Entra ID issues a token for
Microsoft Graph, and HFS rejects it. Use the client id itself, not
`api://{client-id}`. The UI and the API share one registration here, and
Entra ID only lets an application request a token for itself through its
GUID.

## Sign-in page branding

The sign-in page is Entra ID's, not HFS's, so HFS cannot theme it the way it
does the bundled Keycloak realm.

- **Logo of this app:** under **Branding & properties**. It is shown on the
  consent screen and in My Apps, on any license.
- **Logo, background and text of the sign-in page:** **Custom branding** sets
  them for the whole tenant. It requires Entra ID P1 or P2, or Office 365.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401`, and the token's `iss` is `https://sts.windows.net/...` | v1 token | Step 2 |
| `403` on every request with a valid token | The token has no `roles` | Grant admin consent (step 6), or assign the user a role (step 9) |
| `AADSTS90009 ... requesting a token for itself` | The login scope uses `api://{client-id}` | Use `{client-id}/.default` |
| `AADSTS50011` redirect URI mismatch | The URI differs from `HFS_UI_LOGIN_REDIRECT_URI` | Register exactly `{HFS_BASE_URL}/ui/callback` |
| `403 Insufficient scope` for a signed-in user who has a role | The role was replaced instead of added | One assignment per role (step 9); sign out and in again |
| *Application assignment failed* in the portal | Missing admin role, or a role change not yet effective | Use an account with Cloud Application Administrator; sign out and in again |
