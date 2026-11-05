# Helios Keycloak login theme

A branded login screen for the local Keycloak, matching the HFS web UI
(Figma "Dashboard V1.1"): Figtree, Helios accent `#33b8ff`, light surface,
the Helios logo. It follows the recommendation in
[Login-screen branding across IdPs](#login-screen-branding-across-idps) — do
login-screen branding **in the IdP**, since HFS is not the auth server.

```
themes/helios/login/
├── theme.properties          # extends the stock `keycloak` login theme
└── resources/
    ├── css/helios.css        # CSS-only overrides (no template forks)
    ├── img/logo.png          # copied from crates/ui/assets/logo.png
    └── fonts/figtree-latin.woff2
```

## How it's wired

- **Mounted** into the container read-only:
  `./themes:/opt/keycloak/themes:ro` (see `docker-compose.yml`).
- **Selected** by the realm: `"loginTheme": "helios"` in `realm.json`.
- **CSS-only.** It extends the built-in `keycloak` login theme and layers
  branding via `resources/css/helios.css` — no forked FreeMarker templates, so
  it keeps working across Keycloak upgrades. Palette/font come from
  `crates/ui/assets/app.css`.

## Seeing it

`start-dev` disables theme caching, so edits under `themes/` show up on the next
page load (no restart needed; a container restart *is* needed only to pick up a
newly mounted volume). Render the login screen via the interactive client added
to the realm for this purpose (`hfs-web`, public, Authorization Code + PKCE):

```
http://localhost:8180/realms/fhir/protocol/openid-connect/auth
  ?client_id=hfs-web&response_type=code&scope=openid
  &redirect_uri=http://localhost:8080/ui/callback
  &state=x&code_challenge=<S256>&code_challenge_method=S256
```

Test user: `demo` / `demo`. The `hfs-web` client and `demo` user are the
scaffolding the browser-login work (#198 §3) will build on.

## Portability

None — this is Keycloak-specific (FreeMarker themes). Okta/Auth0/Entra each have
their own branding model; nothing here transfers. See the matrix below.

## Login-screen branding across IdPs

Because the login screen is IdP-owned, branding is done in the IdP, not in HFS.
Portability is low: each IdP has its own theming model, and a theme built for one
does not transfer to another (#198).

| IdP | Customization mechanism | Logo / colors / CSS | Full layout / HTML control | Portable? | Notes |
|-----|------------------------|---------------------|----------------------------|-----------|-------|
| **Keycloak** | **Themes** (FreeMarker templates + CSS), configurable per realm; also v2 "declarative" simple styling | ✅ full | ✅ full (custom `.ftl` templates) | ❌ Keycloak-specific | Self-hosted, so we ship this theme as a mounted dir. Most control of the four. |
| **Okta** | Brands API + custom sign-in widget (hosted or embedded); custom domain | ✅ logo/colors via console; CSS on the widget | ⚠️ widget config, not arbitrary HTML | ❌ | Embedded widget gives more control but pulls the login into our app surface. |
| **Auth0** | Universal Login + Branding (logo, colors, page templates via Liquid), custom domain | ✅ | ⚠️ page templates (Liquid), bounded | ❌ | "New" Universal Login is config-driven; "Classic" allows more HTML/JS. Hosted by Auth0, so branding is tenant config; the repo keeps it as a theme JSON plus a Management API script ([`docker/auth0/branding/`](../../../auth0/branding/README.md)). Page templates need a custom domain. |
| **Entra ID** | Company Branding (logo, background, text) per tenant | ✅ limited (logo/background/text strings) | ❌ no template/CSS control | ❌ | Most locked-down; branding only, no layout control. |

- **Do IdP-native theming; keep HFS out of the login-screen business.** HFS is
  not the auth server, so owning login markup would mean re-implementing an IdP.
- **Keycloak:** this theme. It is the one IdP bundled for dev/self-host and the
  only one where we control the box.
- **Auth0:** the same branding as tenant config, in
  [`docker/auth0/branding/`](../../../auth0/branding/README.md) — no Liquid page
  templates, since those need a custom domain.
- **Okta / Entra ID:** document the branding knobs (logo, colors, custom domain)
  and stop there. Don't attempt a portable theme — nothing transfers.
- **HFS-side involvement is limited to the redirect:** send users to the IdP and
  handle the callback.
