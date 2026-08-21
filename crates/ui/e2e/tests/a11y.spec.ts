import { test, expect } from "../pages/fixtures";
import AxeBuilder from "@axe-core/playwright";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";

// Tier 1 of the strategy (issue #249): WCAG 2.2 AA is the spec, axe-core the
// harness. Contrast and target-size verdicts differ per theme, so every route
// is scanned in both light and dark. The route list is shared with the other
// cross-page guards (#543); the bulk-import detail page has no static URL, so
// it is seeded and scanned as its own named test below.
const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const THEMES = ["light", "dark"] as const;

for (const theme of THEMES) {
  for (const route of [...ROUTES, "bulk-import detail"]) {
    test(`${route} is free of WCAG 2.2 AA violations — ${theme}`, async ({ page, chrome, request }) => {
      await chrome.seedTheme(theme);
      const target = route === "bulk-import detail" ? await seedBulkImportDetail(request) : route;
      await page.goto(target, { waitUntil: "networkidle" });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

      const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();

      // Name the offenders — with axe's per-node check message (it carries the
      // measured geometry for e.g. target-size) so a red run is actionable.
      const summary = violations
        .map(
          (v) =>
            `${v.impact ?? "?"}  ${v.id}: ${v.help}\n    ${v.nodes
              .map((n) => {
                const why = [...n.any, ...n.all]
                  .map((c) => c.message)
                  .filter(Boolean)
                  .join("; ");
                return `${n.target.join(" ")}${why ? ` — ${why}` : ""}`;
              })
              .join("\n    ")}`,
        )
        .join("\n");
      expect(
        violations,
        `axe found ${violations.length} violation(s) on ${route} (${theme}):\n${summary}`,
      ).toEqual([]);
    });
  }
}
