# Okta setup log for the #1180 manual test pass

> ## SCOPE OF THIS TASK (owner's decision, 2026-10-07): `sqlite` ONLY
>
> This pass covers exactly one storage configuration: **`HFS_STORAGE_BACKEND=sqlite`
> (SQLite alone, NO Elasticsearch)**, with authentication enabled against Okta.
> It does **not** cover `sqlite-es`, `postgres`, `pg-es`, `mongodb`, `mongo-es`, `s3`,
> `s3-es` (AWS or MinIO), and no infrastructure for them (Postgres, Elasticsearch,
> MongoDB, MinIO) is to be started. #1180 is written as an umbrella for all ten
> configurations and `MANUAL_TESTING_TRACKER.md` points every Okta cell at it; the
> other nine rows are out of scope for this task and are left for the per-backend
> sub-issues the issue itself says will be split out. Nothing in this log or in the
> report may claim coverage of any configuration other than `sqlite`.
>
> Port 8080 and the `sqlite` environment of MANUAL_TESTING_MATRIX.md section 5 apply.

Working log of how the Okta trial tenant was configured for the HFS manual test
pass (#1180), including every deviation from `crates/auth/README.md` ("Okta"
section) and from the plan in the issue. Written so the setup can be redone from
scratch. **No secrets live here**: client secrets and API tokens are kept in
`~/.okta-hfs.env` (mode 600) AND, by the owner's explicit decision (disposable
trial tenant, no real data), are also recorded in the "Credentials" section at
the bottom of this file. **Strip that section before this file is committed or
pushed anywhere** (see the note there).

Status legend: DONE (verified), DONE-UNVERIFIED (configured, not yet exercised),
TODO.

## Tenant facts

| Item | Value |
|---|---|
| Okta org | `{okta-domain}` (free trial, 10 active-user cap, expires ~29 days after 2026-10-06) |
| Admin Console | `https://{okta-admin-domain}` (the `/app/UserHome` URL is the end-user dashboard, not the admin console) |
| Custom authorization server | `FHIR`, id `{auth-server-id}` |
| Issuer | `https://{okta-domain}/oauth2/{auth-server-id}` |
| Audience | `https://fhir.example.com` |
| JWKS | `<issuer>/v1/keys` |
| `hfs-backend-client` (API Services) | `{backend-client-id}` |
| `hfs-readonly-client` (API Services) | `{readonly-client-id}` |
| `hfs-web` (OIDC Web App, interactive login) | `{web-client-id}` |

## HFS environment for the pass

HFS runs on fixed port **8080** (matches the Keycloak passes and the redirect URIs
registered in Okta; no dynamic ports for this pass).

```bash
HFS_AUTH_ENABLED=true
HFS_AUTH_ISSUER=https://{okta-domain}/oauth2/{auth-server-id}
HFS_AUTH_JWKS_URL=https://{okta-domain}/oauth2/{auth-server-id}/v1/keys
HFS_AUTH_AUDIENCE=https://fhir.example.com
HFS_SMART_TOKEN_ENDPOINT=<issuer>/v1/token
HFS_SMART_AUTHORIZE_ENDPOINT=<issuer>/v1/authorize
HFS_SMART_JWKS_URL=<issuer>/v1/keys
HFS_UI_LOGIN_CLIENT_ID={web-client-id}
HFS_UI_LOGIN_SCOPES="openid profile email offline_access system/*.cruds user/*.cruds system/bulk-submit"
HFS_UI_LOGIN_COOKIE_SECURE=false
HFS_BASE_URL=http://localhost:8080
```

`HFS_UI_LOGIN_SCOPES` must be overridden: the default (`openid profile email`)
carries no SMART scope, so every resource call from the UI would be a 403.

## Configuration steps, in order

1. **Authorization server `FHIR`** (Security > API > Authorization Servers). DONE.
   - Deviation: set **Issuer = "Okta URL"** instead of the default
     "Dynamic (based on request domain)". HFS compares `iss` exactly, so a fixed
     issuer is required.
2. **Scopes** on `FHIR`: `system/*.cruds`, `system/Patient.rs`,
   `system/Observation.r`, `user/*.cruds`, `system/bulk-submit`. DONE.
   - User consent Implicit, not default, **Include in public metadata checked**
   - Re-checked on 2026-10-08 with the Management API (read-only,
     `GET /api/v1/authorizationServers/<auth-server-id>/scopes`, HTTP 200): the five custom scopes
     (`system/*.cruds`, `system/bulk-submit`, `system/Observation.r`, `system/Patient.rs`, `user/*.cruds`) all have
     `consent: IMPLICIT`, `default: false`, `metadataPublish: ALL_CLIENTS`. So "not default" means the
     "Set as a default scope" box is unticked. The book page (step 2) says the same; the labels of the Admin Console
     were not re-read, only the values.
     (so they show in `scopes_supported` and can be verified without admin access).
   - Verified against `<issuer>/.well-known/oauth-authorization-server`.
   - Deviation from the README: it lists only three scopes. `user/*.cruds` and
     `system/bulk-submit` are needed for the interactive login and for T3 (import).
3. **API Services apps** `hfs-backend-client`, `hfs-readonly-client`. DONE.
   - Applications > Create App Integration > **Classic experience** > API Services.
     The default "Okta Integration Wizard" tab does not create the right app.
   - Choose "Use Okta-generated client ID" (the form will not save otherwise).
   - Deviation: **DPoP is ticked by default** ("Require Demonstrating Proof of
     Possession header in token requests"). Untick it; `docker/okta/get-token.sh`
     and HFS do not do DPoP and the token request fails otherwise.
   - Client authentication = Client secret (already the default).
4. **OIDC Web app** `hfs-web`. DONE (redirect URI verified).
   - Classic experience > OIDC > Web Application. The creation form only asks for a
     name and client definition; grant types, redirect URIs, etc. are edited
     after saving.
   - Grant types: Authorization Code + Refresh Token (not Client Credentials).
   - Sign-in redirect URI `http://localhost:8080/ui/callback`; sign-out redirect
     URI `http://localhost:8080/ui` (HFS sends `post_logout_redirect_uri =
     {HFS_BASE_URL}/ui`, `crates/ui/src/login.rs:272`).
   - Assignments: "Allow everyone in your organization"; Federation Broker Mode
     left on (only hides the app from the end-user dashboard).
   - DPoP unticked, PKCE required.
   - Verified: `GET <issuer>/v1/authorize` with the registered redirect URI and
     PKCE S256 returns 200 (login page); an unregistered redirect URI returns 400.
5. **Access policies** on `FHIR`. PARTLY DONE.
   - Deviation (important): the admin UI's "Add Rule" dialog **did not offer the
     Client Credentials grant** (not under Core grants, not under Advanced), with
     the policy assigned to a specific client and also to "All clients". The rules
     for the two M2M policies were therefore created through the Okta Management
     API instead of the UI.
   - `m2m-backend` (policy `{okta-id}`) assigned to
     `hfs-backend-client`; rule `client-credentials`, grant `client_credentials`,
     scopes `system/*.cruds` + `system/bulk-submit`, access token 60 min.
     DONE-UNVERIFIED (no token requested yet).
   - `m2m-readonly` (policy `{okta-id}`) assigned to
     `hfs-readonly-client`; rule `client-credentials`, scope `system/Patient.rs`
     only, access token 60 min. DONE-UNVERIFIED.
   - `ui-login` (policy `{okta-id}`, rule `authorization-code`): created
     through the API as well (the owner could not find the place in the UI).
     DONE-UNVERIFIED. Assigned to `hfs-web`; grants Authorization Code + Refresh
     Token; scopes `user/*.cruds`, `system/bulk-submit`, `openid`, `profile`,
     `email`, `offline_access`; access token 60 min.
   - Never use "Any scopes" on the read-only client or the 403 check is void.

API call used for the rules (token in `$OKTA_API_TOKEN`, header
`Authorization: SSWS <token>`):

```
POST /api/v1/authorizationServers/{asId}/policies            # create policy
  {"type":"OAUTH_AUTHORIZATION_POLICY","name":"m2m-readonly","priority":2,
   "status":"ACTIVE","conditions":{"clients":{"include":["<clientId>"]}}}
POST /api/v1/authorizationServers/{asId}/policies/{policyId}/rules
  {"type":"RESOURCE_ACCESS","name":"client-credentials","status":"ACTIVE",
   "conditions":{"people":{"groups":{"include":["EVERYONE"]}},
                 "grantTypes":{"include":["client_credentials"]},
                 "scopes":{"include":["system/Patient.rs"]}},
   "actions":{"token":{"accessTokenLifetimeMinutes":60,
                       "refreshTokenLifetimeMinutes":0,
                       "refreshTokenWindowMinutes":10080}}}
```

## Deviations and gotchas (summary)

1. Admin Console host differs from the user dashboard host.
2. Fix the issuer ("Okta URL"), do not leave it dynamic.
3. Classic experience, not the Integration Wizard, for both app types.
4. DPoP is on by default for new apps: turn it off (M2M apps and `hfs-web`).
5. "Use Okta-generated client ID" must be selected to save an app.
6. Access-policy rule dialog lacks Client Credentials in this tenant: use the
   Management API.
7. `HFS_UI_LOGIN_SCOPES` default has no SMART scopes: override it.
8. The shell of whoever drives the admin console is not the shell of the agent:
   secrets must be handed over through a file (`~/.okta-hfs.env`), not `export`.
9. A pasted admin API token is exposed in chat history: revoke it right after
   the setup (Security > API > Tokens) and delete `~/.okta-hfs.env`.
10. HFS does not read `groups`/`roles` claims and does no DPoP; the login flow
    does not verify the ID token signature, `iss`, `aud` or `nonce`
    (`crates/auth/src/session.rs`). Candidate findings for #724, not blockers.
11. The README Okta section only covers client credentials; there is no Okta
    interactive-login setup documented. This log is the first such write-up.

## Findings from the session (chronological)

- 2026-10-06: tenant, auth server, scopes, three apps created in the UI.
- 2026-10-07: admin API token `hfs-setup` created (network zone: Any IP) and handed
  over in chat. Management API calls confirmed all three apps ACTIVE, DPoP off,
  grants as expected (backend/readonly: `client_credentials`; web:
  `authorization_code`, `refresh_token`), auth method `client_secret_basic`.
- Client secrets are NOT returned by `GET /api/v1/apps/{id}`; they ARE returned by
  `GET /api/v1/apps/{id}/credentials/secrets` (field `client_secret`).
- Rules for `m2m-backend`, `m2m-readonly` and policy+rule `ui-login` created via the
  Management API (all HTTP 201).
- **BLOCKER:** `POST <issuer>/v1/token` with `grant_type=client_credentials`, for
  both M2M clients, returns `invalid_grant`: "The NHI Authentication Tokens SKU is
  not enabled. Contact your Account Executive to enable the SKU to use the
  requested grant type or token exchange flow." This is almost certainly why the
  access-policy rule dialog never offered Client Credentials. The trial tenant
  cannot issue client-credentials tokens, so the SMART Backend Services path
  assumed by #1180 and `docker/okta/get-token.sh` is unavailable here.
  `docker/okta/get-token.sh` itself was not run; the same request was made by curl.

- Decision (owner + agent): since client credentials is unavailable, the pass runs
  on **user tokens** (Authorization Code + PKCE through `hfs-web`). This is what
  #1449 intends ("IdP-agnostic ... Later: the same flow against Okta / Auth0 /
  Entra (#1180-#1182)"), and MANUAL_TESTING_MATRIX.md section 15 already requires the
  browser login for T3 and the Tenants page. The "interactive login is out of
  scope" line in #1180 predates #1449 and is stale.
  What is NOT exercised: SMART Backend Services (client credentials) and
  `docker/okta/get-token.sh`; the Import page's `auth=backend-services`
  (private_key_jwt) variant.
- **Attempt A (no policy change): failed.** Signing in as `{admin-user}`
  to `hfs-web` showed "No se puede iniciar sesion. Comuniquese con el servicio de
  asistencia". System Log: `policy.evaluate_sign_on | DENY ... UNSATISFIABLE` and
  `application.policy.sign_on.deny_access` ("policy requirements could not be
  satisfied by the users' current set of available authenticator enrollments").
  Cause: new apps get the system policy **"Any two factors"**, whose catch-all rule
  requires a **device-bound, phishing-resistant** possession factor (FastPass /
  security key); an Okta Verify push or code does not satisfy it.
- **Policy relaxation (deviation, scoped to `hfs-web` only):** created the
  authentication policy `HFS test: password only` (id `{okta-id}`),
  rule `Catch-all Rule` (id `{okta-id}`) set to `factorMode 1FA`,
  `knowledge: password`; assigned with `PUT /api/v1/apps/{appId}/policies/{policyId}`
  (HTTP 204). Gotcha: a new policy's default catch-all rule cannot be PUT with a
  partial body ("Cannot modify the priority,conditions attribute because it is
  read-only"); GET the rule, change only `actions.appSignOn.verificationMethod`,
  PUT the whole object back. Revert at the end: assign the original system policy
  `{okta-id}` ("Any two factors") back to `hfs-web`.
- **Test user** created by API: `{test-user}` (id `{okta-id}`),
  `POST /api/v1/users?activate=true` with `credentials.password.value` (no
  activation email is possible in the trial). Password in the Credentials section.
  Other users in the tenant (`{other-user}`, `{admin-user}`)
  are untouched.
- `ui-login` rule scopes extended with `system/*.cruds` and `system/Patient.rs` so
  one client can mint a full token and a read-only token by requesting different
  scopes (A2/A3 without needing `hfs-readonly-client`).
- **Headless login works** with the test user (no factor prompt): Playwright
  (`playwright-core` in `~/tmp/pw`, Chromium from `~/.cache/ms-playwright`) opens
  the authorize URL, fills username/password, aborts the redirect to
  `http://localhost:8080/ui/callback` and reads the `code`; the script
  `~/tmp/pw/get-token.sh <name> "<scopes>"` then exchanges it with the PKCE
  verifier and the `hfs-web` secret (`client_secret_basic`). The code is single use.
  The HFS server must not be listening on 8080 while minting tokens this way.
- **Token claims observed (partial #724 inventory, user token, `hfs-web`):**
  access token: `iss` = issuer, `aud` = `https://fhir.example.com`, `sub` = login
  (`{test-user}`), `scp` = **array** (`system/*.cruds`, `user/*.cruds`,
  `system/bulk-submit`, `openid`, `profile`, `email`, `offline_access`), `cid` =
  client id, no tenant claim, `exp` = +3600 s. Read-only request returns
  `scp = [openid, system/Patient.rs]`. ID token: `sub` = Okta user id, `name`,
  `email`, `preferred_username`; **no `picture` claim**. `refresh_token` returned
  only when `offline_access` is requested.

## Results so far: `sqlite` + Okta (user tokens), main `5f01d3fd0` + this branch

Environment: HFS built with `cargo build --all-features` (debug, `CARGO_BUILD_JOBS=4`,
5 min 53 s, exit 0; no `target/` existed, so a cold build). DB at
`~/tmp/okta-pass/work/hfs.db`, `HFS_DATA_DIR` = repo `data/`. Start script
`~/tmp/okta-pass/start-hfs.sh`, log `~/tmp/okta-pass/work/hfs-sqlite-okta.log`.
HFS listens on `127.0.0.1:8080`, `HFS_BASE_URL=http://localhost:8080`.
Tokens are **user tokens** (see the decision above), minted 2026-10-07 ~10:40 UTC,
lifetime 3600 s. `HFS_OUTBOUND_BEARER_TOKEN` = a `full` token (expires ~11:42 UTC).

| Check | Result |
|---|---|
| T0 build | PASS (exit 0) |
| T1 start | PASS: log shows `Authentication ENABLED jwks_url=<okta>/v1/keys issuer=... audience=Some(https://fhir.example.com)`, `JWKS cache refreshed`, `Interactive browser login ENABLED client_id={web-client-id}`, `Server listening 127.0.0.1:8080`; `/health` 200 |
| A1 no token | PASS: `GET /Patient` 401 OperationOutcome (`login`, "Missing Authorization header"); `/health`, `/metadata`, `/.well-known/smart-configuration`, `/ui` 200 |
| A2 full token (`system/*.cruds`) | PASS: `GET /Patient` 200, `POST /Patient` 201 |
| A3 read-only token (`scp = openid, system/Patient.rs`) | PASS: `GET /Patient` 200; `POST /Patient` 403 "insufficient scope for create on Patient"; `GET /Observation` 403 "insufficient scope for search on Observation" |
| A4 tampered signature | PASS: 401 "Invalid signature"; garbage token 401 "Invalid token format" |
| A4 expired token | PASS (2026-10-07 12:30 UTC, debug build): `full` and `readonly` tokens, expired at 11:24 UTC, probed 3,962 s / 3,956 s after `exp`: `GET /Patient` 401 `Token expired` in ~3 ms; server log `WARN helios_rest::middleware::auth: Authentication failed error=Token expired`. NOT measured: the 60 s leeway edge (the probe was far past `exp`), which #1173 documented as a note. |
| A5 smart-configuration | PASS: `issuer`, `authorization_endpoint`, `token_endpoint`, `jwks_uri` are the Okta server URLs; `grant_types_supported` `client_credentials`, `authorization_code`; `capabilities` `permission-v2`, `client-confidential-asymmetric`; `code_challenge_methods_supported` `S256`. Observation: `grant_types_supported` advertises `client_credentials`, which this Okta tenant cannot issue. |
| Interactive login (headless, test user) | PASS: `/ui` -> Okta -> `/ui/callback` -> `/ui`; cookie `hfs_session` (HttpOnly); in-page `fetch('/Patient')` with no Authorization header -> 200 (session bridge); log `web login completed subject={okta-id} user={test-user-name}`. Screenshot `~/tmp/okta-pass/shots/ui-after-login.png` |

Notes: the redirect to Okta from `/ui` happens on the first request when no session
exists; `/ui/` with a trailing slash returns an HTTP error status (not investigated,
the matrix uses `/ui`). Okta does not need `HFS_AUTH_AUDIENCE` unset: it is set and
validated (`aud` = `https://fhir.example.com`).

## HFS configuration used for the `sqlite` pass (exact, from `~/tmp/okta-pass/start-hfs.sh`)

Source: MANUAL_TESTING_MATRIX.md section 5 ("Common environment" + the `sqlite` row),
plus the Okta auth block of this log.

Applied as the matrix prescribes: `HFS_SERVER_HOST=127.0.0.1`, `HFS_SERVER_PORT=8080`,
`HFS_BASE_URL=http://localhost:8080`, `HFS_LOG_LEVEL=info`,
`HFS_DEFAULT_FHIR_VERSION=R4`, `HFS_REQUEST_TIMEOUT=600`,
`HFS_SUBSCRIPTIONS_ENABLED=true`, `HFS_BULK_EXPORT_OUTPUT_DIR` and `HFS_EXPORT_DIR`
(under `~/tmp/okta-pass/work/`), `HFS_STORAGE_BACKEND=sqlite`,
`HFS_DATA_DIR=<repo>/data` (search-parameter files). `HFS_MAX_BODY_SIZE` and
`HFS_BULK_SUBMIT_DEFER_INDEXING` are left at their defaults (128 MiB; `true`, which
the matrix prescribes for rows without Elasticsearch).

Added for Okta: `HFS_AUTH_ENABLED=true`, `HFS_AUTH_ISSUER`, `HFS_AUTH_JWKS_URL`,
`HFS_AUTH_AUDIENCE=https://fhir.example.com`, `HFS_SMART_TOKEN_ENDPOINT`,
`HFS_SMART_AUTHORIZE_ENDPOINT`, `HFS_SMART_JWKS_URL`, `HFS_UI_LOGIN_CLIENT_ID`,
`HFS_UI_LOGIN_CLIENT_SECRET`, `HFS_UI_LOGIN_SCOPES` (SMART scopes added),
`HFS_UI_LOGIN_COOKIE_SECURE=false`, `HFS_OUTBOUND_BEARER_TOKEN` (a user token).

Declared deviations from the matrix text:
1. **Debug build** (`cargo build --all-features`, `target/debug/hfs`) instead of the
   `--release` binary named in section 3/5. #1170 also ran on a debug build; timings
   are therefore not comparable with release runs.
2. **Database file** at `~/tmp/okta-pass/work/hfs.db` (`HFS_DATABASE_URL`) instead of
   `./data/hfs.db`, to keep the repo clean; the matrix's `HFS_DATA_DIR` is unchanged.
3. **Not set:** `HFS_COMPOSITE_SYNC_MODE`, `HFS_ELASTICSEARCH_WRITE_REFRESH` and
   `HFS_ELASTICSEARCH_REINDEX_REFRESH`. The matrix lists them under "every backend",
   but they only act on composite (Elasticsearch) backends, which are out of scope.
4. **Not applied on purpose** (Elasticsearch/Postgres only): the bulk-load tuning
   from the `helios-rest` README and #937/#939/#1173 (`INDEX_QUEUE`, `INDEX_CONCURRENCY`,
   `INDEX_COALESCE`, `ELASTICSEARCH_REFRESH_INTERVAL`, `HFS_PG_MAX_CONNECTIONS`,
   `FILE_CONCURRENCY`, `BATCH_SIZE`).
5. Not yet verified against the running process: the effective environment was not
   dumped from `/proc`; this section reflects the start script.

## How to explain the deviation (argument for the report)

1. **What #1180 asks:** run the matrix with auth against Okta, using a
   client-credentials (SMART Backend Services) client issuing `system/*.cruds`, plus a
   second read-only client; mint `HFS_OUTBOUND_BEARER_TOKEN` from the Okta client.
2. **Why that was impossible:** on the Okta trial org `{okta-tenant}`, a token request
   with `grant_type=client_credentials` for both M2M apps fails with `invalid_grant`:
   "The NHI Authentication Tokens SKU is not enabled. Contact your Account Executive
   to enable the SKU to use the requested grant type or token exchange flow."
   (System Log: `app.oauth2.as.token.grant | FAILURE |
   invalid_grant_type_or_token_exchange_sku_not_enabled`.) The same limitation is why
   the access-policy rule dialog never offered the Client Credentials grant. The
   apps, scopes and policy rules were configured correctly (read back via the
   Management API: apps ACTIVE, DPoP off, `client_credentials` grant,
   `m2m-backend` / `m2m-readonly` rules with `client_credentials`), so this is a
   tenant licensing limit, not a configuration error. Resolution needs Okta to enable
   the SKU on the account (account executive / paid plan).
3. **What was done instead:** Authorization Code + PKCE through `hfs-web`, the
   IdP-agnostic flow implemented in #1449, whose own text says "Later: the same flow
   against Okta / Auth0 / Entra (#1180-#1182)"; MANUAL_TESTING_MATRIX.md section 15
   already requires the browser login for T3 and the Tenants page. Tokens are user
   access tokens (`sub` = the user's login), issued by the same custom authorization
   server and carrying the same `iss`, `aud` and `scp` shape HFS validates for any
   bearer. A1-A5 were run with such tokens; the read-only case is a token requested
   with only `system/Patient.rs`.
4. **A second obstacle and its workaround:** the interactive sign-in to `hfs-web` was
   denied for the maintainer's own account (`policy.evaluate_sign_on | DENY ...
   UNSATISFIABLE`, "policy requirements could not be satisfied by the users' current
   set of available authenticator enrollments"). New OIDC apps inherit the system
   policy "Any two factors", whose rule requires a device-bound, phishing-resistant
   factor. Workaround, scoped to `hfs-web` only: a separate authentication policy
   (`HFS test: password only`, 1FA password) and a dedicated test user
   `{test-user}` created by API; reverting means re-assigning the system
   policy. This weakens sign-in for one disposable app in a disposable trial and
   must be declared as such.
5. **What is therefore NOT covered by this pass:** SMART Backend Services /
   client credentials against Okta; `docker/okta/get-token.sh` (still unverified, #724
   acceptance item); the Import page's `auth=backend-services` (`private_key_jwt`)
   variant; any claim that the Okta setup works with a real enterprise MFA policy.
6. **What this pass does show:** HFS validates Okta-issued JWTs (issuer, audience,
   JWKS, `scp` array), enforces scopes (200/201 vs 403), rejects tampered tokens,
   advertises Okta in the SMART discovery document, and completes the interactive
   login with an Okta session.

## Decisions of 2026-10-07 (owner) and the state after them

Owner decisions: (1) scope `sqlite` only; (2) **release, R4-only build**; (3)
`HFS_BULK_SUBMIT_DEFER_INDEXING=false` (index in the batch's own transaction); the
Elasticsearch-only knobs of #937 (`REFRESH_INTERVAL`, `INDEX_QUEUE/CONCURRENCY/COALESCE/MAX_WAIT`,
Elasticsearch request timeout) have no target on plain sqlite and are NOT set; (4)
credentials may live in this log and anywhere on this branch (disposable trial, the branch
is never merged); (5) the ui-login access token is raised to **1440 min** for the long
T3, with A4 expiry done on a separate 5-minute token; (6) MinIO / S3 output variants
(matrix 9.5 and 11.7) are **N/A by scope** (a different technology; the sqlite row of the
matrix includes them as optional S3 *output* variants); (7) T3 uses the **local full
corpus** (`~/hfs-t3/corpus`, 41 GB, 24 NDJSON + manifest; its counts equal the hosted
manifest: 18,955,865 resources, identical per type), served by nginx on :8000 (HTTP/1.1
keep-alive) with the matrix's byte check; the hosted S3 manifest (37.4 GB, ~13 MB/s measured
from this host) is the fallback; (8) the owner runs T3 by hand, the agent prepares and monitors;
(9) a step that cannot be evidenced with Playwright screenshots stops and asks the owner.

State (13:0x UTC):
- Release build: `cargo build --release -p helios-hfs --no-default-features --features
  R4,ui,sqlite,subscriptions`, 5 min 47 s, exit 0, `target/release/hfs` 148,787,664 bytes.
  Deviation from the matrix command (`--workspace --all-features`): single-version
  R4-only build, fewer features (no postgres, mongodb, elasticsearch, s3, cloudwatch, otel);
  declared like #1173 D2. The multi-version check (`fhirVersion=5.0`) is N/A.
- The debug HFS of the first smoke run was stopped; its database was kept as
  `hfs.db*.debug-bak`. A fresh database is used for the pass.
- Okta: ui-login access-token lifetime 60 -> 1440 min; briefly 5 min to mint one token
  for A4 and back to 1440. Tokens now last 86,400 s (`outbound`, `full` with refresh
  token, `readonly` = `openid system/Patient.rs`, no refresh).
- Release HFS started 13:02:42 UTC (pid 1370539) with `~/tmp/okta-pass/bin/start-hfs.sh`; the
  effective environment (secrets redacted) is `outputs/env-effective.txt`
  (`HFS_BULK_SUBMIT_DEFER_INDEXING=false`, `HFS_STORAGE_BACKEND=sqlite`, port 8080).
  Observation: a freshly created sqlite database already has a ~206 MB `-wal` file.
- A1-A5 and the T1 smoke re-run on the release: A1 401 (OperationOutcome) / open paths 200;
  A2 GET 200, POST 201; A3 GET 200, POST 403, GET /Observation 403; A4 tampered 401 "Invalid
  signature", garbage 401; A5 smart-configuration 200. T1: `/health` 200, `/metadata`
  200 (`fhirVersion` 4.0.1, 147 resource types, operations bulk-submit, bulk-submit-status,
  export, group-export, patient-export, reindex, sql-export, sql-run, validate, versions),
  `/ui` 200 (0.41 s). New vs the debug run: `rest.security` now advertises SMART-on-FHIR with
  the Okta authorize/token URIs (matrix finding #1441 fixed on main). Every response is in
  `outputs/*.body|hdr`; durations in `metrics/timings.tsv`.
- Probe Patient created by A2 deleted before T2 (deviation D6 of #1173); Patient total 0.
- **A4 expired token on the release: PASS** (13:10:09 UTC): a 5-minute user token, 92 s past `exp` (beyond the ~60 s `jsonwebtoken` leeway), `GET /Patient` 401 `Token expired`, 0.98 ms; log `Authentication failed error=Token expired`. A1-A5 are therefore complete on the release build.
- T3 prerequisites ready: nginx container `hfs-corpus` on :8000 (HTTP/1.1 keep-alive) serving `~/hfs-t3/corpus`; byte check 13:05:03 UTC, 12 s, **24/24 ok, 0 TRUNC** (`outputs/corpus-served-bytes.txt`, `outputs/corpus-counts.tsv`). T2 fixtures unpacked in `~/tmp/okta-pass/work/batch` (transaction 662 entries; batches of 9 and 8; same as the matrix) plus the three 6.4 negatives in `work/fixtures`.
- First Playwright capture `screenshots/T1-01-dashboard.png` (signed in, avatar "HT",
  1.4k stored resources).

Tooling (all outside the repo in `~/tmp/okta-pass/bin/` and `~/tmp/pw/`): `start-hfs.sh`
(release start + redacted env dump), `step.sh <id> <curl args>` (response, headers,
timing TSV), `sample.sh` (every 10 s: RSS, CPU ticks, db/wal/shm bytes, free disk, open
fds -> `metrics/samples.tsv`), `logscan.sh` (WARN/ERROR/refresh/reindex/bulk lines ->
`metrics/log-alerts.log`), `token.sh refresh <name>`, `corpus-serve.sh start|check|stop`,
and the Playwright helper `lib.mjs` (one saved login, numbered screenshots, stops on any
dialog/file chooser/download/popup with `STOP-AND-ASK`).

## Results: T2 Batch / Transaction (agent run, Playwright + curl), sqlite + Okta, release R4

Run 2026-10-07 13:1x-13:3x UTC on the empty server; signed in through `hfs-web` as the test user.
Method: 6.1 negative first by curl (no side effects), then every step through the UI with
Playwright (files attached with `setInputFiles` on the hidden `#batch-file` input), then API
verification. Owner decision on the upload question: option (a).

| Step | Result |
|---|---|
| 6.1 curl | PASS: `POST /` of the 662-entry transaction -> **400** `OperationOutcome` `invalid`: "Conditional reference 'Location?identifier=...' matches no existing resource" (51 ms); Patient / Encounter / Observation totals stay 0 |
| 6.1 UI | PASS: strip "POST [base] . Bundle . transaction . 662 entries", notice "Transaction: all or nothing - if any entry fails, the server rolls the whole bundle back.", error "The request failed. - Conditional reference 'Location?identifier=...' matches no existing resource" (execute 181 ms); Resources still empty; Cancel returns to Upload with the error cleared |
| 6.2 UI | PASS: hospital batch strip "batch . 9 entries", notice "Batch: entries run independently ...", outcome "9 created" HTTP 200, every row 201 (58 ms); practitioner batch "8 entries" -> 8 created (62 ms); rail Organization 4, Location 5, Practitioner 4, PractitionerRole 4 (API counts agree); `Organization?identifier=...756ed90d` -> 1, MEDWAY COUNTRY MANOR SKILLED NURSING & REHABILITAT |
| 6.3 UI | PASS: 662 entries -> "662 created" HTTP 200, all 201 (627 ms); `Patient?given=Nicky270&family=Larkin917&birthdate=1996-04-19` -> 1 (LPID `01a1168e-8679-7672-a93d-39ba5c6e6f93`); Encounter 49, Observation 106, Condition 33 for LPID; the Encounter carries literal `Organization/<id>`, `Practitioner/<id>` and `subject` `Patient/<LPID>` |
| 6.4 UI | PASS: "That JSON is not a FHIR Bundle.", "Only Bundles of type batch or transaction can be executed here.", "That file is not valid JSON." |

Differences from the Keycloak passes: no "Missing Authorization header" on the Batch page,
because the page runs with a signed-in session (the #1439/#1454 situation, fixed).

Deviations / problems found while running T2:
1. File upload by `setInputFiles` instead of drag-and-drop (owner-approved); the drag gesture is
   left to the owner's manual pass.
2. The first screenshots were taken with `fullPage`, which on the 662-entry lists produced images
   31,000 px tall (unreadable). Four were re-cut to their top 1440x1100 region
   (`*-top.png`) with Chromium; the long originals are kept in `screenshots/full/`. The
   helper now captures the visible window by default.
3. My first verification script asked for `total` on plain searches; HFS omits `total` unless
   `_total=accurate` is requested. Not an HFS defect, the script was fixed.
4. After T2 the HFS was restarted on a fresh database so the owner can repeat T2 by hand from an
   empty server; the T2 database was kept as `hfs.db*.after-T2-playwright`; the Playwright
   login state was discarded (sessions are stored in the database).

## Results: T8 and T9 steps 1-3 (agent run, Playwright + API), run before T3

Order deviation (declared like D7 of #1173): T8 and T9 do not use the corpus, so they ran on
the post-T2 state while T3 is still pending. Before them the HFS was restarted on the saved
post-T2 database (`hfs.db*.after-T2-playwright`), which restored Patient 1, Encounter 49,
Observation 106, Condition 33, Organization 4, Location 5, Practitioner 4, PractitionerRole 4.

| Step | Result |
|---|---|
| 12.1 receiver | rest-hook receiver on 127.0.0.1:9999 (`bin/webhook.sh`, log `work/webhook.log`); a self-test POST was cleared before the run |
| 12.2 topic + subscription (editor, Edit raw -> Save Changes) | PASS: both saved, the Unsaved pill disappears (608 ms each); handshake arrived with `"auth": "Bearer manual-token"` (1 line) |
| 12.2.4 activation | PASS: `GET /Subscription?_id=manual-sub&_elements=status` -> `active`; `_history` 2 versions (v1 `requested` POST, v2 `active` PUT); UI History tab shows `requested -> active` diff |
| 12.3.1 3 Encounters | PASS: batch "3 created" (65 ms); receiver log 4 lines (1 handshake + 3); last line `Bearer manual-token`, `events-since-subscription-start` = 3 |
| 12.3.2 Condition | PASS: saved; no new receiver line (a Condition does not match the topic) |
| T9.1-2 dashboard | PASS: ACTIVE 1 delivering, FAILING 0, IDLE 0, DELIVERED IN 24 H 3 (100.0% first try); row `manual-sub`, topic `encounter-start`, `rest-hook`, `http://127.0.0.1:9999/webhook`, Active, Sent 3, Fail streak 0 |
| T9.3 3 more Encounters | PASS: Delivered in 24 h 6, Sent 6; receiver log 7 lines |

Anchor patient note: T8's Encounters reference `Patient/7d24f7a0-...` which does not exist yet (the
corpus is not loaded); HFS accepts it (no referential integrity on create), as in #937.

## Results: T8 12.3.3 and T9 step 4 (failure path), run before T3

| Step | Result |
|---|---|
| 12.3.3 recovery inside the retry window | PASS: receiver killed 13:44:34, `encounters.json` uploaded, at 30 s the dashboard still reads ACTIVE 1, FAILING 0, streak 0 (Sent 9); receiver restarted 13:45:05; the 3 queued notifications arrived (3 new lines, 13:45:37 at the latest); dashboard Delivered in 24 h 9, **66.7% first try**, Active, streak 0 |
| T9.4 failure path | PASS: receiver killed 13:45:41, upload 13:45:42; at 30 s dashboard `Active`, Failing 0, streak 0 (Sent 12), while the log already held `Connection failed` lines (30 in total), exactly as the matrix says; `Max retries exhausted` at 13:50:44 (**~5 min** after the upload); then ACTIVE 0, **FAILING 1**, row chip **Error**, **Fail streak 3**, Delivered 9 (66.7% first try) |
| T9.4 Sort menu | the Sort control was clicked (`T9-B-sort-menu` screenshot); its options were not individually exercised by the script |
| T9.4 recovery after the window | PASS on the second attempt: the subscription is `status=error`; `PUT` with `status: requested` -> new handshake with `Bearer manual-token` -> `active` (v7), dashboard ACTIVE 1, FAILING 0, streak 0, Sent 0 |

Harness error found while running it (NOT an HFS defect): the first reactivation PUT was sent ~0.3 s
after starting the receiver, which was not yet listening; HFS tries the handshake once
(`max_attempts=1`), got `Connection failed` and returned the subscription to `error` (v5). Repeating
the PUT with the receiver listening worked. A self-test POST to the receiver (`/x`, `/ready`) left
two extra lines in `work/webhook.log`.

Clean-up before T3 (deviation, to avoid notifications while ~828k Encounters are imported and
because the engine's behaviour on bulk ingest was not verified): `Subscription/manual-sub` and
`Basic/manual-topic` were deleted (204) and the receiver stopped. T8 12.2 and T9 steps 5-6
(restart rehydration, engine disabled) are to be re-run after T3.

State before T3 (API counts): Patient 1, Encounter 61, Observation 106, Condition 34,
Organization 4, Location 5, Practitioner 4, PractitionerRole 4, Subscription 0, Basic 0.
The T2 data is Patient 1 / Encounter 49 / Observation 106 / Condition 33 / reference data;
T8-T9 added **12 Encounters and 1 Condition** (all referencing the anchor patient
`Patient/7d24f7a0-...`, which T3 creates): T4 counts for that patient must allow +12 Encounters
and +1 Condition. HFS RSS 164 MB, db 14.0 MB, wal 206 MB, disk free 766 GB, 0 ERROR lines.

T3 tooling added: `bin/t3-monitor.sh` (every 30 s: submission and manifest status,
processed/total/failed entries, files done, index_pending -> `metrics/t3-progress.tsv`; `STALL`
alert after 15 min without progress) and `bin/t3-counts.sh` (per-type API counts vs
`corpus-counts.tsv` -> `outputs/t3-counts.tsv`).

## T3: full Synthea import (run by the owner, monitored by the agent)

- Submission `synthea-sqlite`, id `1d95ae55-5ba3-4677-8c97-6c71ef806241`, created by the owner from
  `/ui/bulk-import` (through the SSH tunnel, signed in as the test user), manifest
  `http://localhost:8000/manifest.json` (local nginx, HTTP/1.1, 24/24 byte check ok),
  authentication None, Data Recipient `http://localhost:8080`.
- **Kick-off 2026-10-07T14:07:57.580Z** (manifest accepted by the recipient 200 at 14:07:57.592Z,
  status kick-off request 14:07:57.593Z). Settings: release R4 build, sqlite,
  `HFS_BULK_SUBMIT_DEFER_INDEXING=false`, HFS_REQUEST_TIMEOUT 600, 24 h Okta tokens.
- First readings (`metrics/t3-progress.tsv`): 14:08:18 processed 44,953; 14:08:48 processed 110,391
  (~2,100 resources/s, 3 files done); HFS RSS 464 MB; db 1,165 MB; wal 201 MB; free disk 766 GB;
  health 200; 0 ERROR lines. HFS logs one `sqlite WAL checkpoint after a bulk-submit file` per file.
- Observation: the status card of the detail page read "Processing 0% - 4,600 Resources written"
  while the database already held >100k processed entries (the card lags, as the matrix section 5
  warns for the 5 s poll).

### T3 progress notes (agent monitoring)

| UTC | processed | % | files done | note |
|---|---|---|---|---|
| 14:07:57 | 0 | 0 | 0 | kick-off |
| 14:08:48 | 110,391 | 0.6 | 3 | ~2,100 res/s at the start |
| 15:08 | 3,127,404 | 16.5 | 6 | 870 res/s average, 533/s last 5 min |
| 15:22 | 3,542,604 | 18.7 | 6 | ~480 res/s |
| 18:22 | 10,336,127 | 54.5 | 16 | 677/s average over 4.24 h; 444/s last 10 min; 646/s last 60 min |

At 18:22 UTC: failed 0, `ERROR`/`panic` lines 0, HFS RSS 1,449 MB, db 153.8 GB, wal 242 MB, free disk 736 GB,
health 200, 8 `sqlite busy during heartbeat; retrying` WARN lines (14:46 ... 18:16, each retried successfully),
no real stall (longest flat stretch < 5 min). Files finish at uneven intervals (e.g. file 6 at 14:43, file 7 at
15:37, file 10 at 17:16), consistent with a few large files (Observation, 7.7 M lines) among many small ones.

Harness issue: the monitor logged one `STALL` at 14:07:48 UTC, 9 s **before** the kick-off: its 15-minute
no-progress timer ran while no submission existed. Not an HFS problem. `t3-monitor.sh` now raises `STALL`
only while the submission is `in-progress` (the running instance keeps the old logic; ignore that line).

## Results: T3 full Synthea import (sqlite, release R4, DEFER_INDEXING=false, Okta user session)

| Item | Result |
|---|---|
| Result card | **Status Completed**; "Processing finished at 2026-10-08T00:55:55.104Z"; **Output files 24, Error files 0**; log ends `Status: got 200 OK - processing finished cleanly (24 outputs); submission completed.` (screenshot `T3-01-submission-detail-final.png`, text `outputs/T3-detail-submission-detail-final.txt`) |
| Ingest time (kick-off 2026-10-07T14:07:57.580Z -> finished 2026-10-08T00:55:55.104Z) | **10 h 47 min 57.5 s** (38,878 s), average **488 resources/s** |
| Rate by hour (res/s, from `metrics/t3-progress.tsv`) | 14h 905, 15h 492, 16h 581, 17h 811, 18h 434, 19h 333, 20h 307, 21h 296, 22h 274, 23h 457, 00h 444 (sustained decline as the database grew; the last ~2 % (97 % to 99 %, 00:31 to 00:51) advanced only ~500 resources per 2-minute poll) |
| Processed / failed | 18,955,865 of 18,955,865 entries, 24 of 24 files, **0 failed, 0 skipped** |
| 7.4 search rebuild | none needed (`DEFER_INDEXING=false`: the index is written in each batch's transaction); no `reindex` line in the log, `index_pending` 0. The matrix's `bulk-submit indexed every resource during ingest; no deferred reindex needed` message was **not** printed on this plain-sqlite path (it is documented for Elasticsearch composites); not a failure |
| 7.5 per-type counts (`outputs/t3-counts.tsv`) | **PASS**: no type below the corpus (0 shortfall); total on the server 18,956,557 = corpus 18,955,865 + 692. The +692 is exactly what was created before T3: 662 (T2 patient transaction) + 17 (reference data: Organization 4, Location 5, Practitioner 4, PractitionerRole 4) + 13 (T8/T9: 12 Encounter, 1 Condition) |
| Index-dependent searches right after the import | Patient `gender=female` **5,814**, `gender:not=female` **5,891**, SSN identifier **1**, anchor patient identifier `7d24f7a0-...` **1**, Observation `loinc|8302-2` **175,355** (> 175,000): all as the matrix expects |
| Search latency note | `Observation?code=loinc|8302-2&_total=accurate&_count=1` took **86.1 s** (exact total over 7.7 M Observations); the other four searches 1.4 ms to 0.97 s |
| HFS resources during the ingest | RSS peak 2,334 MB (464 MB at the start); database 257.4 GB at the end; **WAL peak 9,685 MB** (typically 200-250 MB when sampled); free disk min 711 GB; open fds max 38; `health` 200 throughout |
| Log health | 0 `ERROR`/`panic`; 35 `sqlite busy during heartbeat; retrying` WARN lines (worst case attempt 6, ~7.6 s of a 30 s budget), every one recovered; no worker/lease loss; no stall |

Observations / possible findings to review at the end (nothing filed during the pass):
1. Throughput falls from ~900 to ~300 resources/s as the sqlite database grows with inline indexing; the
   ingest takes 10.8 h on this host (4 vCPU-class LXC, NVMe/ZFS). For comparison #937 (sqlite + Elasticsearch)
   took 4 h 36 min and #1173 (pg-es) 5 h 49 min; not comparable (different backend and settings).
2. The `bulk_submissions.status` column of the sqlite table still read `in-progress` (and `completed_at`
   empty) hours after the page and the manifest said Completed; the page derives its status from the
   recipient's status report. Not investigated.
3. An exact-total search over the largest type is slow (86 s).
4. WAL grew to 9.7 GB at some point although each file triggers a passive checkpoint.

Harness notes: the T3 monitor, sampler and log scanner ran for the whole import without gaps; the nginx
corpus container was stopped afterwards (the owner's `~/hfs-t3/corpus` was only read).

## Parallel phase: T4, T5, T6, T7 (started 2026-10-08 ~03:10 UTC)

Owner decision: run the independent tracks at the same time with sub-agents, monitoring and timing
everything. The lead agent (this log's author) keeps the server, the log and the owner relationship;
four sub-agents work on separate tracks against the one running HFS (release R4, sqlite, 8080) using the
brief `~/tmp/okta-pass/AGENT-BRIEF.md` (rules: no restart, no Okta changes, no repository edits, one heavy
request at a time, health guard, STOP-AND-ASK protocol, every step timed in `metrics/steps.tsv`, each
agent writes only its own `outputs/T<n>-RESULTS.md`).

| Track | Scope (matrix) | Dependencies / sync markers |
|---|---|---|
| T4 | 8.1 fixtures, 8.2-8.4 searches (API sweep `outputs/t4-api-sweep.tsv` + UI QUERY box screenshots) | creates `RiskAssessment/manual-risk` and `ValueSet/manual-test-vs`, then `work/t4-fixtures-done` |
| T6 | 10.1-10.3 ViewDefinitions in the UI | saves `patient_demographics` and `observation_flat`, then `work/t6-vds-saved`; never deletes the originals |
| T5 | 9.1-9.4 `$export` (9.5 N/A) | creates `Group/manual-group`; rows needing the T4 fixtures wait for `t4-fixtures-done` |
| T7 | 11.1-11.8 SQL export (11.7 N/A) | waits for `t6-vds-saved`; steps needing an HFS restart are DEFERRED to the lead |

Ordering rationale: T8 12.2 and T9 steps 5-6 (engine restart / disabled) run last, after these tracks, because
they restart HFS. Because the tracks overlap, **durations measured in this phase are not benchmark values**
(they share one sqlite database, one process and 8 cores). Baseline at 03:07 UTC: health 200, HFS RSS 2,460 MB,
db 257.4 GB, free disk 714 GB, free memory 20 GB.

## Owner confirmations of 2026-10-08

1. **Browser downloads (T5 ZIP and pills, T7 pills) and the T7 "Copy job id" label:** the owner verified
   them by hand before and during this pass; together with the agent's curl evidence they are accepted as
   verified. The same rule applies to any similar download.
2. **Row cap on SQL Query dependencies (`observation_flat` over 1,000,000 rows, HTTP 422, known issue #1473):**
   reported in this run as known and expected; no new issue.

## Resource measurement: HFS restart and a single clean export (2026-10-08, 09:37-09:42 UTC)

Done with nothing else running on the server (the parallel tracks had finished). Sampler every 2 s,
`metrics/mem-clean-after-restart.tsv`.

| Item | Result |
|---|---|
| Graceful stop (SIGTERM) and start | stopped in 2 s, started in ~2 s; data intact (Patient 11,705, Encounter 827,980, Observation 7,699,987); the browser login session survived the restart (sessions are stored in the database); `Subscription engine rehydrated ... subscriptions=0` |
| Idle, new process | RSS 130-465 MB (n=36 samples) |
| One export `mem-clean-both-vd` (SQL Export of `patient_demographics` + `observation_flat`, NDJSON, 7,699,987 + 11,705 rows) | Complete in **2 m 04 s**, 17 files; RSS grew roughly linearly from 467 MB to a **peak of 9,936 MB** at 09:41:02 (mean 5,617 MB), CPU mean 122 % / max 156 % of one core, 14 threads max, system available memory never below 13,304 MB |
| After the export | RSS 9,386 MB at the first sample after completion, **1,218 MB** about 60 s later (memory is returned, no growth left behind) |
| System swap | constant at 10,548 MB used throughout (virtual swap of the container; unrelated to the run) |

## Results: T4, T5, T6, T7 (four parallel tracks, 2026-10-08 03:10 UTC onwards; T4 resumed 08:50 UTC)

Per-track detail: `outputs/T4-RESULTS.md`, `T5-RESULTS.md`, `T6-RESULTS.md`, `T7-RESULTS.md`; every step has a line with
start/end/result in `metrics/steps.tsv`; screenshots in `screenshots/`. The tracks overlapped on one server, so
**durations are not benchmark values**. HFS pid unchanged and `health` 200 throughout; 0 `ERROR`/`panic`.

| Track | Result | Notes |
|---|---|---|
| T4 (8.1-8.4, 90 API rows + first query of every row in the QUERY box, 71 screenshots) | **PASS except 4.11** | fixtures `RiskAssessment/manual-risk` and `ValueSet/manual-test-vs` created in the editor; expectations adjusted for the +692 extras (PID: 36 Encounters = 24 + 12, 16 Conditions); 4.3d, 4.20e, 4.22b, 4.10b were wrong expectations or script errors and pass once corrected |
| T5 (9.1-9.4; 9.5 N/A) | **PASS** | exports 5.1-5.13 as in the matrix with adjusted counts; Patient exports come in 1,000-line files (12 files for 11,705 Patients, the matrix says 1 file); download pills and the ZIP verified with curl and by the owner |
| T6 (10.1-10.3) | **PASS** | `patient_demographics` and `observation_flat` saved; PID row `female, 2015-12-29, Parker433, Everett`; lint typo gives 2 issues (matrix 1); Ctrl+. opens the lint panel when two fixes exist |
| T7 (11.1-11.8; 11.7 N/A) | **PASS except the SQL Query exports** | the exports that depend on `observation_flat` inside a SQL Query fail with the known 422 row cap (known #1473, expected); `both-vd` (patient_demographics + observation_flat) 2 m 20 s, 17 files, 7,699,987 Observation lines = the server total |

T4 row 4.11 (`Patient?_has:Observation:patient:code=http://loinc.org|8302-2`) **FAIL**: no response after 600 s and
again after 900 s (08:49:45 to 09:04:45); HFS stayed up (same pid, `health` 200) but ran at 100-120 % CPU for ~25 more
minutes and other reads were slow or timed out meanwhile; no 504 came back although `HFS_REQUEST_TIMEOUT` is 600 s.
**Re-run (2026-10-08, 18:25:11 to 18:40:11 UTC)** with a verified live session (token valid 15.2 h, a read with it returned 200), HFS idle (0 connections, nothing else running) and resource sampling: the same request again got **no response after 900 s** (client timeout, HTTP 000, 0 bytes). HFS stayed up (same pid, `health` 200); RSS grew from 139 MB to 561 MB, CPU averaged 15 % and peaked at 112 % of one core, 0 errors or panics in the log. Row 4.11 stays **FAIL**, reproducible.
Not run in the UI. Slowest other T4 searches (with contention): `Observation` 4.21c 492 s for 2 results, 4.27b 211 s,
4.21d 183 s, 4.5a 165 s, 4.6a 137 s, 4.10a 104 s; `Encounter?patient=PID&date=ge2016` 31-38 s.

T4 deviations from the matrix text: the builder rail seeds `_summary=true` (extra parameter in the URL); 4.29c gives 54
included entries (matrix 42) because of the 12 extra Encounters; 4.13 `_content=Everett` returns 83; the result
modals of 4.5, 4.21, 4.22 and 4.27 were checked on the JSON bodies, not in the UI. **"Open in New Tab"
(matrix lines 744 and 828): the link does not exist in the Results card** (not in the templates nor in the live DOM);
the executed URLs were taken from the browser's network log and verified with curl. **Owner decision
(2026-10-08): N/A.** The button was removed on purpose by #958 (closed 2026-09-08, "The 'Open in New Tab' button is
gone"); the matrix text (lines 744, 784, 806 and 828) predates that change. It is outdated text, not a UI defect.

## Results: T8 12.2, T9 steps 5-6 and the deferred T7 restart (2026-10-08 09:35-09:55 UTC)

| Step | Result |
|---|---|
| T8 12.2 again after T3 (topic + subscription through the editor) | PASS: saved in ~0.6 s each, handshake received with the configured `Authorization` header, subscription `active` |
| T9.5 restart rehydration | PASS: log `Subscription engine rehydrated tenants=1 topics=1 subscriptions=1 dormant=0 failed=0 handshakes=0`; subscription still `active` (same version, nothing re-created); dashboard ACTIVE 1, FAILING 0; **0** `Failed to persist subscription status transition` lines in the whole log |
| T7 11.3 restart with a job In progress | PASS: the running `restart-job` card resolves to "Cancelled - the server no longer knows this job" (jobs live in memory), the complete card `mem-clean-both-vd` stays Complete and its files still download (HTTP 200 after the restart) |
| T9.6 engine disabled (`HFS_SUBSCRIPTIONS_ENABLED=false`) | PASS: the page renders only "The subscriptions engine is not enabled on this server. Turn it on by starting HFS with: HFS_SUBSCRIPTIONS_ENABLED=true . --features subscriptions"; the sidebar entry is still present; then restarted back to normal (rehydrated again) |

## Matrix row: `sqlite` + Okta (user tokens), release R4, `DEFER_INDEXING=false`

| A1-A5 | T0 | T1 | T2 | T3 | T4 | T5 | T6 | T7 | T8 | T9 |
|---|---|---|---|---|---|---|---|---|---|---|
| PASS | PASS (release R4-only) | PASS | PASS | PASS (10 h 47 min 57.5 s) | PASS except 4.11 (FAIL); "Open in New Tab" N/A (button removed by #958) | PASS | PASS | PASS except the SQL Query exports (known 422 row cap) | PASS | PASS |

Not run: 9.5 and 11.7 (MinIO / S3, out of scope), `fhirVersion=5.0` check (R4-only build). SMART Backend Services /
client credentials with Okta: not verified (tenant limitation, see above).

## Still to do before the matrix

- [x] `ui-login` policy and rule (API).
- [x] Client secrets stored.
- [x] Access tokens without client credentials: user tokens via `hfs-web` (done, see above).
- [x] HFS on 8080 with the Okta env; A1, A2, A3, A4 (tamper), A5 done.
- [x] A4 expired-token check (done 12:30 UTC; see table).
- [x] T2 to T9 done. [x] owner answer on "Open in New Tab": N/A (#958); sanitise and commit the evidence; the owner pushes; update the tracker cell; book chapter; Okta revert.
- [ ] Revoke the admin API token `hfs-setup`; delete the local env file at the end.
- [ ] A1-A5, then T0-T9 per backend.

## After the pass: Okta sign-in page branding (2026-10-09)

Not part of the matrix. The owner asked how the Okta sign-in page compares with the Helios-themed Keycloak login
(`docker/keycloak/themes/helios/`, PR #207) and what can be done on the Okta side. Every change below was made in
the **Okta trial org**; HFS, its configuration and the running server were not touched.

Baseline, read-only Management API calls (all HTTP 200 unless noted):
- `GET /api/v1/brands`: one brand, default, `removePoweredByOkta: false`.
- `GET /api/v1/brands/<brand-id>/themes`: one theme, `primaryColorHex #1662dd`, `secondaryColorHex #ebebed`, Okta logo,
  no background image, every touch-point variant `OKTA_DEFAULT`.
- `GET /api/v1/brands/<brand-id>/pages/sign-in/customized`: **404**, no customized sign-in page.
- `GET /api/v1/domains`: only the org's own Okta domain, so the fully custom HTML/CSS sign-in page is not available.
- `GET /api/v1/apps?q=hfs-web`: the application logo is Okta's default image (a gear, 36 x 36).

The sign-in page is the stock Okta page: a "Connecting to" band with the application's icon, a card with the Okta
logo, an identifier-first form, a blue button and a "Powered by Okta" footer.

| Experiment | Where | Result |
|---|---|---|
| A. Application logo | Okta, application `hfs-web` only | **Done, works.** `POST /api/v1/apps/<app-id>/logo` with `crates/ui/assets/logo.png` (PNG, 120 x 117) returned HTTP 201. On the next page load the gear next to "Connecting to" became the Helios logo; no HFS restart. The card logo, the button colour and the footer did not change. |
| B. Brand theme (logo, primary colour) | Okta, whole org | **Not run** (owner decision): it changes every sign-in page of the org, the Admin Console sign-in included. |
| C. Remove "Powered by Okta" | Okta, whole org | **Not run.** |
| D. Custom HTML/CSS sign-in page | Okta plus a custom domain | **Not available** on this org (no custom domain). |

Screenshots (Playwright, 1280 x 800, no address bar): `screenshots/BR-A-01-okta-sign-in-before-app-logo.png` and
`screenshots/BR-A-02-okta-sign-in-after-app-logo.png`.

Not verified: the Admin Console path for the application logo, and going back to the default gear (no call for
removing an application logo was tried; the original image was saved outside the repository so it can be uploaded
again). The steps are documented for readers in the Okta page of the book (issue #1878, PR #1879).

## Credentials

No credential, token, password or user name is recorded in the repository. The full unredacted copy of this evidence is kept outside the repository by the owner.
