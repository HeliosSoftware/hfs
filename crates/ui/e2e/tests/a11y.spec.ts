import { test, expect } from "../pages/fixtures";
import AxeBuilder from "@axe-core/playwright";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
import { VdEditor } from "../pages/vd-editor";

// Tier 1 of the strategy (issue #249): WCAG 2.2 AA is the spec, axe-core the
// harness. Contrast differs per theme, so every route is scanned in both light
// and dark. axe-core does not currently execute its disabled target-size rule;
// design-system.spec.ts carries an explicit >=24px action-target guard. The
// route list is shared with the other cross-page guards (#543); the bulk-import
// detail page has no static URL, so it is seeded and scanned separately below.
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

  test(`open Bulk Import dialogs are free of WCAG 2.2 AA violations — ${theme}`, async ({
    page,
    chrome,
    request,
  }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/bulk-import", { waitUntil: "networkidle" });
    await page.locator("summary.btn", { hasText: "New Submission" }).click();
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    await page.locator(".disclosure__summary", { hasText: "Advanced options" }).click();
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    await page.keyboard.press("Escape");

    const detail = await seedBulkImportDetail(request);
    await page.goto(detail, { waitUntil: "networkidle" });
    await page.locator("summary.btn", { hasText: "Edit" }).click();
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
  });

  test(`invalid Resources create state is accessible — ${theme}`, async ({ page, chrome }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/resources?type=patient", { waitUntil: "networkidle" });

    const create = page.locator("#resource-create");
    const reason = page.locator("#resource-create-reason");
    await expect(create).toBeDisabled();
    await expect(create).toHaveAttribute("aria-describedby", "resource-create-reason");
    await expect(reason).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  test(`expanded CapabilityStatement JSON is accessible — ${theme}`, async ({ page, chrome }) => {
    test.setTimeout(120_000);
    await chrome.seedTheme(theme);
    await page.goto("/ui/capability-statement", { waitUntil: "networkidle" });
    const body = page.locator("#capability-json-body");
    await page.locator("[data-capability-json-expand-all]").click();
    await expect(body).not.toHaveAttribute("aria-busy", "true");
    await expect(page.locator("[data-capability-json-tree] > [data-expansion-state]")).toBeVisible();
    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  // #821: the ViewDefinition editor's own three floating UIs — none of them
  // is on the page by default, so the general `ROUTES` sweep (`?vd=new`
  // above) never opens any of them. `withTags(WCAG)` scans the whole page
  // as it stands at that moment, popup/tooltip/panel included, exactly like
  // every other targeted state test in this file.

  test(`ViewDefinition editor completion popup is free of WCAG 2.2 AA violations — ${theme}`, async ({
    page,
    chrome,
  }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/sql/view-definitions?vd=new", { waitUntil: "networkidle" });
    const ed = new VdEditor(page);
    const doc = `{
  "resourceType": "ViewDefinition",
  "resource": "Patient",
  "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }] }],
  "status": "active"
}`;
    await ed.setDoc(doc);
    await ed.setCursorAfter(doc, '"status": "active"');
    await page.keyboard.press("Control+Space");
    await expect(ed.completionPopup).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  test(`ViewDefinition editor lint tooltip is free of WCAG 2.2 AA violations — ${theme}`, async ({
    page,
    chrome,
  }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/sql/view-definitions?vd=new", { waitUntil: "networkidle" });
    const ed = new VdEditor(page);
    const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "columns": [{ "name": "id", "path": "getResourceKey()" }]
    }
  ]
}`;
    await ed.setDoc(doc);
    const errorRange = page.locator(".cm-lintRange-error", { hasText: '"columns"' });
    await errorRange.hover();
    await expect(ed.lintTooltip).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  test(`ViewDefinition editor lint panel is free of WCAG 2.2 AA violations — ${theme}`, async ({
    page,
    chrome,
  }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/sql/view-definitions?vd=new", { waitUntil: "networkidle" });
    const ed = new VdEditor(page);
    const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "columns": [{ "name": "id", "path": "getResourceKey()" }]
    }
  ]
}`;
    await ed.setDoc(doc);
    // "columns" carries two fixes (rename, remove) — Ctrl+. with more than
    // one action under the cursor opens the panel (see
    // vd-editor-lint.spec.ts's own test of this exact mechanism).
    await ed.setCursorAt(doc, '"columns"');
    await page.keyboard.press("ControlOrMeta+.");
    await expect(ed.lintPanel).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });
}

test("terminal export delete disclosure is accessible and viewport-bound", async ({ page }) => {
  await page.goto("/ui/bulk-export/new");
  const exportName = `a11y-terminal-${Date.now()}`;
  const form = page.locator('form[action="/ui/bulk-export"]');
  await form.locator('input[name="name"]').fill(exportName);
  await form.locator('input[name="scope"][value="system"]').check();
  await form.getByRole("button", { name: "Start Export" }).click();
  let card = page.locator(".job-card").filter({ hasText: exportName });
  await card.getByRole("button", { name: "Cancel" }).click();

  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto("/ui/bulk-export", { waitUntil: "networkidle" });
    card = page.locator(".job-card").filter({ hasText: exportName });
    const disclosure = card.locator("details.job-card__delete");
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);

    await disclosure.locator("summary").click();
    await expect(disclosure).toHaveAttribute("open", "");
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    const panel = await disclosure.locator(".job-card__delete-confirm").boundingBox();
    expect(panel).not.toBeNull();
    expect(panel!.x).toBeGreaterThanOrEqual(0);
    expect(panel!.y).toBeGreaterThanOrEqual(0);
    expect(panel!.x + panel!.width).toBeLessThanOrEqual(viewport.width);
    expect(panel!.y + panel!.height).toBeLessThanOrEqual(viewport.height);
  }
});
