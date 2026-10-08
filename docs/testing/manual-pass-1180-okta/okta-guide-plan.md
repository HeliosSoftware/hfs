# Plan: "Using HFS with Okta" chapter for the book, and the skill to write it

**This file is the single source of truth for this work.** Every decision taken in the chat and
every finding that changes the plan is recorded here, newest entries in the change log at the end.
The chapter itself is written only after the manual test pass of #1180 is finished, so that it
contains what was actually verified. Nothing under `book/` or `.claude/skills/` has been created yet.

Related artifacts: `OKTA_SETUP_LOG.md` (every step, deviation and credential of the pass),
`MANUAL_TESTING_MATRIX.md`, `crates/auth/README.md` ("Okta" section, client-credentials only),
`docs/auth-verification.md`.

## 1. Goal

Final goal (owner): anyone who wants to run HFS with Okta can follow the exact steps and
configuration in the book, in the same format as the existing book pages.

Two deliverables:

1. A **skill** that knows how to write book documentation in this repo's format.
2. A **book chapter** with the complete Okta setup for HFS, written from what the #1180 pass verified.

## 2. Decisions (all confirmed by the owner, 2026-10-07)

| # | Decision | Value |
|---|---|---|
| 1 | Where the chapter goes | `book/src/components/okta-authentication.md`, one new entry in `book/src/SUMMARY.md` next to the existing "Web UI Self-Calls and Authentication" entry. A multi-provider "Authentication" section is not created now. |
| 2 | Language | English for the chapter and for the skill (the book is English). |
| 3 | When | After the #1180 pass, so the chapter contains what T3 to T9 taught us. The plan is updated as findings arrive. |
| 4 | mdBook installation for preview | Deferred until the Synthea import (T3) has finished: `cargo install mdbook` competes for CPU with it. Nothing that competes for resources runs meanwhile. |
| 5 | Tenant values in the guide | Placeholders only (`{domain}`, `{auth-server-id}`, `{client-id}`), never the identifiers of the test tenant. |
| 6 | What may be done now | Only text work that does not compete for resources (this plan, analysis). No builds, no previews, no installs. (Superseded for the install by decision 9, 2026-10-08.) |
| 7 | Order of work (2026-10-08) | The **skill first**, then the book chapter written with it. The #1180 manual pass is finished, so the chapter now has the verified facts it needs. |
| 8 | Who does what (2026-10-08) | **Opus 5.5 is the advisor**: mandatory for designing and reviewing the skill, and called for the hard parts of the chapter. **Sonnet 5.5 and Haiku are the only implementers** (the lead agent coordinates and also runs as Sonnet): Haiku for simple, mechanical tasks (extracting tables from code with grep, link checks, formatting), Sonnet for prose and step-by-step procedures. Independent tasks run in parallel on different files. |
| 9 | mdBook install and preview (2026-10-08) | Allowed again, but only when no heavy test is running on the dev box (it compiles from source); the 4.11 re-run is the last heavy test of the pass. |
| 10 | Topics kept out of the docs (2026-10-08) | Out-of-scope observations of the test run that the owner decided not to document are not written into the chapter or the skill. |

## 3. Technology facts about the book (verified)

- Generator: **mdBook 0.4.40** (pinned in `.github/workflows/ci.yml`), plain markdown.
- `book/book.toml`: title "Helios FHIR Server", `src = "src"`, `site-url = "/hfs/"`, edit-url template
  pointing at `book/src/{path}`, the only preprocessor is `links`. No custom theme, CSS, JS,
  mermaid, admonition plugin, image folder, or link checker.
- Local: `cargo install mdbook`, then from `book/`: `mdbook serve` (http://localhost:3000) or
  `mdbook build` (output `book/book/`, git-ignored). **mdbook is not installed on the dev box.**
- Publishing: job `publish-report` in `ci.yml`, **only on tags `v*`** (verified). `book/README.md` says
  pushes to `main` deploy it; that is wrong, so the book is updated only when a version is tagged.
- CI only runs `mdbook build`: no link, spelling or lint checks. Relative links must be checked by hand.
- `SUMMARY.md`: flat list of 16 entries (ch01..ch14 plus two `components/` pages), then `---` and
  `# Appendices`. Only pages listed in `SUMMARY.md` are rendered.
- `book/src/configuration/`, `getting-started/`, `development/` exist but are not in `SUMMARY.md`
  (not rendered, apparently leftovers). Not to be touched by this work.
- Published at https://heliossoftware.github.io/hfs/.

## 4. Page conventions (from `components/web-ui-self-calls.md` and `components/natural-language-search.md`)

- One `# Title`, an intro paragraph (what and why, bold key terms), `##` sections, `###` sparingly,
  hard-wrapped at about 78 columns, 40 to 160 lines.
- Second person, imperative, present tense; limits stated openly.
- Tables are the main structure: configuration tables `| Variable | Default | Description |` with
  env vars in backticks and `*(unset)*` for no default; mode matrices with bold labels in column 1.
- No admonition syntax: asides are prose, bold lead-ins, or headings such as "Degraded state".
- Code blocks tagged `bash` or `json`, plain commands, `export HFS_...=...`, no `$` prompt.
- Cross-links are relative `.md` paths. No images, no diagrams.
- New page: create `components/<kebab-name>.md` and add one line to `SUMMARY.md`.

## 5. The skill (`work-with-book`, to be created after the pass)

Follows the layout of the 19 existing skills (`.claude/skills/<name>/SKILL.md`, mirrored in
`.agents/skills/`). No documentation skill exists today; `work-with-auth` covers the auth code.

Contents:

1. Technology and commands (mdBook 0.4.40, serve/build, the tag-only publishing).
2. The conventions of section 4, with an empty page template to copy.
3. Workflow: create the `.md`, add the `SUMMARY.md` entry, preview with `mdbook build`, check every
   relative link by hand, and no edits to the unlisted directories.
4. Content rules: every environment variable comes from the code (grep it), every command was run,
   anything not verified is stated as not verified, tenant-specific values are placeholders.
5. A pointer to this plan and to `OKTA_SETUP_LOG.md` as source material for the Okta chapter.

## 6. The chapter: outline of "Using HFS with Okta"

Source material is what #1180 verified (see `OKTA_SETUP_LOG.md`). Draft outline:

1. Intro: what works (interactive login with user tokens, bearer validation, scopes) and what does
   not (see "Not verified").
2. Prerequisites: an Okta org with **API Access Management** (custom authorization server).
3. Okta setup, in order:
   - custom authorization server `FHIR`, fixed issuer ("Okta URL", not "Dynamic"), audience;
   - the scopes (`system/*.cruds`, `system/Patient.rs`, `system/Observation.r`, `user/*.cruds`,
     `system/bulk-submit`), published in the public metadata, not default;
   - the login app (OIDC Web Application, Authorization Code + Refresh Token, redirect URI
     `{HFS_BASE_URL}/ui/callback`, sign-out `{HFS_BASE_URL}/ui`, DPoP off, PKCE);
   - machine-to-machine apps (API Services) and their access policies, with the caveat in
     "Not verified";
   - access policies for the login app (Authorization Code, the SMART scopes, token lifetime);
   - the sign-in policy of the login app (the default "Any two factors" requires a device-bound,
     phishing-resistant factor; what to change and the consequence).
4. HFS configuration: `HFS_AUTH_*`, `HFS_SMART_*`, `HFS_UI_LOGIN_*`, `HFS_BASE_URL` with a table;
   the scopes of `HFS_UI_LOGIN_SCOPES` must include SMART scopes (default has none).
5. Run and verify: curl checks for 401 without token, 200/201 with a full token, 403 with a
   read-only token, tampered and expired tokens, and the discovery document; the UI login.
6. Troubleshooting table (issuer mismatch, audience, `scp` array, DPoP, `UNSATISFIABLE` sign-on,
   token lifetime vs long imports, redirect URI mismatch, one-shot subscription handshake...).
7. Not verified / limits: see below.

### Not verified or limits to state in the chapter

- **Client credentials (SMART Backend Services) with Okta was not verified**: on the trial org the
  token request fails with `invalid_grant` "The NHI Authentication Tokens SKU is not enabled", and the
  access-policy rule dialog does not offer the Client Credentials grant. The guide must say so and
  must not present that flow as working.
- HFS does not read `groups`/`roles` claims; authorization is only SMART-scope based.
- The interactive login does not verify the ID token signature, `iss`, `aud` or `nonce`
  (`crates/auth/src/session.rs`).
- `docker/okta/get-token.sh` was not run (client credentials unavailable).
- The sign-in policy was relaxed to password-only for the test app only; production tenants should
  keep their own MFA policy and need an authenticator that satisfies it.

## 7. Facts gathered so far that feed the chapter (verified in the pass)

- Issuer fixed to the "Okta URL" form; metadata at `<issuer>/.well-known/oauth-authorization-server`
  lists the published scopes; JWKS at `<issuer>/v1/keys`.
- User access tokens carry `iss`, `aud`, `sub` (the login), `scp` as an **array**, `cid`, `exp`; the
  ID token carries `name`, `email`, `preferred_username`, no `picture`; no tenant claim.
- `scp` array is parsed by HFS; scopes are enforced (read-only token: 403 on create and on other types).
- A tampered signature gives 401 `Invalid signature`; an expired token gives 401 `Token expired`
  (JWT validation has a ~60 s leeway).
- New OIDC and API Services apps have **DPoP enabled by default**; HFS does not do DPoP.
- The default policy "Any two factors" denies users without a device-bound, phishing-resistant
  authenticator (`policy.evaluate_sign_on ... UNSATISFIABLE`).
- The Management API can create the access-policy rules when the admin UI cannot.
- Okta access-token lifetime on a custom authorization server rule: 5 minutes to 1 day
  (set to 1440 min for the long import).
- A subscription handshake is attempted once; start the receiver before activating.
- Port: the redirect URI is fixed, so HFS must run on the registered port (`HFS_BASE_URL` moves with it).

## 8. Open items

- Create the skill and the chapter after the pass (needs: T3 finished, T4 to T9 results).
- Decide whether to add an `HFS_AUTH_*` section to `book/src/configuration/environment-variables.md`
  (that directory is not rendered today; out of scope unless the owner asks).
- Install mdBook and preview, once T3 has finished.
- Decide whether to correct `book/README.md` (publishing trigger) or open an issue for it.

## 9. Change log

| Date (UTC) | Change |
|---|---|
| 2026-10-07 | Plan created. Decisions 1 to 6 confirmed by the owner. Book technology, conventions, CI and the missing skill analysed; the tag-only publishing and the unrendered directories verified. |
| 2026-10-08 | The #1180 pass is finished (A1-A5, T0-T9 on sqlite + Okta). Decisions 7 to 10 added: skill first, then the chapter; Opus 5.5 advisor (mandatory for the skill), Sonnet 5.5 and Haiku the only implementers; mdBook install allowed when no heavy test runs; out-of-scope run observations stay out of the docs. Opus advisor review of the skill design requested. |
