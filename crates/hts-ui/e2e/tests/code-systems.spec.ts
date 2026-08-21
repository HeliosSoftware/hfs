import { expect, test } from "@playwright/test";

// Phase 2 Slice B: CodeSystem browser + detail with embedded workbench.
// These specs walk the four §7.2 / §7.3 interactions that HTML-only Rust
// http tests cannot exercise: the debounced filter form, the Load-more
// paginator's `hx-swap="beforeend"` append, the WAI-ARIA tabs pattern on
// the detail page, and the workbench result panel's `aria-live` update.
//
// Boot fixture: the e2e/boot.mjs script starts `hts` with a stable seed
// CodeSystem (id="ex-cs-1", url="http://example.org/cs", version="1.0.0",
// two active concepts). Every test uses that fixture; details on the
// seed live in e2e/README.md.

test.describe("HTS CodeSystem browser (§7.2)", () => {
  test("renders the browser heading and status pills at /ui/hts/code-systems", async ({ page }) => {
    const response = await page.goto("/ui/hts/code-systems");
    expect(response?.status(), "browser route must respond 200").toBe(200);
    await expect(
      page.getByRole("heading", { name: "CodeSystems", exact: true, level: 1 }),
    ).toBeVisible();
    // The seed CS is active, so the browser table's first row shows an
    // "active" pill translated by the Fluent catalog.
    await expect(page.getByRole("cell").filter({ hasText: "active" }).first()).toBeVisible();
  });

  test("filter input debounces and swaps rows via htmx", async ({ page }) => {
    await page.goto("/ui/hts/code-systems");
    // Type into the URL filter; htmx `hx-trigger="input changed delay:300ms"`
    // waits ~300ms before firing the swap. A CS whose canonical URL does
    // not contain "no-such-system" must not survive the filter.
    await page.getByLabel("Canonical URL", { exact: false }).fill("no-such-system");
    await expect(page.getByText("No CodeSystems match these filters.")).toBeVisible({
      timeout: 3_000,
    });
    // Reset returns us to the full listing (empty-anchor href navigation).
    await page.getByRole("link", { name: "Reset", exact: true }).click();
    // Match the seed CS's canonical exactly; the 30 filler CSs each carry
    // a URL that contains "http://example.org/cs" as a substring
    // ("http://example.org/cs/filler-N"), which would otherwise trip
    // Playwright strict mode on multiple matches.
    await expect(
      page.getByRole("cell", { name: "http://example.org/cs", exact: true }),
    ).toBeVisible();
  });

  test("load-more appends the next page beforeend without replacing rows", async ({ page }) => {
    // The seed loader injects 34 CodeSystems total (ex-cs-1 + ex-cs-2..
    // ex-cs-31 + ex-cs-source + ex-cs-target + ex-cs-limbs). The default
    // `_count=25` page shows 25 rows and Load-more fetches the remaining 9.
    // The footer is OOB-swapped so the button's `_offset` advances; without
    // that, a second click would re-append page 2 (duplicates).
    await page.goto("/ui/hts/code-systems");
    const rows = page.locator("table tbody tr");
    const loadMore = page.getByRole("button", { name: "Load more", exact: true });
    const before = await rows.count();
    expect(before).toBeGreaterThanOrEqual(25);
    await expect(loadMore).toBeVisible();
    await loadMore.click();
    await expect
      .poll(async () => await rows.count(), { timeout: 3_000 })
      .toBeGreaterThan(before);
    const after = await rows.count();
    expect(after).toBeLessThanOrEqual(34);
    // Terminal page: fewer than `_count` rows returned → button gone.
    await expect(loadMore).toHaveCount(0);
    // Row identity must stay unique (guards the stale-offset duplication).
    const hrefs = await page.locator("table tbody tr td.col-name a").evaluateAll((as) =>
      as.map((a) => (a as HTMLAnchorElement).getAttribute("href")),
    );
    expect(new Set(hrefs).size).toBe(hrefs.length);
  });
});

test.describe("HTS CodeSystem detail + workbench (§7.3)", () => {
  test("landing on /ui/hts/code-systems/{id} redirects to /lookup with Lookup tab active", async ({ page }) => {
    // §8.3 operation-first landing: the naked `/{id}` URL 308-redirects
    // to the default operation tab (`/{id}/lookup`); Playwright follows
    // the redirect transparently, so the final URL and the active tab
    // are both Lookup. The Metadata tab was retired — the facts block
    // stays visible above the tab strip regardless of which operation
    // is active.
    const response = await page.goto("/ui/hts/code-systems/ex-cs-1");
    expect(response?.status()).toBe(200);
    expect(page.url()).toContain("/ui/hts/code-systems/ex-cs-1/lookup");
    await expect(
      page.getByRole("tab", { name: "Lookup", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    // Facts block above the tab strip renders the seed CS's canonical URL.
    await expect(page.getByText("http://example.org/cs")).toBeVisible();
    // §8.3: the Metadata tab is gone from the tab strip.
    await expect(
      page.getByRole("tab", { name: "Metadata", exact: true }),
    ).toHaveCount(0);
  });

  test("clicking a filler row from the browser opens its detail page with Lookup active", async ({ page }) => {
    // Regression: HTS's summary-mode search projects `id="{fhir_id}|{version}"`.
    // Row projection must strip the `|version` suffix or the /detail Alt E
    // lookup (see upstream.rs base_id + resolve_canonical_url) never
    // matches and the page renders only chrome + a not-found banner.
    //
    // §8.3: browser row links point at `/{id}` and the redirect chain
    // resolves to `/{id}/lookup` with the Lookup tab active.
    await page.goto("/ui/hts/code-systems");
    const link = page.getByRole("link", { name: "FillerCS2", exact: true });
    await expect(link).toBeVisible();
    // Link href must not carry a composite id (`|` character).
    const href = await link.getAttribute("href");
    expect(href).not.toContain("|");
    await link.click();
    // Detail page lands on the Lookup tab with the canonical URL visible
    // in the facts block above — proving Alt E fully resolved the resource.
    await expect(
      page.getByRole("tab", { name: "Lookup", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByText("http://example.org/cs/filler-2")).toBeVisible();
    // And the not-found banner must be absent.
    await expect(page.locator(".hts-outcome--error")).toHaveCount(0);
  });

  test("clicking the Lookup tab swaps in the workbench input via htmx", async ({ page }) => {
    // Start on `/validate` so the "click Lookup" step is a real
    // navigation, not a no-op self-click.
    await page.goto("/ui/hts/code-systems/ex-cs-1/validate");
    await page.getByRole("tab", { name: "Lookup", exact: true }).click();
    // The GET /lookup handler renders the input partial; the run button
    // is Fluent-resolved from hts-workbench-run ("Run" in en).
    await expect(
      page.getByRole("button", { name: "Run", exact: true }),
    ).toBeVisible({ timeout: 3_000 });
  });

  test("clicking a tab moves the aria-current highlight (Bug 1 regression)", async ({
    page,
  }) => {
    // Region-wrap contract (design doc §8.1). Before the region wrapper
    // the tabs strip lived outside the htmx swap target, so a tab click
    // updated the panel without refreshing aria-current. §8.3 replaces
    // the original "Metadata → Lookup" click path with a "Lookup →
    // Validate" one: the naked `/{id}` URL now lands directly on
    // `/lookup`, so Lookup is the active tab on arrival. Clicking
    // Validate must move `aria-current` off Lookup and onto Validate.
    await page.goto("/ui/hts/code-systems/ex-cs-1");
    const lookup = page.getByRole("tab", { name: "Lookup", exact: true });
    const validate = page.getByRole("tab", { name: "Validate", exact: true });
    await expect(lookup).toHaveAttribute("aria-current", "true");
    await validate.click();
    // htmx swap completes when the Validate form's Code input appears.
    await expect(
      page.getByLabel("Code", { exact: true }),
    ).toBeVisible({ timeout: 3_000 });
    await expect(validate).toHaveAttribute("aria-current", "true");
    await expect(lookup).not.toHaveAttribute("aria-current", "true");
  });

  test("running $lookup with the seed code renders a designation + property panel", async ({
    page,
  }) => {
    await page.goto("/ui/hts/code-systems/ex-cs-1/lookup");
    await page.getByLabel("Code", { exact: true }).fill("A");
    await page.getByRole("button", { name: "Run", exact: true }).click();
    // The workbench result panel is aria-live="polite"; after the POST
    // the seed CS's concept A must be echoed with its display.
    const result = page.locator("#hts-workbench-result");
    await expect(result).toContainText("Alpha", { timeout: 3_000 });
    await expect(result.locator(".hts-cs-workbench__properties")).toBeVisible();
  });

  test("running $validate-code with a missing code renders the invalid badge", async ({ page }) => {
    await page.goto("/ui/hts/code-systems/ex-cs-1/validate");
    await page.getByLabel("Code", { exact: true }).fill("NOT-A-CODE");
    await page.getByRole("button", { name: "Run", exact: true }).click();
    await expect(page.locator(".hts-cs-workbench__badge--false")).toBeVisible({
      timeout: 3_000,
    });
  });

  test("running $subsumes between seed codes reports the outcome", async ({ page }) => {
    await page.goto("/ui/hts/code-systems/ex-cs-1/subsumes");
    await page.getByLabel("Code A", { exact: true }).fill("A");
    await page.getByLabel("Code B", { exact: true }).fill("B");
    await page.getByRole("button", { name: "Run", exact: true }).click();
    // The seed CS defines A > B, so the outcome is `subsumes`.
    await expect(page.getByText("Code A subsumes code B.")).toBeVisible({
      timeout: 3_000,
    });
  });

  test("a soft-deleted or unknown id renders an outcome partial inside the shell", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/code-systems/does-not-exist");
    // The design contract (§7.3 states matrix) requires a 200 with an
    // OperationOutcome partial rather than a page 404.
    expect(response?.status()).toBe(200);
    await expect(page.locator(".hts-outcome--error")).toBeVisible();
  });
});
