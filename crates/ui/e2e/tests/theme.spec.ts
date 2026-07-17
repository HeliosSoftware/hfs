import { test, expect } from "../pages/fixtures";

// Behavior that only a real engine can observe: theme.js runs in <head> without
// defer, caches in localStorage, and roams the choice via PATCH /_user/settings
// (RFC 7386). None of this is reachable by the tower::oneshot tests.

test("a returning dark-mode user sees no flash of light", async ({ page, chrome }) => {
  await chrome.seedTheme("dark");
  // domcontentloaded, not networkidle: the attribute must already be right
  // before the settings fetch resolves — that is the FOUC guard.
  await page.goto("/ui", { waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("with no stored choice, the OS preference decides", async ({ browser }) => {
  const ctx = await browser.newContext({ colorScheme: "dark" });
  const page = await ctx.newPage();
  await page.goto("/ui", { waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await ctx.close();
});

test("toggling roams the choice via an RFC 7386 merge-patch", async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });

  const patch = page.waitForRequest(
    (r) => r.url().endsWith("/_user/settings") && r.method() === "PATCH",
  );
  await chrome.themeButton("dark").click();

  const req = await patch;
  expect(JSON.parse(req.postData() ?? "{}")).toEqual({ theme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(await page.evaluate(() => localStorage.getItem("hfs-theme"))).toBe("dark");
});

test("a roamed server choice wins over the local cache", async ({ page, chrome }) => {
  // Another device already saved "dark"; this browser cached "light".
  await page.route("**/_user/settings", (route) =>
    route.request().method() === "GET"
      ? route.fulfill({ contentType: "application/json", body: JSON.stringify({ theme: "dark" }) })
      : route.fulfill({ status: 200, body: "{}" }),
  );
  await chrome.seedTheme("light");
  await page.goto("/ui", { waitUntil: "networkidle" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("an unavailable settings store degrades to the cached choice", async ({ page, chrome }) => {
  await page.route("**/_user/settings", (route) => route.fulfill({ status: 503, body: "" }));
  await chrome.seedTheme("dark");
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto("/ui", { waitUntil: "networkidle" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(errors, errors.join("\n")).toEqual([]);
});
