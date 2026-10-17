import { expect, type APIRequestContext, type Page } from "@playwright/test";
import { createResource, deleteResources, waitSearchable } from "./api";
import { Editor } from "./editor";

/** All five consumers of the same Guided form, with fields after padding. */
export const EDITOR_SCENARIOS = [
  { name: "standalone", type: "Patient", root: "#editor-body", primitive: "birthDate", primitivePath: "birthDate", complex: "contact" },
  { name: "Resources", type: "Patient", root: "#resource-editor-body", primitive: "birthDate", primitivePath: "birthDate", complex: "contact" },
  { name: "ViewDefinition", type: "ViewDefinition", root: "#vd-editor-grid", primitive: "profile", primitivePath: "profile.0", complex: "where" },
  { name: "SQL Queries", type: "Library", root: "#lib-details-grid", primitive: "title", primitivePath: "title", complex: "contact" },
  { name: "SQL Views", type: "Library", root: "#lib-details-grid", primitive: "title", primitivePath: "title", complex: "contact" },
] as const;
export type EditorScenario = typeof EDITOR_SCENARIOS[number];

/** The existing R4 schemas place identifier before all the tested fields. */
export async function openEditorScenario(page: Page, request: APIRequestContext, scenario: EditorScenario) {
  const identifier = Array.from({ length: 40 }, (_, i) => ({
    system: "https://example.org/editor-reveal", value: String(20000 + i),
  }));
  const name = `reveal_${scenario.name.replace(/\W/g, "_")}_${Date.now()}`;
  const body: Record<string, unknown> = scenario.type === "Patient"
    ? { identifier, name: [{ family: name, given: ["Ana", "Bea"] }] }
    : scenario.type === "ViewDefinition"
      ? { identifier, name, status: "draft", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] }
      : {
          identifier, name, status: "draft",
          type: { coding: [{ system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes", code: scenario.name === "SQL Views" ? "sql-view" : "sql-query" }] },
          content: [{ contentType: "application/sql", data: Buffer.from("SELECT 1 AS value").toString("base64") }],
        };
  const id = await createResource(request, scenario.type, body);
  try {
    await waitSearchable(request, scenario.type, id);
    const route = scenario.name === "standalone"
      ? `/ui/editor?type=Patient&id=${id}`
      : scenario.name === "Resources"
        ? `/ui/resources?url=${encodeURIComponent(`/Patient?_id=${id}`)}`
        : scenario.name === "ViewDefinition"
          ? `/ui/sql/view-definitions?vd=${id}`
          : `/ui/sql/${scenario.name === "SQL Views" ? "views" : "queries"}?lib=${id}`;
    await page.goto(route, { waitUntil: "networkidle" });
    if (scenario.name === "Resources") {
      await page.locator(`#query-results-body a.result-id[data-resource-id='${id}']`).click();
    }
    const ed = new Editor(page, page.locator(scenario.root));
    await expect(ed.rowAt("identifier.39.value")).toBeAttached();
    return { ed, cleanup: () => deleteResources(request, scenario.type, [id]) };
  } catch (error) {
    await deleteResources(request, scenario.type, [id]);
    throw error;
  }
}

export async function addFromRoot(ed: Editor, field: string) {
  await ed.openAddPanel();
  await ed.addFilter().fill(field);
  const tree = ed.root.locator(".editor-tree");
  expect(await tree.evaluate(node => node.scrollHeight > node.clientHeight)).toBe(true);
  await tree.evaluate(node => node.scrollTop = 0);
  const tailOffscreen = await ed.rowAt("identifier.39.value").evaluate(row => {
    const pane = row.closest(".editor-tree")!.getBoundingClientRect();
    return row.getBoundingClientRect().top > Math.min(pane.bottom, window.innerHeight);
  });
  expect(tailOffscreen, "the padding before the new field is outside the visible tree").toBe(true);
  await ed.addItem(field).click();
}

/** Observe actual focus/geometry without a test-side scroll or focus repair.
 * Sampling for 250ms catches the paired editor's former 120ms caret reveal. */
export async function expectStableReveal(ed: Editor, path: string, primitive: boolean) {
  await expect(ed.form).toHaveAttribute("data-focus", path);
  const target = primitive ? ed.rowAt(path).locator("[data-set]") : ed.rowAt(path);
  await expect(target).toBeFocused();
  if (primitive) {
    const selection = await target.evaluate(node => {
      const input = node as HTMLInputElement;
      return { start: input.selectionStart, end: input.selectionEnd, length: input.value.length };
    });
    expect(selection).toEqual({ start: 0, end: selection.length, length: selection.length });
  }
  const observation = await target.evaluate(node => new Promise<{ ok: boolean; detail?: string }>(resolve => {
    const tree = node.closest(".editor-tree") as HTMLElement;
    const started = performance.now();
    function check() {
      const rect = node.getBoundingClientRect();
      const pane = tree.getBoundingClientRect();
      const top = Math.max(0, pane.top + tree.clientTop);
      const bottom = Math.min(window.innerHeight, pane.top + tree.clientTop + tree.clientHeight);
      const left = Math.max(0, pane.left + tree.clientLeft);
      const right = Math.min(window.innerWidth, pane.left + tree.clientLeft + tree.clientWidth);
      const visible = rect.width > 0 && rect.height > 0 && rect.top >= top - 1 && rect.bottom <= bottom + 1 && rect.left >= left - 1 && rect.right <= right + 1;
      if (document.activeElement !== node || !visible) {
        resolve({ ok: false, detail: JSON.stringify({ focus: document.activeElement === node, rect: rect.toJSON(), top, bottom, left, right }) });
      } else if (performance.now() - started >= 250) {
        resolve({ ok: true });
      } else requestAnimationFrame(check);
    }
    requestAnimationFrame(check);
  }));
  expect(observation, observation.detail).toEqual({ ok: true });
}
