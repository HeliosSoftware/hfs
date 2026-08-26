# Grupo D (Small drift) — Diagnosis

Independent root causes for five Playwright failures in `crates/hts-ui/e2e/tests/`. Evidence from `playwright-alt-e.log` (seed import: `CS=34 VS=4 CM=2`).

---

## D1. dashboard.spec.ts:29 — sidebar strict-mode collision

### Root cause

The dashboard page exposes **three** `<nav>` landmarks: sidebar (`.sidebar nav`), quick-links (`nav.hts-quick-links` inside the cards fragment), and the topbar language switcher. The spec uses `page.getByRole("navigation").getByText(label)`, which matches **every** nav on the page. Five section labels also appear as dashboard quick-link chips (`Code Systems`, `Value Sets`, `Concept Maps`, `Operations`, `Import`), so Playwright resolves two elements and throws a strict-mode violation (confirmed for `Code Systems` in the log).

### Files touched

- `crates/hts-ui/e2e/tests/dashboard.spec.ts` (spec fix only)

### Exact edit (with 5-line context)

```typescript
  test("sidebar lists every canonical HTS UI section", async ({ page }) => {
    await page.goto("/ui/hts");
    const sidebar = page.locator("#sidebar nav");
    for (const label of [
      "Dashboard",
      "Code Systems",
      "Value Sets",
      "Concept Maps",
      "Operations",
      "Import",
      "Diagnostics",
    ]) {
      await expect(sidebar.getByText(label, { exact: false })).toBeVisible();
    }
  });
```

### Confidence

**High** — log shows the exact two-element collision (`.nav-item__label` vs `.hts-quick-link`); scoping to `#sidebar nav` matches the test intent (“sidebar lists …”).

---

## D2. diagnostics.spec.ts:193 — /health Status label

### Root cause

The test does **not** look for `<dt>Status</dt><dd>up</dd>` nor JSON inside a `<pre>`. It asserts the `#diag-panel` tabpanel contains `/Health/i` and `/\b(ok|up|healthy|degraded)\b/i`. The health tab template (`hts-diagnostics-panel.html`) renders a `<dl>` with Fluent label `Status` and raw `{{ h.status }}` in the `<dd>` — that path is correct.

The failure is upstream JSON decode: the tabpanel shows `hts-outcome.html` with  
`upstream 'health' at …/health returned an unrecognized body: error decoding response body`.

HTS `GET /health` emits `uptime_seconds` as a **JSON number from `f64`** (`helios_observability::uptime::uptime_seconds()` returns `f64`; see `crates/observability/src/uptime.rs` and `crates/hts/src/operations/health.rs`). `UpstreamHealth` in `crates/hts-ui/src/upstream.rs` declares `uptime_seconds: u64`, so `serde_json` rejects fractional values (typical within seconds of boot). The dashboard “Status” tile test still passes because it only checks the static label text `"Status"`, not the parsed health value.

### Files touched

- `crates/hts-ui/src/upstream.rs` — change `UpstreamHealth.uptime_seconds` to `f64` (or add a deserializer that accepts float and truncates); update `uptime_pretty()` to cast/floor before formatting
- Optionally `crates/hts-ui/tests/diagnostics.rs` / `tests/import.rs` mocks (already use integer `42`; still valid)

No spec or template edit required once decode succeeds; the existing regex already matches HTS `"status": "ok"`.

### Exact edit (with 5-line context)

In `crates/hts-ui/src/upstream.rs`:

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamHealth {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub uptime_seconds: f64,
}

impl UpstreamHealth {
    pub fn uptime_pretty(&self) -> String {
        let mut secs = self.uptime_seconds.floor() as u64;
        let days = secs / 86_400;
        // … rest unchanged …
```

Update `Default for UpstreamHealth` if it sets `uptime_seconds: 0` (still fine as `0.0`).

### Confidence

**High** — log shows decode error, not missing markup; type mismatch is the only plausible decode failure for the documented `/health` JSON shape.

---

## D3. value-sets.spec.ts:132 — too-costly Run button

### Root cause

**Seed gap**, not template drift. `ex-vs-too-costly` is documented in `e2e/README.md` and referenced by the spec, but **`seed.mjs` does not define it** (grep returns no matches). Navigating to `/ui/hts/value-sets/ex-vs-too-costly/expand` renders the unknown-id shell (`hts-outcome--error`) without the expand input form, so `getByRole('button', { name: 'Run', exact: true })` never appears (30 s timeout in log).

The expand input template (`hts-vs-expand-input.html`) and too-costly result partial (`hts-vs-expand-result.html`) are already implemented; Rust HTTP tests cover the banner via mock 422 (`expand_422_renders_too_costly_banner_with_raise_form`).

### Files touched

- `crates/hts-ui/e2e/seed.mjs` — add `ex-vs-too-costly` ValueSet
- `crates/hts-ui/e2e/boot.mjs` — set a low `HTS_MAX_EXPANSION_SIZE` so default `$expand` trips `too-costly` against the seeded CS (default server cap is 3500; `ex-cs-limbs` alone has 60 concepts and will not fail)

### Exact edit (with 5-line context)

**boot.mjs** — inside the `env` block passed to `spawn`:

```javascript
    HTS_UI_ENABLED: "true",
    HTS_MAX_EXPANSION_SIZE: "5",
  },
});
```

**seed.mjs** — after `ex-vs-tree`, before supporting VS entries:

```javascript
  // -- ex-vs-too-costly: flat VS over ex-cs-limbs (60 concepts). Under the
  //    e2e HTS_MAX_EXPANSION_SIZE cap, default $expand returns 422 too-costly.
  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-too-costly",
      url: "http://example.org/vs/too-costly",
      version: "1.0.0",
      name: "ExampleTooCostlyVS",
      status: "active",
      compose: {
        include: [{ system: "http://example.org/cs/limbs" }],
      },
    },
  });

  // -- Supporting VSs referenced by the ConceptMap source/target.
```

Update the file header comment roster to list `ex-vs-too-costly`.

### Confidence

**High** — missing seed id explains missing Run button; cap + limbs VS matches HTS too-costly semantics (`crates/hts/tests/value_set_ops.rs::expand_exceeds_limit_returns_422_too_costly`).

---

## D4. code-systems.spec.ts:26 — filter debounce

### Root cause

Filter debounce and empty-state assertion **work**. The failure is on **Reset** (lines 37–39): `getByRole("cell", { name: "http://example.org/cs" })` uses Playwright’s default **substring** name matching. After reset, the first page shows 25 rows; filler URLs are `http://example.org/cs/filler-N`, all of which **contain** the substring `http://example.org/cs`, so the locator matches 25 cells (strict-mode violation). Log lists `ex-cs-1` plus `filler-2` … `filler-10` (truncated).

Not a template or htmx bug — `hts-cs-rows.html` and the filter form behave as designed.

### Files touched

- `crates/hts-ui/e2e/tests/code-systems.spec.ts` (spec fix only)

### Exact edit (with 5-line context)

```typescript
    await page.getByRole("link", { name: "Reset", exact: true }).click();
    await expect(
      page.getByRole("cell", { name: "http://example.org/cs", exact: true }),
    ).toBeVisible();
  });

  test("load-more appends the next page beforeend without replacing rows", async ({ page }) => {
```

Alternative: `page.locator("code.hts-cs-browser__url", { hasText: /^http:\/\/example\.org\/cs$/ })`.

### Confidence

**High** — log shows 25 matches including filler URLs; substring semantics documented in Playwright `getByRole` name option.

---

## D5. code-systems.spec.ts:42 — load-more beforeend

### Root cause

Load-more **append works** (`hx-swap="beforeend"` in `hts-cs-rows.html` is correct). The spec arithmetic is stale: it assumes **31** CodeSystems (`ex-cs-1` + `ex-cs-2..31`) and expects `before + 6` after the second fetch (25 + 6 = 31). The seed also imports **`ex-cs-source`, `ex-cs-target`, and `ex-cs-limbs`** for other slices, for **34** total CS (`playwright-alt-e.log`: `CS=34`). First page: 25 rows; second page adds **9**, total **34**. Log: `Expected: 31`, `Received: 34`.

Not an `hx-swap` regression.

### Files touched

- `crates/hts-ui/e2e/tests/code-systems.spec.ts` (spec fix only)

### Exact edit (with 5-line context)

```typescript
  test("load-more appends the next page beforeend without replacing rows", async ({ page }) => {
    // Seed: ex-cs-1 + fillers ex-cs-2..31 + ex-cs-source/target/limbs => 34 CS.
    // Default _count=25 => first page 25, Load-more adds the remaining 9.
    await page.goto("/ui/hts/code-systems");
    const rows = page.locator("table tbody tr");
    const before = await rows.count();
    expect(before).toBeGreaterThanOrEqual(25);
    await page.getByRole("button", { name: "Load more", exact: true }).click();
    await expect(rows).toHaveCount(before + 9, { timeout: 3_000 });
  });
```

Or derive dynamically: `await expect(rows).toHaveCount(34)` / compare `await rows.count()` to `before` with `expect(after).toBeGreaterThan(before)`.

### Confidence

**High** — log shows row count jumped to 34 (append succeeded); only the hard-coded `+ 6` is wrong.

---

## Summary

| Test | Kind of fix |
|------|-------------|
| D1 dashboard sidebar | **Spec only** — scope locators to `#sidebar nav` |
| D2 diagnostics /health | **Rust upstream type** — `uptime_seconds: f64` in `UpstreamHealth` |
| D3 value-sets too-costly | **Seed + boot env** — add `ex-vs-too-costly`, set `HTS_MAX_EXPANSION_SIZE` |
| D4 CS filter debounce | **Spec only** — `exact: true` on canonical URL cell |
| D5 CS load-more | **Spec only** — expect `before + 9` (34-row seed) |

**Files the whole group touches**

| File | D1 | D2 | D3 | D4 | D5 |
|------|:--:|:--:|:--:|:--:|:--:|
| `crates/hts-ui/e2e/tests/dashboard.spec.ts` | ✓ | | | | |
| `crates/hts-ui/src/upstream.rs` | | ✓ | | | |
| `crates/hts-ui/e2e/seed.mjs` | | | ✓ | | |
| `crates/hts-ui/e2e/boot.mjs` | | | ✓ | | |
| `crates/hts-ui/e2e/tests/code-systems.spec.ts` | | | | ✓ | ✓ |

**No template HTML changes** are required for this group. Templates already implement the health DL, CS load-more `beforeend`, and VS too-costly banner; failures are locator scope, JSON typing, seed roster, and outdated row-count expectations.
