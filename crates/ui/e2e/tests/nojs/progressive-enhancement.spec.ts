import { test, expect } from "@playwright/test";

// This whole file runs in the `nojs` project (javaScriptEnabled: false), which
// exercises the README's core promise: the UI works with JavaScript off. htmx,
// theme.js, and every data-* handler are inert here — only real <a href>/<form>
// fallbacks can carry the behavior.

test("the landing page renders server-side with no JavaScript", async ({ page }) => {
  await page.goto("/ui");
  await expect(page.locator("body")).toContainText("Helios FHIR Server");
  // The sidebar (full layout) is present — not a bare fragment.
  await expect(page.locator("aside.sidebar")).toBeVisible();
});

test("the language switcher works as plain links", async ({ page }) => {
  await page.goto("/ui");
  // The switcher is real anchors with ?lang= — no JS needed.
  const es = page.locator(".lang-switcher a[href*='lang=es']");
  await expect(es).toHaveAttribute("href", /lang=es/);
  await es.click();
  await expect(page.locator("html")).toHaveAttribute("lang", "es");
});

test("primary nav entries are real links that navigate", async ({ page }) => {
  await page.goto("/ui");
  const resources = page.locator("a.nav-item[href='/ui/resources']");
  await expect(resources).toBeVisible();
  await resources.click();
  await expect(page).toHaveURL(/\/ui\/resources/);
  await expect(page.locator("h1.page-head__title")).toBeVisible();
});

test("a hard navigation returns the full page, not an htmx fragment", async ({ page }) => {
  // /ui/status is fragment-or-full depending on HX-Request; a plain load (no JS,
  // no htmx header) must get the whole document.
  await page.goto("/ui/status");
  await expect(page.locator("aside.sidebar")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", /.+/);
});
