import type { APIRequestContext, Page, Response } from "@playwright/test";
import { test, expect } from "../pages/fixtures";
import { createResource } from "../pages/api";
import { parseFirstRender, type DashboardPage, type FirstRender } from "../pages/dashboard";

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

/** `types` as a `?types=` value no earlier attempt of this test requested:
 * rotated by the retry count, since the snapshot cache keys on the joined
 * order (#1078). */
function coldSelection(...types: string[]): string {
  const turn = test.info().retry % types.length;
  return [...types.slice(turn), ...types.slice(0, turn)].join(",");
}

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
// response a click produced), because the waiting page's htmx auto-retry — or
// the periodic refresh every ready page schedules (see the "live refresh"
// describe below) — would otherwise swap the first render for a newer one
// before the DOM is looked at. Each view uses a selection no other test in the suite requests
// (Substance and Specimen are seeded only here), so its snapshot cache key is
// cold. The selection is part of that key in the order it was requested, so a
// CI retry rotates the order (`coldSelection`) and meets a cold key again
// instead of the snapshot its failed first attempt left in the cache.
test.describe("first view of a window (#1078)", () => {
  test.beforeEach(async ({ request }) => {
    await createResource(request, "Substance", { code: { text: "dashboard first view (#1078)" } });
    await createResource(request, "Specimen", { status: "available" });
  });

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
      expect(render.liveRefresh, "every ready page polls itself").toBe(true);
      expect(render.state, "the refresh carries a digest of the figures").toMatch(/^[0-9a-f]{16}$/);
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

    // A picker toggle swaps only the chart card (#599). A waiting page's
    // still-scheduled bounded auto-retry of the whole #dash-live region would
    // overwrite that swap with the previous selection (a known race, out of
    // scope here), so let that retry settle first — it is not a reload loop,
    // and a ready render has none to settle. A ready page's periodic refresh
    // (`data-dash-refresh`) is deliberately not waited on: every ready page
    // polls for as long as it is open, it follows the picker's URL, it stands
    // down while a picker fetch is in flight, and it keeps the open picker's
    // own node (covered below).
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

// #1078 follow-up: every ready dashboard re-requests its own #dash-live region —
// every 5s while its figures are moving (approximate: counted from recent
// writes, not yet reconciled with storage; or an import running), marked
// `data-dash-moving`, and every 10s once they settle — so an operator watching
// an import sees the figures rise without reloading, and a tab opened before
// the import started notices it too. While both the region on screen and the
// response are settled with the same `data-dash-state`, the response is
// dropped: an idle page is not re-rendered under the user. The poll follows
// the URL the picker pushed and drops a response whose URL went stale. It
// stands down only while the tab is hidden, a picker fetch is in flight, or
// keyboard focus sits inside the region outside the type picker: an open type
// picker (kept as the very same node across the swap), an open data table
// (reopened), a visible tooltip (re-shown for the pointer) and focus left by a
// mouse click all let the refresh through, since figures that froze until the
// user moved or closed something were the bug.
//
// Timing: a seeded tenant's snapshot is cached for at most 2s and a refresh
// waits up to 500ms for the fresh value, so a write reaches an open page on the
// next tick (5s moving, 10s settled). The e2e server also reconciles every 30s;
// once a reconcile makes the figures exact the swapped-in region slows to the
// settled tick. Each test therefore writes after marking the region, so the
// figures it waits for always differ from the ones on screen and the response
// is swapped, never dropped. Device, Location and Medication are seeded only
// here, and every test views a selection no other test (or retry, see
// coldSelection) requests.
test.describe("live refresh (#1078)", () => {
  test.skip(noChartData, "no count read path on this backend");

  /** How long new figures may take to land on an open page: one 10s settled
   * tick plus the ≤2s cache, with headroom for a machine busy running the
   * suite. */
  const FIGURES_LAND_MS = 30_000;

  /** How long a quiet default tenant may take to settle: a 30s reconcile pass
   * that begins after the last write (the beforeEach seeding) must run, and
   * the page's own 5s poll must then bring the exact figures in. */
  const SETTLE_MS = 90_000;

  /** The settled refresh interval, in seconds (`data-dash-refresh`). */
  const SETTLED_SECS = 10;

  test.beforeEach(async ({ request }) => {
    await createResource(request, "Device", { status: "active" });
    await createResource(request, "Location", { name: "live refresh (#1078)" });
    await createResource(request, "Medication", { code: { text: "live refresh (#1078)" } });
  });

  /** Every periodic-refresh request the page sends from now on: htmx's own
   * `HX-Request` GETs of `/ui` (the picker's swap is a plain fetch, and a
   * navigation is not an htmx request). */
  function recordRefreshes(page: Page): URL[] {
    const seen: URL[] = [];
    page.on("request", (r) => {
      const url = new URL(r.url());
      if (url.pathname === "/ui" && r.headers()["hx-request"] === "true" && !r.isNavigationRequest()) {
        seen.push(url);
      }
    });
    return seen;
  }

  /** Hard navigations (reloads included) the page starts from now on. */
  function recordNavigations(page: Page): string[] {
    const seen: string[] = [];
    page.on("request", (r) => {
      if (r.isNavigationRequest() && r.frame() === page.mainFrame()) seen.push(r.url());
    });
    return seen;
  }

  function exactCount(text: string | null): number {
    const value = Number((text ?? "").replace(/,/g, "").trim());
    if (!Number.isFinite(value)) throw new Error(`not an exact count: ${text}`);
    return value;
  }

  /** Opens `/ui?types=…&window=1h` on a selection cold for this test and
   * checks it is a ready, approximate page that polls itself. */
  async function openLive(dashboard: DashboardPage, ...types: string[]): Promise<void> {
    const render = await dashboard.gotoFirstRender(`?types=${coldSelection(...types)}&window=1h`);
    expect(render.notices, "a ready page").not.toContain("pending");
    expect(render.notices, "a ready page").not.toContain("series-pending");
    expect(render.notices, "figures written moments ago are not reconciled yet").toContain("approximate");
    expect(render.liveRefresh, "an approximate ready page polls itself").toBe(true);
    expect(render.moving, "approximate figures are moving").toBe(true);
    expect(render.autoRetry, "never the waiting page's bounded retry").toBe(false);
    await expect(dashboard.liveRefresh).toHaveCount(1);
  }

  /** Opens `/ui{query}` and waits — without reloading — until the page's own
   * poll has brought in settled figures: a `data-dash-refresh` region with no
   * `data-dash-moving`, and a plain `live` first notice. */
  async function waitSettled(dashboard: DashboardPage, query: string): Promise<void> {
    await dashboard.goto(query);
    await expect
      .poll(
        async () =>
          (await dashboard.settledRefresh.count()) === 1 &&
          (await dashboard.pendingAutoRetry.count()) === 0 &&
          (await dashboard.noticeKinds())[0] === "live",
        { message: "the quiet tenant's figures settle (a reconcile runs every 30s)", timeout: SETTLE_MS, intervals: [1000] },
      )
      .toBe(true);
  }

  /** Every refresh request (`/ui?…&notices=…`, htmx's own GET) the page sends
   * from now on, with the response's own `data-dash-state` and whether it was
   * moving once it arrives. */
  function recordRefreshResponses(page: Page): { url: URL; response: Promise<FirstRender | null> }[] {
    const seen: { url: URL; response: Promise<FirstRender | null> }[] = [];
    page.on("request", (r) => {
      const url = new URL(r.url());
      if (url.pathname !== "/ui" || !url.searchParams.has("notices") || r.headers()["hx-request"] !== "true") return;
      seen.push({
        url,
        response: r
          .response()
          .then(async (res) => (res ? parseFirstRender(await res.text()) : null))
          .catch(() => null),
      });
    });
    return seen;
  }

  const BODIES = {
    Device: { status: "active" },
    Location: { name: "live refresh (#1078)" },
    Medication: { code: { text: "live refresh (#1078)" } },
  } as const;

  /** `count` more `type` resources, five requests at a time. */
  async function write(request: APIRequestContext, type: keyof typeof BODIES, count: number): Promise<void> {
    for (let i = 0; i < count; i += 5) {
      await Promise.all(
        Array.from({ length: Math.min(5, count - i) }, () => createResource(request, type, BODIES[type])),
      );
    }
  }

  /** What the figures read before a write, to compare a refresh against. */
  async function figures(dashboard: DashboardPage, type: string): Promise<{ legend: number; chart: number }> {
    const legend = await dashboard.legendTotal(type);
    expect(legend, `${type} is charted`).not.toBeNull();
    return { legend: legend ?? 0, chart: exactCount(await dashboard.chartTotal.textContent()) };
  }

  /** Waits until a refresh has replaced the marked region (see
   * DashboardPage.markLive) and `type`'s legend entry and the chart total
   * both rose by `added` — no reload, no interaction. */
  async function expectFiguresRose(
    dashboard: DashboardPage,
    type: string,
    before: { legend: number; chart: number },
    added: number,
    message: string,
  ): Promise<void> {
    await expect
      .poll(
        async () => {
          if ((await dashboard.unrefreshedLive.count()) > 0) return false;
          const legend = await dashboard.legendTotal(type);
          const chart = Number(((await dashboard.chartTotal.allTextContents())[0] ?? "").replace(/,/g, "").trim());
          return legend !== null && legend >= before.legend + added && chart >= before.chart + added;
        },
        { message, timeout: FIGURES_LAND_MS, intervals: [500] },
      )
      .toBe(true);
  }

  test("the stored-resources card rises without a reload", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    await openLive(dashboard, "Device", "Location");
    await expect(dashboard.live).toHaveAttribute("hx-trigger", /every 5s/);
    await expect(dashboard.live).toHaveAttribute("data-dash-refresh", "5");
    await expect(dashboard.movingRefresh).toHaveCount(1);

    const url = page.url();
    const navigations = recordNavigations(page);
    const refreshes = recordRefreshes(page);
    // Survives a same-document swap, not a reload.
    await page.evaluate(() => {
      (window as unknown as { __e2eNoReload: boolean }).__e2eNoReload = true;
    });

    const asOfBefore = await dashboard.asOfDatetime();
    expect(asOfBefore, "the first notice carries an as-of time").not.toBeNull();
    const deviceBefore = await dashboard.legendTotal("Device");
    expect(deviceBefore, "Device is charted").not.toBeNull();
    const chartBefore = exactCount(await dashboard.chartTotal.textContent());
    await expect(dashboard.storedResourcesValue).toHaveText(/^\d+(\.\d[kM])?$/);
    const storedBefore = (await dashboard.storedResourcesValue.textContent()) ?? "";

    // An import in miniature: 25 more Devices, five requests at a time.
    const added = 25;
    await write(request, "Device", added);

    await expect
      .poll(
        async () => {
          const asOf = await dashboard.asOfDatetime();
          const device = await dashboard.legendTotal("Device");
          return (
            asOf !== null &&
            Date.parse(asOf) > Date.parse(asOfBefore ?? "") &&
            device !== null &&
            device >= (deviceBefore ?? 0) + added
          );
        },
        { message: "the as-of time advances and Device rises by the writes", timeout: FIGURES_LAND_MS, intervals: [500] },
      )
      .toBe(true);

    // Every figure fed by the same snapshot moved with it.
    expect(exactCount(await dashboard.chartTotal.textContent())).toBeGreaterThanOrEqual(chartBefore + added);
    await expect(dashboard.storedResourcesValue).toHaveText(/^\d+(\.\d[kM])?$/);
    if (/^\d+$/.test(storedBefore)) {
      // Compact past 999 ("1.4k" may not move for 25 writes); exact below.
      expect(Number(await dashboard.storedResourcesValue.textContent())).toBeGreaterThanOrEqual(
        Number(storedBefore) + added,
      );
    }

    // It was the page's own poll, not a reload or a navigation.
    expect(page.url()).toBe(url);
    expect(navigations, "no navigation, reload included").toEqual([]);
    expect(await page.evaluate(() => (window as unknown as { __e2eNoReload?: boolean }).__e2eNoReload)).toBe(true);
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);
    for (const sent of refreshes) {
      expect(sent.searchParams.get("types"), "the refresh keeps the charted set").toBe(new URL(url).searchParams.get("types"));
      expect(sent.searchParams.get("window")).toBe("1h");
      expect(sent.searchParams.has("notices"), "the refresh names the notices on screen").toBe(true);
    }
  });

  test("a refresh lands while the type picker is open and keeps it as it was", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    await openLive(dashboard, "Location", "Medication");
    const refreshes = recordRefreshes(page);

    // Open the picker and type into its filter, then mark the picker node: a
    // swap that re-rendered it would bring a node without the property.
    await dashboard.openPicker();
    await dashboard.pickerFilter.fill("med");
    await expect(dashboard.pickerOption("Medication")).toBeVisible();
    await expect(dashboard.pickerOption("Location")).toBeHidden();
    await dashboard.picker.evaluate((el) => {
      (el as unknown as { __e2ePicker: boolean }).__e2ePicker = true;
    });
    await dashboard.markLive();
    const before = await figures(dashboard, "Medication");

    const added = 10;
    await write(request, "Medication", added);
    await expectFiguresRose(dashboard, "Medication", before, added, "a refresh lands with the picker open and the figures rise");
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);

    // The very same picker node, exactly as the user left it.
    await expect(dashboard.picker).toHaveCount(1);
    expect(
      await dashboard.picker.evaluate((el) => (el as unknown as { __e2ePicker?: boolean }).__e2ePicker === true),
      "the open picker is the same element after the refresh",
    ).toBe(true);
    await expect(dashboard.picker).toHaveAttribute("open", "");
    await expect(dashboard.pickerFilter).toHaveValue("med");
    await expect(dashboard.pickerFilter).toBeFocused();
    await expect(dashboard.pickerOption("Medication")).toBeVisible();
    await expect(dashboard.pickerOption("Location")).toBeHidden();
  });

  test("a refresh lands while the mouse rests on the chart", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    await openLive(dashboard, "Device", "Medication");
    const refreshes = recordRefreshes(page);

    const box = await dashboard.chart.boundingBox();
    if (!box) throw new Error("chart has no box");
    await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.5);
    await expect(dashboard.tooltip).toBeVisible();
    await dashboard.markLive();
    const before = await figures(dashboard, "Device");

    const added = 10;
    await write(request, "Device", added);
    await expectFiguresRose(dashboard, "Device", before, added, "a refresh lands under the resting pointer and the figures rise");
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);

    // The pointer never moved: the new chart's tooltip is showing for it.
    await expect(dashboard.tooltip).toBeVisible();
    await expect(dashboard.tooltip.locator(".chart-tip__row").first()).toBeVisible();
    await expect(dashboard.tooltip).toContainText("Device");
  });

  test("a refresh keeps the data table open", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    await openLive(dashboard, "Location", "Device");
    const refreshes = recordRefreshes(page);

    await dashboard.dataTableToggle.click();
    await expect(dashboard.dataTable).toHaveAttribute("open", "");
    await expect(dashboard.dataTable.locator("table.data-table")).toBeVisible();
    await dashboard.markLive();
    const before = await figures(dashboard, "Location");

    const added = 10;
    await write(request, "Location", added);
    await expectFiguresRose(dashboard, "Location", before, added, "a refresh lands with the data table open and the figures rise");
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);

    await expect(dashboard.dataTable).toHaveAttribute("open", "");
    await expect(dashboard.dataTable.locator("table.data-table")).toBeVisible();
  });

  test("a mouse click inside the chart does not stop the refresh", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    await openLive(dashboard, "Medication", "Location");
    const refreshes = recordRefreshes(page);

    // Open and close the data table by mouse: nothing navigates, and the
    // click leaves focus on its <summary>, inside #dash-live — focus that is
    // not :focus-visible, so it must not hold the poll back.
    await dashboard.dataTableToggle.click();
    await expect(dashboard.dataTable).toHaveAttribute("open", "");
    await dashboard.dataTableToggle.click();
    await expect(dashboard.dataTable).not.toHaveAttribute("open", "");
    expect(
      await page.evaluate(() => {
        const active = document.activeElement;
        return !!active && active !== document.body && !!document.getElementById("dash-live")?.contains(active);
      }),
      "the click left focus inside the region",
    ).toBe(true);
    await dashboard.markLive();
    const before = await figures(dashboard, "Medication");

    const added = 10;
    await write(request, "Medication", added);
    await expectFiguresRose(dashboard, "Medication", before, added, "a refresh lands after a mouse click inside the region");
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);
    await expect(page).not.toHaveURL(/focus=/);
  });

  test("a refresh follows the picker's URL", async ({ page, request, dashboard }) => {
    test.setTimeout(120_000);
    const render = await dashboard.gotoFirstRender(`?types=${coldSelection("Medication", "Device")}&window=1h`);
    expect(render.liveRefresh, "an approximate ready page polls itself").toBe(true);
    await expect(dashboard.liveRefresh).toHaveCount(1);
    const refreshes = recordRefreshes(page);

    // Toggle Location on: the chart card is swapped in place and the URL
    // pushed; #dash-live itself (and its hx-get) is not replaced.
    await dashboard.openPicker();
    const option = dashboard.pickerOption("Location");
    await expect(option).not.toHaveClass(/chart-pick__option--on/);
    const swapped = page.waitForResponse(
      (r) => r.request().resourceType() === "fetch" && /[?&]types=[^&]*Location/.test(r.url()),
    );
    await option.click();
    await swapped;
    await expect(page).toHaveURL(/[?&]types=[^&]*Location/);
    await expect(dashboard.legendItems.filter({ hasText: "Location" })).toHaveCount(1);
    const pickedUrl = page.url();
    const legendAfterPick = await dashboard.legendItems.count();

    // Close the picker so the refreshed one is re-rendered too (an open
    // picker keeps its own node, option states included).
    await dashboard.picker.locator("summary").click();
    await expect(dashboard.picker).not.toHaveAttribute("open", "");

    // Mark the region on screen; the refresh's outerHTML swap replaces it
    // with a node that has no such mark. A write right after the mark makes
    // sure the next refresh brings different figures: a region that settled
    // in the meantime would otherwise drop an unchanged settled response and
    // keep the mark.
    await dashboard.markLive();
    await createResource(request, "Medication", { code: { text: "live refresh (#1078)" } });
    await expect(dashboard.unrefreshedLive, "the next refresh swaps the region").toHaveCount(0, {
      timeout: FIGURES_LAND_MS,
    });
    await expect(dashboard.live).toHaveCount(1);

    // The refresh asked for the picker's selection and kept it on screen.
    expect(refreshes.length).toBeGreaterThan(0);
    const sent = refreshes[refreshes.length - 1];
    expect(sent.searchParams.get("types") ?? "", "the refresh requests the pushed URL").toContain("Location");
    expect(page.url(), "the refresh does not restore the old URL").toBe(pickedUrl);
    await expect(dashboard.legendItems.filter({ hasText: "Location" })).toHaveCount(1);
    await expect(dashboard.legendItems).toHaveCount(legendAfterPick);
    await expect(dashboard.pickerOption("Location")).toHaveClass(/chart-pick__option--on/);
  });

  test("a refresh re-announces only the notices that changed", async ({ page }) => {
    // What the poll sends: `?notices=` names the kinds already on screen, and
    // exactly those lines come back `aria-live="off"`; any other kind stays
    // `polite`. Read from the response body, independent of which kinds this
    // snapshot happens to carry.
    const view = `/ui?types=${coldSelection("Location", "Device")}&window=1h`;
    const noticeLines = async (query: string) => {
      const res = await page.request.get(`${view}${query}`, { headers: { "HX-Request": "true" } });
      expect(res.ok()).toBe(true);
      return [...(await res.text()).matchAll(/<p\b[^>]*\bdata-dash-notice="([^"]*)"[^>]*>/g)].map((m) => ({
        kind: m[1],
        ariaLive: /\baria-live="([^"]*)"/.exec(m[0])?.[1],
      }));
    };

    const first = await noticeLines("");
    expect(first.length, "a ready page names its figures").toBeGreaterThan(0);
    for (const line of first) expect(line.ariaLive, `${line.kind} on a first render`).toBe("polite");

    const quiet = ["approximate", "live"];
    const again = await noticeLines(`&notices=${quiet.join(",")}`);
    expect(again.length).toBeGreaterThan(0);
    for (const line of again) {
      expect(line.ariaLive, `${line.kind} after notices=${quiet}`).toBe(quiet.includes(line.kind) ? "off" : "polite");
    }
  });

  // The user's bug: a Home tab opened before an import started was rendered
  // with settled, exact figures, carried no poll, and never moved. Settled
  // pages now poll every 10s. Driven on the default tenant: the UI's tenant is
  // the user's stored choice of a provisioned tenant, and provisioning one
  // seeds ~1.4k conformance resources (see dashboard-tenants.spec.ts) — far
  // too slow for this file. Tests run one at a time on the shared server, so
  // the default tenant is quiet once this test's own beforeEach writes have
  // been reconciled.
  test("a tab opened while figures are settled starts refreshing when writes arrive", async ({
    page,
    request,
    dashboard,
  }) => {
    test.setTimeout(240_000);
    const query = `?types=${coldSelection("Device", "Location")}&window=1h`;
    await waitSettled(dashboard, query);

    // Now open the tab the way the user did, on figures that are already
    // settled: the first render polls, slowly, and is not moving.
    const render = await dashboard.gotoFirstRender(query);
    expect(render.notices[0], "exact figures").toBe("live");
    expect(render.notices, "exact figures").not.toContain("approximate");
    expect(render.autoRetry, "never the waiting page's bounded retry").toBe(false);
    expect(render.liveRefresh, "a settled page still polls itself").toBe(true);
    expect(render.moving, "settled figures are not moving").toBe(false);
    expect(render.state).toMatch(/^[0-9a-f]{16}$/);
    await expect(dashboard.settledRefresh).toHaveCount(1);
    await expect(dashboard.live).toHaveAttribute("hx-trigger", new RegExp(`every ${SETTLED_SECS}s`));
    await expect(dashboard.live).toHaveAttribute("data-dash-refresh", String(SETTLED_SECS));

    const url = page.url();
    const navigations = recordNavigations(page);
    const refreshes = recordRefreshes(page);
    await page.evaluate(() => {
      (window as unknown as { __e2eNoReload: boolean }).__e2eNoReload = true;
    });
    await dashboard.markLive();
    const deviceBefore = await dashboard.legendTotal("Device");
    expect(deviceBefore, "Device is charted").not.toBeNull();

    // The import starts after the tab was opened.
    const added = 25;
    await write(request, "Device", added);

    await expect
      .poll(
        async () => {
          if ((await dashboard.unrefreshedLive.count()) > 0) return false;
          if ((await dashboard.movingRefresh.count()) > 0) return true;
          const device = await dashboard.legendTotal("Device");
          return device !== null && device >= (deviceBefore ?? 0) + added;
        },
        { message: "the settled tab picks the writes up on its own poll", timeout: FIGURES_LAND_MS, intervals: [500] },
      )
      .toBe(true);
    await expect
      .poll(async () => ((await dashboard.legendTotal("Device")) ?? 0) >= (deviceBefore ?? 0) + added, {
        message: "Device rises by the writes",
        timeout: FIGURES_LAND_MS,
        intervals: [500],
      })
      .toBe(true);

    expect(page.url()).toBe(url);
    expect(navigations, "no navigation, reload included").toEqual([]);
    expect(await page.evaluate(() => (window as unknown as { __e2eNoReload?: boolean }).__e2eNoReload)).toBe(true);
    expect(refreshes.length, "the figures arrived through the periodic refresh").toBeGreaterThan(0);
  });

  // The other half of the contract: watching a quiet page costs a request per
  // tick, never a re-render. The digest includes the current UTC minute (the
  // 1h chart's buckets roll every minute), so a settled page is legitimately
  // swapped once a minute; the observation below is placed inside one minute.
  test("a settled page is not re-rendered when nothing changed", async ({ page, dashboard }) => {
    test.setTimeout(240_000);
    const query = `?types=${coldSelection("Location", "Medication")}&window=1h`;
    await waitSettled(dashboard, query);

    const OBSERVE_MS = 2 * SETTLED_SECS * 1000 + 2_000;
    const isRefresh = (r: { url(): string; headers(): Record<string, string> }) => {
      const url = new URL(r.url());
      return url.pathname === "/ui" && url.searchParams.has("notices") && r.headers()["hx-request"] === "true";
    };
    // Start early enough in a UTC minute that one tick lands, and the whole
    // observation ends, before the minute rolls over.
    let aligned = false;
    for (let attempt = 0; attempt < 3 && !aligned; attempt++) {
      const second = new Date().getUTCSeconds();
      if (second < 1 || second > 15) await page.waitForTimeout(((61 - second) % 60) * 1000 + 500);
      // Let one tick of this minute land (swapped or dropped), so the region
      // on screen already carries this minute's digest.
      await page.waitForResponse((r) => isRefresh(r.request()), { timeout: 15_000 });
      await page.waitForTimeout(500);
      aligned = new Date().getUTCSeconds() * 1000 + OBSERVE_MS < 55_000;
    }
    expect(aligned, "an observation window inside one UTC minute").toBe(true);

    await dashboard.markLive();
    const stateOnScreen = await dashboard.live.getAttribute("data-dash-state");
    const movingOnScreen = (await dashboard.movingRefresh.count()) > 0;
    const refreshes = recordRefreshResponses(page);
    const navigations = recordNavigations(page);
    await page.waitForTimeout(OBSERVE_MS);

    expect(refreshes.length, "the settled page kept polling").toBeGreaterThanOrEqual(2);
    const responses = await Promise.all(refreshes.map((r) => r.response));
    // Anything else writing to the default tenant (another spec's leftover
    // import job, a write from a concurrent run against a shared server)
    // legitimately moves the figures; the no-re-render contract only holds
    // for a quiet page, so stand down rather than assert on moving figures.
    test.skip(
      movingOnScreen || responses.some((r) => r?.moving),
      "the default tenant's figures moved during the observation window",
    );
    for (const render of responses) {
      expect(render, "each refresh was answered with a ready region").not.toBeNull();
      expect(render?.state, "a quiet page answers with the same digest").toBe(stateOnScreen);
    }
    await expect(dashboard.unrefreshedLive, "no response was swapped in").toHaveCount(1);
    await expect(dashboard.settledRefresh).toHaveCount(1);
    expect(navigations, "no navigation, reload included").toEqual([]);
  });
});
