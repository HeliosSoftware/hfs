import { test, expect } from "../../pages/fixtures";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometries,
} from "../../pages/button-geometry";

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

test("the language switcher works as plain links (en → es → de)", async ({ page, chrome }) => {
  await page.goto("/ui");
  // #725: the language options live behind the avatar's <details> menu,
  // which opens natively — no JS involved.
  await chrome.userMenu.locator("summary").click();
  await expect(chrome.langLink("es")).toHaveAttribute("href", /lang=es/);
  await chrome.langLink("es").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "es");

  await chrome.userMenu.locator("summary").click();
  await chrome.langLink("de").click();
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

test("the CapabilityStatement filter submits a GET form with no JavaScript", async ({
  page,
  capabilityStatement,
}) => {
  await capabilityStatement.goto();
  await capabilityStatement.filter.fill("Patient");
  await capabilityStatement.filter.press("Enter");

  await expect(page).toHaveURL(/\/ui\/capability-statement\?filter=Patient$/);
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toHaveCount(0);
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
  await create.getByRole("button", { name: "Create & Submit" }).click();
  await expect(page).toHaveURL(/\/ui\/bulk-import\/[0-9a-f-]+$/);
  await expect(page.locator(".kv-grid")).toContainText("https://example.test/manifest.json");

  await page.locator("form[action$='/delete'] > button").click();
  await expect(page).toHaveURL(/\/ui\/bulk-import$/);
});

test("Bulk Export can narrow through a conflicting native form without JavaScript", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();

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
  await bulkExport.goto();

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
