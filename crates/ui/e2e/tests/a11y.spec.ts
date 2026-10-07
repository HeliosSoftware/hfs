import { SearchBuilder } from "../pages/search-builder";
import { holdSearches } from "../pages/search-lifecycle";
import { test, expect, confirmDialog, dismissConfirm } from "../pages/fixtures";
import type { Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { axeSummary } from "../pages/axe";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
import { VdEditor } from "../pages/vd-editor";
import { addFromRoot, EDITOR_SCENARIOS, openEditorScenario } from "../pages/editor-scenarios";

// Tier 1 of the strategy (issue #249): WCAG 2.2 AA is the spec, axe-core the
// harness. Contrast differs per theme, so every route is scanned in both light
// and dark. axe-core does not currently execute its disabled target-size rule;
// design-system.spec.ts carries an explicit >=24px action-target guard. The
// route list is shared with the other cross-page guards (#543); the bulk-import
// detail page has no static URL, so it is seeded and scanned separately below.
const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
const THEMES = ["light", "dark"] as const;

// Every `analyze()` opens and closes a throwaway page of its own — axe runs
// `finishRun` there to aggregate the per-frame partials. It is the most
// expensive and by far the most load-sensitive thing these tests do (a stalled
// `browserContext.newPage` has been seen taking 24s on a busy machine), so a
// test that scans several states pays for it several times over, on top of its
// navigations. The suite-wide budget in playwright.config.ts covers one such
// stall; a multi-scan test can meet several, so give those tests a budget that
// grows with the number of scans instead — otherwise one stall fails a run that
// found no violation at all.
const SCAN_BUDGET_MS = 40_000;

/**
 * Scans the page as it currently stands and names any offender.
 *
 * `state` describes what is open, because these tests scan several states of
 * the same page: without it a red run says only which test failed, not which
 * dialog was on screen when axe objected.
 */
async function expectNoViolations(page: Page, state: string): Promise<void> {
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(
    violations,
    `axe found ${violations.length} violation(s) with ${state}:\n${axeSummary(violations)}`,
  ).toEqual([]);
}

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
    // Three scans, two navigations and a seeded submission share one budget.
    test.setTimeout(3 * SCAN_BUDGET_MS);
    await chrome.seedTheme(theme);
    await page.goto("/ui/bulk-import", { waitUntil: "networkidle" });

    // The dialogs are `<details>` disclosures, so a click that lands but does
    // not open one — a toggle arriving on an already-open panel, which the
    // editor-controls specs were bitten by — would leave axe scanning the
    // closed page and this test green. Assert the state the test is named for
    // before every scan, as the targeted-state tests below and in
    // bulk-export.spec.ts do.
    await page.locator("summary.btn", { hasText: "New Submission" }).click();
    await expect(page.locator('[aria-labelledby="bulk-import-create-dialog-title"]')).toBeVisible();
    await expectNoViolations(page, "the New Submission dialog open");

    await page.locator(".disclosure__summary", { hasText: "Advanced options" }).click();
    await expect(page.locator('textarea[name="file_request_headers"]')).toBeVisible();
    await expectNoViolations(page, "the New Submission dialog's Advanced options unfolded");
    await page.keyboard.press("Escape");

    const detail = await seedBulkImportDetail(request);
    await page.goto(detail, { waitUntil: "networkidle" });
    await page.locator("summary.btn", { hasText: "Edit" }).click();
    await expect(page.locator('[aria-labelledby="bulk-import-edit-title"]')).toBeVisible();
    await expectNoViolations(page, "the Edit Submission dialog open");
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
    // Ctrl-Shift-M (`lintKeymap`) opens the bottom panel.
    await ed.setCursorAt(doc, '"columns"');
    await page.keyboard.press("ControlOrMeta+Shift+M");
    await expect(ed.lintPanel).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  test(`ViewDefinition editor quick-fix menu is free of WCAG 2.2 AA violations — ${theme}`, async ({
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
    // "columns" carries two fixes (rename, remove) - Ctrl+. with more than
    // one action under the cursor opens the quick-fix menu (see
    // vd-editor-lint.spec.ts's own test of this exact mechanism).
    await ed.setCursorAt(doc, '"columns"');
    await page.keyboard.press("ControlOrMeta+.");
    await expect(ed.quickFixMenu).toBeVisible();

    const { violations } = await new AxeBuilder({ page }).withTags(WCAG).analyze();
    expect(violations).toEqual([]);
  });

  // #1677: a query with a chained condition, result controls and an include,
  // so the builder shows every row kind that renders a <select> (the chain's
  // target type, a control or include key, an include's target type). The
  // ROUTES sweep above analyzes the bare pages, where none of them exist.
  test(`the search builder's chain, control and include rows are accessible — ${theme}`, async ({ page, chrome }) => {
    await chrome.seedTheme(theme);
    await page.goto("/ui/search", { waitUntil: "networkidle" });
    await page.locator("[data-mode-btn=builder]").click();
    const builder = new SearchBuilder(page);
    await builder.run("Observation?subject:Patient.name=a&_count=5&_include=Observation:subject");
    const selects = page.locator("#builder-sections select:visible");
    await expect(page.locator("select.builder-row__key").first()).toBeVisible();
    await expect(page.locator("select.builder-row__itarget").first()).toBeVisible();
    for (const select of await selects.all()) await expect(select).toHaveAccessibleName(/.+/);
    await expectNoViolations(page, "the search builder showing chain, control and include rows");
  });

  // Open disclosures remain accessible after #1721 closes the picker on Add.
  test(`the resource editor's add picker is accessible — ${theme}`, async ({
    page,
    chrome,
    resources,
  }) => {
    await chrome.seedTheme(theme);
    await resources.goto("Patient");
    await resources.openCreate();
    const ed = resources.modal.editor;

    await ed.openAddPanel();
    await expect(ed.addPanel).toHaveAttribute("open", "");
    await ed.addFilter().fill("birth");
    await ed.addItem("birthDate").click();
    await expect(ed.addPanel).not.toHaveAttribute("open");
    await expect(ed.addUndo()).toBeVisible();
    await ed.openAddPanel();
    await ed.openExtensions();
    await expect(ed.addGroup("extensions")).toHaveAttribute("open", "");

    await expectNoViolations(page, "the reopened add picker and Extensions");
  });

  for (const scenario of EDITOR_SCENARIOS) {
    test(`issue1720 issue1721 ${scenario.name} created fields, groups and discreet Undo are accessible — ${theme}`, async ({ page, chrome, request }) => {
      test.setTimeout(2 * SCAN_BUDGET_MS);
      await page.setViewportSize({ width: 1280, height: 800 });
      await chrome.seedTheme(theme);
      const { ed, cleanup } = await openEditorScenario(page, request, scenario);
      try {
        await addFromRoot(ed, scenario.primitive);
        await expect(ed.rowAt(scenario.primitivePath).locator("[data-set]")).toBeFocused();
        await expect(ed.addUndo()).toBeVisible();
        await expect(ed.root.locator(".editor-add__added")).toHaveCount(0);
        await expectNoViolations(page, `${scenario.name}: created primitive and Undo`);
        await addFromRoot(ed, scenario.complex);
        await expect(ed.rowAt(`${scenario.complex}.0`)).toBeFocused();
        await expect(ed.rowAt(scenario.complex)).toHaveAttribute("data-collection", "");
        await expect(ed.addStatus).toHaveAttribute("role", "status");
        await expectNoViolations(page, `${scenario.name}: named group, focused complex and Undo`);
      } finally { await cleanup(); }
    });
  }
}

test("terminal export delete menu and shared dialog are accessible and viewport-bound", async ({ page }) => {
  // Two viewports, scanned closed, menu open and dialog open: six scans in one test.
  test.setTimeout(6 * SCAN_BUDGET_MS);
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

    // Menu open: axe, then the shared dialog open: axe and viewport fit.
    await card.locator("details.menu > summary").click();
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    await disclosure.locator("summary").click();
    await expect(disclosure).not.toHaveAttribute("open", /.*/);
    const dialog = confirmDialog(page);
    await expect(dialog).toBeVisible();
    expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    const box = await dialog.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
    expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
    await dismissConfirm(page);
  }
});

for (const theme of THEMES) {
  test(`issue1577 pending and slow search are accessible — ${theme}`, async ({ page, chrome }) => {
    test.setTimeout(2 * SCAN_BUDGET_MS);
    await chrome.seedTheme(theme);
    await page.clock.install();
    await holdSearches(page);
    await page.goto("/ui/search", { waitUntil: "networkidle" });
    await page.locator("[data-mode-btn=builder]").click();
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
    const builder = new SearchBuilder(page);
    await builder.run("Patient?_id=issue1577-axe");
    await expect(builder.status).toBeVisible();
    await expect(page.locator(".builder-row__modifier")).toHaveAccessibleName("Modifiers");
    await expect(builder.status).toBeInViewport();
    await page.clock.resume();
    await expectNoViolations(page, "pending search");
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
    await page.clock.runFor(60000);
    await expect(builder.slow).toBeVisible();
    await page.clock.resume();
    await expectNoViolations(page, "slow search");
    await builder.cancel.click();
    await page.clock.resume();
  });
}

for (const theme of THEMES) {
  test(`export detail page is accessible — ${theme}`, async ({ page, request, chrome }) => {
    test.setTimeout(2 * SCAN_BUDGET_MS);
    const previous = (await (await request.get("/_user/settings")).json()).bulkExport ?? null;
    const jobs = {
      "a11y-detail": {
        name: "A11y detail export", status: "complete", scope: "group", groupId: "grp-1",
        types: "Patient,Observation", remoteJob: "no-remote-job",
        startedAt: "2026-01-01T09:00:00Z", finishedAt: "2026-01-01T09:00:30Z",
        files: [{ type: "Patient", url: "http://files.test/p1" }, { type: "Observation", url: "http://files.test/o1" }],
      },
    };
    try {
      expect((await request.patch("/_user/settings", { data: { bulkExport: null } })).ok()).toBe(true);
      expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs } } })).ok()).toBe(true);
      await chrome.seedTheme(theme);
      await page.goto("/ui/bulk-export/active/a11y-detail", { waitUntil: "networkidle" });
      await expect(page.locator("table.data-table a[download]")).toHaveCount(2);
      expect((await new AxeBuilder({ page }).withTags(WCAG).analyze()).violations).toEqual([]);
    } finally {
      await request.patch("/_user/settings", { data: { bulkExport: null } });
      await request.patch("/_user/settings", { data: { bulkExport: previous } });
    }
  });
}
