import { test, expect } from "../pages/fixtures";

// The persistent chrome: the collapsible nav. Like the theme, it caches in
// localStorage and roams via a /_user/settings merge-patch — behavior only the
// browser can observe.

test("the nav toggle collapses and expands, syncing aria and the cache", async ({
  page,
  chrome,
}) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  // Default is expanded.
  await expect(page.locator("html")).toHaveAttribute("data-nav", "expanded");
  await expect(chrome.navToggle).toHaveAttribute("aria-expanded", "true");

  const patch = page.waitForRequest(
    (r) => r.url().endsWith("/_user/settings") && r.method() === "PATCH",
  );
  await chrome.navToggle.click();

  await expect(page.locator("html")).toHaveAttribute("data-nav", "collapsed");
  await expect(chrome.navToggle).toHaveAttribute("aria-expanded", "false");
  const req = await patch;
  expect(JSON.parse(req.postData() ?? "{}")).toEqual({ nav: "collapsed" });
  expect(await page.evaluate(() => localStorage.getItem("hfs-nav"))).toBe("collapsed");

  // Toggling back expands again.
  await chrome.navToggle.click();
  await expect(page.locator("html")).toHaveAttribute("data-nav", "expanded");
});

test("a returning user's collapsed choice is applied before paint", async ({ page }) => {
  // Isolate the cache path: the settings doc is shared across tests (no auth →
  // one anonymous user), so pin the server response to empty and let the
  // localStorage cache be the sole source, the way a first-visit-elsewhere
  // returning user has it.
  await page.route("**/_user/settings", (route) =>
    route.request().method() === "GET"
      ? route.fulfill({ contentType: "application/json", body: "{}" })
      : route.fulfill({ status: 200, body: "{}" }),
  );
  await page.addInitScript(() => {
    try {
      localStorage.setItem("hfs-nav", "collapsed");
    } catch {}
  });
  await page.goto("/ui", { waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute("data-nav", "collapsed");
});
