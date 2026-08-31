import { test, expect } from "../pages/fixtures";

test("All Resources is the enhanced default", async ({ bulkExport }) => {
  await bulkExport.goto();

  await expect(bulkExport.allResources).toBeChecked();
  await expect(bulkExport.typeCheckboxes).not.toHaveCount(0);
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);
});

test("keyboard narrowing submits exactly the two selected resource types", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();

  await bulkExport.allResources.focus();
  await bulkExport.allResources.press("Space");
  await expect(bulkExport.allResources).not.toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every(
        (type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
      ),
    ),
  ).toBe(true);

  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.typeCheckbox("Observation").check();

  const expectedBody = await bulkExport.form.evaluate((form) => {
    const body = new URLSearchParams();
    for (const [name, value] of new FormData(form as HTMLFormElement).entries()) {
      body.append(name, String(value));
    }
    return body.toString();
  });
  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const request = await submitted;
  expect(request.postData()).toBe(expectedBody);
  const params = new URLSearchParams(request.postData() ?? "");
  expect(params.has("all_types")).toBe(false);
  expect(params.getAll("types").sort()).toEqual(["Observation", "Patient"]);
});

test("re-checking and Clear restore the All Resources state", async ({ bulkExport }) => {
  await bulkExport.goto();

  await bulkExport.allResources.uncheck();
  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.allResources.check();

  await expect(bulkExport.allResources).toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);

  await bulkExport.allResources.uncheck();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every(
        (type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
      ),
    ),
  ).toBe(true);

  await bulkExport.typeCheckbox("Observation").check();
  await bulkExport.form.locator('input[name="name"]').fill("temporary name");
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.clearButton.click();

  await expect(bulkExport.form.locator('input[name="name"]')).toHaveValue("");
  await expect(bulkExport.scopeRadio("system")).toBeChecked();
  await expect(bulkExport.allResources).toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);
});
