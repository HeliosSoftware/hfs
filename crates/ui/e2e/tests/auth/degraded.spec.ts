import { test, expect } from "../../pages/fixtures";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

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

// #1560: auth on and no interactive login — the shell says so once on every
// page, and the Batch page's 401 names the missing setting instead of the
// API's raw header text.
test("the shell says that no browser sign-in is configured", async ({ page }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  const notice = page.locator("#auth-bearer-only");
  await expect(notice).toHaveCount(1);
  await expect(notice).toContainText("HFS_UI_LOGIN_CLIENT_ID");
});

test("executing a bundle explains the missing sign-in, not the raw 401", async ({ page }) => {
  const file = join(tmpdir(), `hfs-e2e-bearer-only-${Date.now()}.json`);
  writeFileSync(
    file,
    JSON.stringify({
      resourceType: "Bundle",
      type: "batch",
      entry: [{ request: { method: "GET", url: "Patient?_count=1" } }],
    }),
  );
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(file);
  await page.locator("#batch-execute-top").click();
  const error = page.locator("#batch-execute-error");
  await expect(error).toBeVisible();
  await expect(error).toContainText("HFS_UI_LOGIN_CLIENT_ID");
  await expect(error).not.toContainText("Missing Authorization header");
});
