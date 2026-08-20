import { test, expect } from "../pages/fixtures";
import { createResource } from "../pages/api";

// The landing dashboard (/ui) and its functional chart (#555): the type
// picker, the window selector, and the expand toggle are plain links (they
// work without JS — see the nojs project); the hover tooltip and the picker
// filter are the layered enhancements. Seeding rides through the ordinary
// FHIR API; the snapshot cache is outlasted by DashboardPage.waitForSeries.

// Backends whose primary store has no count read path (S3 — the composite
// delegates counts to the primary) cannot feed the chart; the matrix sets
// this flag for them and the chart specs stand down. The job-cards spec
// still runs — the cards read job state, not counts.
const noChartData = process.env.HFS_E2E_NO_CHART_DATA === "1";

test.beforeEach(async ({ request }) => {
  await createResource(request, "Patient", { name: [{ family: "Chart" }] });
  await createResource(request, "Observation", {
    status: "final",
    code: { coding: [{ system: "http://loinc.org", code: "8867-4" }] },
  });
  await createResource(request, "Encounter", {
    status: "finished",
    class: { system: "http://terminology.hl7.org/CodeSystem/v3-ActCode", code: "AMB" },
  });
});

test("the dashboard renders its stat cards and a charted series", async ({ dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await expect(dashboard.statCards).toHaveCount(5);
  await dashboard.waitForSeries();
  await expect(dashboard.chart).toBeVisible();
  // The SVG has an accessible name, not aria-hidden (#555).
  await expect(dashboard.chart).toHaveAttribute("aria-label", /./);
  expect(await dashboard.chart.getAttribute("aria-hidden")).toBeNull();
});

test("export and import job cards show real counts and link to their pages", async ({ dashboard }) => {
  await dashboard.goto();

  const exportCard = dashboard.exportJobsCard;
  await expect(exportCard).toBeVisible();
  await expect(exportCard).toHaveAttribute("href", "/ui/bulk-export");
  await expect(exportCard.locator(".stat__value")).toHaveText(/^\d+$/);
  await expect(exportCard.locator(".stat__sub")).toHaveText(/running \(\d+ queued\)/);

  const importCard = dashboard.importJobsCard;
  await expect(importCard).toBeVisible();
  await expect(importCard).toHaveAttribute("href", "/ui/bulk-import");
  await expect(importCard.locator(".stat__value")).toHaveText(/^\d+$/);
  await expect(importCard.locator(".stat__sub")).toHaveText("active");
});

test("the time-window selector re-renders over the chosen window", async ({ page, dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.windowOption(/24h/i).first().click();
  await expect(page).toHaveURL(/window=24h/);
  await expect(dashboard.chart).toBeVisible();
});

test("the picker toggles types on, capped at the palette", async ({ page, dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();

  // Toggle every offered type on, a click at a time; the plotted set is
  // capped at six (the palette) — past that, the oldest swaps out (#555).
  for (let i = 0; i < 7; i++) {
    await dashboard.openPicker();
    const off = page.locator(".chart-pick__option:not(.chart-pick__option--on)");
    if ((await off.count()) === 0) break;
    await off.first().click();
    await expect(page).toHaveURL(/types=/);
  }
  expect(await dashboard.seriesLines.count()).toBeGreaterThan(1);
  expect(await dashboard.seriesLines.count()).toBeLessThanOrEqual(6);
});

test("legend click focuses a series; clicking it again restores the shared view", async ({
  page,
  dashboard,
}) => {
  test.skip(process.env.HFS_E2E_NO_CHART_DATA === "1", "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  const before = await dashboard.seriesLines.count();
  test.skip(before < 2, "focus needs at least two series");

  // Focus: nothing is removed, the URL carries the focus, the focused line
  // and legend entry are marked, the rest recede (#602).
  await dashboard.legendItems.first().click();
  await expect(page).toHaveURL(/focus=/);
  expect(await dashboard.seriesLines.count()).toBe(before);
  await expect(page.locator(".series--focused")).toHaveCount(1);
  await expect(page.locator(".series--receded")).toHaveCount(before - 1);
  await expect(page.locator(".chart-legend__item--focused")).toHaveCount(1);

  // The way back is the same entry.
  await page.locator(".chart-legend__item--focused").click();
  await expect(page).not.toHaveURL(/focus=/);
  await expect(page.locator(".series--focused")).toHaveCount(0);
  expect(await dashboard.seriesLines.count()).toBe(before);

  // The line itself is the same link: clicking a plotted series focuses it
  // (native SVG anchor, via the widened hit corridor). Playwright's default
  // click aims at the bounding-box centre — empty air for a polyline with
  // pointer-events: stroke — so aim at an actual vertex, mapped from
  // viewBox units to screen pixels.
  const hit = page.locator(".series-hit").first();
  const vertex = (await hit.getAttribute("points"))!.split(" ")[2].split(",").map(Number);
  const svg = page.locator("svg.chart");
  const viewBox = (await svg.getAttribute("viewBox"))!.split(" ").map(Number);
  const svgBox = (await svg.boundingBox())!;
  await page.mouse.click(
    svgBox.x + (vertex[0] / viewBox[2]) * svgBox.width,
    svgBox.y + (vertex[1] / viewBox[3]) * svgBox.height,
  );
  await expect(page).toHaveURL(/focus=/);
  await expect(page.locator(".series--focused")).toHaveCount(1);
});

test("the picker filter narrows the offered types", async ({ dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.openPicker();
  const all = await dashboard.page.locator("[data-pick-name]:not([hidden])").count();
  await dashboard.pickerFilter.fill("patient");
  const narrowed = await dashboard.page.locator("[data-pick-name]:not([hidden])").count();
  expect(narrowed).toBeLessThanOrEqual(all);
  expect(narrowed).toBeGreaterThan(0);
  await expect(dashboard.pickerOption("Patient")).toBeVisible();
});

test("hovering the chart shows the tooltip readout", async ({ dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  const box = await dashboard.chart.boundingBox();
  if (!box) throw new Error("chart has no box");
  await dashboard.page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.5);
  await expect(dashboard.tooltip).toBeVisible();
  await expect(dashboard.tooltip.locator(".chart-tip__row").first()).toBeVisible();
  await dashboard.page.mouse.move(box.x - 40, box.y - 40);
  await expect(dashboard.tooltip).toBeHidden();
});

test("expand renders the taller plot and collapses back", async ({ page, dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.expandToggle.click();
  await expect(page).toHaveURL(/expand=1/);
  await expect(dashboard.chart).toHaveAttribute("viewBox", /0 0 1060 520/);
  await dashboard.expandToggle.click();
  await expect(page).not.toHaveURL(/expand=1/);
  await expect(dashboard.chart).toHaveAttribute("viewBox", /0 0 1060 300/);
});

test("the chart's numbers are readable as a table", async ({ dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.dataTableToggle.click();
  const table = dashboard.page.locator(".chart-table table.data-table");
  await expect(table).toBeVisible();
  expect(await table.locator("tbody tr").count()).toBeGreaterThan(0);
});
