import { test, expect } from "../pages/fixtures";

// The landing dashboard (/ui): stat cards, the resources-over-time chart, and
// its two link-based controls — the time-window selector and the per-type
// legend. Both are plain links (they work without JS), so they navigate.

test("the dashboard renders its stat cards and chart", async ({ dashboard }) => {
  await dashboard.goto();
  await expect(dashboard.statCards).toHaveCount(4);
  await expect(dashboard.chart).toBeVisible();
});

test("the time-window selector re-renders over the chosen window", async ({ page, dashboard }) => {
  await dashboard.goto();
  await dashboard.windowOption(/24h|24 h|day/i)
    .first()
    .click();
  await expect(page).toHaveURL(/window=/);
  await expect(dashboard.chart).toBeVisible();
});

test("the legend charts a chosen resource type", async ({ page, dashboard }) => {
  await dashboard.goto();
  const legend = dashboard.legendItems;
  if ((await legend.count()) === 0) test.skip(true, "no series to chart on an empty server");
  await legend.first().click();
  await expect(page).toHaveURL(/type=/);
  await expect(dashboard.chart).toBeVisible();
});
