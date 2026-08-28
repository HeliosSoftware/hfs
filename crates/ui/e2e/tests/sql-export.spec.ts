// SQL Export + Files (#649): start a $sql-export over a stored
// ViewDefinition from the form, follow the job to Finished, and land on the
// Files page with the manifest's download links.
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

test("an export runs through to downloadable manifest files", async ({ page, request }) => {
  const patientId = await createResource(request, "Patient", { name: [{ family: "ExportE2E" }] });
  const vdId = await createResource(request, "ViewDefinition", {
    name: "e2e_export_patients",
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });

  // ES composites index asynchronously: the form lists subjects and the
  // job reads its rows through search (#596).
  await waitSearchable(request, "ViewDefinition", vdId);
  await waitSearchable(request, "Patient", patientId);

  await page.goto("/ui/sql/export");
  await page.locator(`input[value='ViewDefinition/${vdId}']`).check();
  await page.locator("form[action='/ui/sql/export'] button[type='submit']").click();
  await expect(page).toHaveURL(/job=/);

  // The page is stateless — reload until the status poll answers Finished.
  await expect(async () => {
    if (!(await page.locator("a[href^='/ui/sql/files?job=']").isVisible())) {
      await page.reload();
      throw new Error("still running");
    }
  }).toPass({ timeout: 20000 });

  await page.locator("a[href^='/ui/sql/files?job=']").click();
  // One output named after the subject's id, with a download link.
  await expect(page.locator(".data-table td", { hasText: vdId }).first()).toBeVisible();
  await expect(page.locator(".data-table a").first()).toBeVisible();
});
