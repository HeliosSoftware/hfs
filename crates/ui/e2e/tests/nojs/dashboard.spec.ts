import { test, expect } from "../../pages/fixtures";
import { createResource } from "../../pages/api";

// The chart card without JavaScript (#555): the picker, the window selector,
// the expand toggle, and the legend are plain links, and the tabular
// alternative is a native <details> — everything works, only the hover
// tooltip is absent.

test("the chart's controls work as plain links with JavaScript off", async ({ page, request }) => {
  await createResource(request, "Patient", { name: [{ family: "NojsChart" }] });

  // Outlast the snapshot cache without any client script.
  let series = 0;
  for (let attempt = 0; attempt < 12 && series === 0; attempt++) {
    await page.goto("/ui", { waitUntil: "domcontentloaded" });
    series = await page.locator("svg.chart polyline").count();
    if (series === 0) await page.waitForTimeout(2000);
  }
  expect(series).toBeGreaterThan(0);

  // The picker is a native <details>; its options navigate.
  await page.locator(".chart-pick summary").click();
  const option = page.locator("[data-pick-name]").first();
  await option.click();
  await expect(page).toHaveURL(/types=/);
  await expect(page.locator("svg.chart")).toBeVisible();

  // Window selector and expand are links too.
  await page.locator(".window-picker__option", { hasText: "24h" }).click();
  await expect(page).toHaveURL(/window=24h/);
  await page.locator(".chart-card__tools a.pill--square").click();
  await expect(page).toHaveURL(/expand=1/);

  // The tabular alternative opens natively.
  await page.locator(".chart-table > summary").click();
  await expect(page.locator(".chart-table table")).toBeVisible();
});
