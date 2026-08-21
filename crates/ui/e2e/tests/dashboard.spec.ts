import { test, expect } from "../pages/fixtures";
import { createResource } from "../pages/api";

// The landing dashboard (/ui) and its functional chart (#555): the type
// picker and the window selector are plain links (they work without JS —
// see the nojs project); the hover tooltip, the picker filter, and the
// picker's in-place chart-card swap (#599 — every option row, not just
// "view all") are the layered enhancements. Seeding rides through the
// ordinary FHIR API; the snapshot cache is outlasted by
// DashboardPage.waitForSeries.

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
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.windowOption(/24h/i).first().click();
  await expect(page).toHaveURL(/window=24h/);
  await expect(dashboard.chart).toBeVisible();
});

test("the picker toggles types on, capped at the palette", async ({ page, dashboard }) => {
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.openPicker();

  // Toggle every offered type on, a click at a time; the plotted set is
  // capped at six (the palette) — past that, the oldest swaps out (#555).
  // Picking swaps the chart card in place instead of navigating (#599
  // follow-up), so the picker only needs opening once — it stays open
  // across the whole loop.
  for (let i = 0; i < 7; i++) {
    await expect(dashboard.picker).toHaveAttribute("open", "");
    const off = page.locator(".chart-pick__option:not(.chart-pick__option--on)");
    if ((await off.count()) === 0) break;
    await off.first().click();
    await expect(page).toHaveURL(/types=/);
  }
  expect(await dashboard.seriesLines.count()).toBeGreaterThan(1);
  expect(await dashboard.seriesLines.count()).toBeLessThanOrEqual(6);

  // The legend removes: clicking an entry drops that series.
  const before = await dashboard.seriesLines.count();
  if (before > 1) {
    await dashboard.legendItems.first().click();
    await expect(page).toHaveURL(/types=/);
    expect(await dashboard.seriesLines.count()).toBe(before - 1);
  }
});

test("\"View all resources\" offers empty types, charts a flat line at 0, and keeps state across window/type changes", async ({
  page,
  dashboard,
}) => {
  await dashboard.goto();
  await dashboard.waitForSeries();

  // Off by default: a type this tenant never stored (Condition) is not
  // offered.
  await dashboard.openPicker();
  await expect(dashboard.pickerOption("Condition")).toHaveCount(0);

  // The toggle is a plain link (works without JS) that flips `?all=1`; with
  // JS it swaps the chart card in place instead of navigating (#599 follow-
  // up), so the picker menu it lives in stays open and there is no full
  // page load. A marker set on `window` before the click only survives a
  // same-document swap, not a hard reload — a simpler tell than watching
  // for navigation events, which also fire for the `history.pushState`
  // call the swap makes.
  await page.evaluate(() => {
    (window as unknown as { __e2eNavMarker: boolean }).__e2eNavMarker = true;
  });
  await dashboard.viewAllToggle.click();
  await expect(page).toHaveURL(/all=1/);
  await expect(dashboard.picker).toHaveAttribute("open", "");
  expect(
    await page.evaluate(() => (window as unknown as { __e2eNavMarker?: boolean }).__e2eNavMarker),
  ).toBe(true);

  // The chart's hover tooltip (#555) still works on the freshly swapped-in
  // nodes — dashboard.js re-binds it via the "hfs:chart-swapped" event.
  const swappedBox = await dashboard.chart.boundingBox();
  if (!swappedBox) throw new Error("chart has no box");
  await page.mouse.move(swappedBox.x + swappedBox.width * 0.6, swappedBox.y + swappedBox.height * 0.5);
  await expect(dashboard.tooltip).toBeVisible();
  await page.mouse.move(swappedBox.x - 40, swappedBox.y - 40);
  await expect(dashboard.tooltip).toBeHidden();

  // With the flag, the never-stored type is offered, with a real 0 count.
  const empty = dashboard.pickerOption("Condition");
  await expect(empty).toBeVisible();
  await expect(empty.locator(".chart-pick__count")).toHaveText("0");

  // Picking it charts a flat zero line — a real plotted series, not absent.
  // This click is also swapped in place (#599 follow-up covers every picker
  // option, not just the toggle), so the menu stays open here too.
  await empty.click();
  await expect(page).toHaveURL(/types=.*Condition/);
  await expect(page).toHaveURL(/all=1/);
  await expect(dashboard.picker).toHaveAttribute("open", "");
  expect(await dashboard.seriesLines.count()).toBeGreaterThan(0);

  // The flag survives a window change alongside the charted set.
  await dashboard.windowOption(/24h/i).first().click();
  await expect(page).toHaveURL(/window=24h/);
  await expect(page).toHaveURL(/all=1/);
});

test("the picker filter narrows the offered types", async ({ dashboard }) => {
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

test("the chart has no expand/collapse toggle", async ({ dashboard }) => {
  await dashboard.goto();
  await dashboard.waitForSeries();
  await expect(dashboard.page.locator('[href*="expand=1"]')).toHaveCount(0);
  await expect(dashboard.page.locator(".chart-card__tools a.pill--square")).toHaveCount(0);
  await expect(dashboard.chart).toHaveAttribute("viewBox", /0 0 1060 300/);
});

test("the chart's numbers are readable as a table", async ({ dashboard }) => {
  await dashboard.goto();
  await dashboard.waitForSeries();
  await dashboard.dataTableToggle.click();
  const table = dashboard.page.locator(".chart-table table.data-table");
  await expect(table).toBeVisible();
  expect(await table.locator("tbody tr").count()).toBeGreaterThan(0);
});
