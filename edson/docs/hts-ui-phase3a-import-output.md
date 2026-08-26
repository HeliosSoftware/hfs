# Phase 3a — Playwright spec for Slice F (`/ui/hts/import`)

- **Design ref:** `edson/docs/hts-ui-design.md` §7.7 Import + §7.10 row 7.7 states matrix.
- **Slice F HEAD:** `0aaf22775` (Rust ring 78/0 green per `edson/docs/hts-ui-slice-f-output.md`).
- **Branch:** `feat/551-hts-ui` (unchanged — Phase 3a does not commit).
- **Working copy delta:** one new untracked file plus this output note.

## 1. File created

| Path | Lines |
|---|---|
| `crates\hts-ui\e2e\tests\import.spec.ts` | **322** |

No other file was touched.

## 2. Describe blocks + test counts

| # | Describe | Tests | Skips |
|---|---|---:|---:|
| A | `HTS Import page shell (§7.7)` | 5 | 0 |
| B | `HTS Import pre-flight validation (§7.7)` | 2 | 0 |
| C | `HTS Import result variants (§7.7 ImportResult enum)` | 4 | 2 |
| D | `HTS Import a11y and dual-mode contract (§7.7)` | 3 | 0 |
| **Totals** |  | **14** | **2** |

Effective active tests on the JS ring (`chromium`): **12** (Success + Rejected + all shell + validation + a11y). No `nojs/` file was added — Slice F's nojs contract (real `<form action method="post">`) is covered implicitly by the `form.method`/`form.action` assertion in describe D, and adding a dedicated nojs spec would only re-assert what the Rust ring at `tests/import.rs` already exercises against a mocked upstream.

### Per-test breakdown

**A — page shell**

1. `GET /ui/hts/import responds 200 and renders the h1 heading` — `page.goto` + `getByRole("heading", { name: /Import terminology/i, level: 1 })`.
2. `paste-mode textarea renders with an accessible label` — `getByLabel(/FHIR Bundle/i)` + id + name.
3. `submit button carries the hts-import-submit copy` — `getByRole("button", { name: /Import/i, exact: true })` + id + type=submit.
4. `nav lists Import after Operations and before Diagnostics` — sidebar link-order via `nav.getByRole("link").allTextContents()` (same pattern as `operations.spec.ts::"the nav bar exposes the Operations entry after ConceptMaps"`).
5. `dialect chip renders the negotiated locale in the topbar` — `.dialect-chip__value` contains `en` (mirrors `dashboard.spec.ts`).

**B — pre-flight validation**

1. `empty textarea submit renders the invalid-input OperationOutcome banner` — fills empty, clicks submit, asserts `getByRole("alert")` contains "Paste a JSON Bundle before submitting" (Fluent key `hts-import-empty-bundle-error`).
2. `invalid JSON submit renders the invalid-json OperationOutcome banner` — fills `"not-json"`, clicks submit, asserts `getByRole("alert")` contains `/not valid JSON/i` (Fluent key `hts-import-invalid-json-error`).

Both tests exercise the two `import_run` pre-flight gates in `crates/hts-ui/src/import.rs`. The round-trip contract ("no upstream call") is asserted implicitly by the same-request response — Playwright cannot introspect the outgoing hts call, but the Rust ring at `tests/import.rs::import_pre_flight_empty_bundle_returns_outcome_without_calling_hts` already pins the mock-visible side of that contract.

**C — result variants**

1. `Success: a transaction bundle renders the ok status partial` — fills `VALID_TRANSACTION_BUNDLE`, asserts `getByText(/Import complete/i)` + `.hts-import-status.hts-import-status--ok`.
2. `PartialSuccess` — **skipped**. See §6 below.
3. `Rejected: a body without resourceType renders the error status partial` — fills `BUNDLE_MISSING_RESOURCE_TYPE`, asserts `getByText(/Import rejected/i)` + `.hts-import-status.hts-import-status--error` + `.hts-outcome.hts-outcome--error`.
4. `TooLarge` — **skipped**. See §6 below.

**D — a11y + dual-mode**

1. `the paste textarea is associated with a <label> element` — `getByLabel` + explicit `<label for="hts-import-bundle">` check.
2. `the result region is inside a labelled landmark` — `getByRole("region", { name: /Import terminology/i })` (backed by `<section aria-labelledby="hts-import-heading">` in `pages/import.html`) + `#hts-import-status` inside it + `aria-live="polite"` on the status wrapper.
3. `the form advertises the htmx dual-mode contract` — regex-scoped assertions on `method`, `action`, `hx-post`, `hx-target`.

## 3. Inline fixtures

Both fixtures live at the top of `import.spec.ts` as multi-line template-literal `const`s.

### `VALID_TRANSACTION_BUNDLE`

A transaction bundle with a single `CodeSystem` PUT (id `hts-ui-import-spec-cs-1`, url `http://example.org/hts-ui/import-spec/cs-1`, two-concept, `content=complete`). Sits under `entry[0].request.method="PUT"`.

**Parity with `tests/import.rs`.** The Rust ring uses `{"resourceType":"Bundle","type":"collection","entry":[]}` because the mock cans the response and never inspects the body. Playwright hits the real hts, so `VALID_TRANSACTION_BUNDLE` is a strictly larger shape — the smallest bundle that (a) reliably resolves to 200 through hts's `POST /import`, (b) matches the "one CodeSystem create" example the parent brief calls out, and (c) exercises the counts-table numeric arm of `hts-import-status.html` when hts does report counts. Rust-side coverage of the empty-collection shape stays in `tests/import.rs::import_post_200_renders_success_summary`.

### `BUNDLE_MISSING_RESOURCE_TYPE`

`{ "foo": "bar", "note": "no resourceType — HTS must 400 this via its shared error mapping" }`.

**Parity with `tests/import.rs`.** The mock in `CannedResponse::import_rejected` returns an `OperationOutcome` with diagnostics `"Body is not a FHIR Bundle: missing resourceType"` — this fixture is the natural real-hts trigger for that same code path. It parses as JSON (so the pre-flight `serde_json::from_str` gate lets it through) and is refused by hts's `/import` handler.

## 4. DOM selectors / Fluent strings targeted

Selectors follow `dashboard.spec.ts` / `operations.spec.ts` style — role-based where possible, class markers only where the Rust ring already pins them.

### Role / label locators

- `getByRole("heading", { name: /Import terminology/i, level: 1 })`
- `getByRole("button", { name: /Import/i, exact: true })`
- `getByRole("region", { name: /Import terminology/i })`
- `getByRole("alert").first()`  (pre-flight OperationOutcome, severity=error)
- `getByLabel(/FHIR Bundle/i).first()`
- `nav.getByRole("link").filter({ hasText: /Dashboard|Code|Value|Concept|Operations|Import|Diagnostics/ })`

### Text / class markers

- `getByText(/Import complete/i)` (Fluent key `hts-import-status-success`).
- `getByText(/Import rejected/i)` (Fluent key `hts-import-status-rejected`).
- `getByText(/Paste a JSON Bundle before submitting/i)` (Fluent key `hts-import-empty-bundle-error`).
- `getByText(/not valid JSON/i)` (Fluent key `hts-import-invalid-json-error`).
- `.hts-import-status.hts-import-status--ok` (success arm; mirrors `tests/import.rs::import_post_200_renders_success_summary`).
- `.hts-import-status.hts-import-status--error` + `.hts-outcome.hts-outcome--error` (rejected arm; mirrors `tests/import.rs::import_post_400_renders_outcome_partial`).
- `.dialect-chip__value` (topbar chip; same pattern as `dashboard.spec.ts`).

### HTMX / form attributes

- `form.hts-import__form` with `toHaveAttribute("method", /post/i)`.
- `toHaveAttribute("action", /\/ui\/hts\/import$/)`.
- `toHaveAttribute("hx-post", /\/ui\/hts\/import$/)`.
- `toHaveAttribute("hx-target", /#hts-import-status/)`.

All `hx-*` assertions use regex per the parent brief's "future-refactor-safe" rule.

## 5. `npx tsc --noEmit` result

**Skipped.**

`crates\hts-ui\e2e\node_modules` does not exist locally, and `crates\hts-ui\e2e\node_modules\typescript` is likewise absent. Running `npx tsc --noEmit` from that directory would trigger a `npm exec` fetch against `registry.npmjs.org` — the corporate-proxy rule (`~/.cursor/rules/corporate-proxy-bypass.mdc`) allows clearing `HTTP_PROXY`/`HTTPS_PROXY` for such calls, but the resulting install would need to complete before `tsc` could resolve `@playwright/test`'s type shapes, and that install is well over the parent brief's 60-second cutoff. Phase 3b already runs `pnpm install` + `npx playwright test`; the TS parse happens transitively at that point.

The spec was authored to match the existing `dashboard.spec.ts` / `operations.spec.ts` import shape (`import { expect, test } from "@playwright/test"`), so `tsc` on a real install should be clean modulo Playwright API drift — no new type surface was introduced.

## 6. Skips introduced

Two `test.skip(...)` entries inside describe C. Both carry an inline comment naming the Rust-side test that owns the equivalent coverage.

| Variant | Reason | Rust-side coverage that stays authoritative |
|---|---|---|
| **PartialSuccess (207)** | Triggering `errors[]` on real hts requires a seeded ValueSet / CM topology that reproducibly emits non-fatal import issues. Slice F does not ship such a seed, and `e2e/boot.mjs` provisions an empty SQLite DB. | `crates/hts-ui/tests/import.rs::import_post_207_renders_partial_success_with_issue_list` — asserts `hts-import-status--warn`, "Import partially succeeded" title, `<details>` issue expander, plural-selected "2 issues" heading. |
| **TooLarge (413)** | A 13 MB+ paste over Chromium's default input path is impractical (browser memory + WS frame pressure + the fact that Playwright's webServer runs on the same box). | `crates/hts-ui/tests/import.rs::import_post_413_renders_too_large_guidance` — asserts `hts-import-status--warn`, "Bundle too large" title, and the `hts-import-too-large-hint` split-the-Bundle copy. |

Both skips are called out in the code with `test.skip("...", async () => { /* Rust-side coverage note */ })` so a future author can flip them to `test(...)` once the underlying trigger is in reach.

## 7. Red flags for Phase 3b

1. **Real-hts Success dependency.** `VALID_TRANSACTION_BUNDLE` assumes hts's `POST /import` accepts a transaction bundle with a `PUT CodeSystem/{id}` entry against an empty SQLite DB. If hts's import path is stricter about transaction semantics (e.g. requires `entry.fullUrl` to match `entry.request.url`, or refuses PUT-with-id creates) the test will land on the `Rejected` arm rather than `Success`. Fix path if that happens: fall back to a `collection` bundle with a single top-level `CodeSystem` entry, or check `POST /` (root batch) semantics in `crates/hts/src/operations/import_bundle.rs`.
2. **Nav-order regex breadth.** The `nav.getByRole("link").filter({ hasText: /.../ })` filter in describe A picks up EVERY sidebar link whose text matches one of the seven keywords, including the "Concept Maps" entry that also matches `/Concept/`. Order comparisons stay correct because `Operations`, `Import`, and `Diagnostics` do not collide with the earlier keywords — but a future rename that introduces (say) "Import Operations" would shadow the index lookup. Kept the current regex to mirror `operations.spec.ts`.
3. **Idempotency across Playwright runs.** The Success test PUTs `CodeSystem/hts-ui-import-spec-cs-1`. `boot.mjs` deletes the SQLite files on startup, so run-to-run state is fresh. But if Phase 3b starts caching that DB across runs (or if a developer sets `HTS_E2E_BASE_URL` pointing at an external server), the second run would exercise the "update" arm instead of "create" — the Fluent title stays "Import complete", so the assertion still passes; only the counts-table numbers differ. Called out here in case the counts-table assertions get tightened later.
4. **`role="alert"` may steal focus.** The pre-flight OperationOutcome banner has `role="alert"` (severity=error branch in `partials/hts-outcome.html`). Screen readers announce it immediately. That is by design (§11.1) but is worth calling out in case Phase 3b adds axe-core sweeps: the shared `hts-import-status` wrapper has its own `role="status"` + `aria-live="polite"`, so the two roles nest — axe should not flag this because the two aria-live regions have separate purposes.
5. **PartialSuccess / TooLarge deferrals.** Called out again here (in addition to §6) because Phase 3c will need a real seed for 207. `crates/hts` currently emits 207 whenever `has_errors` on the import — the trigger is "any bundle where at least one entry is well-formed and at least one is not". A future e2e seed that includes one broken ValueSet reference would unlock the 207 leg without touching Slice F code.

---

**One-line summary:** `import.spec.ts` created with 4 describe / 14 tests (2 skips), tsc skipped (no local node_modules, install > 60 s cutoff).
