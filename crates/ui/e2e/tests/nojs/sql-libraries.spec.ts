import { expect, test } from "../../pages/fixtures";
import { createResource, createSqlQueryLibrary, readResource, waitSearchable } from "../../pages/api";

// Details (#840) with JavaScript disabled: the guided-form card never
// appears — CodeMirror, editor-pair.js, and the whole guided-form loop are
// all inert here — but both the Details JSON textarea and the SQL textarea
// are plain, visible fields that post together (the JSON one via its
// `form="lib-editor-form"` attribute, HTML5 form-associated even though it
// lives outside that `<form>` in the DOM). Save merges them server-side
// exactly as it does with JavaScript, and `?saved=1` still runs the just-
// stored Library through $sql-run.
test("with JavaScript disabled, editing both textareas and saving persists the merged Library and shows results", async ({
  page,
  request,
}) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "LibNojsE2E" }],
  });
  const canonical = `http://example.org/ViewDefinition/e2e-lib-nojs-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_lib_nojs_source",
    url: canonical,
    status: "active",
    resource: "Patient",
    where: [{ path: "name.family = 'LibNojsE2E'" }],
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "Patient", patientId);
  const libId = await createSqlQueryLibrary(
    request,
    `e2e_lib_nojs_${Date.now()}`,
    canonical,
    "SELECT id FROM v",
  );
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);

  // The guided-form card stays out of the accessibility tree and off-screen
  // (`needs-js`, never revealed without `theme.js`'s own `<html class="js">`
  // marker) — the grid collapses to the JSON card alone.
  await expect(page.locator("html")).not.toHaveClass(/\bjs\b/);
  await expect(page.locator("section.editor-form")).toBeHidden();

  const jsonField = page.locator("textarea[name='json']");
  const sqlField = page.locator("textarea[name='sql']");
  await expect(jsonField).toBeVisible();
  await expect(sqlField).toBeVisible();
  await expect(page.locator("a[href*='run=1']")).toHaveCount(0);
  await expect(page.locator(".data-table")).toHaveCount(0);

  const details = JSON.parse(await jsonField.inputValue());
  details.name = "e2e_lib_nojs_renamed";
  await jsonField.fill(JSON.stringify(details, null, 2));
  await sqlField.fill("SELECT id AS pid FROM v");

  await page.locator("#lib-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`lib=${libId}&saved=1`));
  await expect(page.locator(".notice", { hasText: "Saved." })).toBeVisible();
  await expect(page.locator(".data-table th")).toHaveText(["pid"]);
  await expect(page.locator(".data-table td", { hasText: patientId }).first()).toBeVisible();

  const saved = await readResource(request, "Library", libId);
  expect(saved.name).toBe("e2e_lib_nojs_renamed");
  const content = saved.content as Array<{ contentType: string; data: string }>;
  const sqlAttachment = content.find((a) => a.contentType === "application/sql");
  expect(sqlAttachment).toBeTruthy();
  expect(Buffer.from(sqlAttachment!.data, "base64").toString()).toBe("SELECT id AS pid FROM v");
});
