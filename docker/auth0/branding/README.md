# Helios Auth0 login branding

The Auth0 counterpart of the Keycloak login theme in
[`docker/keycloak/themes/helios/`](../../keycloak/themes/helios/README.md): the
Helios logo, Figtree, accent `#33b8ff` and a light surface on Auth0's Universal
Login page, matching the HFS web UI. It follows the recommendation in
[`docs/auth-verification.md`](../../../docs/auth-verification.md) §4: brand the
login screen **in the IdP**, since HFS is not the auth server.

```
docker/auth0/branding/
├── theme.json         # Universal Login theme: colors, borders, fonts, widget, page
└── apply-branding.sh  # pushes the theme + tenant branding through the Management API
```

## How it differs from Keycloak

Auth0 serves the login page itself, so there is nothing to mount. The branding
lives in the tenant and the repo keeps it as config: `apply-branding.sh` sends
it with the Management API, and re-running it overwrites whatever is there. It
sets:

| What | Management API call |
|------|---------------------|
| Logo, primary color, page background, font | `PATCH /api/v2/branding` |
| Theme (all of `theme.json`) | `POST /api/v2/branding/themes`, or `PATCH` on the existing default theme |
| Login subtitle ("Log in to continue to the Helios FHIR Server.") instead of the generated tenant name | `PUT /api/v2/prompts/{login,login-id}/custom-text/en` |

Like the Keycloak theme, it changes settings only and forks no page markup. Auth0's
Liquid page templates (`PUT /api/v2/branding/templates/universal-login`) would
allow more, but they need a custom domain on the tenant, which a dev tenant
doesn't have.

**Assets are fetched by Auth0, so they must be public HTTPS URLs.** By default
the script points at this repository on `raw.githubusercontent.com`:

- logo: `crates/ui/assets/logo.png` (the same file the HFS UI and Keycloak theme use)
- font: `docker/keycloak/themes/helios/login/resources/fonts/figtree-latin.woff2`

Set `HELIOS_BRANDING_ASSET_BASE` to serve them from somewhere else (same
relative paths).

## Applying it

The tenant must use the **new** Universal Login experience (the default for new
tenants); the script warns if it doesn't.

```bash
# With a logged-in Auth0 CLI (auth0 login):
AUTH0_DOMAIN=dev-abc123.us.auth0.com docker/auth0/branding/apply-branding.sh

# Or with an M2M app authorized for the Management API
# (read:branding update:branding create:branding read:prompts update:prompts):
AUTH0_DOMAIN=... AUTH0_MGMT_CLIENT_ID=... AUTH0_MGMT_CLIENT_SECRET=... \
  docker/auth0/branding/apply-branding.sh

# Print the payloads without calling Auth0:
AUTH0_DOMAIN=... docker/auth0/branding/apply-branding.sh --dry-run
```

## Seeing it

Sign in to the HFS web UI with Auth0 as the IdP (`HFS_UI_LOGIN_CLIENT_ID` set to
a Regular Web App with callback `{HFS_BASE_URL}/ui/callback`), or open the
authorize URL directly:

```
https://<AUTH0_DOMAIN>/authorize?client_id=<web client id>&response_type=code
  &scope=openid&redirect_uri=http://localhost:8080/ui/callback
```

Branding changes show up on the next page load.

> **Auth0 needs an `audience` for the UI login.** HFS's login redirect does not
> send an `audience` parameter, and without one Auth0 issues an opaque access
> token that HFS rejects ("Invalid token format"), so every FHIR call after
> login fails with 401. Until HFS can send one, set the tenant's **Default
> Audience** (Dashboard → Settings → API Authorization Settings) to the API
> Identifier used for `HFS_AUTH_AUDIENCE`. That is a login concern, not a
> branding one, so this script does not change it.

## Portability

None. These settings exist only in Auth0. Keycloak, Okta and Entra ID each have
their own branding model; see the matrix in `docs/auth-verification.md` §4.
