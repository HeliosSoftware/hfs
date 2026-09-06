// SQL Queries workspace (#649): a stored SQLQuery Library lists in the rail,
// its SQL decodes into the editor pane, and the results region runs it over
// its depends-on ViewDefinition through $sql-run on arrival — no Run button
// (#839, generalizing #752's View Definitions playground here).
import { expect, test } from "../pages/fixtures";
import { createResource, createSqlQueryLibrary, readResource, waitSearchable } from "../pages/api";
import { Editor } from "../pages/editor";

test("a stored SQLQuery lists, decodes its SQL, and previews rows on arrival", async ({ page, request }) => {
  const patientId = await createResource(request, "Patient", { name: [{ family: "SqlLibE2E" }] });
  const canonical = `http://example.org/ViewDefinition/e2e-lib-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_lib_patients",
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  const sql = "SELECT COUNT(*) AS n FROM v";
  const libId = await createResource(request, "Library", {
    name: "e2e_patient_count",
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
    relatedArtifact: [{ type: "depends-on", resource: canonical, label: "v" }],
    content: [
      { contentType: "application/sql", data: Buffer.from(sql).toString("base64") },
    ],
  });

  // ES composites index asynchronously: the rail, the depends-on
  // resolution, and the run preview all read through search (#596).
  await waitSearchable(request, "Library", libId);
  await waitSearchable(request, "Patient", patientId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  await expect(page.locator(`#lib-rail-list [data-type='${libId}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
  // The SQL pane holds the decoded query, not base64.
  await expect(page.locator("textarea[name='sql']")).toContainText("SELECT COUNT(*)");

  const createNew = page.locator("a[href$='?lib=new']");
  await expect(createNew).toHaveClass(/\bbtn--primary\b/);
  await expect(createNew).not.toHaveClass(/\bbtn--accent\b/);
  await expect(createNew).toHaveCSS("height", "30px");
  await expect(createNew).toHaveCSS("padding-left", "12px");

  // The results region loads itself on arrival (#839) — no click, no Run
  // link at all.
  await expect(page.locator("a[href*='run=1']")).toHaveCount(0);
  await expect(page.locator("#run-results .data-table")).toBeVisible();
  await expect(page.locator("#run-results .data-table th", { hasText: "n" }).first()).toBeVisible();
});

/** Seeds a Library of `code` ("sql-query" | "sql-view") holding `sql`. */
async function createSqlLibrary(request: import("@playwright/test").APIRequestContext, code: string, sql: string) {
  const libId = await createResource(request, "Library", {
    name: `e2e_${code.replace("-", "_")}_${Date.now()}`,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code,
        },
      ],
    },
    content: [{ contentType: "application/sql", data: Buffer.from(sql).toString("base64") }],
  });
  await waitSearchable(request, "Library", libId);
  return libId;
}

/**
 * #838: the SQL pane's CodeMirror editor — mounted by
 * sql-editor.js over `textarea[name='sql']` on both /ui/sql/queries and
 * /ui/sql/views (one template, `sql-library.html`, serves both kinds). Closes
 * the same kind of test gap #820 originally left for the ViewDefinition
 * editor: a real typing round-trip, plus the token coloring and
 * theme-follow behavior this editor promises.
 */
test("the SQL editor highlights keywords, follows the theme, and syncs typed keystrokes to the hidden textarea", async ({
  page,
  request,
}) => {
  const sql = "SELECT id FROM v WHERE ward = :ward";
  const queryLibId = await createSqlLibrary(request, "sql-query", sql);

  await page.goto(`/ui/sql/queries?lib=${queryLibId}`);

  const textarea = page.locator("textarea[name='sql']");
  const editor = page.locator(".sql-editor .cm-content[role='textbox']");
  await expect(editor).toBeVisible();
  await expect(textarea).toBeHidden();
  // The decoded SQL, not the base64 attachment data.
  await expect(editor).toContainText(sql);

  // At least one keyword token gets its own class.
  const keyword = editor.locator(".cmt-sql-keyword").first();
  await expect(keyword).toBeVisible();

  // Purely CSS-variable-driven — toggling [data-theme] recolors the
  // token with no reload and no theme logic in sql-editor.js itself.
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "light"));
  const lightColor = await keyword.evaluate((el) => getComputedStyle(el).color);
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  const darkColor = await keyword.evaluate((el) => getComputedStyle(el).color);
  expect(darkColor).not.toBe(lightColor);

  // Every keystroke lands in the hidden textarea; Save posts exactly
  // that and the redirect renders it back into both the editor and the
  // textarea.
  const updated = "SELECT name FROM v WHERE active = 1";
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.insertText(updated);
  await expect(textarea).toHaveValue(updated);

  await page.locator("button[name='action'][value='save']").click();
  await page.waitForURL(/saved=1/);
  await expect(page.locator("textarea[name='sql']")).toHaveValue(updated);
  await expect(page.locator(".sql-editor .cm-content")).toContainText(updated);

  // The same mount happens on /ui/sql/views for a sql-view Library.
  const viewLibId = await createSqlLibrary(request, "sql-view", sql);
  await page.goto(`/ui/sql/views?lib=${viewLibId}`);
  const viewEditor = page.locator(".sql-editor .cm-content[role='textbox']");
  await expect(viewEditor).toBeVisible();
  await expect(viewEditor).toContainText(sql);
  await expect(viewEditor.locator(".cmt-sql-keyword").first()).toBeVisible();
});

/** A minimal savable sql-query Library, named for the rail. */
function starterLibrary(name: string) {
  return {
    name,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
  };
}

// "Recently used" group (#754/#755): SQL Queries restores its own stored
// `last` on plain arrival, and an explicit `?lib=` deep link always wins
// over it — the same resolution order the View Definitions rail proves,
// exercised here through the Library-backed page instead.
test("restores the stored last selection on plain arrival; a deep link with ?lib= wins", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const libA = await createResource(request, "Library", starterLibrary(`zq_${stamp}_a`));
  const libB = await createResource(request, "Library", starterLibrary(`zq_${stamp}_b`));
  await Promise.all([libA, libB].map((id) => waitSearchable(request, "Library", id)));

  await page.goto(`/ui/sql/queries?lib=${libA}`);
  await page.goto("/ui/sql/queries");
  await expect(page.locator(`#lib-rail-list [data-type='${libA}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );

  // A deep link always wins over the stored last, even to a different item.
  await page.goto(`/ui/sql/queries?lib=${libB}`);
  await expect(page.locator(`#lib-rail-list [data-type='${libB}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
});

// Editor-first layout (#839): the results card follows the SQL editor's
// *current* text, live, on both SQL Queries and SQL Views — the same
// live-preview contract View Definitions proves for its own JSON editor.
// Both a saved-then-edited Library and its title row's chips are exercised
// once per kind, over the very same depends-on ViewDefinition.
const LIVE_RUN_KINDS = [
  { code: "sql-query", path: "/ui/sql/queries", failed: "Could not run the query" },
  { code: "sql-view", path: "/ui/sql/views", failed: "Could not run the view" },
] as const;

for (const { code, path, failed } of LIVE_RUN_KINDS) {
  test(`${path}: editing the SQL in CodeMirror refreshes the results live, reports a broken edit, and recovers`, async ({
    page,
    request,
  }) => {
    const patientId = await createResource(request, "Patient", {
      name: [{ family: `SqlLiveE2E_${code}` }],
    });
    const canonical = `http://example.org/ViewDefinition/e2e-live-${code}-${Date.now()}`;
    await createResource(request, "ViewDefinition", {
      name: `e2e_live_${code.replace("-", "_")}_source`,
      url: canonical,
      status: "active",
      resource: "Patient",
      where: [{ path: `name.family = 'SqlLiveE2E_${code}'` }],
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    await waitSearchable(request, "Patient", patientId);

    const libId = await createResource(request, "Library", {
      name: `e2e_live_${code.replace("-", "_")}_${Date.now()}`,
      status: "active",
      type: {
        coding: [
          {
            system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
            code,
          },
        ],
      },
      relatedArtifact: [{ type: "depends-on", resource: canonical, label: "v" }],
      content: [
        {
          contentType: "application/sql",
          data: Buffer.from("SELECT id AS pid FROM v").toString("base64"),
        },
      ],
    });
    await waitSearchable(request, "Library", libId);

    await page.goto(`${path}?lib=${libId}`);

    // The title row's two chips (#839).
    const titleRow = page.locator("h2.page-head__title--kind");
    await expect(titleRow.locator(".tag--type")).toBeVisible();
    await expect(titleRow.locator(".tag--active")).toHaveText("active");

    await expect(page.locator("#run-results .data-table th")).toHaveText(["pid"]);
    await expect(page.locator("#run-results .data-table td", { hasText: patientId }).first()).toBeVisible();

    // Replacing the SQL with another valid query — a different column —
    // refreshes the table live, with no navigation.
    const editor = page.locator(".sql-editor .cm-content[role='textbox']");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("SELECT id AS newcol FROM v");
    await expect(page.locator("#run-results .data-table th")).toHaveText(["newcol"], {
      timeout: 3000,
    });
    await expect(page).toHaveURL(new RegExp(`lib=${libId}$`));
    await expect(page.locator("#run-results-meta")).toHaveText(/^\d+ rows · \d+ ms$/);

    // Invalid SQL reports the failure, keeps the last good table on screen,
    // and relabels its meta.
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("SELECT id AS newcol FRM v");
    await expect(page.locator(".notice--warn")).toContainText(failed, { timeout: 3000 });
    await expect(page.locator("#run-results .data-table th")).toHaveText(["newcol"]);
    await expect(page.locator("#run-results-meta")).toHaveText("last successful run");

    // Fixing the SQL clears the notice and refreshes the meta again.
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("SELECT id AS newcol FROM v");
    await expect(page.locator(".notice--warn")).toHaveCount(0, { timeout: 3000 });
    await expect(page.locator("#run-results-meta")).toHaveText(/^\d+ rows · \d+ ms$/);

    // Export as files: only SQL Query offers it, only with a saved id.
    const exportLink = page.locator(`a[href="/ui/sql/export/new?subject=Library/${libId}"]`);
    if (code === "sql-query") {
      await expect(exportLink).toBeVisible();
    } else {
      await expect(exportLink).toHaveCount(0);
    }
  });
}

// #839: a sqlparser parse failure's `data-error-line` (extracted server-side
// from `… at Line: N, Column: M`, sql_views::extract_error_line) tints that
// line in the mounted CodeMirror editor — sql-editor.js's own
// `htmx:afterSwap` listener on `#run-notice`. A SQLite execution error (a
// valid statement referencing an unknown column) carries no line at all, so
// nothing gets tinted for that case either.
test("a parse error's line is tinted in the SQL editor; an execution error and a fix both clear it", async ({
  page,
  request,
}) => {
  const patientId = await createResource(request, "Patient", { name: [{ family: "SqlLineE2E" }] });
  const canonical = `http://example.org/ViewDefinition/e2e-line-${Date.now()}`;
  await createResource(request, "ViewDefinition", {
    name: "e2e_line_source",
    url: canonical,
    status: "active",
    resource: "Patient",
    where: [{ path: "name.family = 'SqlLineE2E'" }],
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "Patient", patientId);

  const libId = await createResource(request, "Library", {
    name: `e2e_line_${Date.now()}`,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
    relatedArtifact: [{ type: "depends-on", resource: canonical, label: "v" }],
    content: [
      {
        contentType: "application/sql",
        data: Buffer.from("SELECT id AS pid FROM v").toString("base64"),
      },
    ],
  });
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  await expect(page.locator("#run-results .data-table th")).toHaveText(["pid"]);

  const editor = page.locator(".sql-editor .cm-content[role='textbox']");
  const notice = page.locator(".notice--warn");
  const lines = page.locator(".sql-editor .cm-line");
  const taggedLines = page.locator(".sql-editor .cm-line.sql-editor__error-line");

  // Two lines, the second one broken ("FRM" — sqlparser's own error names
  // line 2). The first line is untouched.
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText("SELECT id");
  await page.keyboard.press("Enter");
  await page.keyboard.insertText("FRM v");
  await expect(notice).toHaveAttribute("data-error-line", "2", { timeout: 3000 });
  await expect(lines.nth(1)).toHaveClass(/\bsql-editor__error-line\b/);
  await expect(lines.nth(0)).not.toHaveClass(/\bsql-editor__error-line\b/);

  // A SQLite execution error (unknown column, still valid SQL) reports a
  // failure with no line — nothing to tint.
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText("SELECT nope FROM v");
  await expect(notice).toBeVisible({ timeout: 3000 });
  await expect(notice).not.toHaveAttribute("data-error-line");
  await expect(taggedLines).toHaveCount(0);

  // Back to valid SQL: the notice clears and so does any tint.
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText("SELECT id AS pid FROM v");
  await expect(notice).toHaveCount(0, { timeout: 3000 });
  await expect(taggedLines).toHaveCount(0);
});

// Details (#840): the JSON editor + guided-form pairing over the Library
// minus its SQL attachment — the same shared host (`editor-pair.js`) View
// Definitions proves in sql-view-definitions.spec.ts, exercised here for
// the Library-backed pages. Both routes share one template, so a route not
// named below behaves identically — only the gate test (route-specific by
// nature) exercises both.
test.describe("Details", () => {
  test("editing the guided form updates the JSON pane, and Save persists the merged document", async ({
    page,
    request,
  }) => {
    const canonical = `http://example.org/ViewDefinition/e2e-details-${Date.now()}`;
    await createResource(request, "ViewDefinition", {
      name: "e2e_details_source",
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const libId = await createSqlQueryLibrary(request, `e2e_details_${Date.now()}`, canonical);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const ed = new Editor(page, page.locator("#lib-details-grid"));
    const nameField = ed.rowAt("name").locator("[data-set='name']");
    await nameField.fill("e2e_details_renamed");
    await nameField.blur();

    const jsonPane = page.locator("textarea[name='json']");
    await expect(jsonPane).toHaveValue(/e2e_details_renamed/, { timeout: 3000 });
    // The SQL attachment never shows up in the Details JSON pane.
    expect(await jsonPane.inputValue()).not.toContain("application/sql");

    await page.locator("button[name='action'][value='save']").click();
    await page.waitForURL(new RegExp(`lib=${libId}&saved=1`));
    await expect(page.locator("h2.page-head__title--kind")).toContainText("e2e_details_renamed");

    const saved = await readResource(request, "Library", libId);
    expect(saved.name).toBe("e2e_details_renamed");
    const content = saved.content as Array<{ contentType: string }>;
    expect(content.some((a) => a.contentType === "application/sql")).toBe(true);
  });

  test("an invalid value typed in the JSON pane errors on its row and reports the issue count, without saving; fixing it clears both", async ({
    page,
    request,
  }) => {
    const libId = await createResource(
      request,
      "Library",
      starterLibrary(`e2e_details_bogus_${Date.now()}`),
    );
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const ed = new Editor(page, page.locator("#lib-details-grid"));
    await expect(ed.validity).toContainText("No issues");

    const cmContent = page.locator("#lib-details-editor .cm-content");
    const before = await page.locator("textarea[name='json']").inputValue();
    const broken = before.replace('"status": "active"', '"status": "bogus"');

    await cmContent.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText(broken);
    await expect(ed.rowAt("status")).toHaveClass(/editor-row--error/, { timeout: 3000 });
    await expect(ed.validity).toContainText("1 issue");

    // Nothing was ever posted to Save.
    const untouched = await readResource(request, "Library", libId);
    expect(untouched.status).toBe("active");

    await cmContent.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText(before);
    await expect(ed.rowAt("status")).not.toHaveClass(/editor-row--error/, { timeout: 3000 });
    await expect(ed.validity).toContainText("No issues");
  });

  test("Save fuses an edited SQL pane and an edited Details title into one Library", async ({
    page,
    request,
  }) => {
    const canonical = `http://example.org/ViewDefinition/e2e-merge-${Date.now()}`;
    await createResource(request, "ViewDefinition", {
      name: "e2e_merge_source",
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const libId = await createSqlQueryLibrary(request, `e2e_merge_${Date.now()}`, canonical, "SELECT 1");

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const ed = new Editor(page, page.locator("#lib-details-grid"));
    const nameField = ed.rowAt("name").locator("[data-set='name']");
    await nameField.fill("e2e_merge_renamed");
    await nameField.blur();
    await expect(page.locator("textarea[name='json']")).toHaveValue(/e2e_merge_renamed/, {
      timeout: 3000,
    });

    const sqlEditor = page.locator(".sql-editor .cm-content[role='textbox']");
    await sqlEditor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("SELECT 2");

    await page.locator("button[name='action'][value='save']").click();
    await page.waitForURL(new RegExp(`lib=${libId}&saved=1`));

    const saved = await readResource(request, "Library", libId);
    expect(saved.name).toBe("e2e_merge_renamed");
    const content = saved.content as Array<{ contentType: string; data: string }>;
    const sqlAttachment = content.find((a) => a.contentType === "application/sql");
    expect(sqlAttachment).toBeTruthy();
    expect(Buffer.from(sqlAttachment!.data, "base64").toString()).toBe("SELECT 2");
  });

  test("Ctrl+Z after a guided-form edit restores the previous JSON as one step", async ({
    page,
    request,
  }) => {
    const libId = await createResource(
      request,
      "Library",
      starterLibrary(`e2e_details_undo_${Date.now()}`),
    );
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const textarea = page.locator("textarea[name='json']");
    const before = await textarea.inputValue();

    const ed = new Editor(page, page.locator("#lib-details-grid"));
    const statusField = ed.rowAt("status").locator("[data-set='status']");
    await statusField.fill("retired");
    await statusField.blur();
    await expect(textarea).toHaveValue(/retired/, { timeout: 3000 });
    const after = await textarea.inputValue();
    expect(after).not.toBe(before);

    await page.locator("#lib-details-editor .cm-content").click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(textarea).toHaveValue(before);
  });

  test("the JSON and guided-form cards share one height and each scrolls inside itself", async ({
    page,
    request,
  }) => {
    // Enough relatedArtifact entries to make both the JSON text and the
    // guided-form rows tall — the same document, so this only proves the
    // shared-height/scroll-inside contract, not that the two cards' heights
    // are independent (View Definitions' own version of this test proves
    // that independence with two different fields).
    const relatedArtifact = Array.from({ length: 30 }, (_, i) => ({
      type: "depends-on",
      resource: `http://example.org/ViewDefinition/e2e-stretch-${i}`,
      label: `v${i}`,
    }));
    const libId = await createResource(request, "Library", {
      ...starterLibrary(`e2e_details_stretch_${Date.now()}`),
      relatedArtifact,
    });
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const grid = page.locator("#lib-details-grid");
    const cards = grid.locator("> .card");
    await expect(cards).toHaveCount(2);

    const [jsonBox, formBox] = await Promise.all([
      cards.nth(0).boundingBox(),
      cards.nth(1).boundingBox(),
    ]);
    expect(jsonBox).not.toBeNull();
    expect(formBox).not.toBeNull();
    expect(Math.abs(jsonBox!.height - formBox!.height)).toBeLessThanOrEqual(1);

    const viewportHeight = page.viewportSize()!.height;
    expect(jsonBox!.height).toBeLessThanOrEqual(viewportHeight * 0.7 + 1);

    const scroller = page.locator("#lib-details-editor .cm-scroller");
    const tree = page.locator("#lib-details-grid .editor-tree");
    await expect
      .poll(async () => scroller.evaluate((el) => el.scrollHeight - el.clientHeight))
      .toBeGreaterThan(0);
    await expect
      .poll(async () => tree.evaluate((el) => el.scrollHeight - el.clientHeight))
      .toBeGreaterThan(0);

    const overflowsX = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
    );
    expect(overflowsX).toBe(false);
  });

  // The Save gate (#840): a document whose type code names the other kind
  // is rejected with a warning naming the route's own expected code —
  // exercised on both routes, since each has its own expected code.
  const GATE_CASES = [
    { path: "/ui/sql/queries", wrongCode: "sql-view", expectedCode: "sql-query" },
    { path: "/ui/sql/views", wrongCode: "sql-query", expectedCode: "sql-view" },
  ] as const;

  for (const { path, wrongCode, expectedCode } of GATE_CASES) {
    test(`${path}: changing the type code to the other kind rejects Save with a warning naming "${expectedCode}"`, async ({
      page,
      request,
    }) => {
      const libId = await createResource(
        request,
        "Library",
        starterLibrary(`e2e_details_gate_${Date.now()}`),
      );
      await waitSearchable(request, "Library", libId);

      await page.goto(`${path}?lib=${libId}`);
      const ed = new Editor(page, page.locator("#lib-details-grid"));
      const codeField = ed.rowAt("type.coding.0.code").locator("[data-set='type.coding.0.code']");
      await codeField.fill(wrongCode);
      await codeField.blur();
      await expect(page.locator("textarea[name='json']")).toHaveValue(new RegExp(wrongCode), {
        timeout: 3000,
      });

      await page.locator("button[name='action'][value='save']").click();
      // Scoped to the title row's own warning, a direct child of
      // `.filter-center` — `.notice--warn` also matches a live-preview
      // failure nested inside `#run-notice` (this starter Library has no
      // real `relatedArtifact` target, so its own live run fails too).
      const saveNotice = page.locator(".filter-center > p.notice--warn");
      await expect(saveNotice).toContainText(expectedCode);
      await expect(page).toHaveURL(new RegExp(path));

      // Nothing was saved.
      const untouched = await readResource(request, "Library", libId);
      expect(untouched.type).toMatchObject({
        coding: [{ code: "sql-query" }],
      });
    });
  }
});

// Parameters card (#841, SQL Query only): declare an undeclared placeholder,
// bind it a value, watch the live run react, and undo the declaration.
test.describe("Parameters card", () => {
  test("declare → bind → results: an undeclared :fam placeholder is declared, filled, cleared, and undone", async ({
    page,
    request,
  }) => {
    const family = `Garcia_${Date.now()}`;
    await createResource(request, "Patient", { name: [{ family }] });
    const canonical = `http://example.org/ViewDefinition/e2e-params-${Date.now()}`;
    await createResource(request, "ViewDefinition", {
      name: "e2e_params_source",
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
      `e2e_params_${Date.now()}`,
      canonical,
      "SELECT id, family FROM v WHERE family = :fam",
    );

    await page.goto(`/ui/sql/queries?lib=${libId}`);

    // No `Library.parameter[]` declared: the card shows the hint, not a
    // value field, and its own Declare button names the placeholder.
    const paramsCard = page.locator("#lib-params");
    await expect(paramsCard).toBeVisible();
    await expect(paramsCard).toContainText(":fam is used in the SQL but not declared");
    const declareButton = paramsCard.getByRole("button", { name: "Declare :fam" });
    await expect(declareButton).toBeVisible();
    await expect(paramsCard.locator("input[name='param:fam']")).toHaveCount(0);

    await declareButton.click();

    // Declaring writes the parameter into the Details JSON — unsaved — and
    // the card now shows a value field for it, form-associated with the
    // same editor form the SQL/JSON panes submit through.
    const jsonPane = page.locator("textarea[name='json']");
    await expect(jsonPane).toHaveValue(/"name": "fam"/, { timeout: 3000 });
    const famField = page.locator("input[name='param:fam']");
    await expect(famField).toBeVisible();
    await expect(famField).toHaveAttribute("form", "lib-editor-form");
    await expect(paramsCard.getByRole("button", { name: "Declare :fam" })).toHaveCount(0);

    // Required, no default, no value yet: the run waits rather than calling
    // $sql-run, and the previous (never-run) results stay as they are.
    await expect(page.locator("#run-notice")).toContainText("Waiting for a value for :fam", {
      timeout: 3000,
    });

    // A value fills the wait: the table shows the matching row.
    await famField.fill(family);
    await expect(page.locator("#run-results .data-table td", { hasText: family })).toBeVisible({
      timeout: 3000,
    });
    await expect(page.locator("#run-notice")).not.toContainText("Waiting for a value", {
      timeout: 3000,
    });

    // Clearing it waits again — the last successful table is left in place.
    await famField.fill("");
    await expect(page.locator("#run-notice")).toContainText("Waiting for a value for :fam", {
      timeout: 3000,
    });
    await expect(page.locator("#run-results .data-table td", { hasText: family })).toBeVisible();

    // Ctrl+Z in Details undoes the declaration as one step.
    await page.locator("#lib-details-editor .cm-content").click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(jsonPane).not.toHaveValue(/"name": "fam"/, { timeout: 3000 });

    // Nothing was ever saved — the declaration only ever lived in the
    // editor's own unsaved document.
    const untouched = await readResource(request, "Library", libId);
    expect(untouched.parameter).toBeUndefined();
  });

  // #841's own signature rule (NF2): the Parameters card only travels back
  // over `hx-swap-oob` when what it declares actually changed — editing
  // Details (here, the Library's own `name`, never `parameter[]`) still
  // re-fires the live run, but the card itself is left alone. Proven with a
  // DOM-identity marker rather than focus (editing the *Details* `name`
  // field necessarily focuses and blurs *that* field first, so "was
  // `param:ward` focused before this test ever touched Details" is not a
  // meaningful question) — the marker survives only if `#lib-params`'s own
  // root element was never torn out and replaced, which an `outerHTML` swap
  // always does regardless of what value the replacement itself carries.
  test("editing Details re-renders the live run without swapping the Parameters card, and a value input keeps focus once re-given it", async ({
    page,
    request,
  }) => {
    const canonical = `http://example.org/ViewDefinition/e2e-params-focus-${Date.now()}`;
    await createResource(request, "ViewDefinition", {
      name: "e2e_params_focus_source",
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const libId = await createSqlQueryLibrary(
      request,
      `e2e_params_focus_${Date.now()}`,
      canonical,
      "SELECT id FROM v",
      [{ name: "ward", use: "in", type: "string", defaultString: "east" }],
    );
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const paramsCard = page.locator("#lib-params");
    const wardField = page.locator("input[name='param:ward']");
    await expect(wardField).toBeVisible();
    await wardField.fill("west-in-progress");

    // A marker on the card's own root: gone after any `outerHTML` swap of
    // `#lib-params`, regardless of what the replacement renders.
    await paramsCard.evaluate((el) => el.setAttribute("data-e2e-marker", "untouched"));

    // A Details edit that never touches `parameter[]` — the guided form's
    // own round trip, then the JSON pane's own live-preview repost.
    const ed = new Editor(page, page.locator("#lib-details-grid"));
    const nameField = ed.rowAt("name").locator("[data-set='name']");
    const renamed = `e2e_params_focus_renamed_${Date.now()}`;
    await nameField.fill(renamed);
    await nameField.blur();
    await expect(page.locator("textarea[name='json']")).toHaveValue(new RegExp(renamed), {
      timeout: 3000,
    });

    // The live run this triggers settles (a fresh table for the renamed,
    // still-runnable query) well within the window the card's own
    // `hx-swap-oob` companion — had the signature actually changed — would
    // have replaced the card by.
    await expect(page.locator("#run-results-meta")).toHaveText(/^\d+ rows · \d+ ms$/, {
      timeout: 3000,
    });

    // The marker (and so the card's own root element) survived, and the
    // typed-but-unsubmitted value is exactly as left.
    await expect(paramsCard).toHaveAttribute("data-e2e-marker", "untouched");
    await expect(wardField).toHaveValue("west-in-progress");

    // Re-focusing the field now and waiting past every debounce this page
    // schedules confirms nothing swaps it out from under a user who has
    // gone back to typing in it.
    await wardField.click();
    await expect(wardField).toBeFocused();
    await page.waitForTimeout(700);
    await expect(wardField).toBeFocused();
    await expect(paramsCard).toHaveAttribute("data-e2e-marker", "untouched");
  });
});

// Tables panel (#842, both kinds): Reads from / Used by, resolved against
// real storage — no mocked ConformanceSource here, so the combobox's own
// round trip to `/ui/lookup/table-options`, and the resolution
// `document`'s own `add-table` performs, both hit the real server exactly
// as a browser session would.
test.describe("Tables panel", () => {
  test("on a SQL View, Add table searches and selects a ViewDefinition, autofills the alias, adds the resolved row, and Remove clears it; Used by lists the depending SQL Query", async ({
    page,
    request,
  }) => {
    const suffix = Date.now();
    const targetName = `e2e_tables_target_${suffix}`;
    const canonical = `http://example.org/ViewDefinition/e2e-tables-${suffix}`;
    const targetVdId = await createResource(request, "ViewDefinition", {
      name: targetName,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const viewId = await createResource(request, "Library", {
      name: `e2e_tables_view_${suffix}`,
      status: "active",
      type: {
        coding: [
          {
            system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
            code: "sql-view",
          },
        ],
      },
      content: [
        { contentType: "application/sql", data: Buffer.from("SELECT 1").toString("base64") },
      ],
    });
    const dependentQueryId = await createSqlQueryLibrary(
      request,
      `e2e_tables_dependent_${suffix}`,
      `Library/${viewId}`,
      "SELECT * FROM v",
    );
    await waitSearchable(request, "ViewDefinition", targetVdId);
    await waitSearchable(request, "Library", viewId);
    await waitSearchable(request, "Library", dependentQueryId);

    await page.goto(`/ui/sql/views?lib=${viewId}`);
    const tablesCard = page.locator("#lib-tables");
    await expect(tablesCard).toContainText("No tables declared yet.");

    // The disclosure opens with no JavaScript required (a native
    // `<details>`), and works the same with it.
    await tablesCard.locator("details.editor-add > summary").click();
    const search = page.locator('#lib-tables-add-table input[role="combobox"]');
    // The full name, not a timestamp fragment of it: `table_options` runs
    // `name:contains` server-side, so a real name search still narrows to
    // exactly this one ViewDefinition (its name embeds a unique millisecond
    // timestamp) regardless of how many other `e2e_*` ViewDefinitions the
    // rest of the suite has already seeded — matching how a person would
    // actually search rather than an arbitrary substring of an id-like
    // suffix. A generous timeout absorbs the extra round-trip latency a
    // heavily-loaded server (many prior suites, hundreds of resources) adds
    // on top of the combobox's own 300ms debounce.
    await search.fill(targetName);
    const option = page.locator("#lib-tables-add-table [data-combobox-option]", {
      hasText: targetName,
    });
    await expect(option).toBeVisible({ timeout: 10000 });
    await option.click();

    // #842: choosing the option filled the alias with the artifact's own
    // bare name — the combobox's own label carries a " — ViewDefinition"
    // suffix this field must not pick up.
    const aliasField = page.locator("input[name='table_alias']");
    await expect(aliasField).toHaveValue(targetName);

    await page.locator("button[name='op'][value='add-table']").click();

    // The row resolved: chip, link to the ViewDefinition's own page, and
    // the JSON pane (unsaved) now carries the depends-on entry.
    const row = tablesCard.locator("tr", { hasText: targetName });
    await expect(row).toBeVisible({ timeout: 3000 });
    await expect(row.locator("a")).toHaveAttribute("href", `/ui/sql/view-definitions?vd=${targetVdId}`);
    const jsonField = page.locator("textarea[name='json']");
    await expect(jsonField).toHaveValue(new RegExp(canonical.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")), {
      timeout: 3000,
    });

    // Used by: the SQL Query created above, depending on this SQL View by
    // `Library/{id}`.
    const usedBy = page.locator("#lib-tables", { hasText: "Used by" });
    await expect(usedBy.locator("a", { hasText: `e2e_tables_dependent_${suffix}` })).toBeVisible();

    // Remove clears the row and the JSON entry again.
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(tablesCard.locator("tr", { hasText: targetName })).toHaveCount(0, {
      timeout: 3000,
    });
    await expect(jsonField).not.toHaveValue(new RegExp(canonical.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")), {
      timeout: 3000,
    });

    // Nothing here was ever saved.
    const untouched = await readResource(request, "Library", viewId);
    expect(untouched.relatedArtifact).toBeUndefined();
  });

  test("Reads from marks the starter change-me dependency as Not found; removing it turns v into an unknown table (#842/04)", async ({
    page,
  }) => {
    // `?lib=new`'s starter document (`sql_libraries::starter_library_value`)
    // ships one placeholder dependency — alias `v`, resource
    // `http://example.org/ViewDefinition/change-me` — that nothing in a
    // fresh server ever answers to, so *Reads from* renders it unresolved
    // from the very first paint, with no setup needed.
    await page.goto("/ui/sql/queries?lib=new");
    const tablesCard = page.locator("#lib-tables");
    const row = tablesCard.locator("tbody tr");
    await expect(row).toHaveCount(1);
    await expect(row.locator("code")).toHaveText("v");
    await expect(row.locator(".tag--failed")).toHaveText("Not found");
    await expect(row).toContainText(
      "No ViewDefinition or SQL View answers to http://example.org/ViewDefinition/change-me. Fix the canonical in Details or remove the row.",
    );

    // #842/04: the starter's own SQL ("SELECT * FROM v") still reads `v` —
    // removing its only declaration does not empty the card, it turns `v`
    // into an unknown table instead (the SQL is unchanged, only what is
    // *declared* is).
    await row.getByRole("button", { name: "Remove" }).click();
    await expect(tablesCard.locator("tbody tr")).toHaveCount(1);
    const unknownRow = tablesCard.locator("tbody tr");
    await expect(unknownRow.locator("code")).toHaveText("v");
    await expect(unknownRow.locator(".tag--failed")).toHaveText("Unknown table");
    await expect(unknownRow.getByRole("button", { name: "Declare v" })).toBeVisible();
  });

  test("Add table rejects a duplicate alias inline and keeps the panel open", async ({
    page,
    request,
  }) => {
    const suffix = Date.now();
    const canonical = `http://example.org/ViewDefinition/e2e-tables-dup-${suffix}`;
    const otherName = `e2e_tables_dup_other_${suffix}`;
    const otherCanonical = `http://example.org/ViewDefinition/e2e-tables-dup-other-${suffix}`;
    await createResource(request, "ViewDefinition", {
      name: `e2e_tables_dup_source_${suffix}`,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const otherVdId = await createResource(request, "ViewDefinition", {
      name: otherName,
      url: otherCanonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    // Already declares alias `v` — the collision *Add table* must catch
    // before ever touching the (unsaved) document.
    const libId = await createSqlQueryLibrary(
      request,
      `e2e_tables_dup_${suffix}`,
      canonical,
      "SELECT * FROM v",
    );
    await waitSearchable(request, "ViewDefinition", otherVdId);
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    const tablesCard = page.locator("#lib-tables");
    await expect(tablesCard.locator("tbody tr")).toHaveCount(1);

    await tablesCard.locator("details.editor-add > summary").click();
    const search = page.locator('#lib-tables-add-table input[role="combobox"]');
    await search.fill(otherName);
    const option = page.locator("#lib-tables-add-table [data-combobox-option]", {
      hasText: otherName,
    });
    await expect(option).toBeVisible({ timeout: 10000 });
    await option.click();

    // Overwrite the autocompleted alias with the label the Library already
    // declares — the exact case-sensitive match this Library's SQL binds
    // to, so the rejection cannot be mistaken for the case-insensitive
    // "V" vs "v" check `router_http.rs`'s own test already covers.
    const aliasField = page.locator("input[name='table_alias']");
    await aliasField.fill("v");
    await page.locator("button[name='op'][value='add-table']").click();

    // The panel stays open with the error inline — no ghost row, no
    // document mutation, `<details>` never closes on a rejected submit.
    await expect(page.locator("#lib-tables-add-error")).toHaveText("Alias v is already declared");
    await expect(tablesCard.locator("details.editor-add")).toHaveAttribute("open", "");
    await expect(tablesCard.locator("tbody tr")).toHaveCount(1);
    await expect(tablesCard.locator("tr", { hasText: otherName })).toHaveCount(0);
  });
});

// Unknown-table lint and Columns (#842/04): the live run's own gate ahead of
// $sql-run — a table the SQL reads that no dependency declares never
// runs at all — and the Columns card it feeds once a run actually succeeds.
test.describe("Unknown-table lint and Columns", () => {
  test("a typo in the SQL is linted live, Declare opens Add table, and resolving it clears the lint and fills Columns", async ({
    page,
    request,
  }) => {
    const suffix = Date.now();
    const vdName = `e2e_unknown_flat_${suffix}`;
    const canonical = `http://example.org/ViewDefinition/e2e-unknown-${suffix}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    const libId = await createSqlQueryLibrary(
      request,
      `e2e_unknown_query_${suffix}`,
      canonical,
      "SELECT id FROM v",
    );
    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", libId);

    await page.goto(`/ui/sql/queries?lib=${libId}`);
    await expect(page.locator("#run-results .data-table th")).toHaveText(["id"]);
    const columnsCard = page.locator("#lib-columns");
    const columnsRow = columnsCard.locator("tbody tr").first();
    await expect(columnsCard.locator("tbody tr")).toHaveCount(1);
    await expect(columnsRow.locator("td").nth(0)).toHaveText("id");
    await expect(columnsRow.locator("td").nth(1)).toHaveText("string");
    await expect(columnsRow.locator("td").nth(2)).toHaveText("v.id");

    // Typing an unknown table name never reaches $sql-run: the notice
    // names it, the previous table (and Columns) stay on screen with a
    // stale meta, and the editor underlines the table's own text.
    const editor = page.locator(".sql-editor .cm-content[role='textbox']");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("SELECT id FROM vv");
    const notice = page.locator(".notice--warn");
    await expect(notice).toContainText("Unknown table vv — line 1.", { timeout: 3000 });
    await expect(page.locator(".sql-editor .cm-line").first()).toHaveClass(/\bsql-editor__error-line\b/);
    await expect(page.locator("#run-results .data-table th")).toHaveText(["id"]);
    await expect(page.locator("#run-results-meta")).toHaveText("last successful run");
    await expect(columnsCard.locator("#lib-columns-meta")).toHaveText("last successful run");
    await expect(columnsRow.locator("td").nth(0)).toHaveText("id");
    await expect(columnsRow.locator("td").nth(2)).toHaveText("v.id");

    // The underline carries a hover tooltip, and there is a marker in the
    // lint gutter.
    const underline = page.locator(".sql-editor .cm-lintRange-error");
    await expect(underline).toBeVisible();
    await underline.hover();
    await expect(page.locator(".cm-tooltip-lint")).toContainText("Unknown table vv — line 1.", {
      timeout: 3000,
    });
    await expect(page.locator(".sql-editor .cm-gutter-lint .cm-lint-marker-error")).toBeVisible();

    // Reads from gained its own red row with a Declare button.
    const tablesCard = page.locator("#lib-tables");
    const unknownRow = tablesCard.locator("tr", { hasText: "vv" });
    await expect(unknownRow.locator(".tag--failed")).toHaveText("Unknown table");
    await expect(unknownRow.getByRole("button", { name: "Declare vv" })).toBeVisible();

    // Declare opens Add table with the alias already the unknown name —
    // never overwritten by picking the target from the combobox.
    await unknownRow.getByRole("button", { name: "Declare vv" }).click();
    await expect(tablesCard.locator("details.editor-add")).toHaveAttribute("open", "");
    const aliasField = page.locator("input[name='table_alias']");
    await expect(aliasField).toHaveValue("vv");

    const search = page.locator('#lib-tables-add-table input[role="combobox"]');
    await search.fill(vdName);
    const option = page.locator("#lib-tables-add-table [data-combobox-option]", { hasText: vdName });
    await expect(option).toBeVisible({ timeout: 10000 });
    await option.click();
    await expect(aliasField).toHaveValue("vv");
    await page.locator("button[name='op'][value='add-table']").click();

    // Resolved: the lint clears, the underline goes away, the table
    // refreshes, and Columns shows the new label's own origin.
    await expect(unknownRow.locator(".tag--failed")).toHaveCount(0, { timeout: 3000 });
    await expect(tablesCard.locator("tr", { hasText: "vv" }).locator("a")).toBeVisible();
    await expect(notice).toHaveCount(0, { timeout: 3000 });
    await expect(page.locator(".sql-editor .cm-lintRange-error")).toHaveCount(0);
    await expect(page.locator("#run-results-meta")).toHaveText(/^\d+ rows · \d+ ms$/, { timeout: 3000 });
    const resolvedColumnsRow = columnsCard.locator("tbody tr").first();
    await expect(resolvedColumnsRow.locator("td").nth(0)).toHaveText("id");
    await expect(resolvedColumnsRow.locator("td").nth(2)).toHaveText("vv.id", { timeout: 3000 });
  });
});
