import { test, expect } from "../pages/fixtures";
import { createResource, waitForSearchHit } from "../pages/api";

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

// CRUD (#238): New deep-links the schema-driven editor, and a stored
// parameter's detail offers Edit (editor deep-link) and Delete (FHIR API).
test("a stored parameter can be created, offers Edit, and deletes", async ({
  page,
  searchParameters,
  request,
}) => {
  const stamp = Date.now();
  const url = `http://example.org/e2e/SearchParameter/crud-${stamp}`;
  const code = `e2e-crud-${stamp}`;
  const id = await createResource(request, "SearchParameter", {
    url,
    name: "e2eCrud",
    code,
    status: "active",
    type: "token",
    base: ["Patient"],
    expression: "Patient.identifier",
  });

  // Composite ES backends index asynchronously; wait before UI refresh.
  await waitForSearchHit(request, "SearchParameter", `url=${encodeURIComponent(url)}`);

  // refresh=1 drops the server's cached snapshot so the new parameter shows.
  await searchParameters.goto(`?refresh=1&sel=${encodeURIComponent(url)}`);
  await expect(page.locator(".page-head__actions a.btn--primary")).toHaveAttribute(
    "href",
    "/ui/editor?type=SearchParameter",
  );
  // Detail actions can land a beat after the registry swap on slow backends.
  await expect
    .poll(async () => page.locator(".detail__actions a.btn").getAttribute("href"), {
      timeout: 15_000,
    })
    .toBe(`/ui/editor?type=SearchParameter&id=${id}`);

  page.once("dialog", (d) => d.accept());
  await page.locator(".detail__actions [data-crud-delete]").click();
  await page.waitForURL("**/ui/search-parameters?refresh=1");

  const res = await request.get(`/SearchParameter/${id}`, {
    headers: { Accept: "application/fhir+json" },
  });
  expect([404, 410]).toContain(res.status());
});
