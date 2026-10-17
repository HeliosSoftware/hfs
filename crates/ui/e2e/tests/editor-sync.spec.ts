import { test, expect } from "../pages/fixtures";
import { Editor } from "../pages/editor";

// The linked editor: the guided form and the JSON editor point at each other
// (editor-pair.js, #1756). Hover or focus a form row and the node's lines in
// the editor light up; move the caret in the editor and the row that edits
// that node answers. Typing valid JSON re-renders the form live, and a guided
// edit lands back in the editor in place.

function standalone(page: import("@playwright/test").Page): Editor {
  return new Editor(page, page.locator("#editor-body"));
}

test("issue1720 nested collection headers and indexed entries highlight their own JSON paths", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", name: [{ given: ["Ana", "Bea"] }] });
  const group = ed.rowAt("name.0.given");
  await expect(group).toHaveAttribute("data-collection", "");
  await expect(group.locator(".editor-row__label")).toHaveText("given");
  await expect(ed.collectionAdd("name.0.given")).toHaveAttribute("data-add", "name.0");
  const groupIndent = await group.evaluate(node => parseFloat(getComputedStyle(node).paddingLeft));
  const itemIndent = await ed.rowAt("name.0.given.0").evaluate(node => parseFloat(getComputedStyle(node).paddingLeft));
  expect(itemIndent - groupIndent).toBe(18);
  await group.locator(".editor-row__label").hover();
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"Ana"' })).toHaveCount(1);
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"Bea"' })).toHaveCount(1);
  await ed.rowAt("name.0.given.0").hover();
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"Ana"' })).toHaveCount(1);
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"Bea"' })).toHaveCount(0);
  // The reverse direction: the caret on the array's first item marks its row.
  const text = await ed.jsonText();
  await ed.setCursor(text.indexOf('"Ana"') + 2);
  await expect(ed.rowAt("name.0.given.0")).toHaveClass(/editor-row--hit/, { timeout: 3000 });
  await ed.collectionAdd("name.0.given").click();
  await expect(ed.form).toHaveAttribute("data-focus", "name.0.given.2");
  expect((await ed.currentDoc()).name).toEqual([{ given: ["Ana", "Bea", ""] }]);
  await expect(ed.rowAt("name.0.given.2").locator("[data-set]")).toHaveAccessibleName("given[2] — name.0.given.2");
});

test("the highlight scrolls its counterpart into view on a large document", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  // Enough repeating structure that both panes overflow their 70vh columns.
  await ed.applyJson({
    resourceType: "Patient",
    identifier: Array.from({ length: 40 }, (_, i) => ({
      system: "http://example.org/mrn",
      value: String(10000 + i),
    })),
    gender: "female",
  });
  await page.locator('.editor-row[data-path="identifier.39.value"]').waitFor();

  const scroller = ed.codeEditor.locator(".cm-scroller");
  const tree = page.locator(".editor-tree");
  expect(await scroller.evaluate((n) => n.scrollHeight > n.clientHeight)).toBe(true);
  expect(await tree.evaluate((n) => n.scrollHeight > n.clientHeight)).toBe(true);

  // Hovering a row deep in the form pulls the editor down to its lines…
  await scroller.evaluate((n) => (n.scrollTop = 0));
  await tree.evaluate((n) => (n.scrollTop = n.scrollHeight));
  await page.locator('.editor-row[data-path="identifier.39.value"]').hover();
  await expect.poll(() => scroller.evaluate((n) => n.scrollTop)).toBeGreaterThan(0);
  const hit = ed.codeEditor.locator(".cm-line--hit").filter({ hasText: '"10039"' });
  await expect(hit).toHaveCount(1);
  expect(await hit.evaluate((n) => {
    const pane = n.closest(".cm-scroller")!.getBoundingClientRect();
    const line = n.getBoundingClientRect();
    return line.top >= pane.top - 1 && line.bottom <= pane.bottom + 1;
  })).toBe(true);

  // …and moving the caret deep into the editor pulls the form pane to its row.
  // The pointer leaves the form first: a row under it would own the link.
  await page.mouse.move(0, 0);
  await tree.evaluate((n) => (n.scrollTop = 0));
  const text = await ed.jsonText();
  await ed.setCursor(text.indexOf('"10035"') + 2);
  await expect.poll(() => tree.evaluate((n) => n.scrollTop), { timeout: 3000 }).toBeGreaterThan(0);
  await expect(page.locator('.editor-row--hit[data-path="identifier.35.value"]')).toHaveCount(1);
});

test("hovering a form row lights the node's JSON lines, and back", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", gender: "female", name: [{ family: "Sync" }] });

  await page.locator('.editor-row[data-path="gender"]').hover();
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"gender"' })).toHaveCount(1);

  // Moving to another row moves the highlight; a parent row lights its whole subtree.
  await page.locator('.editor-row[data-path="name.0"]').hover();
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"gender"' })).toHaveCount(0);
  await expect(ed.root.locator(".cm-line--hit").filter({ hasText: '"family"' })).toHaveCount(1);

  // The reverse direction: the caret in a node marks the row that edits it.
  const text = await ed.jsonText();
  await ed.setCursor(text.indexOf('"female"') + 2);
  await expect(page.locator('.editor-row--hit[data-path="gender"]')).toHaveCount(1, { timeout: 3000 });
});

test("valid JSON re-renders the guided form live, and a guided edit lands in the editor", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.fillRaw({ resourceType: "Patient", gender: "male" });

  // The form catches up on its own (debounced live sync)…
  await expect(page.locator('[data-set="gender"]')).toHaveValue("male", { timeout: 5000 });

  // The other direction: a guided edit refreshes the editor in place.
  await page.fill('[data-set="gender"]', "female");
  await page.locator('[data-set="gender"]').evaluate((n) => (n as HTMLElement).blur());
  await expect.poll(async () => (await ed.jsonText()).includes('"female"')).toBe(true);
  await expect(ed.cm).toContainText('"female"');
});

test("the caret lights the row of the node it sits in", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  const doc = JSON.stringify({ resourceType: "Patient", id: "x", gender: "male" }, null, 2);
  await ed.setJson(doc);
  await expect(page.locator('[data-set="gender"]')).toHaveCount(1, { timeout: 5000 });

  await ed.setCursor(doc.indexOf('"male"') + 2);
  await expect(page.locator('.editor-row--hit[data-path="gender"]')).toHaveCount(1, { timeout: 3000 });
});

test("the element name leads the row; the description sits under it", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", gender: "female" });

  const row = page.locator('.editor-row[data-path="gender"]');
  await expect(row.locator(".editor-row__label")).toHaveText("gender");
  await expect(row.locator(".editor-row__desc")).toContainText("male | female");
  // The description renders outside the head line, as its own block.
  await expect(row.locator(".editor-row__head .editor-row__desc")).toHaveCount(0);
});

test("a save in the Resources modal refreshes the results table behind it", async ({
  resources,
  page,
}) => {
  const family = "Zz" + Date.now().toString(36);
  await resources.goto("Patient");
  await page.fill(".query-builder__url", `GET /Patient?family=${family}`);
  await page.click('[data-intent="run"]');
  await expect(page.locator("#query-results")).toBeVisible();
  await expect(page.locator("#query-results-body tr")).toHaveCount(0);

  await resources.openCreate();
  const ed = resources.modal.editor;
  await ed.applyJson({ resourceType: "Patient", name: [{ family }] });
  await page.click("#resource-save");

  if (process.env.HFS_E2E_EVENTUAL_SEARCH === "1") {
    // Eventually-consistent search (the Elasticsearch matrix legs): the
    // auto-refresh fires before the index catches the write, so the strict
    // no-manual-rerun contract cannot hold — poll by re-running instead.
    // Save keeps the modal open, and the run button sits behind it: close
    // first or the click never becomes actionable. Wait for the save to
    // settle (saved announcement, unsaved pill gone) before closing: a modal
    // still dirty mid-save raises the discard confirmation, which nobody
    // would answer.
    await expect(resources.modal.announce).toContainText(/saved/i);
    await expect(resources.modal.unsavedCue).toBeHidden();
    await resources.modal.close();
    await expect
      .poll(
        async () => {
          await page.click('[data-intent="run"]');
          return page.locator("#query-results-body tr").count();
        },
        { timeout: 15_000 },
      )
      .toBe(1);
  } else {
    // No reload, no manual re-run: the table catches up on its own.
    await expect(page.locator("#query-results-body tr")).toHaveCount(1, { timeout: 5000 });
  }
});
