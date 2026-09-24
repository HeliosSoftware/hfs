import { test, expect, armDialog, dialogsSeen } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { createResource, waitSearchable } from "../pages/api";

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
