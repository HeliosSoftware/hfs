import { expect, test } from "@playwright/test";

// Chrome / visual parity between HTS and HFS (design doc §14, added
// 2026-08-20). Complements crates/hts-ui/tests/chrome_parity.rs: the Rust
// ring pins the server-rendered markup, and this ring pins the browser
// behaviour that only a real page can exercise (Figtree actually loading
// via the font-face fetch, the FHIR selector's <details> toggling on
// click, backlink navigation returning to the browser, and the Import
// FileReader sink populating the paste textarea from a picked file).
//
// Seed dependency: seed.ts already provisions ex-cs-1, ex-vs-1, ex-cm-1
// through /import, so the detail pages actually render `summary =
// Some(...)` and the backlink appears. No extra fixture bytes required.

test.describe("HTS chrome parity (§14 sidebar / topbar)", () => {
  test("Figtree loads under /ui/hts (font-face resolves)", async ({ page }) => {
    // Track A regression: with the old absolute `/ui/assets/fonts/…`
    // URL the browser 404'd the .woff2 and fell back to
    // `ui-sans-serif`. The relative URL is what unblocks Figtree here.
    await page.goto("/ui/hts");
    // Wait for the browser to actually load web fonts before probing
    // computed style — otherwise the fallback stack shows up in the
    // race between DOMContentLoaded and font resolution.
    await page.evaluate(() => document.fonts.ready);
    const fontFamily = await page.evaluate(() =>
      getComputedStyle(document.body).fontFamily,
    );
    expect(fontFamily, "body must resolve to Figtree, not the fallback").toMatch(
      /Figtree/,
    );
    // Cross-check: the .woff2 fetch itself must return 200 under the
    // HTS mount. If this fails the CSS relative URL regressed.
    const wof2 = await page.request.get("/ui/hts/assets/fonts/figtree-latin.woff2");
    expect(wof2.status(), "figtree-latin.woff2 must be served").toBe(200);
  });

  test("every sidebar nav item renders an inline SVG icon", async ({ page }) => {
    // Track B: HFS uses `<span class="icon"><svg …></svg></span>` on
    // every nav-item. Reviewer noted HTS was text-only. Assert both the
    // span slots exist and each one actually contains an <svg>.
    await page.goto("/ui/hts");
    const iconSpans = page.locator("aside.sidebar .nav .nav-item .icon");
    await expect(iconSpans.first()).toBeVisible();
    const count = await iconSpans.count();
    expect(count, "expected 7 nav-item icons").toBeGreaterThanOrEqual(7);
    // Every span must carry a real <svg>, not just a text label.
    const withSvg = await iconSpans.locator("svg").count();
    expect(withSvg, "every nav-item icon slot must contain an <svg>").toBe(count);
  });

  test("FHIR version selector toggles as a <details> disclosure", async ({ page }) => {
    // Track C: the old `<span class="fhir-badge">` is now
    // `<details class="menu menu--up">` matching HFS. Assert both the
    // static shape and that clicking the summary opens the panel — the
    // one behaviour a source-check cannot verify.
    await page.goto("/ui/hts");
    const disclosure = page.locator("aside.sidebar details.menu.menu--up");
    await expect(disclosure).toHaveCount(1);
    const summary = disclosure.locator("summary.selector.selector--outline");
    await expect(summary).toBeVisible();
    // Panel is hidden until the summary is clicked.
    const panel = disclosure.locator(".menu__panel");
    await expect(panel).toBeHidden();
    await summary.click();
    await expect(panel).toBeVisible();
    // Degenerate single-option: the current version is marked current.
    await expect(disclosure.locator('[aria-current="true"]')).toBeVisible();
    // No stray legacy badge.
    await expect(page.locator(".fhir-badge")).toHaveCount(0);
  });
});

test.describe("HTS backlink on detail pages (§14.5 Cat C)", () => {
  test("cs-detail backlink goes to /ui/hts/code-systems", async ({ page }) => {
    // Landing on `/ui/hts/code-systems/ex-cs-1` 308-redirects to
    // `/lookup` per §8.3; Playwright follows the redirect so the final
    // URL is /lookup, and the backlink sits above the header regardless
    // of which operation tab is active.
    await page.goto("/ui/hts/code-systems/ex-cs-1");
    const backlink = page.locator("a.backlink");
    await expect(backlink, "cs-detail must render a backlink anchor").toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/code-systems");
    // Chevron `‹` (U+2039) is part of the visible text.
    await expect(backlink).toContainText("\u2039");
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/code-systems$/);
  });

  test("vs-detail backlink goes to /ui/hts/value-sets", async ({ page }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-1");
    const backlink = page.locator("a.backlink");
    await expect(backlink).toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/value-sets");
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/value-sets$/);
  });

  test("cm-detail backlink goes to /ui/hts/concept-maps", async ({ page }) => {
    await page.goto("/ui/hts/concept-maps/ex-cm-1");
    const backlink = page.locator("a.backlink");
    await expect(backlink).toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/concept-maps");
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/concept-maps$/);
  });

  test("browser back returns from cs-detail to the code-systems browser", async ({ page }) => {
    // The 308 redirect from `/{id}` to `/{id}/lookup` (§8.3) must not
    // trap the user in history: `page.goBack()` from the detail lookup
    // page must return to the list.
    await page.goto("/ui/hts/code-systems");
    await expect(page.getByRole("heading", { name: "CodeSystems", exact: true, level: 1 }))
      .toBeVisible();
    await page.goto("/ui/hts/code-systems/ex-cs-1");
    await expect(page).toHaveURL(/\/lookup$/);
    await page.goBack();
    await expect(page).toHaveURL(/\/ui\/hts\/code-systems$/);
  });
});

test.describe("HTS Import file upload (§14.6 Batch-style)", () => {
  test("file radio hides textarea and shows file input", async ({ page }) => {
    await page.goto("/ui/hts/import");
    const textarea = page.locator("#hts-import-bundle");
    const fileInput = page.locator("#hts-import-file");
    // Paste is the default landing mode.
    await expect(textarea).toBeVisible();
    // Switch to file mode.
    await page.getByRole("radio", { name: /file/i }).check();
    await expect(fileInput).toBeVisible();
    // Paste field's .field wrapper is `hidden`.
    await expect(page.locator(".field", { has: textarea })).toBeHidden();
    // Switch back to paste.
    await page.getByRole("radio", { name: /paste/i }).check();
    await expect(textarea).toBeVisible();
    await expect(page.locator(".field", { has: fileInput })).toBeHidden();
  });

  test("picking a file fills the paste textarea via FileReader", async ({ page }) => {
    // §14.6 wire contract: the file's text is read in the browser and
    // written into `#hts-import-bundle` so the existing urlencoded
    // handler sees `bundle=<contents>` on submit. No multipart, no new
    // Rust extractor. This spec exercises the sink without triggering
    // the actual submit — the paste round-trip is already covered by
    // import.spec.ts.
    await page.goto("/ui/hts/import");
    await page.getByRole("radio", { name: /file/i }).check();

    const bundleJson = JSON.stringify({
      resourceType: "Bundle",
      type: "collection",
      entry: [],
    });
    await page.locator("#hts-import-file").setInputFiles({
      name: "empty-bundle.json",
      mimeType: "application/fhir+json",
      buffer: Buffer.from(bundleJson, "utf-8"),
    });

    // FileReader fires asynchronously — poll the textarea until it has
    // the content. The textarea itself is hidden in file mode; assert
    // on `.value` rather than visible-text semantics.
    await expect
      .poll(async () => await page.locator("#hts-import-bundle").inputValue(), {
        timeout: 3_000,
      })
      .toBe(bundleJson);
  });
});
