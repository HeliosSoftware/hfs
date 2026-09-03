import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

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

// #754/#755: the server remembers a chosen base type, and "All types" is
// its own explicit, remembered state — never masked by a real
// type recorded earlier — reachable in one click no matter what.
test("a chosen type and All types both survive leaving the page and coming back", async ({
  page,
  chrome,
  searchParameters,
}) => {
  const base = () => new URL(page.url()).searchParams.get("base");

  await searchParameters.goto();
  await searchParameters.railItem("Encounter").click();
  // hx-boost swaps in place and pushes the URL itself; wait on that
  // deterministically rather than the "networkidle" heuristic.
  await page.waitForURL((url) => url.searchParams.get("base") === "Encounter");
  expect(base()).toBe("Encounter");

  await chrome.navLink("/ui/resources").click();
  await page.waitForURL(/\/ui\/resources/);
  await chrome.navLink("/ui/search-parameters").click();
  await page.waitForURL(/\/ui\/search-parameters/);
  expect(base()).toBeNull(); // no explicit ?base= on this deep link
  await expect(searchParameters.railItem("Encounter")).toHaveAttribute("aria-current", "true");

  await searchParameters.allTypesLink.click();
  await page.waitForURL((url) => url.searchParams.get("base") === "");
  expect(base()).toBe(""); // explicit "All types" marker, not omitted
  await expect(searchParameters.allTypesLink).toHaveAttribute("aria-current", "true");

  await chrome.navLink("/ui/resources").click();
  await page.waitForURL(/\/ui\/resources/);
  await chrome.navLink("/ui/search-parameters").click();
  await page.waitForURL(/\/ui\/search-parameters/);
  // Still "All types" — a stored Encounter from earlier in this test must not
  // resurface now that "All types" is itself the remembered state.
  await expect(searchParameters.allTypesLink).toHaveAttribute("aria-current", "true");
  await expect(searchParameters.railItem("Encounter")).not.toHaveAttribute(
    "aria-current",
    "true",
  );
});

// The group is present-but-hidden until a base is picked, then shows it
// with a live, current-marked entry — all server-rendered, no reload needed
// since the base link is itself a real navigation (hx-boost).
test("the recently-used group appears after a pick and marks it current", async ({
  page,
  searchParameters,
}) => {
  await searchParameters.goto();
  await expect(searchParameters.railRecent).toBeHidden();

  await searchParameters.railItem("Patient").click();
  await page.waitForLoadState("networkidle");
  await expect(searchParameters.railRecent).toBeVisible();
  await expect(searchParameters.recentItem("Patient")).toHaveAttribute("aria-current", "true");
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
  const id = await createResource(request, "SearchParameter", {
    url,
    name: "e2eCrud",
    code: `e2e-crud-${stamp}`,
    status: "active",
    type: "token",
    base: ["Patient"],
    expression: "Patient.identifier",
  });
  // The page lists via FHIR search; on the ES composites the write is not
  // searchable until the index refreshes, and the refetched snapshot would
  // cache without it.
  await waitSearchable(request, "SearchParameter", id);

  // refresh=1 drops the server's cached snapshot so the new parameter shows.
  await searchParameters.goto(`?refresh=1&sel=${encodeURIComponent(url)}`);
  // The primary action sits in the page-head row next to the title (the
  // Resources pattern), not in a standalone actions block under the lede.
  await expect(page.locator(".page-head--row > a.btn--primary")).toHaveAttribute(
    "href",
    "/ui/editor?type=SearchParameter",
  );
  await expect(page.locator(".detail__actions a.btn")).toHaveAttribute(
    "href",
    `/ui/editor?type=SearchParameter&id=${id}`,
  );

  page.once("dialog", (d) => d.accept());
  await page.locator(".detail__actions [data-crud-delete]").click();
  await page.waitForURL("**/ui/search-parameters?refresh=1");

  const res = await request.get(`/SearchParameter/${id}`, {
    headers: { Accept: "application/fhir+json" },
  });
  expect([404, 410]).toContain(res.status());
});

// #679: the conformance delete rides the shared busy helper. A failed DELETE
// must re-enable the button next to its inline error — the old ad-hoc code
// did, and the helper must not regress it into a permanently dead control.
test("a failed delete shows the busy state, then re-enables the button", async ({
  page,
  searchParameters,
  request,
}) => {
  const stamp = Date.now();
  const url = `http://example.org/e2e/SearchParameter/busy-${stamp}`;
  const id = await createResource(request, "SearchParameter", {
    url,
    name: "e2eBusyDelete",
    code: `e2e-busy-${stamp}`,
    status: "active",
    type: "token",
    base: ["Patient"],
    expression: "Patient.identifier",
  });
  await waitSearchable(request, "SearchParameter", id);
  await searchParameters.goto(`?refresh=1&sel=${encodeURIComponent(url)}`);

  let release!: () => void;
  const parked = new Promise<void>((resolve) => { release = resolve; });
  await page.route(new RegExp(`/SearchParameter/${id}$`), async (route) => {
    if (route.request().method() !== "DELETE") return route.continue().catch(() => {});
    await parked;
    await route
      .fulfill({
        status: 500,
        contentType: "application/fhir+json",
        body: JSON.stringify({
          resourceType: "OperationOutcome",
          issue: [{ severity: "error", code: "exception", diagnostics: "boom" }],
        }),
      })
      .catch(() => {});
  });

  page.once("dialog", (d) => d.accept());
  const del = page.locator(".detail__actions [data-crud-delete]");
  await del.click();
  await expect(del).toHaveAttribute("aria-busy", "true");
  await expect(del).toBeDisabled();

  release();
  await expect(page.locator(".detail__actions .alert")).toBeVisible();
  await expect(del).toBeEnabled();
  await expect(del).not.toHaveAttribute("aria-busy", "true");
});
