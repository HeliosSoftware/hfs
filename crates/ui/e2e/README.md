# helios-ui browser tests (Playwright + axe-core)

The outer ring of the UI test pyramid (issue #249): behavior only a real browser
can observe — WCAG 2.2 AA conformance, `theme.js` before first paint, the
`/_user/settings` merge-patch, progressive enhancement with JS off, and the
no-CDN invariants. The fast inner ring stays in Rust (`crates/ui/tests/*.rs`,
`tower::oneshot`).

Everything Node lives here; the cargo workspace is untouched.

## Layout

Specs live in `tests/`; the **Page Object Model** lives in `pages/` — one class
per page/component, wired onto Playwright's `test` via `pages/fixtures.ts`. Specs
import `{ test, expect }` from `../pages/fixtures` and receive page objects as
fixtures (`async ({ resources, history }) => …`); `pages/api.ts` seeds resources
over the REST API for state-dependent tests.

Most projects share one server and one user (`l2:`, the default when auth is
disabled) across every test. The server-side rail state introduced by
#754/#755 — "recently used" + "last selected" per page, stored in `rails`
under `/_user/settings` — is therefore per-*user*, not per-test: without a
reset, a selection recorded by one test would still be there for the next one
to restore from. `pages/fixtures.ts`'s `page` fixture resets `rails` (a merge
patch of `{"rails": null}`) before every test runs. Nothing else under
`/_user/settings` is touched by the reset.

The reset is **100% best-effort** and never fails a test over its own result:
the whole request is wrapped in `try`/`catch`, and no response status is
checked at all. This matters beyond the plain SQLite leg — the `auth` and
`auth-degraded` projects run against `HFS_AUTH`-enabled servers and this
fixture carries no bearer token, so the reset there always comes back
401/403; other legs can 501 (no settings store configured) or fail to connect
outright. All of that is silently swallowed; only navigation failures a test
actually depends on should ever fail it.

| Path | What it covers |
|------|----------------|
| `tests/a11y.spec.ts` | axe-core WCAG 2.2 AA over every full page, light × dark |
| `tests/no-cdn.spec.ts` | no off-origin requests, no page errors, no inline `<script>` — every page |
| `tests/theme.spec.ts` | FOUC guard, OS-preference precedence, PATCH merge-patch, server-roam, graceful degradation |
| `tests/chrome.spec.ts` | the collapsible nav: toggle, aria sync, localStorage cache, `/_user/settings` roam |
| `tests/dashboard.spec.ts` | stat cards, chart, time-window + legend link controls |
| `tests/resources.spec.ts` | type rail (filter + live counts), **every resource type is reachable**, modal open/close, delete |
| `tests/resources-editor.spec.ts` | edit flows: Create targets the picked type, inline binding validation, Save blocked on invalid, raw-edit round-trips |
| `tests/editor-controls.spec.ts` | fold/expand, add-node (+filter), remove, `value[x]` choice, ad-hoc extension, standalone `/ui/editor` |
| `tests/history.spec.ts` | version rail, from/to selects, the **show-metadata diff checkbox**, deep-link, not-found |
| `tests/compartments.spec.ts` | rail + tabs, the membership tester's four outcomes (member/self/not-member/fan-out), and the stored `last` restore through the nav |
| `tests/queries.spec.ts` | query builder: run → results, pagination, add-condition, per-type param datalist, Recent |
| `tests/nl-search.spec.ts` | NL mode toggle; translation lands a query and never runs it; refusal; example chips (stubbed `/$nl-search`) |
| `tests/search-parameters.spec.ts` | registry table, htmx rail filter, facet narrowing, row → detail |
| `tests/tenants.spec.ts` | add-tenant slide-over, htmx search filter, delete (skips if no tenant store) |
| `tests/sql-view-definitions.spec.ts` | View Definitions playground: rail, live `$sql-run` preview (#752); the guided-form card beside the editor (#843) — add a column from the form, edit `resource` in CodeMirror and see the row (and an invalid value's error) sync back, Ctrl+Z after a form edit, the two cards' shared internally-scrolling height, and the row↔editor cross-highlight in both directions (hover a row, click a JSON line, reveal stays inside each pane's own scroll) |
| `tests/sql-libraries.spec.ts` | SQL Queries/SQL Views playgrounds (#839): rail split by kind, SQL pane decode/highlight/roundtrip, the live `$sql-run` preview, recents, and a parse error's tinted line — plus, since #840, the Details section's own `describe`: a guided-form edit lands in the JSON pane and Save persists the merge, an invalid value errors on its row without saving, Save fuses an edited SQL pane with an edited Details title into one Library, Ctrl+Z after a form edit, the two Details cards' shared internally-scrolling height, and the type-code Save gate (`sql-view` saved from SQL Queries, and the reverse) on both routes |
| `tests/editor-pair.spec.ts` | The shared editor/guided-form host (`assets/editor-pair.js`, #840) — coverage of the pairing's own contract that is not tied to one page: the invalid-JSON chip still switches after a guided-form round trip has replaced the `.editor-form` card underneath it |
| `tests/nojs/*.spec.ts` | the README promise: the UI works with JavaScript disabled (`nojs` project) — includes `sql-view-definitions.spec.ts`'s and `sql-libraries.spec.ts`'s own cases: the guided-form card (#843/#840) stays hidden and both editors work alone, and — for `sql-libraries.spec.ts` — editing both the Details and SQL textareas by hand and saving persists the merged Library |

Pure-function browser modules (`assets/combobox.js`, `assets/editor-pair.js`'s
`minimalChange`) get a third, faster ring: plain Node tests under `unit/`,
run with `npm run test:unit` — no browser, no server. `editor-pair.js` is
wired UMD-style (`module.exports` under Node, exposes `window.HfsEditorPair`
under a real `document`) specifically so this stays possible without a
second copy of the diff algorithm.

## Run it

```bash
# 1. Build the server once (the suite boots it via boot.mjs).
cargo build -p helios-hfs --features ui

# 2. Install deps + a browser (first time only).
cd crates/ui/e2e
npm ci
npx playwright install chromium

# 3. Run.
npx playwright test              # all projects
npx playwright test theme        # one spec
npx playwright test --ui         # watch mode
npx playwright show-report       # last HTML report
```

`boot.mjs` starts the most recently built `target/{release,debug}/hfs` on
`127.0.0.1:8080` with a throwaway SQLite DB, and Playwright tears it down.
Locally the suite reuses a server you already have up on that port; set `CI=1`
to force a fresh boot.

The axe gate is strict: **every** WCAG 2.2 AA rule (including `color-contrast`,
in both themes) is a hard failure.

## Driving an external server

Set `HFS_E2E_BASE_URL` to point the suite at an hfs you started yourself; the
config then skips `boot.mjs` and drives that server instead:

```bash
HFS_E2E_BASE_URL=http://127.0.0.1:8080 npx playwright test
```

This is how the backend matrix runs it: `ui-tests-matrix.yml` builds hfs with
every storage backend, boots it on the runner host against a containerized
Postgres / Mongo / Elasticsearch / S3, and runs this suite (in the Playwright
container) against `http://<runner-ip>:<port>`. The per-PR `ui-tests.yml` stays
on SQLite for fast feedback; the matrix is manual + nightly.
