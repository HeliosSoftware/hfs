import { expect, test } from "@playwright/test";

// Chrome / visual parity between HTS and HFS (design doc §14, added
// 2026-08-20). Complements crates/hts-ui/tests/chrome_parity.rs: the Rust
// ring pins the server-rendered markup, and this ring pins the browser
// behaviour that only a real page can exercise (Figtree actually loading
// via the font-face fetch, the FHIR selector's <details> toggling on
// click, back-link navigation returning to the browser, and the Import
// FileReader sink populating the paste textarea from a picked file).
//
// The back-link ring also pins *geometry*, not just navigation (#801).
// The three detail pages used to build their back link out of the
// `.row-link` primitive, which is `display:flex; flex-direction:column;
// color:inherit` — so the anchor stretched to the full content width
// (a page-wide click target), inherited the body text colour instead of
// the accent, and left no vertical breathing room before the <h1>. The
// fix adopts the HFS `.back-link` primitive inside
// `header.page-head--back-link`; the specs below assert the computed
// box the primitive is supposed to produce, so a silent regression back
// to `.row-link` (or to a bare <a>) fails here rather than only being
// visible to a human. `.back-link` geometry lives in
// crates/ui/assets/app.css and reaches HTS through the shared
// `#[folder = "../ui/assets"]` embed.
//
// Seed dependency: seed.mjs already provisions ex-cs-1, ex-vs-1, ex-cm-1
// through /import, so the detail pages actually render `summary =
// Some(...)` and the back link appears. No extra fixture bytes required.

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

// The three detail routes, each paired with the browser it must lead
// back to. Landing on `/ui/hts/{type}/{id}` 308-redirects to the default
// operation tab per §8.3 (/lookup, /expand, /translate); Playwright
// follows the redirect, and the back link sits above the header
// regardless of which operation tab ends up active.
const DETAIL_PAGES = [
  {
    name: "cs-detail",
    detail: "/ui/hts/code-systems/ex-cs-1",
    browser: "/ui/hts/code-systems",
    label: /CodeSystems/,
  },
  {
    name: "vs-detail",
    detail: "/ui/hts/value-sets/ex-vs-1",
    browser: "/ui/hts/value-sets",
    label: /ValueSets/,
  },
  {
    name: "cm-detail",
    detail: "/ui/hts/concept-maps/ex-cm-1",
    browser: "/ui/hts/concept-maps",
    label: /ConceptMaps/,
  },
] as const;

test.describe("HTS backlink on detail pages (§14.5 Cat C)", () => {
  test("cs-detail backlink goes to /ui/hts/code-systems", async ({ page }) => {
    // Landing on `/ui/hts/code-systems/ex-cs-1` 308-redirects to
    // `/lookup` per §8.3; Playwright follows the redirect so the final
    // URL is /lookup, and the backlink sits above the header regardless
    // of which operation tab is active.
    await page.goto("/ui/hts/code-systems/ex-cs-1");
    const backlink = page.locator("a.back-link");
    await expect(backlink, "cs-detail must render a backlink anchor").toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/code-systems");
    // #801: the hand-rolled U+2039 text chevron is gone, replaced by the
    // shared 5x8 inline icon (templates/icons/chevron-left.svg). Pin both
    // halves so neither the icon can vanish nor the glyph creep back.
    await expect(
      backlink.locator('svg[width="5"][height="8"]'),
      "back link must render the 5x8 inline chevron icon",
    ).toHaveCount(1);
    await expect(backlink).not.toContainText("\u2039");
    // Visible label is the destination browser's own <h1> text.
    await expect(backlink).toContainText(/CodeSystems/);
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/code-systems$/);
  });

  test("vs-detail backlink goes to /ui/hts/value-sets", async ({ page }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-1");
    const backlink = page.locator("a.back-link");
    await expect(backlink).toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/value-sets");
    // Same icon-not-chevron contract as cs-detail (#801): all three
    // detail pages share the primitive, so all three pin it.
    await expect(backlink.locator('svg[width="5"][height="8"]')).toHaveCount(1);
    await expect(backlink).not.toContainText("‹");
    await expect(backlink).toContainText(/ValueSets/);
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/value-sets$/);
  });

  test("cm-detail backlink goes to /ui/hts/concept-maps", async ({ page }) => {
    await page.goto("/ui/hts/concept-maps/ex-cm-1");
    const backlink = page.locator("a.back-link");
    await expect(backlink).toBeVisible();
    await expect(backlink).toHaveAttribute("href", "/ui/hts/concept-maps");
    // Same icon-not-chevron contract as cs-detail (#801).
    await expect(backlink.locator('svg[width="5"][height="8"]')).toHaveCount(1);
    await expect(backlink).not.toContainText("‹");
    await expect(backlink).toContainText(/ConceptMaps/);
    await backlink.click();
    await expect(page).toHaveURL(/\/ui\/hts\/concept-maps$/);
  });

  test("back link geometry matches the HFS primitive", async ({ page }) => {
    // #801 acceptance criteria, asserted against the rendered box rather
    // than against class names: the old markup carried a plausible-looking
    // anchor that still laid out completely wrong, because it reused the
    // browser-table `.row-link` primitive (`display:flex;
    // flex-direction:column; color:inherit`) instead of `.back-link`.
    for (const { name, detail, browser, label } of DETAIL_PAGES) {
      await page.goto(detail);

      const link = page.locator("a.back-link");
      await expect(link, `${name}: back link must render`).toBeVisible();
      await expect(link).toHaveAttribute("href", browser);
      await expect(link).toContainText(label);

      // --- The primitive's own box -----------------------------------
      // `.back-link` is `display:inline-flex`, but as a grid item of
      // `.page-head--back-link` the browser blockifies it, so computed
      // style reports `flex`. Accept either spelling; what must not hold
      // is `.row-link`'s column flex.
      await expect(link, `${name}: back link must be an (inline-)flex row`)
        .toHaveCSS("display", /^(inline-)?flex$/);
      await expect(link, `${name}: back link must not stack like .row-link`)
        .toHaveCSS("flex-direction", "row");
      // Chrome serializes the `gap` shorthand as "7px" or "7px 7px"
      // depending on version; both mean the icon/label gap the primitive
      // specifies. `.row-link` set no gap at all.
      await expect(link, `${name}: icon/label gap must be 7px`)
        .toHaveCSS("gap", /^7px( 7px)?$/);
      await expect(link, `${name}: back link must reserve 24px before the title`)
        .toHaveCSS("margin-bottom", "24px");
      await expect(link, `${name}: back link is 13px, not body copy size`)
        .toHaveCSS("font-size", "13px");
      // Underline only on :hover — at rest the primitive is undecorated.
      await expect(link, `${name}: back link must not be underlined at rest`)
        .toHaveCSS("text-decoration-line", "none");

      // --- Colour: the `.row-link { color: inherit }` regression -------
      // Inheriting the body colour made the back link read as plain prose
      // rather than an accent-coloured control. Compare against the live
      // body colour instead of hard-coding a hex, so this holds in both
      // the light and dark themes.
      const bodyColor = await page.evaluate(() => getComputedStyle(document.body).color);
      const linkColor = await link.evaluate((el) => getComputedStyle(el).color);
      expect(
        linkColor,
        `${name}: back link must use --accent-text, not the inherited body colour`,
      ).not.toBe(bodyColor);

      // --- Click target: acceptance criterion #3 -----------------------
      // As a column flex container `.row-link` stretched to the full
      // content width, so the entire band above the title was clickable.
      // `inline-flex` shrink-wraps to icon + label, which must be far
      // narrower than the header copy it sits above.
      const linkBox = await link.boundingBox();
      expect(linkBox, `${name}: back link must have a layout box`).not.toBeNull();
      const copyBox = await page.locator(".page-head__copy").first().boundingBox();
      expect(copyBox, `${name}: page-head copy must have a layout box`).not.toBeNull();
      expect(
        linkBox!.width,
        `${name}: back link must shrink-wrap, not span the header width`,
      ).toBeLessThan(copyBox!.width / 2);

      // --- Spacing: acceptance criterion #1 ----------------------------
      // `.row-link` carries no bottom margin, so the <h1> crowded the
      // link. The primitive's 24px margin-bottom must show up as real
      // vertical distance between the link's bottom edge and the title.
      const titleBox = await page.locator(".page-head__title").first().boundingBox();
      expect(titleBox, `${name}: page-head title must have a layout box`).not.toBeNull();
      expect(
        titleBox!.y - (linkBox!.y + linkBox!.height),
        `${name}: title must sit ~24px below the back link, not crowd it`,
      ).toBeGreaterThanOrEqual(20);

      // --- The wrong primitive is gone ---------------------------------
      // `.row-link` belongs to browser table rows, never to a page head.
      await expect(
        page.locator(".row-link"),
        `${name}: no .row-link may survive on a detail page`,
      ).toHaveCount(0);
      await expect(
        page.locator("header.page-head--back-link"),
        `${name}: the header must opt into the back-link grid`,
      ).toHaveCount(1);
    }
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
