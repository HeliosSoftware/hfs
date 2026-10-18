import type { APIRequestContext, Locator } from "@playwright/test";
import { createResource, createSqlQueryLibrary, deleteResources, waitSearchable } from "../pages/api";
import { expect, test } from "../pages/fixtures";

const BIG_INTEGER = "9007199254740993";

async function numericFixture(request: APIRequestContext) {
  const name = `themed_numeric_${Date.now()}`;
  const canonical = `http://example.org/ViewDefinition/${name}`;
  const patient = await createResource(request, "Patient", { name: [{ family: name }] });
  let vd: string | undefined;
  let lib: string | undefined;
  async function cleanup() {
    if (lib) await deleteResources(request, "Library", [lib]);
    if (vd) await deleteResources(request, "ViewDefinition", [vd]);
    await deleteResources(request, "Patient", [patient]);
  }
  try {
    vd = await createResource(request, "ViewDefinition", {
      name, url: canonical, status: "active", resource: "Patient",
      where: [{ path: `name.family = '${name}'` }],
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    lib = await createSqlQueryLibrary(request, name, canonical, "SELECT id FROM v WHERE :min_height < 0", [
      { name: "min_height", use: "in", type: "decimal" },
      { name: "days", use: "in", type: "integer", defaultInteger: 30 },
      { name: "big", use: "in", type: "integer64", defaultInteger64: BIG_INTEGER },
    ]);
    await waitSearchable(request, "Patient", patient);
    await waitSearchable(request, "ViewDefinition", vd);
    await waitSearchable(request, "Library", lib);
    return { patient, lib, cleanup };
  } catch (error) { await cleanup(); throw error; }
}

async function numericAppearance(input: Locator, step: string) {
  await expect(input).toHaveAttribute("type", "number");
  await expect(input).toHaveAttribute("step", step);
  await expect(input).toHaveClass(/field__input--number/);
  expect(await input.evaluate(node => getComputedStyle(node).appearance)).toBe("textfield");
}

async function parameterValues(form: Locator, prefix: string) {
  return form.evaluate((node, prefix) => {
    const data = new FormData(node as HTMLFormElement);
    return ["min_height", "days", "big"].map(name => data.getAll(`${prefix}:${name}`));
  }, prefix);
}

for (const theme of ["light", "dark"] as const) {
  test(`issue2080 SQL Run: themed numeric fields preserve string FormData, defaults and live results in ${theme}`, async ({ page, request, chrome }) => {
    const f = await numericFixture(request);
    try {
      await chrome.seedTheme(theme);
      await page.goto(`/ui/sql/queries?lib=${f.lib}`, { waitUntil: "networkidle" });
      const form = page.locator("#lib-editor-form");
      const amount = page.locator("input[name='param:min_height']");
      const days = page.locator("input[name='param:days']");
      const big = page.locator("input[name='param:big']");
      await numericAppearance(amount, "any");
      await numericAppearance(days, "1");
      await numericAppearance(big, "1");
      await expect(amount).toHaveAttribute("form", "lib-editor-form");
      await expect(page.locator("#run-notice")).toContainText("Waiting for a value for :min_height");
      expect(await parameterValues(form, "param")).toEqual([[""], ["30"], [BIG_INTEGER]]);
      await amount.fill("-150.5");
      await expect(page.locator("#run-results .data-table")).toBeVisible();
      await expect(page.locator("#run-results")).toContainText(f.patient);
      expect(await parameterValues(form, "param")).toEqual([["-150.5"], ["30"], [BIG_INTEGER]]);
      await days.fill("-2");
      await big.fill(BIG_INTEGER);
      expect(await parameterValues(form, "param")).toEqual([["-150.5"], ["-2"], [BIG_INTEGER]]);
      await amount.fill("");
      await days.fill("");
      await expect(page.locator("#run-notice")).toContainText("Waiting for a value for :min_height");
      // The previous successful table remains available while the run waits.
      await expect(page.locator("#run-results .data-table")).toBeVisible();
      expect(await parameterValues(form, "param")).toEqual([[""], [""], [BIG_INTEGER]]);
      await amount.fill("150.5");
      await expect(page.locator("#run-notice")).not.toContainText("Waiting for a value");
      await expect(page.locator("#run-results .data-table tbody tr:not(.data-table__empty)")).toHaveCount(0);
      // Keyboard stepping remains native and useful after the visual arrows go.
      await days.fill("30");
      await days.focus();
      await page.keyboard.press("ArrowUp");
      await expect(days).toHaveValue("31");
    } finally { await f.cleanup(); }
  });

  test(`issue2080 SQL Export: numeric styling preserves form values and required feedback in ${theme}`, async ({ page, request, chrome, sqlExport }) => {
    const f = await numericFixture(request);
    const reference = `Library/${f.lib}`;
    try {
      await chrome.seedTheme(theme);
      await sqlExport.gotoNew();
      await sqlExport.subjectCheckbox(reference).check();
      const amount = sqlExport.paramField(reference, "min_height");
      const days = sqlExport.paramField(reference, "days");
      const big = sqlExport.paramField(reference, "big");
      await numericAppearance(amount, "any");
      await numericAppearance(days, "1");
      await numericAppearance(big, "1");
      const prefix = `param:${reference}`;
      expect(await parameterValues(sqlExport.form, prefix)).toEqual([[""], ["30"], [BIG_INTEGER]]);
      await sqlExport.startButton.click();
      const error = sqlExport.paramError(reference, "min_height");
      await expect(error).toBeVisible();
      await expect(error).toHaveText("This value is required.");
      await expect(amount).toBeFocused();
      await expect(amount).toHaveAccessibleDescription("This value is required.");
      await amount.fill("-150.5");
      await days.fill("-2");
      await big.fill(BIG_INTEGER);
      await expect(error).toBeHidden();
      await expect(amount).not.toHaveAttribute("aria-invalid", "true");
      expect(await parameterValues(sqlExport.form, prefix)).toEqual([["-150.5"], ["-2"], [BIG_INTEGER]]);
      await amount.fill("");
      await days.fill("");
      await sqlExport.rowToggle(reference).click();
      await expect(sqlExport.paramSummary(reference)).toContainText(":days = 30");
      await sqlExport.startButton.click();
      await expect(sqlExport.paramsRow(reference)).toBeVisible();
      await expect(error).toBeVisible();
      await expect(amount).toBeFocused();
      await amount.fill("150.5");
      await expect(error).toBeHidden();
      expect(await parameterValues(sqlExport.form, prefix)).toEqual([["150.5"], [""], [BIG_INTEGER]]);
      // Unchecking preserves the same controls and their values for reselecting.
      await sqlExport.subjectCheckbox(reference).uncheck();
      await expect(amount).not.toHaveAttribute("required", "");
      await sqlExport.subjectCheckbox(reference).check();
      await expect(amount).toHaveValue("150.5");
    } finally { await f.cleanup(); }
  });
}
