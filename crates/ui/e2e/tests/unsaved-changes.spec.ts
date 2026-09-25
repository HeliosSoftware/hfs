import { test, expect, armDialog, dialogsSeen } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { createResource, createSqlQueryLibrary, waitSearchable } from "../pages/api";
import { VdEditor } from "../pages/vd-editor";

// Unsaved-changes tracking (#1240) in the standalone /ui/editor page and the
// Resources modal: the "Unsaved changes" pill next to Save, the browser's own
// beforeunload confirmation on a real navigation, and the modal's own confirm
// on the closes that never navigate at all (the X, the backdrop, Escape).

test("the standalone editor shows the cue only while the document differs from the loaded one", async ({
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "UnsavedCue" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  const cue = page.locator("#editor .tag--unsaved");
  const original = await ed.currentDoc();
  await expect(cue).toBeHidden();

  await ed.applyJson({ ...original, gender: "male" });
  await expect(cue).toBeVisible();

  // Back to the document as loaded — a canonical comparison, not a
  // byte-for-byte one: applyJson reformats it on the way in.
  await ed.applyJson(original);
  await expect(cue).toBeHidden();
});

test("a whitespace-only raw edit does not mark the editor dirty", async ({ page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "WhitespaceOnly" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  const cue = page.locator("#editor .tag--unsaved");
  await expect(cue).toBeHidden();

  await ed.enterRaw();
  const text = await ed.source.inputValue();
  await ed.source.fill(text + "\n\n   ");
  await ed.leaveRaw();

  await expect(cue).toBeHidden();
});

test("saving clears the cue and leaving afterwards asks nothing", async ({
  page,
  request,
  chrome,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "SaveClears" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  const cue = page.locator("#editor .tag--unsaved");
  const original = await ed.currentDoc();
  await ed.applyJson({ ...original, gender: "female" });
  await expect(cue).toBeVisible();

  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-status")).toContainText(/saved/i);
  await expect(cue).toBeHidden();

  dialogsSeen(page); // discard anything unrelated recorded so far.
  await chrome.navLink("/ui/resources").click();
  await page.waitForURL("**/ui/resources");
  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
});

test("leaving the editor with unsaved changes asks the browser confirmation", async ({
  page,
  request,
  chrome,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "AskOnLeave" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const ed = new Editor(page, page.locator("#editor-body"));
  const cue = page.locator("#editor .tag--unsaved");
  const original = await ed.currentDoc();
  await ed.applyJson({ ...original, gender: "other" });
  await expect(cue).toBeVisible();

  armDialog(page, "dismiss");
  await chrome.navLink("/ui/resources").click();
  await expect.poll(() => dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(true);

  // Dismissed: the navigation never happened.
  await expect(page).toHaveURL(/\/ui\/editor/);
  await expect(cue).toBeVisible();
});

test("the Resources modal asks before closing with unsaved changes and keeps them on cancel", async ({
  resources,
  page,
}) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  await ed.applyJson({ resourceType: "Patient", name: [{ family: "ModalDirty" }] });
  await expect(resources.modal.unsavedCue).toBeVisible();

  armDialog(page, "dismiss");
  await page.locator(".modal__x").click();
  await expect(resources.modal.root).toBeVisible();
  expect(dialogsSeen(page)).toContainEqual({
    type: "confirm",
    message: "You have unsaved changes. Discard them and close?",
  });

  // Accepting (the page object's own close()) does discard it.
  await resources.modal.close();
  await expect(resources.modal.root).toBeHidden();
});

test("typing in a form field and closing the modal asks before discarding", async ({
  resources,
  page,
  request,
  chrome,
}) => {
  // A guided-form [data-set] control only round-trips through blur — this
  // covers the value while it is only on screen, before that lands (#1240).
  const id = await createResource(request, "Patient", { name: [{ family: "TypedInModal" }] });
  await waitSearchable(request, "Patient", id);
  await resources.goto("Patient");
  await page
    .locator(
      `#query-results-body a.result-id[data-resource-type='Patient'][data-resource-id='${id}']`,
    )
    .click();
  await resources.modal.waitOpen();
  await expect(resources.modal.unsavedCue).toBeHidden();

  await page.fill('[data-set="name.0.family"]', "TypedInModalEdited");
  await expect(resources.modal.unsavedCue).toBeVisible();

  armDialog(page, "dismiss");
  await page.locator(".modal__x").click();
  await expect(resources.modal.root).toBeVisible();
  expect(dialogsSeen(page)).toContainEqual({
    type: "confirm",
    message: "You have unsaved changes. Discard them and close?",
  });

  // Accepting discards it, and a hidden modal stays clean afterwards.
  await resources.modal.close();
  await expect(resources.modal.root).toBeHidden();

  dialogsSeen(page);
  await chrome.navLink("/ui/tenants").click();
  await page.waitForURL("**/ui/tenants");
  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
});

test("accepting the discard on a fast × click leaves the closed modal clean", async ({
  resources,
  page,
  request,
  chrome,
}) => {
  // A fast click (mousedown and click in the same frame — a tap, or
  // Playwright's own click) can land the blur/change round trip's
  // rAF-coalesced check() *after* closeModal() already ran (#1240): that
  // late check must still see the closed modal as clean.
  const id = await createResource(request, "Patient", { name: [{ family: "FastClose" }] });
  await waitSearchable(request, "Patient", id);
  await resources.goto("Patient");
  await page
    .locator(
      `#query-results-body a.result-id[data-resource-type='Patient'][data-resource-id='${id}']`,
    )
    .click();
  await resources.modal.waitOpen();

  await page.fill('[data-set="name.0.family"]', "FastCloseEdited");
  await expect(resources.modal.unsavedCue).toBeVisible();

  armDialog(page, "accept");
  await page.locator(".modal__x").click();
  await expect(resources.modal.root).toBeHidden();

  dialogsSeen(page);
  await chrome.navLink("/ui/tenants").click();
  await page.waitForURL("**/ui/tenants");
  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
});

test("typing in a form field and leaving the editor asks the browser", async ({
  page,
  request,
  chrome,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "TypedInEditor" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });

  const cue = page.locator("#editor .tag--unsaved");
  await expect(cue).toBeHidden();

  // No blur: the value sits on screen, uncommitted to #editor-doc.
  await page.fill('[data-set="name.0.family"]', "TypedInEditorEdited");
  await expect(cue).toBeVisible();

  armDialog(page, "dismiss");
  await chrome.navLink("/ui/resources").click();
  await expect.poll(() => dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(true);

  // Dismissed: the navigation never happened.
  await expect(page).toHaveURL(/\/ui\/editor/);
});

test("Escape on a clean modal closes without asking", async ({ resources, page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "CleanEscape" }] });
  await waitSearchable(request, "Patient", id);
  await resources.goto("Patient");
  await page
    .locator(
      `#query-results-body a.result-id[data-resource-type='Patient'][data-resource-id='${id}']`,
    )
    .click();
  await resources.modal.waitOpen();

  dialogsSeen(page);
  await resources.modal.closeWithEscape();
  expect(dialogsSeen(page)).toEqual([]);
});

test("saving in the modal clears the cue", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  const ed = resources.modal.editor;
  await ed.applyJson({ resourceType: "Patient", name: [{ family: "ModalSaveClears" }] });
  await expect(resources.modal.unsavedCue).toBeVisible();

  await resources.modal.save();
  await expect(resources.modal.status).toContainText(/saved/i);
  await expect(resources.modal.unsavedCue).toBeHidden();

  dialogsSeen(page);
  await resources.modal.closeWithEscape();
  expect(dialogsSeen(page)).toEqual([]);
});

// ---- View Definitions (#1240) ----------------------------------------------
//
// `vd-editor.js` opts `#vd-editor-form` into HfsUnsaved right after
// `EditorPair.mount` — no `read` of its own: `serialize(form)` already
// covers the hidden `id` field and the `json` textarea, comparing the
// latter in canonical form so a reindent is never a false positive.

/** A minimal savable ViewDefinition, named for the rail. */
function unsavedVdStarter(name: string) {
  return {
    name,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  };
}

test("the View Definition editor shows the cue on a real change and hides it when the JSON is only reformatted", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const vdId = await createResource(
    request,
    "ViewDefinition",
    unsavedVdStarter(`unsaved_vd_${stamp}`),
  );
  await waitSearchable(request, "ViewDefinition", vdId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  const vd = new VdEditor(page);
  const cue = page.locator("#vd-editor-form .tag--unsaved");
  await expect(cue).toBeHidden();

  const original = await vd.doc();
  const parsed = JSON.parse(original);

  // Reformatting only — same document, different whitespace.
  await vd.setDoc(JSON.stringify(parsed, null, 4));
  await expect(cue).toBeHidden();

  // A real change.
  await vd.setDoc(JSON.stringify({ ...parsed, name: `${parsed.name}_renamed` }));
  await expect(cue).toBeVisible();

  // Back to the document as loaded.
  await vd.setDoc(original);
  await expect(cue).toBeHidden();
});

test("saving a View Definition does not ask and lands clean", async ({ page, request }) => {
  const stamp = Date.now().toString(36);
  const vdId = await createResource(
    request,
    "ViewDefinition",
    unsavedVdStarter(`unsaved_vd_save_${stamp}`),
  );
  await waitSearchable(request, "ViewDefinition", vdId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  const vd = new VdEditor(page);
  const cue = page.locator("#vd-editor-form .tag--unsaved");
  const original = await vd.doc();
  const parsed = JSON.parse(original);
  await vd.setDoc(JSON.stringify({ ...parsed, name: `${parsed.name}_edited` }));
  await expect(cue).toBeVisible();

  armDialog(page, "dismiss");
  await page.locator("#vd-editor-form button[value='save']").click();
  await page.waitForURL(/saved=1/);

  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
  await expect(cue).toBeHidden();
});

test("leaving a dirty View Definition asks the browser", async ({ page, request, chrome }) => {
  const stamp = Date.now().toString(36);
  const vdId = await createResource(
    request,
    "ViewDefinition",
    unsavedVdStarter(`unsaved_vd_leave_${stamp}`),
  );
  await waitSearchable(request, "ViewDefinition", vdId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  const vd = new VdEditor(page);
  const cue = page.locator("#vd-editor-form .tag--unsaved");
  const original = await vd.doc();
  const parsed = JSON.parse(original);
  await vd.setDoc(JSON.stringify({ ...parsed, name: `${parsed.name}_dirty` }));
  await expect(cue).toBeVisible();

  armDialog(page, "dismiss");
  await chrome.navLink("/ui/resources").click();
  await expect.poll(() => dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(true);

  // Dismissed: the navigation never happened.
  await expect(page).toHaveURL(/\/ui\/sql\/view-definitions/);
  await expect(cue).toBeVisible();
});

// ---- SQL library (#1240) ---------------------------------------------------
//
// `sql-editor.js` opts a whole `<main>` into HfsUnsaved (`root`), tracking
// `#lib-editor-form` (`form`): the Details JSON textarea sits outside that
// `<form>` in the DOM (associated only by its own `form` attribute), so its
// `input` events never bubble through the form — but they, and a server-
// driven Declare's own `input` dispatch, do bubble through their shared
// `<main>`. `serialize(form)` still reads every associated control (`id`,
// `sql`, `json`) since `form.elements` already includes them regardless of
// DOM position.

test("the SQL library cue follows the SQL pane, the Details JSON and a server-side Declare", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const canonical = `http://example.org/ViewDefinition/e2e-unsaved-${stamp}`;
  await createResource(request, "ViewDefinition", {
    name: `unsaved_lib_source_${stamp}`,
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [
      {
        column: [
          { name: "id", path: "getResourceKey()" },
          { name: "family", path: "name.family.first()" },
        ],
      },
    ],
  });
  const libId = await createSqlQueryLibrary(
    request,
    `unsaved_lib_${stamp}`,
    canonical,
    "SELECT id, family FROM v WHERE family = :fam",
  );
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  const cue = page.locator("#lib-editor-form .tag--unsaved");
  await expect(cue).toBeHidden();

  // The SQL pane.
  const sqlEditor = page.locator(".sql-editor .cm-content[role='textbox']");
  const sqlTextarea = page.locator("textarea[name='sql']");
  const originalSql = await sqlTextarea.inputValue();
  await sqlEditor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.insertText(originalSql + " -- edited");
  await expect(cue).toBeVisible();

  await sqlEditor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.insertText(originalSql);
  await expect(cue).toBeHidden();

  // The Details JSON pane — outside the `<form>`, associated by `form=`.
  const detailsEditor = page.locator("#lib-details-editor .cm-content");
  const jsonTextarea = page.locator("textarea[name='json']");
  const originalJson = await jsonTextarea.inputValue();
  const editedJson = JSON.stringify(
    { ...JSON.parse(originalJson), name: `unsaved_lib_${stamp}_renamed` },
    null,
    2,
  );
  await detailsEditor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(editedJson);
  await expect(cue).toBeVisible();

  await detailsEditor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(originalJson);
  await expect(cue).toBeHidden();

  // A server-side Declare parameter mutation — pushed into the Details
  // textarea via `setDoc`, which already dispatches `input` on it (#841).
  const paramsCard = page.locator("#lib-params");
  const declareButton = paramsCard.getByRole("button", { name: "Declare :fam" });
  await expect(declareButton).toBeVisible();
  await declareButton.click();
  await expect(jsonTextarea).toHaveValue(/"name": "fam"/, { timeout: 3000 });
  await expect(cue).toBeVisible();
});

// The remaining screens with data-writing forms (#1240): the SQL export and
// bulk export builders, the Bulk Import create/edit dialogs, the tenants
// add-tenant panel, and the Queries save-name field. Every native form
// submit here suspends the browser guard on its own; only the addbox
// dialogs and a real navigation ever ask.

test("the SQL export form shows the cue after a change and clears it when the change is undone", async ({
  page,
  sqlExport,
}) => {
  await sqlExport.gotoNew();
  const cue = page.locator("form.bulk-export-form .tag--unsaved");
  const nameInput = page.locator("form.bulk-export-form input[name='name']");
  await expect(cue).toBeHidden();

  await nameInput.fill("Ward census");
  await expect(cue).toBeVisible();

  await nameInput.fill("");
  await expect(cue).toBeHidden();

  await nameInput.fill("   ");
  await expect(cue).toBeHidden();
});

test("starting a SQL export does not ask", async ({ page, request, sqlExport }) => {
  const vdId = await createResource(request, "ViewDefinition", {
    name: `e2e_unsaved_sql_export_${Date.now().toString(36)}`,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "ViewDefinition", vdId);

  await sqlExport.gotoNew();
  await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();

  armDialog(page, "dismiss");
  await sqlExport.startButton.click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
});

test("the bulk export form shows the cue and leaving asks", async ({ page, bulkExport, chrome }) => {
  await bulkExport.goto();
  const cue = page.locator("form.bulk-export-form .tag--unsaved");
  await expect(cue).toBeHidden();

  // A no-op change — switching scope away and straight back — must never
  // leave the pill on: the page's own initial sync (All Resources checking
  // every individual type) is part of the baseline, not a user edit; the
  // tracker has to be captured after that sync, not before it (#1240).
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.scopeRadio("system").check();
  await expect(cue).toBeHidden();

  await bulkExport.nameInput.fill("   ");
  await expect(cue).toBeHidden();

  await bulkExport.nameInput.fill("Nightly export");
  await expect(cue).toBeVisible();
  await bulkExport.nameInput.fill("");
  await expect(cue).toBeHidden();

  await bulkExport.nameInput.fill("Nightly export");
  await expect(cue).toBeVisible();

  armDialog(page, "dismiss");
  await chrome.navLink("/ui").click();
  await expect.poll(() => dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(true);

  // Dismissed: the navigation never happened.
  await expect(page).toHaveURL(/\/ui\/bulk-export\/new$/);
});

test("starting a bulk export does not ask", async ({ page, bulkExport }) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("E2eUnsavedBulkExportStart");

  armDialog(page, "dismiss");
  await bulkExport.startButton.click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  expect(dialogsSeen(page).some((d) => d.type === "beforeunload")).toBe(false);
});

test("the bulk import dialog asks before closing with typed values and keeps them on cancel", async ({
  page,
  bulkImport,
}) => {
  await bulkImport.goto();
  await bulkImport.newSubmission.click();
  await expect(bulkImport.createDialog).toBeVisible();

  const nameInput = page.locator("input[name='name']");
  const cue = bulkImport.createDialog.locator(".tag--unsaved");
  const cancelButton = bulkImport.createDialog.getByRole("button", { name: "Cancel" });
  await expect(cue).toBeHidden();
  await nameInput.fill("Dirty Submission");
  await expect(cue).toBeVisible();

  // Cancel dismissed: the dialog stays open with the typed value.
  armDialog(page, "dismiss");
  await cancelButton.click();
  await expect(bulkImport.createDialog).toBeVisible();
  await expect(nameInput).toHaveValue("Dirty Submission");

  // Cancel accepted: the dialog closes and resets.
  armDialog(page, "accept");
  await cancelButton.click();
  await expect(bulkImport.createDialog).toBeHidden();

  await bulkImport.newSubmission.click();
  await expect(bulkImport.createDialog).toBeVisible();
  await expect(nameInput).toHaveValue("");
  await expect(cue).toBeHidden();
});

test("clicking the modal backdrop asks before discarding typed values", async ({
  page,
  bulkImport,
}) => {
  // The addbox--modal's own <summary> is the full-screen backdrop
  // (app.css): a click on it outside the panel is a native <details> toggle
  // that must still be routed through close() (#1240).
  await bulkImport.goto();
  await bulkImport.newSubmission.click();
  await expect(bulkImport.createDialog).toBeVisible();

  const nameInput = page.locator("input[name='name']");
  const cue = bulkImport.createDialog.locator(".tag--unsaved");
  await nameInput.fill("Backdrop Dirty Submission");
  await expect(cue).toBeVisible();

  const summary = bulkImport.newSubmission;

  // Dismissed: the dialog stays open with the typed value.
  armDialog(page, "dismiss");
  await summary.click({ position: { x: 4, y: 4 }, force: true });
  expect(dialogsSeen(page)).toContainEqual({
    type: "confirm",
    message: "You have unsaved changes. Discard them and close?",
  });
  await expect(bulkImport.createDialog).toBeVisible();
  await expect(nameInput).toHaveValue("Backdrop Dirty Submission");

  // Accepted: the dialog closes and resets.
  armDialog(page, "accept");
  await summary.click({ position: { x: 4, y: 4 }, force: true });
  await expect(bulkImport.createDialog).toBeHidden();

  await bulkImport.newSubmission.click();
  await expect(bulkImport.createDialog).toBeVisible();
  await expect(nameInput).toHaveValue("");
  await expect(cue).toBeHidden();
});

test("clicking the modal backdrop on a clean dialog closes without asking", async ({
  page,
  bulkImport,
}) => {
  await bulkImport.goto();
  await bulkImport.newSubmission.click();
  await expect(bulkImport.createDialog).toBeVisible();

  dialogsSeen(page);
  await bulkImport.newSubmission.click({ position: { x: 4, y: 4 }, force: true });
  await expect(bulkImport.createDialog).toBeHidden();
  expect(dialogsSeen(page)).toEqual([]);
});

test("the tenants dialog: whitespace only is clean, a name is dirty, a successful add is clean", async ({
  page,
  tenants,
}) => {
  await tenants.goto();
  if (await tenants.unavailableNotice.isVisible().catch(() => false)) {
    test.skip(true, "no tenant store on this backend");
  }
  test.setTimeout(300_000);

  await tenants.addToggle.click();
  const cue = tenants.addForm.locator(".tag--unsaved");
  const nameField = tenants.addForm.locator("input[name=display_name]");
  await expect(cue).toBeHidden();

  await nameField.fill("   ");
  await expect(cue).toBeHidden();

  await nameField.fill(`E2eUnsaved${Date.now().toString(36)}`);
  await expect(cue).toBeVisible();

  // Provisioning runs in the background: the server accepts the id and
  // answers with `tenant-created` as soon as it does, well before the
  // conformance seed finishes — the panel closes right away.
  await tenants.addForm.locator("button[type=submit]").click();
  await expect(tenants.addForm).toBeHidden();

  await tenants.addToggle.click();
  await expect(cue).toBeHidden();
});

test("Escape on a dirty tenants dialog asks and cancel keeps the panel open", async ({
  page,
  tenants,
}) => {
  await tenants.goto();
  if (await tenants.unavailableNotice.isVisible().catch(() => false)) {
    test.skip(true, "no tenant store on this backend");
  }

  await tenants.addToggle.click();
  await tenants.addForm.locator("input[name=display_name]").fill("E2eEscapeDirty");
  await expect(tenants.addForm.locator(".tag--unsaved")).toBeVisible();

  armDialog(page, "dismiss");
  await page.keyboard.press("Escape");
  expect(dialogsSeen(page)).toContainEqual({
    type: "confirm",
    message: "You have unsaved changes. Discard them and close?",
  });
  await expect(tenants.addForm).toBeVisible();
});

test("the Queries save-name field shows the cue until the query is saved", async ({
  page,
  queries,
}) => {
  await queries.goto("Patient");
  await queries.builder.setUrl("/Patient?_count=1");
  const cue = page.locator(".query-builder__save .tag--unsaved");
  await expect(cue).toBeHidden();

  await queries.builder.nameInput.fill(`E2eUnsavedQuery${Date.now().toString(36)}`);
  await expect(cue).toBeVisible();

  await queries.builder.saveButton.click();
  await expect(queries.builder.nameInput).toHaveValue("");
  await expect(cue).toBeHidden();
  expect(dialogsSeen(page)).toEqual([]);
});
