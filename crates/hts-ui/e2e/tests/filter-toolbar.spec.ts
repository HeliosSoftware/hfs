import { expect, test } from "@playwright/test";

// #2609: the browse pages' filter strip (search fields + Reset) must stay
// inside its card at every width — fields shrink, then the row wraps —
// and the canonical-URL column must not be starved to a one-glyph strip.
//
// Geometry, not CSS, is asserted: a rule can be present and still lose to
// a flex item's min-content width, which is exactly how this broke.

const PAGES = [
  { path: "/ui/hts/code-systems", form: "#hts-cs-filters", rows: "#hts-cs-rows", lastInput: "#filter-version" },
  { path: "/ui/hts/value-sets", form: "#hts-vs-filters", rows: "#hts-vs-rows", lastInput: "#filter-version" },
  { path: "/ui/hts/concept-maps", form: "#hts-cm-filters", rows: "#hts-cm-rows", lastInput: "#filter-url" },
];

// ~1024px is the window from the report; 700px sits in the old dead zone
// between the 520px wrap breakpoint and the ~1100px the row needed.
const WIDTHS = [1024, 700];

// Sub-pixel rounding slack.
const EPS = 1;

for (const p of PAGES) {
  for (const width of WIDTHS) {
    test(`${p.path} filter row stays inside its card at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(p.path);

      const card = page.locator(`${p.form} .card`);
      const reset = page.locator(`${p.form} .toolbar a.btn`);
      await expect(reset).toBeVisible();

      const cardBox = (await card.boundingBox())!;
      const cardRight = cardBox.x + cardBox.width;
      for (const el of [page.locator(p.lastInput), reset]) {
        const box = (await el.boundingBox())!;
        expect(box.x, "starts inside the card").toBeGreaterThanOrEqual(cardBox.x - EPS);
        expect(box.x + box.width, "ends inside the card").toBeLessThanOrEqual(cardRight + EPS);
      }

      // Nothing in the page scrolls sideways because of the toolbar.
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(EPS);
    });

    test(`${p.path} URL column is not starved at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(p.path);
      // By position, not by class, so this also measures the pre-fix markup.
      // URL is the third column on all three browsers.
      const cell = page.locator(`${p.rows} tbody tr:first-child td:nth-child(3)`);
      await expect(cell).toBeVisible();
      const box = (await cell.boundingBox())!;
      // The bug rendered one character per line (~20px including padding).
      expect(box.width).toBeGreaterThan(100);
    });
  }
}
