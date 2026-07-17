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

  await ed.collapseAll();
  expect(await ed.hiddenLineCount()).toBeGreaterThan(0);
  await ed.expandAll();
  expect(await ed.hiddenLineCount()).toBe(0);
});

test("add-node adds a top-level field to the document", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  expect(await ed.currentDoc()).not.toHaveProperty("gender");

  await ed.addPanel.locator("summary").first().click();
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

  // The value[x] choice select lives inside a collapsed add-node <details>;
  // open it first, then pick a concrete arm.
  const panel = ed.root.locator(".editor-add", {
    has: page.locator("select[data-declarer='value']"),
  });
  await panel.locator("summary").click();
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

  await ed.addPanel.locator("summary").first().click();
  const ext = ed.root.locator(".editor-add__ext").first();
  await ext.locator(".editor-add__ext-url").fill("http://example.org/fhir/StructureDefinition/e2e");
  await ext.locator("[data-extension]").click();

  await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain("extension");
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
