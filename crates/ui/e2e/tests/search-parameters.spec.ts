import { test, expect, acceptConfirm } from "../pages/fixtures";
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

// #1106: row-navigation.js delegates the click from `document`, so the whole
// row — not just its own link cell — opens the detail.
test("clicking a non-link cell of a row opens its detail", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  await searchParameters.rows.first().locator("td:last-child").click();
  await page.waitForLoadState("networkidle");
  await expect(page).toHaveURL(/sel=/);
  await expect(searchParameters.detailTitle).toBeVisible();
});

// #1719: every value in the detail panel — URL `<code>`, Name/Status text,
// the FHIRPath `<pre class="detail__code">`, and the Type/Base/Target badge
// rows — starts at one shared x-offset under its label, and chips in a
// `.detail__tags` row sit exactly the row's 5px gap apart (no per-chip
// `.tag` margin stacked on top). `clinical-patient` is a core parameter
// with many bases and two targets, so both chip rows have neighbours to
// measure.
test("detail panel values share one indent and chips keep the row gap", async ({
  page,
  searchParameters,
}) => {
  const url = "http://hl7.org/fhir/SearchParameter/clinical-patient";
  await searchParameters.goto(`?sel=${encodeURIComponent(url)}`);
  await expect(page.locator(".detail .detail__field").first()).toBeVisible();

  const geometry = await page.locator(".detail").evaluate((panel) => {
    const fields = Array.from(panel.querySelectorAll<HTMLElement>(":scope > .detail__field"));
    const values = fields.flatMap((field) =>
      Array.from(field.querySelectorAll<HTMLElement>(":scope > :not(:first-child)")).map(
        (value) => ({
          label: (field.querySelector(":scope > span")?.textContent ?? "").trim(),
          left: value.getBoundingClientRect().left,
        }),
      ),
    );
    const firstTag = panel.querySelector<HTMLElement>(":scope > .detail__field .tag")!;
    const chipGaps = Array.from(panel.querySelectorAll<HTMLElement>(".detail__tags")).flatMap(
      (row) => {
        const boxes = Array.from(row.querySelectorAll<HTMLElement>(":scope > .tag")).map((tag) =>
          tag.getBoundingClientRect(),
        );
        // Only same-line neighbours: a wrapped chip starts a new line.
        return boxes
          .slice(1)
          .map((box, i) => ({ prev: boxes[i], box }))
          .filter(({ prev, box }) => Math.abs(box.top - prev.top) < 1)
          .map(({ prev, box }) => box.left - prev.right);
      },
    );
    const labelLeft = fields[0].querySelector(":scope > span")!.getBoundingClientRect().left;
    return {
      labelLeft,
      tagLeft: firstTag.getBoundingClientRect().left,
      values,
      chipGaps,
    };
  });

  // The badges are indented under their label; that indent is the column.
  expect(geometry.tagLeft).toBeGreaterThan(geometry.labelLeft);
  expect(geometry.values.length).toBeGreaterThanOrEqual(6);
  for (const value of geometry.values) {
    expect(
      Math.abs(value.left - geometry.tagLeft),
      `${value.label} value starts at ${value.left}, badge column is ${geometry.tagLeft}`,
    ).toBeLessThanOrEqual(1);
  }
  expect(geometry.chipGaps.length).toBeGreaterThan(0);
  for (const gap of geometry.chipGaps) {
    expect(Math.abs(gap - 5)).toBeLessThanOrEqual(1);
  }
});

test("selecting text inside a row does not navigate", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  const before = page.url();
  const cell = searchParameters.rows.first().locator("td").nth(1);
  // In Chromium a synthesized pointer click collapses a pre-existing selection
  // before the click event fires, which would defeat the point of this test.
  // Build the selection programmatically and dispatch the click directly: a
  // click that lands while the row still holds a live selection, as after a
  // drag-select.
  await cell.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  });
  await page.waitForTimeout(200);
  expect(page.url()).toBe(before);
});

test("a modifier click on a non-link cell does not navigate", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  const before = page.url();
  await searchParameters.rows.first().locator("td:last-child").click({ modifiers: ["Shift"] });
  await page.waitForTimeout(200);
  expect(page.url()).toBe(before);
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

  await page.locator(".detail__actions [data-crud-delete]").click();
  await acceptConfirm(page);
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

  const del = page.locator(".detail__actions [data-crud-delete]");
  await del.click();
  await acceptConfirm(page);
  await expect(del).toHaveAttribute("aria-busy", "true");
  await expect(del).toBeDisabled();

  release();
  await expect(page.locator(".detail__actions .alert")).toBeVisible();
  await expect(del).toBeEnabled();
  await expect(del).not.toHaveAttribute("aria-busy", "true");
});
