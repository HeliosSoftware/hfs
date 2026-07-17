# helios-ui browser tests (Playwright + axe-core)

The outer ring of the UI test pyramid (issue #249): behavior only a real browser
can observe — WCAG 2.2 AA conformance, `theme.js` before first paint, the
`/_user/settings` merge-patch, progressive enhancement with JS off, and the
no-CDN invariants. The fast inner ring stays in Rust (`crates/ui/tests/*.rs`,
`tower::oneshot`).

Everything Node lives here; the cargo workspace is untouched.

## Layout

| Path | What it covers |
|------|----------------|
| `tests/a11y.spec.ts` | axe-core WCAG 2.2 AA over `/ui`, `/ui/resources`, `/ui/compartments`, `/ui/search-parameters`, light × dark |
| `tests/theme.spec.ts` | FOUC guard, OS-preference precedence, PATCH merge-patch, server-roam, graceful degradation |
| `tests/no-cdn.spec.ts` | no off-origin requests, no page errors, no inline `<script>` blob |
| `tests/resources-editor.spec.ts` | Resources edit flows: Create targets the picked type, inline binding validation, Save blocked on invalid, raw-edit round-trips to the FHIR API |
| `tests/nojs/*.spec.ts` | the README promise: the UI works with JavaScript disabled (`nojs` project) |

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
