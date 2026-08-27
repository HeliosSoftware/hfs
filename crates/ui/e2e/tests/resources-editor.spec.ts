import { test, expect } from "../pages/fixtures";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometry,
} from "../pages/button-geometry";

// Regression cover for the Resources workspace edit flows — each of these was a
// real bug fixed by hand; the browser is the only place they're observable.

test("Create new targets the resource type picked in the rail, not always Patient", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.openCreate("Observation");

  await expect(resources.modal.subject).toContainText("Observation");
  expect((await resources.modal.editor.currentDoc()).resourceType).toBe("Observation");
  // The editor projects the picked type's schema: Observation requires status + code.
  await expect(resources.modal.editor.form).toHaveAttribute("data-error-count", /[1-9]/);

  const actionMetrics = await Promise.all(
    [resources.modal.deleteButton, resources.modal.saveButton].map((button) =>
      readButtonGeometry(button),
    ),
  );
  expect(actionMetrics[0]).toEqual(actionMetrics[1]);
  expect(actionMetrics[0]).toEqual(CANONICAL_BUTTON_GEOMETRY);
});

test("an encoded HTML-like type stays text and cannot open a draft", async ({ resources, page }) => {
  const value = "/<img src=x onerror=alert(1)>";
  await page.goto("/ui/resources?url=" + encodeURIComponent(value), {
    waitUntil: "networkidle",
  });

  await expect(resources.builder.url).toHaveValue("GET " + value);
  await expect(resources.createButton).toBeDisabled();
  await expect(page.locator("img[src='x']")).toHaveCount(0);
  await expect(resources.modal.root).toBeHidden();
});

test("an out-of-value-set code shows a red issue inline in the editor", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.editor.applyJson({
    resourceType: "Patient",
    gender: "masculino",
    name: [{ family: "E2EInline" }],
  });

  await expect(resources.modal.editor.root.locator("#json-view")).toHaveCount(1);
  await expect(
    resources.modal.editor.root.locator('.json-line[data-jpath="name.0.family"]'),
  ).toHaveCount(1);

  const genderRow = resources.modal.editor.row("gender");
  await expect(genderRow).toHaveClass(/editor-row--error/);
  await expect(genderRow.locator(".editor-row__error")).toContainText("administrative-gender");
  await genderRow.hover();
  await expect(
    resources.modal.editor.root.locator('.json-line--hit[data-jpath="gender"]'),
  ).toHaveCount(1);
  await resources.modal.editor.root.locator('.json-line[data-jpath="gender"] .json-line__code').click();
  await expect(resources.modal.editor.root.locator('[data-set="gender"]')).toBeFocused();
});

test("Save is blocked, in red, when the resource fails validation", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.editor.fillRaw({
    resourceType: "Patient",
    gender: "masculino",
    name: [{ family: "E2EBlocked" }],
  });
  await resources.modal.save();

  await expect(resources.modal.status).toHaveClass(/modal__status--error/);
  await expect(resources.modal.status).toContainText(/validation/i);
  // Nothing was persisted: the subject never took on an id (stays "· new").
  await expect(resources.modal.subject).toContainText("new");
});

test("raw-editing the JSON and saving persists exactly what you typed", async ({
  resources,
  request,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.editor.fillRaw({
    resourceType: "Patient",
    gender: "female",
    name: [{ family: "E2ERawSave" }],
  });
  await resources.modal.save();

  await expect(resources.modal.status).toContainText(/saved/i);
  const id = await resources.modal.savedId();
  expect(id).toBeTruthy();

  // Read it straight back from the FHIR API — the typed edit round-tripped.
  const saved = await request
    .get(`/Patient/${id}`, { headers: { Accept: "application/fhir+json" } })
    .then((r) => r.json());
  expect(saved.name?.[0]?.family).toBe("E2ERawSave");
  expect(saved.gender).toBe("female");
});
