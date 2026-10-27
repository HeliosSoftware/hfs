// The shared editor/guided-form host (`assets/editor-pair.js`, #840,
// extracted out of `vd-editor.js`'s original #843 implementation). View
// Definitions is today's only consumer — the Library Details editor (#840)
// is next — so coverage that is specific to the pairing's own contract
// rather than to the View Definitions page lives here, separate from
// `sql-view-definitions.spec.ts`'s page-level coverage.
import { expect, test } from "../pages/fixtures";
import { Editor } from "../pages/editor";

test("the invalid-JSON chip still lights up after a guided-form round trip", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new Editor(page, page.locator("#vd-editor-grid"));
  await expect(ed.validity).toHaveClass(/editor-validity--ok/);
  const validText = await ed.validity.textContent();

  // A guided-form round trip through `editor-form.js`: the `.editor-form`
  // card it swaps in (`editor-form-pane.html`) always renders with
  // `needs_js` false — only a page's own inline first paint sets it — so
  // its chip carries no `data-msg-invalid-json` attribute at all. Reading
  // that attribute off the *current* chip on every invalid keystroke
  // (rather than the one captured once at mount) would leave the chip
  // stuck showing whatever it already said instead of switching to
  // "Invalid JSON".
  const nameField = ed.rowAt("name").locator("[data-set='name']");
  await nameField.fill("e2e_chip_round_trip");
  await page.keyboard.press("Tab");
  await expect
    .poll(() => ed.currentDoc())
    .toMatchObject({ name: "e2e_chip_round_trip" });

  // Breaking the JSON now — the same "drop the final `}`" shape as the bug
  // repro — must still flip the chip to "Invalid JSON", not silently leave
  // it reading "valid".
  const textarea = page.locator("textarea[name='json']");
  const validDoc = await textarea.inputValue();
  const brokenDoc = validDoc.slice(0, -1);
  const cmContent = page.locator("#vd-editor .cm-content");
  await cmContent.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.insertText(brokenDoc);
  await expect(ed.validity).toHaveText("Invalid JSON", { timeout: 3000 });
  await expect(ed.validity).not.toHaveClass(/editor-validity--ok/);

  // Restoring the closing brace flips the chip back to its earlier text.
  await page.keyboard.insertText("}");
  await expect(ed.validity).toHaveText(validText!, { timeout: 3000 });
  await expect(ed.validity).toHaveClass(/editor-validity--ok/);
});
