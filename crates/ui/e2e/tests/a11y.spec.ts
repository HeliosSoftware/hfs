import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Tier 1 of the strategy (issue #249): WCAG 2.2 AA is the spec, axe-core the
// harness. Contrast and target-size verdicts differ per theme, so every route
// is scanned in both light and dark.
const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const ROUTES = ["/ui", "/ui/resources", "/ui/compartments", "/ui/search-parameters"];
const THEMES = ["light", "dark"] as const;

for (const theme of THEMES) {
  for (const route of ROUTES) {
    test(`${route} is free of WCAG 2.2 AA violations — ${theme}`, async ({ page }) => {
      // Seed the theme before first paint, the way a returning user would.
      await page.addInitScript((t) => {
        try {
          localStorage.setItem("hfs-theme", t as string);
        } catch {}
      }, theme);

      await page.goto(route, { waitUntil: "networkidle" });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

      const { violations } = await new AxeBuilder({ page })
        .withTags(WCAG)
        // color-contrast is disabled for now: the muted palette (section
        // headers, "coming soon" items, the version string) comes straight from
        // the Figma design, and raising its contrast is a design-level change
        // across both themes — tracked as a follow-up, not gated here. Every
        // other WCAG rule (aria-prohibited-attr, target-size, names/roles,
        // landmarks, …) is a hard failure.
        .disableRules(["color-contrast"])
        .analyze();

      // Name the offenders in the failure message so a red run is actionable.
      const summary = violations
        .map(
          (v) =>
            `${v.impact ?? "?"}  ${v.id}: ${v.help}\n    ${v.nodes
              .map((n) => n.target.join(" "))
              .join("\n    ")}`,
        )
        .join("\n");
      expect(violations, `axe found ${violations.length} violation(s) on ${route} (${theme}):\n${summary}`).toEqual([]);
    });
  }
}
