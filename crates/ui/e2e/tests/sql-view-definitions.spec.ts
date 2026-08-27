// View Definitions workspace (#649): a stored ViewDefinition lists in the
// rail, its JSON lands in the editor, Run previews rows through $sql-run, and
// Create New offers the starter document. Everything here is plain links and
// forms, so it also holds with JavaScript disabled (the nojs sweep loads the
// route; the flows are exercised in the chromium project only).
import { expect, test } from "../pages/fixtures";
import { createResource } from "../pages/api";

test("a stored ViewDefinition lists, edits, and previews rows", async ({ page, request }) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "ViewDefE2E" }],
  });
  const vdId = await createResource(request, "ViewDefinition", {
    name: "e2e_patients",
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  // The rail entry, selected; the editor holds the view's JSON.
  await expect(page.locator(`#vd-rail-list [data-type='${vdId}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator("textarea[name='json']")).toContainText("e2e_patients");

  const createNew = page.locator("a[href$='?vd=new']");
  await expect(createNew).toHaveClass(/\bbtn--primary\b/);
  await expect(createNew).not.toHaveClass(/\bbtn--accent\b/);
  await expect(createNew).toHaveCSS("height", "30px");
  await expect(createNew).toHaveCSS("padding-left", "12px");

  // Run previews the output; the seeded patient's key is among the rows.
  await page.locator("a[href*='run=1']").click();
  await expect(page.locator(".data-table")).toBeVisible();
  await expect(page.locator(".data-table td", { hasText: patientId }).first()).toBeVisible();

  // Create New swaps the editor to the starter document.
  await page.goto("/ui/sql/view-definitions?vd=new");
  await expect(page.locator("textarea[name='json']")).toContainText("new_view");
});
