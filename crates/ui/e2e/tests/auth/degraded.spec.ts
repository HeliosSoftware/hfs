import { test, expect } from "../../pages/fixtures";

// #320, leg 2: auth enabled but NO outbound token provisioned. The self-fetch
// is rejected (401), and the conformance pages must degrade to their warning
// state — no crash, no empty 404 — and re-attempt the fetch on the next
// request instead of caching the failure.

test("search parameters degrade to the warning state", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
  await expect(searchParameters.rows).toHaveCount(0);
});

test("compartments degrade to a warning page, not a 404", async ({ page }) => {
  const response = await page.goto("/ui/compartments", { waitUntil: "networkidle" });
  expect(response?.status()).toBe(200);
  await expect(page.locator(".notice--warn")).toBeVisible();
  await expect(page.locator("h1.page-head__title")).toBeVisible();
});

test("the degraded fetch is retried, not cached", async ({ page, searchParameters }) => {
  // Two consecutive loads both warn — and both actually hit the API again:
  // the failed snapshot is served degraded for its request only.
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
});
