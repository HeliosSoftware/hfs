import { test, expect, acceptConfirm, dismissConfirm, dialogsSeen } from "../pages/fixtures";
import { createResource, waitSearchable, deleteResources } from "../pages/api";

const kinds = [
  { name: "ViewDefinition", type: "ViewDefinition", base: "/ui/sql/view-definitions", selection: "vd", root: "#vd-editor-grid", cm: "#vd-editor" },
  { name: "SQL Query", type: "Library", base: "/ui/sql/queries", selection: "lib", root: "#lib-details-grid", cm: "#lib-details-editor", code: "sql-query" },
  { name: "SQL View", type: "Library", base: "/ui/sql/views", selection: "lib", root: "#lib-details-grid", cm: "#lib-details-editor", code: "sql-view" },
] as const;
const system = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";

for (const kind of kinds) {
  for (const mode of ["raw", "guided", "queued", ...(kind.type === "Library" ? ["sql"] : [])]) {
    test(`${kind.name} Cancel checks ${mode} edits without waiting for mutations`, async ({ page, request }) => {
      const body = kind.type === "ViewDefinition"
        ? { name: `cancel_${Date.now()}`, status: "draft", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] }
        : { name: `cancel_${Date.now()}`, status: "draft", type: { coding: [{ system, code: kind.code }] }, content: [{ contentType: "application/sql", data: Buffer.from("SELECT 1 AS value").toString("base64") }] };
      const id = await createResource(request, kind.type, body);
      let release = () => {};
      try {
        await waitSearchable(request, kind.type, id);
        const origin = "/ui/sql/export/new?subject=Library%2Fopening#opening";
        await page.goto(`${kind.base}?${kind.selection}=${id}&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
        const cancel = page.locator(`#${kind.selection}-editor-cancel`);
        await expect(cancel).toHaveAttribute("href", origin);
        if (mode === "raw" || mode === "sql") {
          const editor = page.locator(`${mode === "sql" ? "#sql-editor" : kind.cm} .cm-content`);
          await editor.click();
          await page.keyboard.press("ControlOrMeta+a");
          await page.keyboard.insertText(mode === "sql" ? "SELECT 1723 AS value" : JSON.stringify({ resourceType: kind.type, ...body, id, name: "changed" }));
        } else {
          if (mode === "queued") {
            const hold = new Promise<void>(resolve => { release = resolve; });
            await page.route("**/ui/editor/render", async route => {
              if (new URLSearchParams(route.request().postData() ?? "").get("op") === "set") await hold;
              await route.continue().catch(() => {});
            });
          }
          const input = page.locator(`${kind.root} [data-set='name']`);
          if (mode === "guided") await input.click();
          else await input.fill("changed");
          if (mode === "queued") {
            const pending = page.waitForRequest(r => r.url().endsWith("/ui/editor/render") && new URLSearchParams(r.postData() ?? "").get("op") === "set");
            await input.blur();
            await pending;
          }
        }
        dialogsSeen(page);
        // DOM click avoids waiting for a mutation-rendered projection and
        // exercises the final synchronous guard, including the rAF race.
        await cancel.evaluate((link, immediate) => {
          if (immediate) {
            const input = document.querySelector("[data-set='name']") as HTMLInputElement;
            input.value = "changed";
            input.dispatchEvent(new Event("input", { bubbles: true }));
          }
          (link as HTMLAnchorElement).click();
        }, mode === "guided");
        await dismissConfirm(page, "You have unsaved changes. Discard them and leave this page?");
        expect(dialogsSeen(page)).toEqual([]);
        expect(new URL(page.url()).pathname).toBe(kind.base);
        if (mode === "guided") await expect(page.locator(`${kind.root} [data-set='name']`)).toHaveValue("changed");
        await cancel.evaluate(link => (link as HTMLAnchorElement).click());
        await acceptConfirm(page, "You have unsaved changes. Discard them and leave this page?");
        await page.waitForURL(`**${origin}`);
        expect(dialogsSeen(page)).toEqual([]);
      } finally {
        release();
        await deleteResources(request, kind.type, [id]);
      }
    });
  }

  test(`${kind.name} clean draft Cancel uses section root with no Delete`, async ({ page }) => {
    await page.goto(`${kind.base}?${kind.selection}=new`, { waitUntil: "networkidle" });
    await expect(page.locator("[data-crud-delete]")).toHaveCount(0);
    await expect(page.locator(`#${kind.selection}-editor-cancel`)).toHaveAttribute("href", kind.base);
    dialogsSeen(page);
    await page.locator(`#${kind.selection}-editor-cancel`).click();
    await page.waitForURL(url => url.pathname === kind.base && !url.searchParams.has(kind.selection));
    expect(dialogsSeen(page)).toEqual([]);
  });
}

test("Reads from and Used by capture this editor's query/hash without nesting its inbound return", async ({ page, request }) => {
  const vd = await createResource(request, "ViewDefinition", { name: `origin_vd_${Date.now()}`, status: "draft", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] });
  const view = await createResource(request, "Library", { name: `origin_view_${Date.now()}`, status: "draft", type: { coding: [{ system, code: "sql-view" }] }, relatedArtifact: [{ type: "depends-on", label: "v", resource: `ViewDefinition/${vd}` }], content: [{ contentType: "application/sql", data: Buffer.from("SELECT id FROM v").toString("base64") }] });
  const query = await createResource(request, "Library", { name: `origin_query_${Date.now()}`, status: "draft", type: { coding: [{ system, code: "sql-query" }] }, relatedArtifact: [{ type: "depends-on", label: "v", resource: `Library/${view}` }], content: [{ contentType: "application/sql", data: Buffer.from("SELECT id FROM v").toString("base64") }] });
  try {
    await waitSearchable(request, "Library", query);
    const own = `/ui/sql/views?lib=${view}&filter=keep`;
    for (const [target, selected] of [["/ui/sql/view-definitions", vd], ["/ui/sql/queries", query]]) {
      await page.goto(`${own}&return_to=%2Fui%2Fresources#opening`, { waitUntil: "networkidle" });
      const link = page.locator(`#lib-tables a[data-editor-link][href^='${target}?']`).first();
      await link.click();
      await page.waitForURL(url => url.pathname === target);
      expect(new URL(page.url()).searchParams.get("return_to")).toBe(`${own}#opening`);
      await expect(page.locator(target.endsWith("view-definitions") ? "#vd-editor-cancel" : "#lib-editor-cancel")).toHaveAttribute("href", `${own}#opening`);
      expect(new URL(page.url()).searchParams.get(target.endsWith("view-definitions") ? "vd" : "lib")).toBe(selected);
    }
  } finally {
    await deleteResources(request, "Library", [query, view]);
    await deleteResources(request, "ViewDefinition", [vd]);
  }
});
