import { test, expect, acceptConfirm, dismissConfirm, dialogsSeen } from "../pages/fixtures";
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

test("individual folds work once after boosted navigation and a second server swap", async ({ page, compartments }) => {
  await compartments.goto();
  await page.evaluate(() => { (document as any).hfs1771Original = true; });
  // 18 body-script executions on the baseline: an even number must not
  // conceal duplicate toggles by coincidentally landing in the right state.
  for (let round = 0; round < 3; round++) {
    await compartments.selectDefinition("Encounter");
    for (const tab of [/members/i, /test/i, /definition/i]) await compartments.openTab(tab);
    await compartments.selectDefinition("Patient");
  }
  await compartments.selectDefinition("Device");
  await page.locator(".detail__actions a.btn").click();
  await page.waitForURL("**/ui/editor?type=CompartmentDefinition&id=*");
  await page.waitForLoadState("networkidle");
  expect(await page.evaluate(() => (document as any).hfs1771Original)).toBe(true);
  const ed = new Editor(page, page.locator("#editor-body"));
  const initialRoot = ed.root.locator('.json-line--foldable[data-parents=""] [data-fold]');
  await initialRoot.click();
  await expect(initialRoot).toHaveAttribute("aria-expanded", "false");
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
  await initialRoot.click();
  await expect(initialRoot).toHaveAttribute("aria-expanded", "true");
  // Raw projection is never saved; leave the shared stored definition intact.
  await ed.applyJson({ resourceType: "CompartmentDefinition", status: "draft", code: "Device",
    resource: [{ code: "Patient", param: ["id"] }] });

  const nested = ed.root.locator('.json-line--foldable:not([data-parents=""]) [data-fold]').first();
  await nested.focus();
  await page.keyboard.press("Space");
  await expect(nested).toHaveAttribute("aria-expanded", "false");
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
  await page.keyboard.press("Space");
  await expect(nested).toHaveAttribute("aria-expanded", "true");

  // This performs another real /ui/editor/render replacement. Delegation must
  // bind to the new fragment without an initializer or load-order hook.
  await ed.applyJson({ resourceType: "CompartmentDefinition", status: "draft", code: "Device",
    resource: [{ code: "Observation", param: ["subject"] }] });
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
  // The selected choice disappears from the new picker, so the original
  // locator's `has` filter no longer matches it after the swap.
  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(ed.rowAt("valueString").locator("[data-set]")).toBeFocused();
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
  await expect(ed.form).toHaveAttribute("data-focus", "extension.0");
  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(ed.rowAt("extension.0")).toBeFocused();
  await expect(ed.addStatus).toContainText("extension added");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "extension.0");
  await expect(ed.addUndo()).toBeVisible();
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
  await expect(ed.addStatus).toContainText("name added");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "name.0");
  await expect(ed.addPanel).not.toHaveAttribute("open");

  // The named array owns its own append action.
  await ed.collectionAdd("name").click();

  await expect.poll(async () => ((await ed.currentDoc()).name as unknown[])?.length).toBe(2);
  await expect(ed.addStatus).toContainText("name added");
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "name.1");

  await ed.addUndo().click();

  await expect.poll(async () => ((await ed.currentDoc()).name as unknown[])?.length).toBe(1);
  await expect(ed.collectionAdd("name")).toBeFocused();
});

// #1721 supersedes the keep-open behavior from #1239. Filtering and the
// independent close controls still work after the owning picker closes.

test("adding an element closes its picker, clears the filter and announces it without a success block", async ({
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
  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(ed.addFilter()).toHaveValue("");
  await expect(ed.addStatus).toContainText("birthDate added");
  await expect(ed.root.locator(".editor-add__added")).toHaveCount(0);
  // The filter happened to match Extensions too, but the user never opened
  // that group by hand — clearing the filter folds it back (#1239).
  await expect(ed.addGroup("extensions")).not.toHaveAttribute("open");

  await ed.openAddPanel();
  await ed.addItem("gender").click();
  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("gender");
  await expect(ed.addStatus).toContainText("gender added");
});

test("undo removes the element just added and focuses the parent picker toggle", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;

  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().fill("birth");
  await ed.addItem("birthDate").click();
  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("birthDate");
  await expect(ed.addUndo()).toBeVisible();

  await ed.addUndo().click();

  await expect.poll(async () => Object.keys(await ed.currentDoc())).not.toContain("birthDate");
  // Removing the only field restores the server's empty-document picker.
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await expect(ed.addUndo()).toHaveCount(0);
  await expect(ed.addPanel.locator("summary").first()).toBeFocused();
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

  // Same host, same hidden announcement and discreet Undo.
  await ed.openAddPanel();
  await expect(ed.addPanel).toHaveAttribute("open", "");
  await ed.addFilter().fill("birth");
  await ed.addItem("birthDate").click();
  await expect(ed.addPanel).not.toHaveAttribute("open");
  await expect(ed.addStatus).toContainText("birthDate added");
  await expect(ed.addUndo()).toBeVisible();
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
  // #1649: no visible "Saved." — the confirmation is the pill going away, and
  // the words go to the visually hidden live region only.
  await expect(page.locator("#editor-announce")).toContainText(/saved/i);
  await expect(page.locator("#editor-status")).toBeEmpty();

  const saved = await request
    .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
    .then((r) => r.json());
  expect(saved.name?.[0]?.family).toBe("StandaloneEdited");
});

// #1667: a resource first saved from the standalone page — a PUT under an id
// the document already carries, or a POST the server assigns one — shows its
// new version in the Versions card straight away, without a reload.
test("saving a new resource with an id fills the Versions card", async ({ page }) => {
  const id = `e2e-versions-${Date.now()}`;
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.applyJson({ resourceType: "Patient", id, name: [{ family: "Versioned" }] });
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-announce")).toContainText(/saved/i);

  const versions = page.locator("#editor-versions-list");
  await expect(versions.locator(".editor-version--current")).toHaveCount(1);
  await expect(versions.locator(".editor-version")).toHaveCount(1);
  await expect(page.locator("#editor-subject")).toContainText(`Patient/${id}`);
});

test("saving a new resource without an id adopts the server's id", async ({
  page,
  request,
}) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.applyJson({ resourceType: "Patient", name: [{ family: "Assigned" }] });
  await page.locator("#editor-save").click();

  const versions = page.locator("#editor-versions-list");
  await expect(versions.locator(".editor-version--current")).toHaveCount(1);
  await expect.poll(async () => (await ed.currentDoc()).id).toBeTruthy();
  const id = (await ed.currentDoc()).id as string;
  await expect(page.locator("#editor-subject")).toContainText(`Patient/${id}`);

  // A second save updates that resource rather than creating another one.
  await ed.applyJson({ ...(await ed.currentDoc()), name: [{ family: "AssignedAgain" }] });
  await page.locator("#editor-save").click();
  await expect(versions.locator(".editor-version")).toHaveCount(2);
  const saved = await request
    .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
    .then((r) => r.json());
  expect(saved.name?.[0]?.family).toBe("AssignedAgain");
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


test("issue1772 successful dirty Patient deletion returns to Resources without an unload prompt", async ({ page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Issue1772Delete" }] });
  const path = `/Patient/${id}`;
  try {
    await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
    const editor = new Editor(page, page.locator("#editor-body"));
    await editor.applyJson({ ...(await editor.currentDoc()), gender: "female" });
    await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
    dialogsSeen(page);
    await page.locator("#editor-delete").click();
    await acceptConfirm(page);
    await page.waitForURL(url => url.pathname === "/ui/resources");
    expect([404, 410]).toContain((await request.get(path)).status());
    expect(dialogsSeen(page).filter(dialog => dialog.type === "beforeunload")).toEqual([]);
  } finally {
    await request.delete(path);
  }
});

for (const outcome of ["cancelled", "rejected"] as const) {
  test(`issue1772 ${outcome} editor deletion stays put and retains dirty tracking`, async ({ page, request }) => {
    const id = await createResource(request, "Patient", { name: [{ family: "Issue1772Keep" }] });
    const path = `/Patient/${id}`;
    let deletes = 0;
    const route = (url: URL) => url.pathname === path;
    try {
      await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
      const editor = new Editor(page, page.locator("#editor-body"));
      await editor.applyJson({ ...(await editor.currentDoc()), gender: "female" });
      await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
      const before = page.url();
      await page.route(route, async intercepted => {
        if (intercepted.request().method() !== "DELETE") return intercepted.continue();
        deletes++;
        await intercepted.fulfill({ status: 403, contentType: "application/fhir+json", body: JSON.stringify({
          resourceType: "OperationOutcome", issue: [{ severity: "error", code: "forbidden" }],
        }) });
      });
      await page.locator("#editor-delete").click();
      if (outcome === "cancelled") {
        await dismissConfirm(page);
        expect(deletes).toBe(0);
      } else {
        const rejected = page.waitForResponse(response => new URL(response.url()).pathname === path && response.request().method() === "DELETE");
        await acceptConfirm(page);
        expect((await rejected).status()).toBe(403);
        expect(deletes).toBe(1);
      }
      expect(page.url()).toBe(before);
      await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
      expect((await request.get(path)).ok()).toBe(true);
      dialogsSeen(page);
      await page.goto("/ui/resources", { waitUntil: "networkidle" });
      expect(dialogsSeen(page).some(dialog => dialog.type === "beforeunload")).toBe(true);
    } finally {
      await page.unroute(route);
      await request.delete(path);
    }
  });
}
