// #821: the ViewDefinition editor's lint UI — the gutter marker and inline
// underline `POST /ui/sql/view-definitions/lint`'s diagnostics get, the
// hover tooltip and its fix buttons, applying a fix by click or by Ctrl+.
// (one action applies directly, more than one opens the lint panel), the
// save-with-errors confirmation, and the negotiated-locale rendering of the
// message and fix labels. `vd-editor-completion.spec.ts` is this file's
// sibling for the completion popup; `sql-view-definitions.spec.ts` already
// covers the editor's own mount, sync, and cross-highlight and is not
// duplicated here.
//
// Every document below is a plain template string, never `JSON.stringify` —
// so each test knows its exact text and can locate a cursor position with a
// bare `text.indexOf(...)` (`VdEditor.setCursorAt`/`nthIndexOf`) instead of
// guessing at click coordinates or counting keystrokes.
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";
import { VdEditor, nthIndexOf } from "../pages/vd-editor";

/** A single unknown key ("columns", not "column") with no other output for
 * its `select` — this produces exactly two diagnostics: `unknown-key`
 * (offering both a `rename-key` fix to "column", since nothing else in the
 * object already uses that key, and a `remove-key` fix) and
 * `select-without-output` (since neither `column`, `select`, nor `unionAll`
 * is actually present). Renaming "columns" to "column" resolves both at
 * once — the renamed key is now itself the select's output. */
const UNKNOWN_KEY_DOC = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "columns": [{ "name": "id", "path": "getResourceKey()" }]
    }
  ]
}`;

/** Two columns sharing the name "id" — a single `duplicate-column-name`
 * diagnostic on the second one, with a single fix (`set-string` to
 * "id_2"). */
const DUPLICATE_COLUMN_DOC = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [
        { "name": "id", "path": "getResourceKey()" },
        { "name": "id", "path": "name.family" }
      ]
    }
  ]
}`;

/** A `select` setting both `forEach` and `repeat` — a single
 * `multiple-iteration-directives` diagnostic with a single fix, removing
 * `repeat` (declared second) and keeping `forEach` (declared first). */
const EXTRA_DIRECTIVE_DOC = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "forEach": "name",
      "repeat": ["name"],
      "column": [{ "name": "given", "path": "given" }]
    }
  ]
}`;

/** `%bogus` is declared nowhere (not in `constant[]`, not an environment
 * variable) — a single `undeclared-constant` diagnostic whose `span` covers
 * exactly the `%bogus` token, not the whole expression. */
const UNDECLARED_CONSTANT_DOC = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "computed", "path": "%bogus + 1" }]
    }
  ]
}`;

/** `column` present and non-empty (so `select-without-output` never fires)
 * plus one unrecognized `columns: []` alongside it — since `column` is
 * already the object's own key, no rename is suggested (renaming onto a key
 * already there would just create a second problem), so this is exactly one
 * diagnostic with exactly one fix (`remove-key`). Used for the save-with-
 * errors confirmation, where the singular "1 error" wording matters and the
 * saved document must stay well-formed. */
function oneErrorDoc(name: string, id?: string): string {
  const idLine = id ? `\n  "id": "${id}",` : "";
  return `{
  "resourceType": "ViewDefinition",${idLine}
  "name": "${name}",
  "status": "active",
  "resource": "Patient",
  "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }], "columns": [] }]
}`;
}

test("an unknown key is underlined and marked in the gutter", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(UNKNOWN_KEY_DOC);

  const errorRange = page.locator(".cm-lintRange-error", { hasText: '"columns"' });
  await expect(errorRange).toBeVisible();
  await expect(ed.gutterErrorMarkers.first()).toBeVisible();
});

test("hovering the underlined range shows a tooltip with the message and fix buttons", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(UNKNOWN_KEY_DOC);

  const errorRange = page.locator(".cm-lintRange-error", { hasText: '"columns"' });
  await errorRange.hover();
  await expect(ed.lintTooltip).toBeVisible();
  await expect(ed.lintTooltip.locator(".cm-diagnosticText")).toHaveText('Unknown key "columns"');
  await expect(ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Rename" })).toBeVisible();
  await expect(ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Remove" })).toBeVisible();
});

test("clicking the rename fix applies it and the error disappears", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(UNKNOWN_KEY_DOC);

  const errorRange = page.locator(".cm-lintRange-error", { hasText: '"columns"' });
  await errorRange.hover();
  await expect(ed.lintTooltip).toBeVisible();
  // @codemirror/autocomplete's own `interactionDelay` (75ms) is what makes a
  // *completion* popup ignore an immediate accept; the lint tooltip has no
  // such guard, but this settle before the click matches every other
  // freshly-opened-popup interaction in this file/its sibling spec for the
  // same reason: a click dispatched the instant a tooltip's own
  // position/attach finishes is exactly the kind of race Playwright's own
  // auto-waiting on visibility does not cover.
  await page.waitForTimeout(150);
  await ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Rename" }).click();

  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);
  const doc = await ed.doc();
  expect(doc).toContain('"column"');
  expect(doc).not.toContain('"columns"');
});

test("Ctrl+. with exactly one action applies it directly (duplicate column → _2)", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(DUPLICATE_COLUMN_DOC);
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);

  // Inside the quotes of the *second* "id" — the one the duplicate-name
  // diagnostic actually points at.
  await ed.setCursor(nthIndexOf(DUPLICATE_COLUMN_DOC, '"id"', 2) + 1);
  await page.keyboard.press("ControlOrMeta+.");

  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);
  expect(await ed.doc()).toContain('"id_2"');
});

test("Ctrl+. with more than one action opens the lint panel instead of guessing", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(UNKNOWN_KEY_DOC);

  // Inside "columns" itself — the key with two fixes (rename, remove).
  await ed.setCursorAt(UNKNOWN_KEY_DOC, '"columns"');
  await page.keyboard.press("ControlOrMeta+.");

  await expect(ed.lintPanel).toBeVisible();
});

test("the remove fix for an extra iteration directive leaves the document valid JSON", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(EXTRA_DIRECTIVE_DOC);

  const errorRange = page.locator(".cm-lintRange-error");
  await expect(errorRange).toHaveCount(1);
  await errorRange.hover();
  await expect(ed.lintTooltip).toBeVisible();
  await page.waitForTimeout(150); // see the rename-fix test above.
  await ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Remove" }).click();

  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);
  const parsed = JSON.parse(await ed.doc());
  expect(parsed.select[0].forEach).toBe("name");
  expect(parsed.select[0].repeat).toBeUndefined();
});

test("an undeclared constant is underlined at exactly its own token", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(UNDECLARED_CONSTANT_DOC);

  const errorRange = page.locator(".cm-lintRange-error");
  await expect(errorRange).toHaveCount(1);
  await expect(errorRange).toHaveText("%bogus");
});

test("Ctrl+Z undoes an applied fix in one step", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(DUPLICATE_COLUMN_DOC);
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);

  await ed.setCursor(nthIndexOf(DUPLICATE_COLUMN_DOC, '"id"', 2) + 1);
  await page.keyboard.press("ControlOrMeta+.");
  expect(await ed.doc()).toContain('"id_2"');

  await page.keyboard.press("ControlOrMeta+z");
  expect(await ed.doc()).toBe(DUPLICATE_COLUMN_DOC);
});

test("saving with errors confirms with a plural-correct count; cancelling keeps the page, accepting saves", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const stamp = Date.now().toString(36);
  await ed.setDoc(oneErrorDoc(`e2e_lint_save_confirm_${stamp}`));
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);

  const save = page.locator("#vd-editor-form button[name='action'][value='save']");

  let message = "";
  page.once("dialog", (dialog) => {
    message = dialog.message();
    dialog.dismiss();
  });
  await save.click();
  expect(message).toBe("This view definition still has 1 error. Save it anyway?");
  // Cancelling never navigates — the page, and the errors, stay exactly as
  // they were.
  await expect(page).toHaveURL(/vd=new/);
  await expect(page).not.toHaveURL(/saved=1/);
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);

  page.once("dialog", (dialog) => {
    message = dialog.message();
    dialog.accept();
  });
  await save.click();
  await page.waitForURL(/saved=1/);
  expect(message).toBe("This view definition still has 1 error. Save it anyway?");
});

test("Save with a valid document never confirms", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const stamp = Date.now().toString(36);
  await ed.setDoc(`{
  "resourceType": "ViewDefinition",
  "name": "e2e_lint_valid_${stamp}",
  "status": "active",
  "resource": "Patient",
  "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }] }]
}`);
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);

  let dialogFired = false;
  page.on("dialog", (dialog) => {
    dialogFired = true;
    dialog.dismiss();
  });
  await page.locator("#vd-editor-form button[name='action'][value='save']").click();
  await page.waitForURL(/saved=1/);
  expect(dialogFired).toBe(false);
});

test("Duplicate never confirms, even with lint errors present", async ({ page, request }) => {
  // Duplicate only renders for an already-stored view (`{% if !is_new %}`,
  // sql-view-definitions.html) — a fresh `?vd=new` document has nothing to
  // duplicate from.
  const stamp = Date.now().toString(36);
  const vdId = await createResource(request, "ViewDefinition", {
    name: `e2e_lint_duplicate_${stamp}`,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "ViewDefinition", vdId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  const ed = new VdEditor(page);
  await ed.setDoc(oneErrorDoc(`e2e_lint_duplicate_${stamp}`, vdId));
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);

  let dialogFired = false;
  page.on("dialog", (dialog) => {
    dialogFired = true;
    dialog.dismiss();
  });
  await page.locator("button[name='action'][value='duplicate']").click();
  await page.waitForURL(/saved=1/);
  expect(dialogFired).toBe(false);
});

test("the diagnostic message and fix labels render in Spanish under ?lang=es", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new&lang=es");
  const ed = new VdEditor(page);
  await ed.setDoc(UNKNOWN_KEY_DOC);

  const errorRange = page.locator(".cm-lintRange-error", { hasText: '"columns"' });
  await errorRange.hover();
  await expect(ed.lintTooltip).toBeVisible();
  await expect(ed.lintTooltip.locator(".cm-diagnosticText")).toHaveText(
    'Clave desconocida "columns"',
  );
  await expect(
    ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Renombrar" }),
  ).toBeVisible();
  await expect(
    ed.lintTooltip.locator(".cm-diagnosticAction", { hasText: "Quitar" }),
  ).toBeVisible();
});
