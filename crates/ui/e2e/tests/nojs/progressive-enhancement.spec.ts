import { test, expect } from "../../pages/fixtures";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometries,
} from "../../pages/button-geometry";
import { createResource, waitSearchable } from "../../pages/api";
import { langLink, openUserMenu } from "../../pages/user-menu";

// This whole file runs in the `nojs` project (javaScriptEnabled: false), which
// exercises the README's core promise: the UI works with JavaScript off. htmx,
// theme.js, and every data-* handler are inert here — only real <a href>/<form>
// fallbacks can carry the behavior.

test("the landing page renders server-side with no JavaScript", async ({ page, chrome }) => {
  await page.goto("/ui");
  await expect(page.locator("body")).toContainText("Helios FHIR Server");
  // The sidebar (full layout) is present — not a bare fragment.
  await expect(chrome.sidebar).toBeVisible();
});

test("the sidebar brand is an accessible native Home link", async ({ page }) => {
  await page.goto("/ui/resources");
  const brand = page.locator("a.brand");
  const name = /^Helios FHIR Server hfs v.+ — Home$/;

  await expect(brand).toHaveAttribute("href", "/ui");
  await expect(brand).not.toHaveAttribute("aria-current", "page");
  await expect(brand).toHaveAccessibleName(name);
  await expect(brand.locator("img")).toHaveAttribute("alt", "");

  await brand.focus();
  await expect(brand).toHaveAccessibleName(name);
  await brand.press("Enter");
  await expect(page).toHaveURL(/\/ui$/);
  await expect(page.locator("h1.page-head__title")).toHaveText("Home");
});

test("the language switcher works as plain links (en → es → de)", async ({ page }) => {
  await page.goto("/ui");
  // #725: the language options live behind the avatar's <details> menu, which
  // opens natively — no JS involved. `openUserMenu` (#799) is the shared
  // summary-click idiom; it must never become `evaluate(d => d.open = true)`,
  // which would be a no-op in this project.
  await openUserMenu(page);
  await expect(langLink(page, "es")).toHaveAttribute("href", /lang=es/);
  await langLink(page, "es").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "es");

  await openUserMenu(page);
  await langLink(page, "de").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "de");
});

// Every primary nav entry is a real link that hard-navigates without JS.
const NAV = [
  { href: "/ui/resources", url: /\/ui\/resources/ },
  { href: "/ui/compartments", url: /\/ui\/compartments/ },
  { href: "/ui/tenants", url: /\/ui\/tenants/ },
  { href: "/ui/search-parameters", url: /\/ui\/search-parameters/ },
];
for (const { href, url } of NAV) {
  test(`nav link ${href} navigates`, async ({ page, chrome }) => {
    await page.goto("/ui");
    // Enter the rail and let it finish expanding (#438) so the link's position
    // is stable before the click — pure CSS, so it works with JS disabled too.
    await chrome.sidebar.hover();
    await expect
      .poll(async () => (await chrome.sidebar.boundingBox())?.width)
      .toBeGreaterThan(290);
    const link = chrome.navLink(href);
    await expect(link).toBeVisible();
    await link.click();
    await expect(page).toHaveURL(url);
    await expect(page.locator("h1.page-head__title, h1.page-head__title")).toBeVisible();
  });
}

test("no nav entry is a dead 'coming soon' placeholder", async ({ page }) => {
  // The last placeholder became the SQL on FHIR section (#649). Every entry
  // in the menu now navigates; a reintroduced inert span would regress that.
  await page.goto("/ui");
  await expect(page.locator(".nav-item--soon")).toHaveCount(0);
});

test("the Resources type rail navigates via plain links with no JavaScript", async ({
  page,
}) => {
  await page.goto("/ui/resources");
  const item = page.locator("#type-rail-list a.filter-rail__item[data-type='Observation']");
  await expect(item).toBeVisible();
  await item.click();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);
  await expect(item).toHaveAttribute("aria-current", "true");
});

// "Tests esperados" #15 (RF8): a rail click is a real navigation with no
// JavaScript at all, so the server itself records the selection (RF3) — and
// the "Recently used" group, entirely server-rendered (RF6), shows it on the
// very next load without any client script populating it.
test("a Resources rail click without JavaScript is remembered in the recently-used group", async ({
  page,
}) => {
  await page.goto("/ui/resources");
  await page
    .locator("#type-rail-list a.filter-rail__item[data-type='Observation']")
    .click();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);

  await page.goto("/ui/resources");
  const recentGroup = page.locator("#type-rail-recent");
  await expect(recentGroup).toBeVisible();
  await expect(recentGroup.locator("[data-type='Observation']")).toBeVisible();
});

// "Tests esperados" #6 (#754/#755 ticket 04): a Compartments rail click is a
// real navigation with no JavaScript at all — there is no "Recently used"
// group here (only 4-5 definitions), just `rails.compartments.last` — so the
// server itself records the selection (RF2), and a later plain arrival with
// no `?def=` at all restores it (RF1), the same route the nojs sweep above
// already proved is navigable.
test("a Compartments rail click without JavaScript is remembered on a later plain arrival", async ({
  page,
  compartments,
}) => {
  await compartments.goto();
  await compartments.railItem("Encounter").click();
  await expect(page).toHaveURL(/def=Encounter/);
  await expect(compartments.railItem("Encounter")).toHaveAttribute("aria-current", "true");

  await compartments.goto();
  await expect(compartments.railItem("Encounter")).toHaveAttribute("aria-current", "true");
});

// "Tests esperados" #12 (#754/#755 ticket 03): the three SQL rails (View
// Definitions, SQL Queries, SQL Views) render their rail and "Recently used"
// group entirely server-side, and a rail click is a real navigation the
// server itself records — no client script required either way.
const SQL_RAILS = [
  { path: "/ui/sql/view-definitions", list: "vd-rail-list", recent: "vd-rail-recent", param: "vd" },
  { path: "/ui/sql/queries", list: "lib-rail-list", recent: "lib-rail-recent", param: "lib" },
  { path: "/ui/sql/views", list: "lib-rail-list", recent: "lib-rail-recent", param: "lib" },
];
for (const { path, list, recent, param } of SQL_RAILS) {
  test(`${path} renders its rail and group, and a click without JavaScript is remembered`, async ({
    page,
    request,
  }) => {
    const stamp = Date.now().toString(36);
    const id =
      param === "vd"
        ? await createResource(request, "ViewDefinition", {
            name: `znojs_${stamp}`,
            status: "active",
            resource: "Patient",
            select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
          })
        : await createResource(request, "Library", {
            name: `znojs_${stamp}`,
            status: "active",
            type: {
              coding: [
                {
                  system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
                  code: path.endsWith("queries") ? "sql-query" : "sql-view",
                },
              ],
            },
          });
    await waitSearchable(request, param === "vd" ? "ViewDefinition" : "Library", id);

    await page.goto(path);
    const item = page.locator(`#${list} a.filter-rail__item[data-type='${id}']`);
    await expect(item).toBeVisible();
    await item.click();
    await expect(page).toHaveURL(new RegExp(`${param}=${id}`));
    await expect(item).toHaveAttribute("aria-current", "true");

    await page.goto(path);
    const recentGroup = page.locator(`#${recent}`);
    await expect(recentGroup).toBeVisible();
    await expect(recentGroup.locator(`[data-type='${id}']`)).toBeVisible();
  });
}

test("the CapabilityStatement filter submits a GET form with no JavaScript", async ({
  page,
  capabilityStatement,
}) => {
  await capabilityStatement.goto();
  await capabilityStatement.filter.fill("Patient");
  await capabilityStatement.filter.press("Enter");

  await expect(page).toHaveURL(
    /\/ui\/capability-statement\?version=R4&filter=Patient$/,
  );
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toHaveCount(0);
});

test("the CapabilityStatement raw fallback is plain JSON with no JavaScript", async ({
  page,
  capabilityStatement,
}) => {
  await capabilityStatement.goto("?filter=Patient");
  await capabilityStatement.rawSummary.click();
  await expect(capabilityStatement.rawLoading).toBeHidden();
  await capabilityStatement.rawLoadLink.click();

  const url = new URL(page.url());
  expect(url.pathname).toBe("/ui/capability-statement");
  expect(url.searchParams.get("raw")).toBe("1");
  expect(url.searchParams.get("version")).toBe("R4");
  expect(url.searchParams.get("filter")).toBe("Patient");
  await expect(capabilityStatement.rawDisclosure).toHaveAttribute("open", "");
  await expect(capabilityStatement.rawBody.locator("pre.detail__code")).toBeVisible();
  await expect(capabilityStatement.rawBody.getByText("Plain JSON fallback", { exact: false })).toBeVisible();
  await expect(capabilityStatement.jsonView).toHaveCount(0);
  await expect(capabilityStatement.rawBody.locator(".json-line")).toHaveCount(0);
});

test("a hard navigation returns the full page, not an htmx fragment", async ({ page, chrome }) => {
  // /ui/status is fragment-or-full depending on HX-Request; a plain load (no JS,
  // no htmx header) must get the whole document.
  await page.goto("/ui/status");
  await expect(chrome.sidebar).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", /.+/);
});

test("native Addbox dialogs keep canonical actions and the open-summary backdrop", async ({ page }) => {
  await page.goto("/ui/bulk-import");
  const details = page.locator("details.addbox--modal");
  const summary = details.locator("summary.btn");
  await expect(summary).toHaveCSS("height", "30px");
  await expect(summary).toHaveCSS("padding-left", "12px");
  await expect(summary).toHaveCSS("border-radius", "9px");

  await summary.click();
  await expect(details).toHaveAttribute("open", "");
  const backdrop = await summary.boundingBox();
  const viewport = page.viewportSize()!;
  expect(backdrop!.height).toBeGreaterThan(viewport.height * 0.9);
  await expect(summary).toHaveCSS("padding-left", "0px");
  await expect(summary).toHaveCSS("border-radius", "0px");

  const actionMetrics = await readButtonGeometries(details.locator(".addbox__actions .btn"));
  expect(actionMetrics).toHaveLength(2);
  for (const geometry of actionMetrics) expect(geometry).toEqual(CANONICAL_BUTTON_GEOMETRY);
});

test("Bulk Import one-shot create and delete work without JavaScript", async ({ page }) => {
  await page.goto("/ui/bulk-import");
  const create = page.locator("details.addbox--modal");
  await create.locator("> summary").click();
  await create.locator("input[name='name']").fill("no-js-submission");
  await create
    .locator("input[name='manifest_url']")
    .fill("https://example.test/manifest.json");
  // The Advanced fold is a native disclosure — it opens without JS.
  await create.locator(".disclosure__summary").click();
  await expect(create.locator("input[name='output_format']")).toBeVisible();
  await create.getByRole("button", { name: "Submit", exact: true }).click();
  await expect(page).toHaveURL(/\/ui\/bulk-import\/[0-9a-f-]+$/);
  await expect(page.locator(".kv-grid")).toContainText("https://example.test/manifest.json");

  await page.locator("form[action$='/delete'] > button").click();
  await expect(page).toHaveURL(/\/ui\/bulk-import$/);
});

test("Bulk Export can narrow through a conflicting native form without JavaScript", async ({
  page,
  bulkExport,
}) => {
  await page.goto("/ui/bulk-export/new");

  await expect(bulkExport.allResources).toBeChecked();
  await expect(bulkExport.typeCheckboxes).not.toHaveCount(0);
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every(
        (type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
      ),
    ),
  ).toBe(true);

  // With no JavaScript the user can select a resource type while the native
  // All Resources checkbox remains checked. The UI handler resolves this
  // intentionally conflicting payload in favor of All Resources.
  await bulkExport.typeCheckbox("Patient").check();
  await expect(bulkExport.allResources).toBeChecked();
  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const request = await submitted;
  const params = new URLSearchParams(request.postData() ?? "");
  expect(params.get("all_types")).toBe("on");
  expect(params.getAll("types")).toEqual(["Patient"]);
});

test("Bulk Export submits an exact custom instant without JavaScript", async ({
  page,
  bulkExport,
}) => {
  await page.goto("/ui/bulk-export/new");

  const instant = "2026-08-01T00:00:00Z";
  await bulkExport.sincePreset.selectOption("custom");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await bulkExport.sinceCustom.fill(instant);

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const request = await submitted;
  const params = new URLSearchParams(request.postData() ?? "");
  expect(params.get("since_preset")).toBe("custom");
  expect(params.get("since_custom")).toBe(instant);
});

test("Bulk Export lifecycle works without JavaScript", async ({ page }) => {
  await page.goto("/ui/bulk-export");
  const newExport = page.getByRole("link", { name: "New Export" });

  // Backends without a settings store cannot track jobs and intentionally do
  // not offer the builder from the management page.
  if ((await newExport.count()) === 0) {
    await expect(page.locator(".notice")).toContainText(/settings store/i);
    return;
  }

  await newExport.click();
  await expect(page).toHaveURL(/\/ui\/bulk-export\/new$/);

  const backLink = page.locator("a.back-link");
  await expect(backLink).toHaveAttribute("href", "/ui/bulk-export");
  const form = page.locator('form[action="/ui/bulk-export"]');
  await expect(form).toHaveAttribute("method", "post");
  await expect(form.locator(".form-actions > a")).toHaveCount(0);
  const startExport = form.getByRole("button", { name: "Start Export" });
  await expect(startExport).toBeVisible();
  await expect(startExport).toBeEnabled();
  expect(await startExport.getAttribute("aria-busy")).toBeNull();

  const exportName = `no-js-export-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  await form.locator('input[name="name"]').fill(exportName);
  await form.locator('input[name="scope"][value="system"]').check();
  await startExport.click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  let card = page.locator(".job-card").filter({ hasText: exportName });
  await expect(card).toBeVisible();

  await card.getByRole("button", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  card = page.locator(".job-card").filter({ hasText: exportName });
  await expect(card).toContainText("Cancelled");

  const disclosure = card.locator("details.job-card__delete");
  await disclosure.locator("summary").click();
  await expect(disclosure).toHaveAttribute("open", "");
  await disclosure.getByRole("link", { name: "Keep export" }).click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  card = page.locator(".job-card").filter({ hasText: exportName });
  await expect(card).toBeVisible();

  const reopened = card.locator("details.job-card__delete");
  await reopened.locator("summary").click();
  await reopened.getByRole("button", { name: "Delete export" }).click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  await expect(page.locator(".job-card").filter({ hasText: exportName })).toHaveCount(0);
});

// #838: sql-editor.js never loads (this project runs with
// javaScriptEnabled: false), so the SQL pane stays the plain textarea it
// is today — visible, holding the decoded SQL, editable, and saved by a
// real form POST.
test("the SQL Queries pane is a plain textarea and Save persists without JavaScript", async ({
  page,
  request,
}) => {
  const sql = "SELECT id FROM v";
  const libId = await createResource(request, "Library", {
    name: `nojs_sql_query_${Date.now()}`,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
    content: [{ contentType: "application/sql", data: Buffer.from(sql).toString("base64") }],
  });
  await waitSearchable(request, "Library", libId);

  await page.goto(`/ui/sql/queries?lib=${libId}`);
  const textarea = page.locator("textarea[name='sql']");
  await expect(textarea).toBeVisible();
  await expect(textarea).toHaveValue(sql);
  // No editor script ran, so no CodeMirror wrapper was inserted.
  await expect(page.locator(".sql-editor")).toHaveCount(0);

  const updated = "SELECT name FROM v";
  await textarea.fill(updated);
  await page.locator("button[name='action'][value='save']").click();
  await expect(page).toHaveURL(/saved=1/);
  await expect(page.locator("textarea[name='sql']")).toHaveValue(updated);
});

test("Bulk Export accepts comma- and newline-separated Patient IDs without JavaScript", async ({
  page,
  bulkExport,
}) => {
  await page.goto("/ui/bulk-export/new");
  await bulkExport.scopeRadio("patient").check();

  await expect(bulkExport.patientFallback).toBeVisible();
  await expect(bulkExport.patientFallback).toBeEnabled();
  const patientRefs = "Patient/p-104, Patient/p-205\nPatient/p-306";
  await bulkExport.patientFallback.fill(patientRefs);

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const params = new URLSearchParams((await submitted).postData() ?? "");
  expect(params.get("scope")).toBe("patient");
  expect(params.getAll("patient").map((value) => value.replace(/\r\n/g, "\n"))).toEqual([
    patientRefs,
  ]);
});
