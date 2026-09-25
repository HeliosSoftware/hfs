import { test, expect } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { createResource } from "../pages/api";

// The schema-driven editor's structural controls, exercised through the
// Resources modal (they are delegated in resources.js): fold/expand, add-node
// (+ filter), remove, the value[x] choice select, and the ad-hoc extension —
// plus the standalone /ui/editor page's own raw round-trip and fold.

test("collapse-all and expand-all fold the JSON view", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  // Give it nesting so there is something to fold.
  await ed.applyJson({
    resourceType: "Patient",
    name: [{ family: "Fold", given: ["A", "B"] }],
    address: [{ city: "Springfield" }],
  });

  await expect(ed.root.locator("#json-view")).toHaveCount(1);
  await expect(ed.root.locator('.json-line[data-jpath="name.0.family"]')).toHaveCount(1);

  await ed.collapseAll();
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
  await expect(
    ed.root.locator('.json-line--foldable[data-parents=""]'),
  ).not.toHaveClass(/json-line--collapsed/);
  await ed.expandAll();
  expect(await ed.hiddenLineCount()).toBe(0);
});

test("individual folds remain scoped and keyboard-accessible after a second server swap", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.applyJson({ resourceType: "Patient", name: [{ family: "First", given: ["A"] }] });

  const nested = ed.root.locator('.json-line--foldable:not([data-parents=""]) [data-fold]').first();
  await nested.focus();
  await page.keyboard.press("Space");
  await expect(nested).toHaveAttribute("aria-expanded", "false");
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
  await page.keyboard.press("Space");
  await expect(nested).toHaveAttribute("aria-expanded", "true");

  // This performs another real /ui/editor/render replacement. Delegation must
  // bind to the new fragment without an initializer or load-order hook.
  await ed.applyJson({ resourceType: "Patient", address: [{ city: "Second" }] });
  await expect(ed.root.locator("#json-view")).toHaveCount(1);
  const rootFold = ed.root.locator('.json-line--foldable[data-parents=""] [data-fold]');
  await rootFold.click();
  await expect(rootFold).toHaveAttribute("aria-expanded", "false");
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
});

test("add-node adds a top-level field to the document", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  expect(await ed.currentDoc()).not.toHaveProperty("gender");

  await ed.openAddPanel();
  await ed.addFilter().fill("gender");
  await ed.addItem("gender").click();

  await expect
    .poll(async () => Object.keys(await ed.currentDoc()))
    .toContain("gender");
});

test("removing a node drops it from the document", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  await ed.applyJson({ resourceType: "Patient", gender: "female" });
  expect(await ed.currentDoc()).toHaveProperty("gender");

  await ed.root.locator("[data-remove]").first().click();
  await expect.poll(async () => Object.keys(await ed.currentDoc())).not.toContain("gender");
});

test("the value[x] choice select adds the chosen variant", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.openCreate("Observation");
  const ed = resources.modal.editor;

  // The value[x] choice select lives inside the add-node <details>, which
  // auto-opens on an empty document (#547) -- open it only when closed.
  const panel = ed.root.locator(".editor-add", {
    has: page.locator("select[data-declarer='value']"),
  });
  if ((await panel.getAttribute("open")) === null) {
    await panel.locator("summary").click();
  }
  const choose = panel.locator("select[data-declarer='value']");
  const arms = await choose.locator("option").allInnerTexts();
  const arm = arms.find((a) => /string/i.test(a)) ?? arms[1];
  await choose.selectOption({ label: arm });

  await expect
    .poll(async () => Object.keys(await ed.currentDoc()).join(","))
    .toMatch(/value[A-Z]/);
});

test("an ad-hoc extension can be attached by URL", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await ed.openExtensions();
  await expect(ed.addGroup("extensions")).toHaveAttribute("open", "");
  const ext = ed.root.locator(".editor-add__ext").first();
  await ext.locator(".editor-add__ext-url").fill("http://example.org/fhir/StructureDefinition/e2e");
  // The ad-hoc button is the plain .btn; profiled-extension entries carry
  // data-extension too but render as .editor-add__item (#363).
  await ext.locator("button.btn[data-extension]").click();

  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("extension");
  // The dotted repetition index (`extension.0`) still names the created
  // node and its Undo correctly (#1239 follow-up), and the group the user
  // opened by hand — with an empty filter — survives the re-render.
  await expect(ed.addAdded()).toBeVisible();
  await expect(ed.addAdded()).toContainText("extension");
  await expect(ed.addAdded()).toContainText("added");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "extension.0");
  await expect(ed.addGroup("extensions")).toHaveAttribute("open", "");
});

test("adding a repeatable element again reports it and undo removes only that repetition", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addItem("name").click();

  await expect.poll(async () => ((await ed.currentDoc()).name as unknown[])?.length).toBe(1);
  await expect(ed.addAdded()).toContainText("name");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "name.0");

  // The same button is now "add another" — a second repetition.
  await ed.addItem("name").click();

  await expect.poll(async () => ((await ed.currentDoc()).name as unknown[])?.length).toBe(2);
  await expect(ed.addAdded()).toContainText("name");
  await expect(ed.addAdded()).toContainText("added");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "name.1");

  await ed.addUndo().click();

  await expect.poll(async () => ((await ed.currentDoc()).name as unknown[])?.length).toBe(1);
  await expect(ed.addPanel).toHaveAttribute("open", "");
});

// #1239: the picker itself — adding in a row with the filter and "added"
// signal, Undo, the three ways to close it, and Extensions folding.

test("adding an element clears the filter, keeps the picker open and reports what was added", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().fill("birth");
  await ed.addItem("birthDate").click();

  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("birthDate");
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await expect(ed.addFilter()).toHaveValue("");
  await expect(ed.addAdded()).toBeVisible();
  await expect(ed.addAdded()).toContainText("birthDate");
  // The filter happened to match Extensions too, but the user never opened
  // that group by hand — clearing the filter folds it back (#1239).
  await expect(ed.addGroup("extensions")).not.toHaveAttribute("open");

  // Nothing was cleared by hand — add straight from the still-open panel.
  await ed.addItem("gender").click();
  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("gender");
  await expect(ed.addAdded()).toContainText("gender");
});

test("undo removes the element just added and keeps the picker open", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().fill("birth");
  await ed.addItem("birthDate").click();
  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("birthDate");
  await expect(ed.addAdded()).toBeVisible();

  await ed.addUndo().click();

  await expect.poll(async () => Object.keys(await ed.currentDoc())).not.toContain("birthDate");
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await expect(ed.addAdded()).toBeHidden();
  // Undo itself is about to vanish with the re-render — focus lands on the
  // filter the user would continue from (#1239).
  await expect(ed.addFilter()).toBeFocused();
});

test("Escape closes the picker and leaves the Resources modal open", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().focus();
  await resources.page.keyboard.press("Escape");

  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(resources.modal.root).toBeVisible();
});

test("clicking outside the picker closes it", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");

  await resources.modal.subject.click();
  await expect(ed.addPanel).not.toHaveAttribute("open");

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
});

test("the close control closes the picker and returns focus to its toggle", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");

  await ed.addClose().click();

  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(ed.addPanel.locator("summary").first()).toBeFocused();
});

test("Extensions stay folded by default and unfold when the filter matches one", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  const extensions = ed.addGroup("extensions");
  await expect(extensions).not.toHaveAttribute("open");

  // The group renders regardless (it always carries the ad-hoc URL row) —
  // count the profiled-extension items themselves before reading one.
  const items = extensions.locator("[data-add-name]");
  test.skip((await items.count()) === 0, "no known extensions for Patient on this server");
  const firstExtension = await items.first().getAttribute("data-add-name");

  await ed.addFilter().fill(firstExtension!);
  await expect(extensions).toHaveAttribute("open", "");

  await ed.addFilter().fill("");
  await expect(extensions).not.toHaveAttribute("open");
  const elements = ed.addPanel.locator("details.editor-add__group").first();
  await expect(elements).toHaveAttribute("open", "");
});

test("the standalone editor page closes its picker with Escape and the close control", async ({
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "PickerStandalone" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().focus();
  await page.keyboard.press("Escape");
  await expect(ed.addPanel).not.toHaveAttribute("open");

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addClose().click();
  await expect(ed.addPanel).not.toHaveAttribute("open");

  // Same host, same signal (#1239).
  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().fill("birth");
  await ed.addItem("birthDate").click();
  await expect(ed.addAdded()).toBeVisible();
  await expect(ed.addAdded()).toContainText("birthDate");
});

test("the standalone editor page loads a resource and round-trips a raw edit", async ({
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Standalone" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  await expect(ed.doc).toHaveCount(1);
  expect((await ed.currentDoc()).id).toBe(id);

  // Fold controls work here too.
  await ed.collapseAll();
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);

  // Raw round-trip: change the family name and save.
  await ed.applyJson({ resourceType: "Patient", id, name: [{ family: "StandaloneEdited" }] });
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-status")).toContainText(/saved/i);

  const saved = await request
    .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
    .then((r) => r.json());
  expect(saved.name?.[0]?.family).toBe("StandaloneEdited");
});

test("a refused save lands its issue on the row the expression names", async ({
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Anchored" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  // The live pass is clean — the deferred half is exactly what it cannot see.
  await expect(ed.form).toHaveAttribute("data-error-count", "0");

  // Stand in for a server refusing on a constraint the editor defers. The
  // outcome spells the location as bracket-indexed FHIRPath while rows are
  // keyed on the validator's dotted form, and the two have to meet.
  await page.route(`**/Patient/${id}`, async (route) => {
    if (route.request().method() !== "PUT") return route.fallback();
    await route.fulfill({
      status: 422,
      contentType: "application/fhir+json",
      body: JSON.stringify({
        resourceType: "OperationOutcome",
        issue: [
          {
            severity: "error",
            code: "invariant",
            details: { text: "pat-1: refused on save" },
            expression: ["Patient.name[0].family"],
          },
        ],
      }),
    });
  });

  await page.locator("#editor-save").click();

  await expect(page.locator("#editor-status")).toContainText("refused on save");
  const row = ed.rowAt("name.0.family");
  await expect(row).toHaveClass(/editor-row--error/);
  await expect(row.locator(".editor-row__error")).toHaveText("pat-1: refused on save");
});
