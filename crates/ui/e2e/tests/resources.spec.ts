import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

// The Resources workspace beyond the edit flows: the type rail (filter + live
// counts), the modal's open/close/tab surface, the delete flow, and the promise
// that every FHIR resource type is reachable — "test all the resources".

test("the type rail lists the full resource-type set", async ({ resources }) => {
  await resources.goto("Patient");
  const types = await resources.railTypes();
  // The R4 Patient compartment enumerates the whole resource set (145 types).
  expect(types.length).toBeGreaterThan(140);
  expect(types).toEqual(expect.arrayContaining(["Patient", "Observation", "Encounter", "Bundle"]));
});

test("the rail filter narrows the visible types", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.railFilter.fill("observ");
  await expect(resources.railItem("Observation")).toBeVisible();
  await expect(resources.railItem("Patient")).toBeHidden();
  await resources.railFilter.fill("");
  await expect(resources.railItem("Patient")).toBeVisible();
});

// Picking a type updates the URL (and back navigates) without a full reload —
// the click handler is an enhancement over the rail's real <a href> (#541).
test("picking a rail type updates the URL and back navigates", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.pickType("Observation");
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);
  await expect(resources.railItem("Observation")).toHaveAttribute("aria-current", "true");

  await page.goBack();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Patient/);
});

test("counts render next to each type from the dashboard snapshot", async ({
  resources,
  request,
}) => {
  // Seed one so the count is unambiguous and non-empty. The dashboard
  // snapshot is cached briefly (#541), so poll a fresh page load rather than
  // waiting on a client-side hydration fetch.
  await createResource(request, "Device", {});
  await expect
    .poll(
      async () => {
        await resources.goto("Device");
        return await resources.count("Device").textContent();
      },
      { timeout: 20_000, intervals: [1_000, 2_000, 4_000] },
    )
    .not.toBe("");
});

test("every resource type is reachable — Create targets each one", async ({ resources }) => {
  await resources.goto("Patient");
  const types = await resources.railTypes();

  for (const type of types) {
    await resources.pickType(type);
    await resources.createButton.click();
    await resources.modal.waitOpen();
    expect(
      (await resources.modal.editor.currentDoc()).resourceType,
      `Create for ${type} projected the wrong resourceType`,
    ).toBe(type);
    await resources.modal.closeWithEscape();
  }
});

// "Recently used" group (#603): a per-browser convenience, populated by
// resource-filter.js from explicit rail picks and re-rendered on load.
test("the recently-used group tracks picks, most-recent-first, with counts matching the full list", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.pickType("Encounter");
  await resources.pickType("Observation");
  await resources.pickType("Encounter"); // re-selection: moves to front, no duplicate

  // The group is client-rendered from localStorage on load, so it only
  // reflects the picks above once the page (re)loads.
  await resources.goto("Patient");
  await expect(resources.recentGroup).toBeVisible();

  const types = await resources.recentGroup
    .locator("a.filter-rail__item[data-type]")
    .evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.type));
  expect(types).toEqual(["Encounter", "Observation"]);

  for (const type of types) {
    const listCount = await resources.count(type).textContent();
    const recentCount = await resources.recentItem(type).locator(".count").textContent();
    expect(recentCount).toBe(listCount);
  }
});

test("clicking a recently-used entry selects that type for real", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.pickType("Encounter");
  await resources.goto("Patient"); // reload to populate the group

  await resources.recentItem("Encounter").click();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Encounter/);
  await expect(resources.railItem("Encounter")).toHaveAttribute("aria-current", "true");
});

test("Create new does not register a recently-used entry", async ({ resources }) => {
  // The default selection (Patient, unpicked) still stamps "Create new" with
  // data-type="Patient" — clicking it must not be mistaken for a rail pick.
  await resources.goto("Patient");
  await resources.createButton.click();
  await resources.modal.waitOpen();
  await resources.modal.closeWithEscape();

  await resources.goto("Patient");
  await expect(resources.recentGroup).toBeHidden();
});

test("the modal closes via the X and via Escape", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.close();
  await expect(resources.modal.root).toBeHidden();

  await resources.openCreate();
  await resources.modal.closeWithEscape();
  await expect(resources.modal.root).toBeHidden();
});

test("a created resource can be deleted from its modal", async ({ resources, page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "ToDelete" }] });
  await waitSearchable(request, "Patient", id);

  // Open it in the modal by searching for it and clicking the result row.
  await resources.goto("Patient");
  await resources.modal; // ensure page loaded
  await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
  await page.locator("[data-intent='run']").click();
  await page.locator(`#query-results-body a.url`).first().click();
  await resources.modal.waitOpen();
  await expect(resources.modal.subject).toContainText(id);

  page.once("dialog", (d) => d.accept()); // confirm delete
  await resources.modal.deleteButton.click();

  await expect(resources.modal.root).toBeHidden();
  // It's gone from the API.
  const res = await request.get(`/Patient/${id}`, {
    headers: { Accept: "application/fhir+json" },
  });
  expect(res.status()).toBe(410);
});
