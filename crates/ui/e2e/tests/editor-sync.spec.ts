import { test, expect } from "../pages/fixtures";
import { Editor } from "../pages/editor";

// The linked editor: the guided form and the JSON view point at each other
// (editor-sync.js). Hover/focus a form row and the node's JSON lines light
// up; hover or click a JSON line and the row that edits it answers. With
// "Edit raw" open the same link runs through the textarea: valid JSON
// re-renders the form live, the caret drives the row highlight, and the
// hovered row's character range is marked through the mirror.

function standalone(page: import("@playwright/test").Page): Editor {
  return new Editor(page, page.locator("#editor-body"));
}

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

  // Leaving raw shows the previous fold view immediately; the re-render with
  // the big document lands a beat later — wait for its deepest row.
  await page.locator('.editor-row[data-path="identifier.39.value"]').waitFor();

  const jsonView = page.locator("#json-view");
  const tree = page.locator(".editor-tree");
  expect(await jsonView.evaluate((n) => n.scrollHeight > n.clientHeight)).toBe(true);
  expect(await tree.evaluate((n) => n.scrollHeight > n.clientHeight)).toBe(true);

  // Hovering a row deep in the form pulls the JSON pane down to its lines…
  await tree.evaluate((n) => (n.scrollTop = n.scrollHeight));
  await page.locator('.editor-row[data-path="identifier.39.value"]').hover();
  await expect.poll(() => jsonView.evaluate((n) => n.scrollTop)).toBeGreaterThan(0);
  const hit = page.locator('.json-line--hit[data-jpath="identifier.39.value"]');
  expect(await hit.evaluate((n) => {
    const view = document.getElementById("json-view")!;
    const line = n.getBoundingClientRect();
    const pane = view.getBoundingClientRect();
    return line.top >= pane.top && line.bottom <= pane.bottom;
  })).toBe(true);

  // …and hovering a deep JSON line pulls the form pane to its row.
  await jsonView.evaluate((n) => (n.scrollTop = n.scrollHeight));
  await tree.evaluate((n) => (n.scrollTop = 0));
  await page.locator('.json-line[data-jpath="identifier.35.value"]').hover();
  await expect.poll(() => tree.evaluate((n) => n.scrollTop)).toBeGreaterThan(0);
  await expect(page.locator('.editor-row--hit[data-path="identifier.35.value"]')).toHaveCount(1);
});

test("hovering a form row lights the node's JSON lines, and back", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", gender: "female", name: [{ family: "Sync" }] });

  await page.locator('.editor-row[data-path="gender"]').hover();
  await expect(page.locator('.json-line--hit[data-jpath="gender"]')).toHaveCount(1);

  // Moving to another row moves the highlight; a parent row lights its whole subtree.
  await page.locator('.editor-row[data-path="name.0"]').hover();
  await expect(page.locator('.json-line--hit[data-jpath="gender"]')).toHaveCount(0);
  await expect(page.locator('.json-line--hit[data-jpath="name.0.family"]')).toHaveCount(1);

  // The reverse direction: hovering a JSON line lights the row that edits it.
  await page.locator('.json-line[data-jpath="gender"]').hover();
  await expect(page.locator('.editor-row--hit[data-path="gender"]')).toHaveCount(1);
});

test("clicking a JSON line focuses the row's input", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.applyJson({ resourceType: "Patient", gender: "female" });

  await page.locator('.json-line[data-jpath="gender"] .json-line__code').click();
  await expect(page.locator('[data-set="gender"]')).toBeFocused();
});

test("valid raw JSON re-renders the guided form without leaving raw mode", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.enterRaw();
  await ed.source.fill(JSON.stringify({ resourceType: "Patient", gender: "male" }));

  // The form catches up on its own (debounced live sync)…
  await expect(page.locator('[data-set="gender"]')).toHaveCount(1, { timeout: 5000 });
  // …and raw mode is still the active view.
  await expect(page.locator("#editor-json-raw")).toBeVisible();

  // The other direction: a guided edit refreshes the textarea in place
  // instead of snapping back to the fold view.
  await page.fill('[data-set="gender"]', "female");
  await page.locator('[data-set="gender"]').evaluate((n) => (n as HTMLElement).blur());
  await expect(page.locator("#editor-json-raw")).toBeVisible();
  await expect
    .poll(async () => (await ed.source.inputValue()).includes('"female"'))
    .toBe(true);
});

test("the raw-mode caret lights the row of the node it sits in", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.enterRaw();
  const doc = JSON.stringify({ resourceType: "Patient", id: "x", gender: "male" }, null, 2);
  await ed.source.fill(doc);
  await expect(page.locator('[data-set="gender"]')).toHaveCount(1, { timeout: 5000 });

  await ed.source.click();
  const caret = doc.indexOf('"male"') + 2;
  await ed.source.evaluate((n, at) => (n as HTMLTextAreaElement).setSelectionRange(at, at), caret);
  await page.keyboard.press("ArrowRight");
  await expect(page.locator('.editor-row--hit[data-path="gender"]')).toHaveCount(1, { timeout: 3000 });
});

test("with raw open, hovering a form row marks the node's text in the mirror", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = standalone(page);
  await ed.enterRaw();
  await ed.source.fill(JSON.stringify({ resourceType: "Patient", gender: "male" }, null, 2));
  await expect(page.locator('[data-set="gender"]')).toHaveCount(1, { timeout: 5000 });

  await page.locator('.editor-row[data-path="gender"]').hover();
  const mark = page.locator(".editor__source-mirror mark");
  await expect(mark).toHaveCount(1);
  await expect(mark).toContainText('"gender"');

  // The mirror is an overlay, never a click target: the textarea under it
  // still takes the click.
  await ed.source.click({ position: { x: 30, y: 10 } });
  await expect(ed.source).toBeFocused();
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
    // first or the click never becomes actionable.
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
