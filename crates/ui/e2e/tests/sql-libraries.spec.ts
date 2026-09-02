// SQL Queries workspace (#649): a stored SQLQuery Library lists in the rail,
// its SQL decodes into the editor pane, and Run executes it over its
// depends-on ViewDefinition through $sql-run.
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

test("a stored SQLQuery lists, decodes its SQL, and previews rows", async ({ page, request }) => {
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

  await page.locator("a[href*='run=1']").click();
  await expect(page.locator(".data-table")).toBeVisible();
  await expect(page.locator(".data-table th", { hasText: "n" }).first()).toBeVisible();
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
