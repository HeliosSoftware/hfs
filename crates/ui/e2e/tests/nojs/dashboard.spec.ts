import { test, expect } from "../../pages/fixtures";
import { createResource } from "../../pages/api";

// The chart card without JavaScript (#555): the picker, the window selector,
// and the legend are plain links, and the tabular alternative is a native
// <details> — everything works, only the hover tooltip is absent.

test("the chart's controls work as plain links with JavaScript off", async ({ page, request }) => {
  test.skip(process.env.HFS_E2E_NO_CHART_DATA === "1", "no count read path on this backend");
  await createResource(request, "Patient", { name: [{ family: "NojsChart" }] });

  // Outlast the snapshot cache without any client script.
  let series = 0;
  for (let attempt = 0; attempt < 12 && series === 0; attempt++) {
    await page.goto("/ui", { waitUntil: "domcontentloaded" });
    series = await page.locator("svg.chart polyline.series").count();
    if (series === 0) await page.waitForTimeout(2000);
  }
  expect(series).toBeGreaterThan(0);

  // The picker is a native <details>; its options navigate.
  await page.locator(".chart-pick summary").click();
  const option = page.locator("[data-pick-name]").first();
  await option.click();
  await expect(page).toHaveURL(/types=/);
  await expect(page.locator("svg.chart")).toBeVisible();

  // The legend is plain links too: clicking focuses a series without
  // removing anything (#602), and works with no script at all.
  const plotted = await page.locator("svg.chart polyline.series").count();
  if (plotted > 1) {
    await page.locator("a.chart-legend__item").first().click();
    await expect(page).toHaveURL(/focus=/);
    expect(await page.locator("svg.chart polyline.series").count()).toBe(plotted);
    await page.locator(".chart-legend__item--focused").click();
    await expect(page).not.toHaveURL(/focus=/);
  }

  // Window selector is a link too.
  await page.locator(".window-picker__option", { hasText: "24h" }).click();
  await expect(page).toHaveURL(/window=24h/);

  // The tabular alternative opens natively.
  await page.locator(".chart-table > summary").click();
  await expect(page.locator(".chart-table table")).toBeVisible();
});
