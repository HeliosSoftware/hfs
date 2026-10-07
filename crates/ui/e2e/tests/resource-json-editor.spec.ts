import { test, expect } from "../pages/fixtures";
import type { Page } from "@playwright/test";
import { Editor } from "../pages/editor";
import { createResource, seedTwoVersions, deleteResources } from "../pages/api";

// The resource editor's JSON pane (#1756): one pane, always editable — the
// shared CodeMirror editor paired with the guided form, on the editor page and
// in the Resources modal alike. No read-only view, no "Edit raw" toggle.

function standalone(page: Page): Editor {
  return new Editor(page, page.locator("#editor-body"));
}

/** The editor's caret offset, read from CodeMirror's own state. */
async function caret(ed: Editor): Promise<number> {
  return ed.cm.evaluate((dom) => {
    const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
    return CM.EditorView.findFromDOM(dom).state.selection.main.head as number;
  });
}

// ---- the editor page ---------------------------------------------------

test("typing in the JSON updates the guided form after the pause", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.fillRaw({ resourceType: "Patient", gender: "female" });
  await expect(ed.root.locator('[data-set="gender"]')).toHaveValue("female", { timeout: 5000 });
});

test("editing a form field updates the JSON without moving the editor's caret", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", gender: "male", name: [{ family: "Caret" }] });

  const before = (await ed.jsonText()).indexOf('"gender"');
  await ed.setCursor(before);
  expect(await caret(ed)).toBe(before);

  const gender = ed.root.locator('[data-set="gender"]');
  await gender.fill("female");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await ed.jsonText()).includes('"female"')).toBe(true);
  await expect(ed.cm).toContainText('"female"');
  // The change is after the caret: the caret stays where it was.
  expect(await caret(ed)).toBe(before);
});

test("there is no Edit raw button, and Format re-indents with Shift+Alt+F", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await expect(page.getByRole("button", { name: /edit raw/i })).toHaveCount(0);
  await expect(ed.root.locator("#editor-json-edit")).toHaveCount(0);
  await expect(ed.formatButton).toBeVisible();

  await ed.setJson('{"resourceType":"Patient","gender":"male"}');
  await ed.cm.click();
  await page.keyboard.press("Shift+Alt+F");
  await expect
    .poll(() => ed.jsonText())
    .toBe(JSON.stringify({ resourceType: "Patient", gender: "male" }, null, 2));

  // The button does the same.
  await ed.setJson('{"resourceType":"Patient","gender":"female"}');
  await ed.formatButton.click();
  await expect
    .poll(() => ed.jsonText())
    .toBe(JSON.stringify({ resourceType: "Patient", gender: "female" }, null, 2));
});

test("Tab leaves the editor instead of inserting a tab", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.cm.click();
  await expect(ed.cm).toBeFocused();
  const before = await ed.jsonText();
  await page.keyboard.press("Tab");
  await expect(ed.cm).not.toBeFocused();
  expect(await ed.jsonText()).toBe(before);
  expect(await ed.codeEditor.evaluate((n) => n.contains(document.activeElement))).toBe(false);
});

test("Collapse all folds the editor and Expand all unfolds it", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({
    resourceType: "Patient",
    name: [{ family: "Fold", given: ["A", "B"] }],
    address: [{ city: "Springfield" }],
  });
  expect(await ed.foldedCount()).toBe(0);
  await ed.collapseAll();
  await expect.poll(() => ed.foldedCount()).toBeGreaterThan(0);
  await ed.expandAll();
  await expect.poll(() => ed.foldedCount()).toBe(0);
});

test("a change typed into the JSON is what Save persists", async ({ page, request }) => {
  const family = "Zj" + Date.now().toString(36);
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.fillRaw({ resourceType: "Patient", gender: "female", name: [{ family }] });
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-announce")).toContainText(/saved/i);

  const id = ((await page.locator("#editor-subject").textContent()) ?? "").match(/Patient\/([A-Za-z0-9.-]+)/)?.[1];
  expect(id).toBeTruthy();
  try {
    await page.reload({ waitUntil: "networkidle" });
    const saved = await request
      .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
      .then((r) => r.json());
    expect(saved.name?.[0]?.family).toBe(family);
    expect(saved.gender).toBe("female");
  } finally {
    await deleteResources(request, "Patient", [id!]);
  }
});

test("loading a saved resource shows it in the editor, and Save keeps it undoable", async ({ page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Loaded" }] });
  try {
    await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
    const ed = standalone(page);
    await expect(ed.cm).toContainText('"Loaded"');
    await ed.fillRaw({ resourceType: "Patient", id, name: [{ family: "LoadedEdited" }] });
    await page.locator("#editor-save").click();
    await expect(page.locator("#editor-announce")).toContainText(/saved/i);
    await expect(ed.cm).toContainText('"LoadedEdited"');
    // The canonical document replaced the text as one transaction: Ctrl+Z
    // goes back, not to a blank editor.
    await ed.cm.click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(ed.cm).toContainText('"Loaded');
  } finally {
    await deleteResources(request, "Patient", [id]);
  }
});

test("without the CodeMirror bundle the textarea is visible and stays in sync", async ({ page }) => {
  await page.route("**/codemirror.bundle.js", (route) => route.abort());
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await expect(ed.source).toBeVisible();
  await expect(ed.codeEditor).toHaveCount(0);

  await ed.source.fill(JSON.stringify({ resourceType: "Patient", gender: "female" }, null, 2));
  await expect(ed.root.locator('[data-set="gender"]')).toHaveValue("female", { timeout: 5000 });

  await ed.root.locator('[data-set="gender"]').fill("male");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await ed.source.inputValue()).includes('"male"')).toBe(true);
});

test("loading a history version replaces the document, and Ctrl+Z restores the text", async ({ page, request }) => {
  const id = await seedTwoVersions(
    request,
    "Patient",
    { name: [{ family: "VersionOne" }] },
    (first) => ({ ...first, name: [{ family: "VersionTwo" }] }),
  );
  try {
    await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
    const ed = standalone(page);
    await expect(ed.cm).toContainText('"VersionTwo"');

    // The list is newest first: the second entry is the first version.
    const rows = page.locator("#editor-versions-list .editor-version");
    await expect(rows).toHaveCount(2);
    await rows.nth(1).click();
    await expect(ed.cm).toContainText('"VersionOne"');
    await ed.formCaughtUp();
    await expect(ed.root.locator('[data-set="name.0.family"]')).toHaveValue("VersionOne");

    await ed.cm.click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(ed.cm).toContainText('"VersionTwo"');
  } finally {
    await deleteResources(request, "Patient", [id]);
  }
});

// ---- the Resources modal ----------------------------------------------

test.describe("in the Resources modal", () => {
  test("typing in the JSON updates the guided form", async ({ resources }) => {
    await resources.goto("Patient");
    await resources.openCreate();
    const ed = resources.modal.editor;
    await ed.fillRaw({ resourceType: "Patient", gender: "female" });
    await expect(ed.root.locator('[data-set="gender"]')).toHaveValue("female", { timeout: 5000 });
  });

  test("editing a form field updates the JSON without moving the caret", async ({ resources, page }) => {
    await resources.goto("Patient");
    await resources.openCreate();
    const ed = resources.modal.editor;
    await ed.applyJson({ resourceType: "Patient", gender: "male", name: [{ family: "ModalCaret" }] });

    const before = (await ed.jsonText()).indexOf('"gender"');
    await ed.setCursor(before);
    await ed.root.locator('[data-set="gender"]').fill("female");
    await page.keyboard.press("Tab");
    await expect.poll(async () => (await ed.jsonText()).includes('"female"')).toBe(true);
    expect(await caret(ed)).toBe(before);
  });

  test("there is no Edit raw button and Format works", async ({ resources, page }) => {
    await resources.goto("Patient");
    await resources.openCreate();
    const ed = resources.modal.editor;
    await expect(page.getByRole("button", { name: /edit raw/i })).toHaveCount(0);
    await expect(ed.formatButton).toBeVisible();
    await ed.setJson('{"resourceType":"Patient","gender":"male"}');
    await ed.cm.click();
    await page.keyboard.press("Shift+Alt+F");
    await expect
      .poll(() => ed.jsonText())
      .toBe(JSON.stringify({ resourceType: "Patient", gender: "male" }, null, 2));
  });

  test("reopening the modal mounts a fresh editor that handles each click once", async ({ resources, page }) => {
    await resources.goto("Patient");
    for (let round = 0; round < 2; round++) {
      await resources.openCreate();
      const ed = resources.modal.editor;
      await ed.applyJson({ resourceType: "Patient", name: [{ given: ["Again"] }] });
      await ed.collectionAdd("name.0.given").click();
      await expect(ed.rowAt("name.0.given.1")).toHaveCount(1);
      await expect(ed.rowAt("name.0.given.2")).toHaveCount(0);
      await resources.modal.close({ discard: true });
    }
    await expect(page.locator("#resource-editor-body .code-editor--resource")).toHaveCount(0);
  });

  test("closing and reopening the modal destroys the previous code editor", async ({ resources, page }) => {
    await resources.goto("Patient");
    await page.evaluate(() => {
      const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
      const w = window as unknown as { __destroyed: number };
      w.__destroyed = 0;
      const original = CM.EditorView.prototype.destroy;
      CM.EditorView.prototype.destroy = function (...args: unknown[]) {
        w.__destroyed++;
        return original.apply(this, args);
      };
    });
    for (let round = 0; round < 3; round++) {
      await resources.openCreate();
      await expect(resources.modal.editor.cm).toBeVisible();
      await resources.modal.close();
    }
    // One per close; a re-render in between would add more, never fewer.
    expect(await page.evaluate(() => (window as unknown as { __destroyed: number }).__destroyed)).toBeGreaterThanOrEqual(3);
  });

  test("a change typed into the JSON is what Save persists", async ({ resources, request }) => {
    const family = "Zm" + Date.now().toString(36);
    await resources.goto("Patient");
    await resources.openCreate();
    await resources.modal.editor.fillRaw({ resourceType: "Patient", name: [{ family }] });
    await resources.modal.save();
    await expect(resources.modal.announce).toContainText(/saved/i);
    const id = await resources.modal.savedId();
    expect(id).toBeTruthy();
    try {
      const saved = await request
        .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
        .then((r) => r.json());
      expect(saved.name?.[0]?.family).toBe(family);
    } finally {
      await deleteResources(request, "Patient", [id!]);
    }
  });
});
