// SQL Queries workspace (#649): a stored SQLQuery Library lists in the rail,
// its SQL decodes into the editor pane, and the results region runs it over
// its depends-on ViewDefinition through $sql-run on arrival — no Run button
// (#839, generalizing #752's View Definitions playground here).
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

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
