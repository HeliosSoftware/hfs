import { test, expect } from "../../pages/fixtures";
import { createResource, waitSearchable, deleteResources } from "../../pages/api";

for (const code of ["view-definition", "sql-query", "sql-view"]) {
  test(`no JS ${code}: failed Save, native mutation and successful Save retain explicit Cancel origin`, async ({ page, request }) => {
    const vd = code === "view-definition";
    const type = vd ? "ViewDefinition" : "Library";
    const base = vd ? "/ui/sql/view-definitions" : `/ui/sql/${code === "sql-query" ? "queries" : "views"}`;
    const selection = vd ? "vd" : "lib";
    const body = vd ? { name: `nojs_cancel_${Date.now()}`, status: "draft", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] }
      : { name: `nojs_cancel_${Date.now()}`, status: "draft", type: { coding: [{ system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes", code }] }, content: [{ contentType: "application/sql", data: Buffer.from("SELECT 1 AS value").toString("base64") }] };
    const id = await createResource(request, type, body);
    try {
      await waitSearchable(request, type, id);
      const origin = "/ui/sql/export/new?subject=Library%2Fopening#opening";
      await page.goto(`${base}?${selection}=${id}&return_to=${encodeURIComponent(origin)}`);
      const textarea = page.locator("textarea[name='json']");
      const original = await textarea.inputValue();
      await textarea.fill("{invalid");
      await page.locator(`#${selection}-editor-form button[value='save']`).click();
      await expect(page.locator(`#${selection}-editor-cancel`)).toHaveAttribute("href", origin);
      await expect(textarea).toHaveValue("{invalid");
      await textarea.fill(original);
      // With no JS, only a native successful post rebuilds the panels
      // that an invalid JSON response could not render.
      await page.locator(`#${selection}-editor-form button[value='save']`).click();
      await page.waitForURL(url => url.searchParams.get("saved") === "1");
      if (code === "sql-query") {
        await page.locator("#lib-params summary").click();
        await page.locator("#lib-params input[name='param_name']").fill("ward");
        await page.locator("#lib-params button[value='add-parameter']").click();
        await expect(textarea).toContainText('"ward"');
        await expect(page.locator("#lib-editor-cancel")).toHaveAttribute("href", origin);
      } else if (code === "sql-view") {
        await page.locator("#lib-tables input[name='table_alias']").fill("");
        await page.locator("#lib-tables button[value='add-table']").click();
        await expect(page.locator("#lib-editor-cancel")).toHaveAttribute("href", origin);
      }
      await page.locator(`#${selection}-editor-form button[value='save']`).click();
      await page.waitForURL(url => url.searchParams.get("saved") === "1");
      expect(new URL(page.url()).searchParams.get("return_to")).toBe(origin);
      await expect(page.locator(`[data-crud-delete][data-id='${id}']`)).toBeVisible();
      await page.locator(`#${selection}-editor-cancel`).click();
      await page.waitForURL(`**${origin}`);
    } finally { await deleteResources(request, type, [id]); }
  });
}
