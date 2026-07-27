import { test, expect } from "../pages/fixtures";

// The SearchParameter registry viewer (/ui/search-parameters): the htmx filter
// rail, the type/source facet chips, row selection into the detail panel, and
// pagination. Read-only, registry-fed.

test("the registry table renders rows and a detail placeholder", async ({ searchParameters }) => {
  await searchParameters.goto();
  await expect(searchParameters.rows.first()).toBeVisible();
  await expect(searchParameters.detailTitle).toBeVisible();
});

test("the rail search filters the type list (htmx)", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  await searchParameters.railSearch.fill("Patient");
  // htmx swaps #sp-rail-list; the Patient row survives, an unrelated one drops.
  await expect(searchParameters.railItem("Patient")).toBeVisible();
  await expect
    .poll(async () => searchParameters.railList.locator(".filter-rail__item").count())
    .toBeLessThan(50);
});

test("a type facet narrows the table", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  const before = await searchParameters.rows.count();
  await searchParameters.railItem("Observation").click();
  await page.waitForLoadState("networkidle");
  // The URL now scopes to the type, and the table reflects the narrower set.
  await expect(page).toHaveURL(/Observation/);
  expect(await searchParameters.rows.count()).toBeLessThanOrEqual(before);
});

test("selecting a row opens its detail", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  await searchParameters.rowLinks.first().click();
  await page.waitForLoadState("networkidle");
  await expect(page).toHaveURL(/sel=/);
  await expect(searchParameters.detailTitle).toBeVisible();
});
