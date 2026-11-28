import { test, expect } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { createResource, readResource, deleteResources } from "../pages/api";

// A document past the guided form's budget (#2876): the form card shows a
// notice in place of its rows, and the JSON pane still opens, checks and
// saves the document.

function valueSet(concepts: number, title: string): Record<string, unknown> {
  return {
    resourceType: "ValueSet",
    status: "active",
    title,
    compose: {
      include: [
        {
          system: "http://example.org/large-document",
          concept: Array.from({ length: concepts }, (_, i) => ({ code: `c${i}`, display: `Concept ${i}` })),
        },
      ],
    },
  };
}

/** Replaces the JSON as one paste and waits for the guided form to follow.
 * Typing a document this size through the keyboard takes tens of seconds. */
async function paste(ed: Editor, doc: unknown): Promise<void> {
  await ed.cm.evaluate((dom, text) => {
    const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
    const view = CM.EditorView.findFromDOM(dom);
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, userEvent: "input.paste" });
  }, JSON.stringify(doc, null, 2));
  await ed.formCaughtUp();
}

function conceptCount(resource: Record<string, unknown>): number {
  const compose = resource.compose as { include: { concept: unknown[] }[] };
  return compose.include[0].concept.length;
}

test("a stored document past the budget opens in the JSON pane and saves", async ({ page, request }) => {
  const id = await createResource(request, "ValueSet", valueSet(800, "Large"));
  try {
    await page.goto(`/ui/editor?type=ValueSet&id=${id}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await expect(ed.root.locator("[data-editor-too-large]")).toBeVisible();
    await expect(ed.root.locator(".editor-row")).toHaveCount(0);
    expect(await ed.jsonText()).toContain('"c799"');
    await expect(page.locator("#editor-status")).toHaveText("");

    // Save runs its validation render first; past the budget that render
    // still succeeds, so the write goes through.
    await page.locator("#editor-save").click();
    await expect
      .poll(
        async () => ((await readResource(request, "ValueSet", id)).meta as { versionId: string }).versionId,
        { timeout: 15_000 },
      )
      .toBe("2");
    expect(conceptCount(await readResource(request, "ValueSet", id))).toBe(800);
    await expect(ed.root.locator("[data-editor-too-large]")).toBeVisible();
  } finally {
    await deleteResources(request, "ValueSet", [id]);
  }
});

test("a document grown past the budget swaps the rows for the notice, stays checked, and saves", async ({ page, request }) => {
  const id = await createResource(request, "ValueSet", valueSet(20, "Small"));
  try {
    await page.goto(`/ui/editor?type=ValueSet&id=${id}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await expect(ed.root.locator("[data-editor-too-large]")).toHaveCount(0);
    await expect(ed.root.locator(".editor-row").first()).toBeVisible();

    // Still validated: an invalid status blocks the save and is listed.
    await paste(ed, { ...valueSet(800, "Grown"), id, status: "not-a-status" });
    await expect(ed.root.locator("[data-editor-too-large]")).toBeVisible();
    await expect(ed.root.locator(".editor-row")).toHaveCount(0);
    expect(await ed.errorCount()).toBeGreaterThan(0);
    await expect(ed.root.locator(".editor__orphan").first()).toContainText("status");
    await page.locator("#editor-save").click();
    await expect(page.locator("#editor-status")).not.toHaveText("");
    expect(conceptCount(await readResource(request, "ValueSet", id))).toBe(20);

    await paste(ed, { ...valueSet(800, "Grown"), id });
    expect(await ed.errorCount()).toBe(0);
    await page.locator("#editor-save").click();
    await expect
      .poll(async () => conceptCount(await readResource(request, "ValueSet", id)), { timeout: 15_000 })
      .toBe(800);

    // Back under the budget, the rows return.
    await paste(ed, { ...valueSet(20, "Small again"), id });
    await expect(ed.root.locator("[data-editor-too-large]")).toHaveCount(0);
    await expect(ed.root.locator(".editor-row").first()).toBeVisible();
  } finally {
    await deleteResources(request, "ValueSet", [id]);
  }
});
