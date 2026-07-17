import { test, expect } from "@playwright/test";

// Regression cover for the Resources workspace edit flows — each of these was a
// real bug fixed by hand; the browser is the only place they're observable.

async function openCreate(page: import("@playwright/test").Page, type: string) {
  await page.goto("/ui/resources?type=Patient", { waitUntil: "networkidle" });
  if (type !== "Patient") {
    await page.locator(`[data-rail-type="${type}"]`).click();
  }
  await page.locator("#resource-create").click();
  await expect(page.locator("#resource-modal")).toBeVisible();
  await expect(page.locator("#resource-editor-body #editor-doc")).toHaveCount(1);
}

async function rawEdit(page: import("@playwright/test").Page, doc: object) {
  await page.locator("#editor-json-edit").click();
  await page.locator("#editor-source").fill(JSON.stringify(doc, null, 2));
}

async function savedId(page: import("@playwright/test").Page): Promise<string | undefined> {
  const subject = (await page.locator("#resource-modal-subject").textContent()) ?? "";
  return subject.match(/Patient\/(\S+)/)?.[1];
}

test("Create new targets the resource type picked in the rail, not always Patient", async ({ page }) => {
  await openCreate(page, "Observation");
  await expect(page.locator("#resource-modal-subject")).toContainText("Observation");
  const doc = await page.locator("#resource-editor-body #editor-doc").inputValue();
  expect(JSON.parse(doc).resourceType).toBe("Observation");
  // The editor projects the picked type's schema: Observation requires status + code.
  await expect(page.locator(".editor-validity")).not.toHaveClass(/editor-validity--ok/);
});

test("an out-of-value-set code shows a red issue inline in the editor", async ({ page }) => {
  await openCreate(page, "Patient");
  await rawEdit(page, { resourceType: "Patient", gender: "masculino", name: [{ family: "E2EInline" }] });
  // Apply the raw edit (toggle back re-renders the guided form + validates).
  await page.locator("#editor-json-edit").click();
  const genderRow = page.locator(".editor-row", { hasText: "gender" });
  await expect(genderRow).toHaveClass(/editor-row--error/);
  await expect(genderRow.locator(".editor-row__error")).toContainText("administrative-gender");
});

test("Save is blocked, in red, when the resource fails validation", async ({ page }) => {
  await openCreate(page, "Patient");
  await rawEdit(page, { resourceType: "Patient", gender: "masculino", name: [{ family: "E2EBlocked" }] });
  await page.locator("#resource-save").click();

  const status = page.locator("#resource-modal-status");
  await expect(status).toHaveClass(/modal__status--error/);
  await expect(status).toContainText(/validation/i);
  // Nothing was persisted: the subject never took on an id (stays "· new").
  await expect(page.locator("#resource-modal-subject")).toContainText("new");
  expect(await savedId(page)).toBeUndefined();
});

test("raw-editing the JSON and saving persists exactly what you typed", async ({ page }) => {
  await openCreate(page, "Patient");
  await rawEdit(page, { resourceType: "Patient", gender: "female", name: [{ family: "E2ERawSave" }] });
  await page.locator("#resource-save").click();

  await expect(page.locator("#resource-modal-status")).toContainText(/saved/i);
  const id = await savedId(page);
  expect(id).toBeTruthy();
  // Read it straight back from the FHIR API — the typed edit round-tripped.
  const saved = await page.evaluate(
    (i) => fetch(`/Patient/${i}`, { headers: { Accept: "application/fhir+json" } }).then((r) => r.json()),
    id,
  );
  expect(saved.name?.[0]?.family).toBe("E2ERawSave");
  expect(saved.gender).toBe("female");
});
