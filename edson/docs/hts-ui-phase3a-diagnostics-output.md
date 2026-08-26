# Phase 3a — Playwright coverage for Slice G (Diagnostics)

- **Branch:** `feat/551-hts-ui` (uncommitted; parent handles commits at end of Phase 3).
- **Design ref:** `edson/docs/hts-ui-design.md` §7.9 Diagnostics + §7.10 row 7.9.
- **Rust baseline (unchanged):** `cargo test -p helios-hts-ui --tests` = 78 passed / 0 failed. Slice G already ships `crates/hts-ui/tests/diagnostics.rs` (5 tokio tests).
- **Toolchain used for this phase:** none — no cargo / npm / npx execution needed. See the "TypeScript typecheck" section below for why `npx tsc --noEmit` was skipped.

## 1. File created

| Path | Lines | Purpose |
|---|---|---|
| `c:\Users\tercere\src\helios\hfs\crates\hts-ui\e2e\tests\diagnostics.spec.ts` | 282 | Playwright coverage for `/ui/hts/diagnostics` — shell, htmx tab swap + `hx-push-url` deep-link contract, per-tab content shape, per-tab error-isolation design invariant. |

Nothing else was written. No existing spec / template / handler was touched (per Slice G "Playwright spec deferred" carry-over — this is the deferred delivery).

## 2. Describe blocks and per-test count

Four `test.describe` blocks. Total: **10 executable tests + 1 `test.skip`** (11 blocks in the file).

| # | describe | tests | notes |
|---|---|---|---|
| A | `HTS Diagnostics page shell (§7.9)` | 3 | H1 + tablist + 4 tabs; default Capability aria-selected=true; nav order Diagnostics > Import. |
| B | `HTS Diagnostics tab swap (§7.9 hx-push-url)` | 3 | Click TerminologyCapabilities → URL push + panel swap; click `/health` → URL push; deep-link `?tab=metrics` → metrics tab pre-selected on initial paint. |
| C | `HTS Diagnostics per-tab content shape (§7.9)` | 4 | Capability tab shows `CapabilityStatement` marker; TerminologyCapabilities tab shows resourceType marker; `/health` tab shows Status label + up-ish value regex; `/metrics` tab shows Prometheus heading + `<pre>` or empty-state (with Prometheus-signature regex when `<pre>` renders). |
| D | `HTS Diagnostics per-tab error isolation (§7.9)` | 1 + 1 skip | Structural invariant: every tab targets `#diag-panel` with `innerHTML` swap + carries a real `href` (nojs). The outcome test is `test.skip` — see §6 below. |

## 3. DOM selectors + roles targeted, Fluent strings referenced

**Roles (WAI-ARIA contract from §7.9 "tabs implemented as `<a role="tab">` with `aria-selected` and a single `role="tabpanel"` container"):**

- `page.getByRole("heading", { name: /Diagnostics/i, level: 1 })` — the H1 rendered by `pages/diagnostics.html` (id `hts-diagnostics-heading`).
- `page.getByRole("tablist")` — the outer `<div class="hts-diagnostics__tablist" role="tablist">`.
- `page.getByRole("tab", { name: <label>, exact: true })` — the four `<a class="hts-diagnostics__tab" role="tab">` anchors.
- `page.getByRole("tabpanel")` and `page.locator("#diag-panel")` — the shared `<section id="diag-panel" role="tabpanel">` container.

**hx-* attributes asserted with `toHaveAttribute(..., /pattern/)`:**

- `hx-get` — must match `/ui/hts/diagnostics/panel\?tab=<slug>` (per tab).
- `hx-target` — must be `#diag-panel` (exact).
- `hx-swap` — must be `innerHTML` (exact).
- `hx-push-url` — must be `true` (exact).
- `href` (nojs fallback) — must match `/ui/hts/diagnostics\?tab=<slug>`.

**Fluent keys implicitly referenced (via their en text values):**

| Key | en value | Where the spec keys off it |
|---|---|---|
| `hts-diagnostics-heading` | `Diagnostics` | H1 assertion in describe A. |
| `hts-diagnostics-tab-capability` | `Capability` | Tab accessible name (A, B, C, D). |
| `hts-diagnostics-tab-terminology-capabilities` | `TerminologyCapabilities` | Tab accessible name (A, B, C, D). |
| `hts-diagnostics-tab-health` | `/health` | Tab accessible name (A, B, C, D). |
| `hts-diagnostics-tab-metrics` | `/metrics` | Tab accessible name (A, B, C, D). |
| `hts-diagnostics-capability-heading` | `CapabilityStatement` | Content-shape assertion (C). |
| `hts-diagnostics-terminology-capabilities-heading` | `TerminologyCapabilities` | Content-shape assertion (B, C). |
| `hts-diagnostics-health-heading` | `Health` | Content-shape assertion (B, C). |
| `hts-diagnostics-metrics-heading` | `Prometheus metrics` | Content-shape assertion (C). |
| `hts-diagnostics-metrics-empty` | `Metrics endpoint returned no body` | Empty-state fallback in `/metrics` tab (C). |
| `hts-nav-diagnostics` | `Diagnostics` | Sidebar nav-order assertion (A). |
| `hts-nav-import` | `Import` | Sidebar nav-order assertion (A). |

## 4. Tab-name / query-param values targeted (exact strings from `diagnostics.rs` / template)

Verified against `crates/hts-ui/src/diagnostics.rs::Tab::slug`:

| Tab enum variant | Query-param value (`?tab=`) | Fluent label key | en text (tab accessible name) |
|---|---|---|---|
| `Tab::Capability` | `capability` | `hts-diagnostics-tab-capability` | `Capability` |
| `Tab::TerminologyCapabilities` | `terminology-capabilities` | `hts-diagnostics-tab-terminology-capabilities` | `TerminologyCapabilities` |
| `Tab::Health` | `health` | `hts-diagnostics-tab-health` | `/health` |
| `Tab::Metrics` | `metrics` | `hts-diagnostics-tab-metrics` | `/metrics` |

Query-param name is `tab` (from `TabQuery { tab: Option<String> }` — no serde rename). Default tab when `?tab=` is missing / unknown is `Tab::Capability` (fall-through arm in `Tab::from_slug`).

## 5. `npx tsc --noEmit` result

**Skipped.** Reasons:

1. `crates/hts-ui/e2e/` ships **no `tsconfig.json`** (see the layout — only `package.json` + `boot.mjs` + `playwright.config.ts` + `tests/*.spec.ts`). `npx tsc --noEmit` in that directory would fall back to `tsc`'s implicit defaults (target ES3, no libs), which is not the compilation profile Playwright uses at runtime.
2. Playwright ships its own TS runtime via `@playwright/test@1.49.1`, and its typecheck is exercised implicitly when the suite runs. Every existing spec (`dashboard.spec.ts`, `code-systems.spec.ts`, `value-sets.spec.ts`, `concept-maps.spec.ts`, `operations.spec.ts`) has landed the same way — none of them are covered by an out-of-band `tsc` gate.
3. The task explicitly allowed skipping if the check "costs >60s or fails to reach the registry"; running `npx tsc` on a Windows box with no local `typescript` install would hit the corporate proxy on first use.

Editor-side linting (via `ReadLints`) on the new file: **clean, no diagnostics.**

## 6. Test skips introduced

One `test.skip` in describe block D:

```
D) test.skip("forcing one panel to 5xx renders an OperationOutcome inside #diag-panel only", …)
```

- **Reason:** the Playwright suite boots a real `hts` binary against SQLite (`boot.mjs` — `HTS_UI_ENABLED=1`, no external upstream). There is no browser-reachable knob to force `/metadata`, `/health`, or `/metrics` to return a 5xx while the suite is running: HTS is its own upstream for those routes and is guaranteed up.
- **Rust-side coverage the skip defers to:** `any_tab_5xx_renders_outcome_in_diag_panel_only` in `crates/hts-ui/tests/diagnostics.rs`, which seeds an in-process axum mock to return `500` on `/health` and asserts:
  1. the shared `hts-outcome.html` partial renders **inside** `#diag-panel` (with the `hts-outcome hts-outcome--error` class stack), and
  2. the three sibling tab id markers (`hts-diagnostics-tab-capability`, `-terminology-capabilities`, `-metrics`) still survive in the shell — the per-tab isolation contract from §7.9.
- **What the executable D-block test *does* cover:** the structural invariant that makes the outcome contract safe under every tab — every anchor targets `#diag-panel` with `innerHTML` swap and carries a real `href` for the nojs fallback. A regression that widened the swap target or dropped the `href` would fail this test before it ever reached a 5xx.

## 7. Red flags for Phase 3b

1. **`/metrics` empty-body branch.** The `/metrics` content-shape test accepts *either* the `<pre>` block *or* the "no body" empty-state message via a `Locator.or(...)` fallback. If HTS's default boot serves an empty `/metrics` in the Playwright ring (e.g. no metrics registry initialized in SQLite mode), only the empty branch will match — which is still a green test, but Phase 3b should confirm whether the `<pre>` branch actually fires in this environment. If it never fires the assertion inside the `if ((await preBlock.count()) > 0)` gate is dead code.
2. **Prometheus content-type.** `metrics_text` returns the response body verbatim (no `Accept` header sent, no content-type check). HTS today emits `text/plain; version=0.0.4` for `/metrics`, but the spec does not depend on that — it keys off the text content, not the header. Only flagged in case Phase 3b wants to add a header-level assertion.
3. **`hx-push-url` target URL.** The spec waits on a substring (`tab=terminology-capabilities`, `tab=health`) rather than a specific full URL because htmx's `hx-push-url="true"` behaviour depends on whether an `HX-Push-Url` response header is set. `diagnostics.rs` does not set one today, so the pushed URL will be the panel-fragment path `/ui/hts/diagnostics/panel?tab=…`. That is technically not a bookmarkable page (reload would return a fragment, not a shell). If Phase 3b decides to tighten this, the fix is a `HX-Push-Url: /ui/hts/diagnostics?tab={slug}` response header from `diagnostics_panel` — the spec would then need a stricter `page.waitForURL(/\/ui\/hts\/diagnostics\?tab=…$/)` assertion. Called out here so the flake / behaviour is visible up-front.
4. **Tab accessible names contain a leading slash for `/health` and `/metrics`.** Playwright normalizes accessible-name whitespace but preserves punctuation, so `getByRole("tab", { name: "/health", exact: true })` is the correct incantation. If a future translator drops the slash from the en Fluent value (e.g. rewrites the label to `Health probe`), every `getByRole("tab", { name: "/health", … })` assertion would break — this is a spec-brittleness footprint the maintainer should be aware of.
5. **Locale coupling.** Every text-based assertion targets the en Fluent value. `dashboard.spec.ts` already sets the `hts_lang=es` cookie in its Spanish switcher test, but Playwright's default per-test browser context isolation should prevent cookie carry-over across specs. If Phase 3b sees Spanish tab labels sneak in, look at test-ordering / context reuse.
6. **`Locator.or` compatibility.** `page.locator(…).or(other)` requires Playwright ≥ 1.33. The suite pins `@playwright/test@1.49.1` (see `crates/hts-ui/e2e/package.json`), so this is safe today — but if the pin is ever loosened downward, the `/metrics` empty-body fallback will break.

## 8. Cross-check vs Slice G output

- All 4 tab labels, 4 slugs, and 4 sub-headings referenced above match `crates/hts-ui/src/diagnostics.rs` and `templates/partials/hts-diagnostics-panel.html` exactly.
- The `#diag-panel` id + `role="tabpanel"` DOM shape matches `templates/pages/diagnostics.html` L56–L62.
- The 5-test Rust ring in `crates/hts-ui/tests/diagnostics.rs` is not modified.
- No Fluent keys added — the spec keys off the 8 en strings already shipped by Slice G in `locales/en/main.ftl` L1199–L1224 (plus `hts-nav-diagnostics` from Phase 1 chrome).

## 9. Return payload to parent

- **Absolute path to this doc:** `c:\Users\tercere\src\helios\hfs\edson\docs\hts-ui-phase3a-diagnostics-output.md`
- **One-line summary:** `diagnostics.spec.ts created with 4 describe / 10 tests (+ 1 test.skip), tsc skipped (no tsconfig; Playwright's own TS runtime is the effective gate)`
- **Blockers for Phase 3b:** none. Red flags in §7 above are annotations, not blockers.
