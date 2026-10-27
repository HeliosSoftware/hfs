# Helios Keycloak login theme

A branded login screen for the local Keycloak, matching the HFS web UI
(Figma "Dashboard V1.1"): Figtree, Helios accent `#33b8ff`, light surface,
the Helios logo. It demonstrates the recommendation in
[`docs/auth-verification.md`](../../../../docs/auth-verification.md) §4 — do
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
their own branding model; nothing here transfers. See the theming matrix in
`docs/auth-verification.md` §4.
