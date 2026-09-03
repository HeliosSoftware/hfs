import { expect, test } from "../../pages/fixtures";
import { createResource, waitSearchable } from "../../pages/api";

// #752: with JavaScript disabled the ViewDefinitions playground has no live
// preview at all — htmx, vd-editor.js, and the CodeMirror bundle are all
// inert here — only Save's own `?saved=1` redirect (a plain form POST
// followed by a plain GET) fills the results region, running the
// just-stored definition through $sql-run server-side.
test("with JavaScript disabled, the results region only fills after Save", async ({
  page,
  request,
}) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "VdNojsE2E" }],
  });
  const vdId = await createResource(request, "ViewDefinition", {
    name: "e2e_nojs_playground",
    status: "active",
    resource: "Patient",
    // Scoped to this spec's own patient so the results row is deterministic
    // however populated the backing store is (#596).
    where: [{ path: "name.family = 'VdNojsE2E'" }],
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "ViewDefinition", vdId);
  await waitSearchable(request, "Patient", patientId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  // The editor is a plain, visible textarea — no CodeMirror bundle mounted,
  // no Run button (removed), and nothing has run yet server-side.
  const editor = page.locator("textarea[name='json']");
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("e2e_nojs_playground");
  await expect(page.locator("a[href*='run=1']")).toHaveCount(0);
  await expect(page.locator(".data-table")).toHaveCount(0);

  // Save is a plain form POST; its redirect (`?vd=<id>&saved=1`) is what
  // finally runs the definition and renders its results.
  await page.locator("#vd-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`vd=${vdId}&saved=1`));
  await expect(page.locator(".notice", { hasText: "Saved." })).toBeVisible();
  await expect(page.locator(".data-table")).toBeVisible();
  await expect(page.locator(".data-table td", { hasText: patientId }).first()).toBeVisible();
});
