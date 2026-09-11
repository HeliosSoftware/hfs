import type { Page, Response } from "@playwright/test";
import { test, expect } from "../pages/fixtures";
import { createResource } from "../pages/api";
import { parseFirstRender, type FirstRender } from "../pages/dashboard";

// The landing dashboard (/ui) and its functional chart (#555): the type
// picker and the window selector are plain links (they work without JS —
// see the nojs project); the hover tooltip, the picker filter, and the
// picker's in-place chart-card swap (#599 — every option row, not just
// "view all") are the layered enhancements. Seeding rides through the
// ordinary FHIR API. The older tests below outlast the snapshot cache with
// DashboardPage.waitForSeries; the "first view" tests (#1078) at the bottom
// deliberately do not — they assert on what the first response rendered.

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

test("\"View all resources\" offers empty types, charts a flat line at 0, and keeps state across window/type changes", async ({
  page,
  dashboard,
}) => {
  test.skip(noChartData, "no count read path on this backend");
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
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  // What is offered comes from the snapshot, which is served stale while it
  // refreshes — the Patient seeded in beforeEach is not necessarily in the
  // first load's option list. Outlast that before filtering for it: this test
  // is about the filter, not about how quickly a new type reaches the picker.
  await dashboard.waitForPickerOption("Patient");
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

test("the chart has no expand/collapse toggle", async ({ dashboard }) => {
  test.skip(noChartData, "no count read path on this backend");
  await dashboard.goto();
  await dashboard.waitForSeries();
  await expect(dashboard.page.locator('[href*="expand=1"]')).toHaveCount(0);
  await expect(dashboard.page.locator(".chart-card__tools a.pill--square")).toHaveCount(0);
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

// #1078: during an import the 1h chart sat on "Waiting for the live figures…"
// because every window switch was a cold snapshot computed from storage
// aggregates. A seeded tenant is now served from in-memory write counters, so
// a window or selection nobody has viewed yet charts on its first render, and
// a window whose series are genuinely slow still shows the real headline
// figures while only the chart area waits (`series-pending`) — never the
// blank `pending` page.
//
// These tests never reload and never lean on waitForSeries: they read the
// server's own response for each view (DashboardPage.gotoFirstRender, or the
// response a click produced), because the waiting page's htmx auto-retry would
// otherwise swap a waiting first render for a ready one before the DOM is
// looked at. Each view uses a selection no other test in the suite requests
// (Substance and Specimen are seeded only here), so its snapshot cache key is
// cold. The selection is part of that key in the order it was requested, so a
// CI retry rotates the order (`coldSelection`) and meets a cold key again
// instead of the snapshot its failed first attempt left in the cache.
test.describe("first view of a window (#1078)", () => {
  test.beforeEach(async ({ request }) => {
    await createResource(request, "Substance", { code: { text: "dashboard first view (#1078)" } });
    await createResource(request, "Specimen", { status: "available" });
  });

  /** `types` as a `?types=` value no earlier attempt of this test requested:
   * rotated by the retry count, since the cache keys on the joined order. */
  function coldSelection(...types: string[]): string {
    const turn = test.info().retry % types.length;
    return [...types.slice(turn), ...types.slice(0, turn)].join(",");
  }

  /** A render that is not the blank waiting page: no "—" in the headline
   * cards, no invented figures, and a chart — or, at worst, this window's
   * series still loading under real cards. */
  function expectFiguresShown(render: FirstRender, step: string): void {
    expect(render.notices, `${step}: never the blank pending page`).not.toContain("pending");
    expect(render.notices, `${step}: no invented figures`).not.toContain("sample");
    expect(render.unavailableCards, `${step}: headline cards show figures`).toBe(0);
    if (render.series === 0) {
      expect(render.notices, `${step}: a chart, or its series pending`).toContain("series-pending");
    }
  }

  /** Reads the live DOM's headline cards once, without auto-waiting: an
   * auto-retrying assertion would wait out a transient "—" instead of
   * catching it. */
  async function expectCardsInDom(dashboard: { cardsShowFigures(): Promise<boolean> }, step: string) {
    expect(await dashboard.cardsShowFigures(), `${step}: headline cards show figures`).toBe(true);
  }

  /** The response a click on a plain link navigates to. */
  function navigationTo(page: Page, pattern: RegExp): Promise<Response> {
    return page.waitForResponse((r) => r.request().isNavigationRequest() && pattern.test(r.url()));
  }

  for (const span of ["1h", "24h", "30d"] as const) {
    test(`${span} renders chart and headline figures on first view`, async ({ page, dashboard }) => {
      test.skip(noChartData, "no count read path on this backend");

      const render = await dashboard.gotoFirstRender(`?window=${span}&types=${coldSelection("Substance", "Patient")}`);
      expect(render.notices).not.toContain("pending");
      expect(render.notices).not.toContain("series-pending");
      expect(render.notices).not.toContain("sample");
      expect(render.autoRetry, "a ready page schedules no auto-retry").toBe(false);
      expect(render.chartEmpty, "the chart area is not waiting").toBe(false);
      expect(render.series).toBeGreaterThanOrEqual(1);
      expect(render.unavailableCards).toBe(0);

      // The DOM is that same render: nothing was retried or reloaded into it.
      await expect(page).not.toHaveURL(/retry=/);
      await expect(dashboard.pendingAutoRetry).toHaveCount(0);
      await expect(dashboard.chart).toBeVisible();
      await expect(dashboard.chartWaiting).toHaveCount(0);
      expect(await dashboard.seriesLines.count()).toBeGreaterThanOrEqual(1);
      // The type written moments ago is charted on this first view.
      await expect(dashboard.legendItems.filter({ hasText: "Substance" })).toHaveCount(1);
      await expectCardsInDom(dashboard, span);
      await expect(dashboard.resourceTypesCard.locator(".stat__value")).toHaveText(/^\d+$/);
      await expect(
        dashboard.statCards.filter({ hasText: "Stored Resources" }).locator(".stat__value"),
      ).toHaveText(/^\d+(\.\d[kM])?$/);
    });
  }

  test("switching windows and charted types keeps the headline cards", async ({ page, dashboard }) => {
    test.skip(noChartData, "no count read path on this backend");

    const first = await dashboard.gotoFirstRender(`?window=1h&types=${coldSelection("Specimen", "Observation")}`);
    expectFiguresShown(first, "1h");
    await expectCardsInDom(dashboard, "1h");

    // The window selector is a plain link even with JS on: each switch is a
    // full navigation to a key this test has not viewed yet.
    for (const span of ["24h", "30d"] as const) {
      const response = navigationTo(page, new RegExp(`[?&]window=${span}(&|$)`));
      await dashboard.windowOption(new RegExp(`^${span}$`)).click();
      const render = parseFirstRender(await (await response).text());
      await page.waitForURL(new RegExp(`[?&]window=${span}(&|$)`), { waitUntil: "domcontentloaded" });
      expectFiguresShown(render, span);
      await expectCardsInDom(dashboard, span);
      await expect(dashboard.notice("pending")).toHaveCount(0);
      await expect(dashboard.chart.or(dashboard.notice("series-pending"))).toBeVisible();
    }

    // A picker toggle swaps only the chart card (#599). A still-scheduled
    // auto-retry of the whole #dash-live region would overwrite that swap with
    // the previous selection (a known race, out of scope here), so let the
    // page's own bounded retry settle first — it is not a reload loop, and a
    // ready render has none to settle.
    await expect(dashboard.pendingAutoRetry).toHaveCount(0, { timeout: 15_000 });
    await dashboard.openPicker();
    const option = dashboard.pickerOption("Encounter");
    await expect(option).not.toHaveClass(/chart-pick__option--on/);
    const swapped = page.waitForResponse(
      (r) => r.request().resourceType() === "fetch" && /[?&]types=[^&]*Encounter/.test(r.url()),
    );
    await option.click();
    const toggled = parseFirstRender(await (await swapped).text());
    await expect(page).toHaveURL(/[?&]types=[^&]*Encounter/);
    expectFiguresShown(toggled, "toggle Encounter");
    await expectCardsInDom(dashboard, "toggle Encounter");
    await expect(dashboard.notice("pending")).toHaveCount(0);
    await expect(dashboard.chart.or(dashboard.notice("series-pending"))).toBeVisible();
    if (toggled.series > 0) {
      await expect(dashboard.legendItems.filter({ hasText: "Encounter" })).toHaveCount(1);
    }
  });

  test("figures carry an as-of label", async ({ request, dashboard }) => {
    test.skip(noChartData, "no count read path on this backend");

    // A write right before the first view of a fresh key: the counters saw it,
    // so the figures are either labelled approximate or — if a reconcile ran
    // in between — plain live. Either is a real reading; which one is timing.
    await createResource(request, "Substance", { code: { text: "as-of label (#1078)" } });
    const viewedAt = Date.now();
    const render = await dashboard.gotoFirstRender(`?window=1h&types=${coldSelection("Specimen", "Substance")}`);
    expect(render.notices).not.toContain("sample");
    expect(render.notices).not.toContain("pending");
    expect(render.notices[0]).toMatch(/^(live|approximate)$/);

    const kinds = await dashboard.noticeKinds();
    expect(kinds, "no invented figures").not.toContain("sample");
    const firstLine = dashboard.notices.first();
    await expect(firstLine).toHaveAttribute("data-dash-notice", /^(live|approximate)$/);
    await expect(firstLine).toHaveAttribute("aria-live", "polite");

    // The stamp rides on the first line only.
    await expect(dashboard.asOfTime).toHaveCount(1);
    const stamp = firstLine.locator("time[datetime]");
    await expect(stamp).toHaveCount(1);
    const datetime = (await stamp.getAttribute("datetime")) ?? "";
    expect(datetime).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/);
    const readAt = Date.parse(datetime);
    expect(Number.isNaN(readAt)).toBe(false);
    // A fresh key is read for this very request. The margin only absorbs
    // clock skew between the browser and a server on another host (the
    // backend matrix); it is not a staleness budget.
    expect(Math.abs(readAt - viewedAt)).toBeLessThan(5 * 60_000);
    // The visible text names the same instant, in UTC.
    await expect(stamp).toContainText(new Date(readAt).toISOString().slice(11, 19));
    await expect(stamp).toContainText("UTC");
  });
});
