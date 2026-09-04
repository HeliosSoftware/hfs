// #821: the ViewDefinition editor's completion popup —
// `POST /ui/sql/view-definitions/complete`'s two request shapes
// (`vdCompletionSource` in `vd-editor.js`), exercised end to end: structural
// JSON keys (with their skeleton insertion and required marker) and partial
// FHIRPath expressions (elements, the `forEach` context, constants,
// functions), activation by typing and by Ctrl+Space, and the one negative
// case — no popup, no request, outside either context. `vd-editor-lint.
// spec.ts` is this file's sibling for the lint/quick-fix UI.
//
// Every document is a plain template string, never `JSON.stringify` — see
// that sibling file's own module doc comment for why (`VdEditor.
// setCursorAfter`/`nthIndexOf` need exact, known offsets).
import { expect, test } from "../pages/fixtures";
import { VdEditor } from "../pages/vd-editor";

/** A second column with only `name` set — `path` (required) is offered,
 * `resource` (not a column field at all) and `name` (already present)
 * are not. */
const COLUMN_MISSING_PATH_DOC = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [
        { "name": "id", "path": "getResourceKey()" },
        { "name": "extra" }
      ]
    }
  ]
}`;

test("key completion inside column[] offers path, not resource, and not a key already present", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(COLUMN_MISSING_PATH_DOC);

  // Right after `"extra"`'s closing quote — the second column's only
  // property, with no trailing comma yet: a new-key gap (#821's
  // `classifyObjectGap`, "last property" case).
  await ed.setCursorAfter(COLUMN_MISSING_PATH_DOC, '"extra"');
  await page.keyboard.press("Control+Space");
  await expect(ed.completionPopup).toBeVisible();

  await expect(ed.optionByLabel("path")).toBeVisible();
  await expect(ed.optionByLabel("resource")).toHaveCount(0);
  await expect(ed.optionByLabel("name")).toHaveCount(0);
});

test("accepting a key completion inserts its skeleton with valid surrounding commas, cursor inside the value", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  await ed.setDoc(COLUMN_MISSING_PATH_DOC);

  await ed.setCursorAfter(COLUMN_MISSING_PATH_DOC, '"extra"');
  await page.keyboard.press("Control+Space");
  await expect(ed.completionPopup).toBeVisible();
  // @codemirror/autocomplete's own `interactionDelay` (75ms): the popup
  // ignores Enter/click for a short window right after it opens, so an
  // "accept immediately" click races it. 100ms comfortably clears it.
  await page.waitForTimeout(100);
  await ed.optionByLabel("path").click();

  const parsed = JSON.parse(await ed.doc());
  expect(parsed.select[0].column[1]).toEqual({ name: "extra", path: "" });

  // The cursor landed inside the freshly inserted empty string, not after
  // it — typing continues straight into the value.
  await page.keyboard.type("z");
  expect(JSON.parse(await ed.doc()).select[0].column[1].path).toBe("z");
});

test("FHIRPath completion after a dot offers the resolved type's own elements", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "n", "path": "Patient.name" }]
    }
  ]
}`;
  await ed.setDoc(doc);

  // Right before the closing quote of "Patient.name" — typing "." makes it
  // "Patient.name.", a member position off HumanName.
  await ed.setCursorAfter(doc, "Patient.name");
  await page.keyboard.type(".");
  await expect(ed.completionPopup).toBeVisible();

  await expect(ed.optionByLabel("given")).toBeVisible();
  await expect(ed.optionByLabel("family")).toBeVisible();
  // #821 (validator-caught during 02): the library's own default
  // `maxRenderedOptions` (100) silently truncated exactly this response —
  // HumanName's own elements plus the entire function catalog. Asserting
  // the full, known count (not just "some" elements) is what actually
  // guards `code-editor.js`'s `maxRenderedOptions: 300` against a future
  // regression back to the default.
  await expect(ed.completionOptions).toHaveCount(114);
});

test("FHIRPath completion resolves the ancestor select's own forEach context", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "forEach": "name",
      "column": [{ "name": "g", "path": "gi" }]
    }
  ]
}`;
  await ed.setDoc(doc);

  // Right after "gi" — typing "v" makes it "giv", root-mode (no dot ahead
  // of it), resolved against %context — the select's own `forEach: "name"`
  // narrows that to HumanName, so "given" is offered even with no explicit
  // "Patient.name." chain typed.
  await ed.setCursorAfter(doc, '"path": "gi');
  await page.keyboard.type("v");
  await expect(ed.completionPopup).toBeVisible();
  await expect(ed.optionByLabel("given")).toBeVisible();
});

test("% offers the document's own constants and the environment variables", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "constant": [{ "name": "official", "valueString": "official" }],
  "select": [
    {
      "column": [{ "name": "c", "path": "" }]
    }
  ]
}`;
  await ed.setDoc(doc);

  // Between the quotes of the empty "path" value.
  await ed.setCursorAfter(doc, '"path": "');
  await page.keyboard.type("%");
  await expect(ed.completionPopup).toBeVisible();

  await expect(ed.optionByLabel("%official")).toBeVisible();
  await expect(ed.optionByLabel("%resource")).toBeVisible();
});

test("a function candidate (fir → first) inserts a call with the cursor after it", async ({
  page,
}) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "c", "path": "id." }]
    }
  ]
}`;
  await ed.setDoc(doc);

  await ed.setCursorAfter(doc, '"path": "id.');
  await page.keyboard.type("fir");
  await expect(ed.completionPopup).toBeVisible();
  await expect(ed.optionByLabel("first")).toBeVisible();

  await page.waitForTimeout(100); // interactionDelay — see the key-completion test above.
  await ed.optionByLabel("first").click();

  expect(JSON.parse(await ed.doc()).select[0].column[0].path).toBe("id.first()");
});

test("no popup and no request outside a completion context", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "c", "path": "id" }]
    }
  ]
}`;
  await ed.setDoc(doc);

  let completeRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/sql/view-definitions/complete") && request.method() === "POST") {
      completeRequests++;
    }
  });

  // "resource" is a plain string — not `path`/`forEach`/`forEachOrNull`/
  // `repeat` — so it is not a FHIRPath completion context, and it isn't a
  // structural key position either (it's inside a value, not a key gap):
  // `vdCompletionSource` returns `null` synchronously and nothing follows.
  await ed.setCursorAfter(doc, '"resource": "Patient');
  await page.keyboard.type("x");
  await expect(ed.completionPopup).not.toBeVisible();

  // A real completion trigger right after, positioned against the *live*
  // document (its text has moved since the "x" above) — its own popup
  // becoming visible is this test's actual (state-based) wait; by then, any
  // request the "resource" keystroke might have sent would already have
  // arrived, so `completeRequests` staying at exactly 1 — this trigger's
  // own — proves it never did.
  const live = await ed.doc();
  await ed.setCursorAfter(live, '"path": "id');
  await page.keyboard.type(".");
  await expect(ed.completionPopup).toBeVisible();
  expect(completeRequests).toBe(1);
});

test("Ctrl+Space opens the popup without typing anything", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "resource": "Patient",
  "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }] }],
  "status": "active"
}`;
  await ed.setDoc(doc);

  // Right after the document's own last property, with no trailing comma —
  // a root-level key gap, reached with no keystroke of its own.
  await ed.setCursorAfter(doc, '"status": "active"');
  await page.keyboard.press("Control+Space");

  await expect(ed.completionPopup).toBeVisible();
  await expect(ed.optionByLabel("constant")).toBeVisible();
});

test("Ctrl+Space right after a comma offers the node keys", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  // A single-property column - typed into below rather than baked in with a
  // trailing comma already present, since a genuinely dangling comma is
  // invalid JSON and `setDoc`'s own wait is for the *server* lint round
  // trip, never reached while the local syntax checker alone is still
  // failing.
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "id" }]
    }
  ]
}`;
  await ed.setDoc(doc);

  // Right after `"id"`'s own closing quote, before the column object's own
  // "}" - typing "," reaches the exact gap `objectGapAt` (`vd-editor.js`)
  // used to misclassify: `lezer-json`'s own error-recovery node
  // (zero-width, spliced in at the same offset as whichever real token
  // follows the gap - here the object's own "}") outscored the real ","
  // as this gap's "before" sibling, so a trailing comma right before a
  // closing brace never offered completion at all - in a document
  // CodeMirror had fully parsed, not a mid-keystroke error state.
  await ed.setCursorAfter(doc, '"name": "id"');
  await page.keyboard.type(",");
  await page.keyboard.press("Control+Space");
  await expect(ed.completionPopup).toBeVisible();

  await expect(ed.optionByLabel("path")).toBeVisible();
  await expect(ed.optionByLabel("name")).toHaveCount(0);

  // Accepting inserts `"label": skeleton` with no extra comma - the one
  // just typed already separates it from `"id"`.
  await page.waitForTimeout(100); // interactionDelay - see the key-completion test above.
  await ed.optionByLabel("path").click();

  const parsed = JSON.parse(await ed.doc());
  expect(parsed.select[0].column[0]).toEqual({ name: "id", path: "" });
});

test("the required-key marker renders translated under ?lang=es", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new&lang=es");
  const ed = new VdEditor(page);
  await ed.setDoc(COLUMN_MISSING_PATH_DOC);

  await ed.setCursorAfter(COLUMN_MISSING_PATH_DOC, '"extra"');
  await page.keyboard.press("Control+Space");
  await expect(ed.completionPopup).toBeVisible();

  const pathOption = ed.optionByLabel("path");
  await expect(pathOption).toBeVisible();
  await expect(pathOption.locator(".cm-completionDetail")).toHaveText("string · obligatorio");
});
