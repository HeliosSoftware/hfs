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
  // Scoped to `#run-results` (#842): the Tables card's own *Reads from*
  // table shares the repo-wide `.data-table` style too, and this Library
  // already declares one dependency.
  await expect(page.locator("#run-results .data-table")).toHaveCount(0);

  const details = JSON.parse(await jsonField.inputValue());
  details.name = "e2e_lib_nojs_renamed";
  await jsonField.fill(JSON.stringify(details, null, 2));
  await sqlField.fill("SELECT id AS pid FROM v");

  await page.locator("#lib-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`lib=${libId}&saved=1`));
  await expect(page.locator(".notice", { hasText: "Saved." })).toBeVisible();
  await expect(page.locator("#run-results .data-table th")).toHaveText(["pid"]);
  await expect(page.locator("#run-results .data-table td", { hasText: patientId }).first()).toBeVisible();

  const saved = await readResource(request, "Library", libId);
  expect(saved.name).toBe("e2e_lib_nojs_renamed");
  const content = saved.content as Array<{ contentType: string; data: string }>;
  const sqlAttachment = content.find((a) => a.contentType === "application/sql");
  expect(sqlAttachment).toBeTruthy();
  expect(Buffer.from(sqlAttachment!.data, "base64").toString()).toBe("SELECT id AS pid FROM v");
});

// Parameters card (#841) with JavaScript disabled: `<details>`/`<summary>`
// is a native disclosure (no JS needed to open it), and the `Add parameter`
// button is a real `type="submit"` with a `formaction` — the whole page
// re-renders around the updated (unsaved) Details JSON. Save then persists
// it like any other field, and the very next render — the `?…&saved=1`
// redirect, with no value ever submitted for the newly required parameter
// — shows the "waiting" notice instead of a table.
test("with JavaScript disabled, Add parameter updates the page, and the saved page waits for the new value", async ({
  page,
  request,
}) => {
  const canonical = `http://example.org/ViewDefinition/e2e-params-nojs-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_params_nojs_source",
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  const libId = await createSqlQueryLibrary(
    request,
    `e2e_params_nojs_${Date.now()}`,
    canonical,
    "SELECT id FROM v WHERE ward = :ward",
  );
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  await expect(page.locator("input[name='param:ward']")).toHaveCount(0);
  await expect(page.locator("#lib-params")).toContainText(":ward is used in the SQL but not declared");

  // The `<details>` disclosure opens natively — no JavaScript required.
  await page.locator("#lib-params details.editor-add > summary").click();
  await page.locator("input[name='param_name']").fill("ward");
  await page.locator("button[name='op'][value='add-parameter']").click();

  // The whole page re-rendered with the updated (still unsaved) document.
  const jsonField = page.locator("textarea[name='json']");
  await expect(jsonField).toHaveValue(/"name": "ward"/);
  const wardField = page.locator("input[name='param:ward']");
  await expect(wardField).toBeVisible();
  await expect(wardField).toHaveAttribute("form", "lib-editor-form");
  expect((await readResource(request, "Library", libId)).parameter).toBeUndefined();

  // Save, leaving the new field empty — nothing enforces it natively
  // (#841: these fields never carry the HTML5 `required` attribute, so a
  // plain Save always goes through regardless of the Parameters card).
  await page.locator("#lib-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`lib=${libId}&saved=1`));

  const saved = await readResource(request, "Library", libId);
  expect(saved.parameter).toMatchObject([{ name: "ward", use: "in", type: "string" }]);

  // The saved page's own server-side run has no value for the now-required
  // `:ward` — it waits rather than showing a stale or empty table.
  await expect(page.locator("#run-notice")).toContainText("Waiting for a value for :ward");
  const wardFieldAfterSave = page.locator("input[name='param:ward']");
  await expect(wardFieldAfterSave).toBeVisible();
  await expect(wardFieldAfterSave).toHaveAttribute("form", "lib-editor-form");
});

// Tables panel (#842) with JavaScript disabled: the combobox's own
// `data-combobox-fallback` textarea is the usable control — a
// `ViewDefinition/{id}` reference typed by hand, per its own hint — and
// *Add table* is a real `type="submit"` with a `formaction`, so the whole
// page re-renders around the updated (unsaved) Details JSON exactly like
// *Add parameter* does. Save then persists it.
test("with JavaScript disabled, Add table with a typed ViewDefinition reference updates the page, and Save persists it", async ({
  page,
  request,
}) => {
  const canonical = `http://example.org/ViewDefinition/e2e-tables-nojs-${Date.now()}`;
  const vdId = await createResource(request, "ViewDefinition", {
    name: "e2e_tables_nojs_source",
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  const libId = await createResource(request, "Library", {
    name: `e2e_tables_nojs_${Date.now()}`,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-view",
        },
      ],
    },
    content: [{ contentType: "application/sql", data: Buffer.from("SELECT 1").toString("base64") }],
  });
  await waitSearchable(request, "ViewDefinition", vdId);
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/views?lib=${libId}`);
  const tablesCard = page.locator("#lib-tables");
  await expect(tablesCard).toContainText("No tables declared yet.");

  // The disclosure opens natively — no JavaScript required.
  await tablesCard.locator("details.editor-add > summary").click();
  await page.locator('#lib-tables-add-table textarea[name="table"]').fill(`ViewDefinition/${vdId}`);
  await page.locator("input[name='table_alias']").fill("patients");
  await page.locator("button[name='op'][value='add-table']").click();

  // The whole page re-rendered with the updated (still unsaved) document.
  const jsonField = page.locator("textarea[name='json']");
  await expect(jsonField).toHaveValue(new RegExp(`"resource": "${canonical.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`));
  await expect(page.locator("#lib-tables tr", { hasText: "patients" })).toBeVisible();
  expect((await readResource(request, "Library", libId)).relatedArtifact).toBeUndefined();

  await page.locator("#lib-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`lib=${libId}&saved=1`));

  const saved = await readResource(request, "Library", libId);
  expect(saved.relatedArtifact).toMatchObject([
    { type: "depends-on", label: "patients", resource: canonical },
  ]);
});

// Unknown-table lint (#842/04) with JavaScript disabled: Save never rejects
// a SQL that reads an undeclared table (only $sql-run itself would, later),
// so `?…&saved=1`'s own server-side render is what shows the lint — the
// same notice and red row a live `/run` fragment would, plus the one
// affordance a no-JS visitor has no other way to reach: *Add table* opens
// itself, alias already the unknown table's own name.
test("with JavaScript disabled, saving a SQL that reads an unknown table shows the lint, the red row, and pre-fills Add table", async ({
  page,
  request,
}) => {
  const canonical = `http://example.org/ViewDefinition/e2e-lib-nojs-unknown-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_lib_nojs_unknown_source",
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  const libId = await createSqlQueryLibrary(
    request,
    `e2e_lib_nojs_unknown_${Date.now()}`,
    canonical,
    "SELECT id FROM v",
  );
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  await page.locator("textarea[name='sql']").fill("SELECT id FROM vv");
  await page.locator("#lib-editor-form button[name='action'][value='save']").click();
  await expect(page).toHaveURL(new RegExp(`lib=${libId}&saved=1`));

  await expect(page.locator(".notice--warn")).toContainText("Unknown table vv");
  const tablesCard = page.locator("#lib-tables");
  const row = tablesCard.locator("tr", { hasText: "vv" });
  await expect(row.locator(".tag--failed")).toHaveText("Unknown table");
  await expect(tablesCard.locator("details.editor-add")).toHaveAttribute("open", "");
  await expect(page.locator("input[name='table_alias']")).toHaveValue("vv");

  // Still never saved — the lint is a live/render-time notice only.
  const untouched = await readResource(request, "Library", libId);
  expect(untouched.relatedArtifact).toMatchObject([{ label: "v" }]);
});
