import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Tier 1 of the a11y strategy, mirrored from `crates/ui/e2e/tests/a11y.spec.ts`:
// WCAG 2.2 AA is the spec, axe-core the harness. Contrast and target-size
// verdicts differ per theme, so every route is scanned in both light and dark.
//
// Unlike the HFS suite this one has no `pages/fixtures` helper — the HTS specs
// drive the bare `@playwright/test` fixtures — so the theme seed that
// `chrome.seedTheme()` provides there is inlined here as `seedTheme()`.
const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const THEMES = ["light", "dark"] as const;

// The fixture roster seeded by `seed.mjs` (globalSetup) — NOT the official
// bootstrap ids. `{id}` routes are pinned to these so the scan is hermetic.
const CS = "ex-cs-1";
const VS = "ex-vs-1";
const CM = "ex-cm-1";

// Every full page the HTS UI router registers (see the `routes()` fns in
// `crates/hts-ui/src/*.rs`). Fragment endpoints (`/rows`, `/home/cards`) are
// htmx partials with no <html> of their own and are deliberately out of scope.
// The former `/ui/hts/operations*` pages were deleted, and the Diagnostics
// page lost its `/diagnostics/panel` tab fragment when it became a stacked-card
// mirror of HFS's Capability page — then was renamed to Capability &
// Conformance to match HFS's label and route. All of those are absent here by
// design; the pre-rename `/ui/hts/diagnostics` path is a 308 and is covered in
// `capability.spec.ts` rather than scanned here.
const ROUTES = [
  "/ui/hts",
  "/ui/hts/code-systems",
  `/ui/hts/code-systems/${CS}/lookup`,
  `/ui/hts/code-systems/${CS}/validate`,
  `/ui/hts/code-systems/${CS}/subsumes`,
  "/ui/hts/value-sets",
  `/ui/hts/value-sets/${VS}/expand`,
  "/ui/hts/concept-maps",
  `/ui/hts/concept-maps/${CM}/translate`,
  "/ui/hts/import",
  "/ui/hts/capability-statement",
];

for (const theme of THEMES) {
  for (const route of ROUTES) {
    test(`${route} is free of WCAG 2.2 AA violations — ${theme}`, async ({ page }) => {
      // `assets/theme.js` (shared with crates/ui) stamps `data-theme` on <html>
      // from localStorage["hfs-theme"] before first paint, so the seed has to
      // land as an init script rather than after navigation.
      await page.addInitScript((t) => {
        try {
          localStorage.setItem("hfs-theme", t as string);
        } catch {}
      }, theme);

      const response = await page.goto(route, { waitUntil: "networkidle" });
      expect(response?.status(), `${route} must respond 200`).toBe(200);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

      const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();

      // Name the offenders — with axe's per-node check message (it carries the
      // measured contrast ratio / geometry) so a red run is actionable.
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
