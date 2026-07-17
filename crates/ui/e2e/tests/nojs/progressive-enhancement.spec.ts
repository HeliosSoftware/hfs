import { test, expect } from "../../pages/fixtures";

// This whole file runs in the `nojs` project (javaScriptEnabled: false), which
// exercises the README's core promise: the UI works with JavaScript off. htmx,
// theme.js, and every data-* handler are inert here — only real <a href>/<form>
// fallbacks can carry the behavior.

test("the landing page renders server-side with no JavaScript", async ({ page, chrome }) => {
  await page.goto("/ui");
  await expect(page.locator("body")).toContainText("Helios FHIR Server");
  // The sidebar (full layout) is present — not a bare fragment.
  await expect(chrome.sidebar).toBeVisible();
});

test("the language switcher works as plain links (en → es → de)", async ({ page, chrome }) => {
  await page.goto("/ui");
  await expect(chrome.langLink("es")).toHaveAttribute("href", /lang=es/);
  await chrome.langLink("es").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "es");

  await chrome.langLink("de").click();
  await expect(page.locator("html")).toHaveAttribute("lang", "de");
});

// Every primary nav entry is a real link that hard-navigates without JS.
const NAV = [
  { href: "/ui/resources", url: /\/ui\/resources/ },
  { href: "/ui/compartments", url: /\/ui\/compartments/ },
  { href: "/ui/tenants", url: /\/ui\/tenants/ },
  { href: "/ui/search-parameters", url: /\/ui\/search-parameters/ },
];
for (const { href, url } of NAV) {
  test(`nav link ${href} navigates`, async ({ page, chrome }) => {
    await page.goto("/ui");
    const link = chrome.navLink(href);
    await expect(link).toBeVisible();
    await link.click();
    await expect(page).toHaveURL(url);
    await expect(page.locator("h1.page-head__title, h1.page-title")).toBeVisible();
  });
}

test("the 'coming soon' nav entries are inert, not links", async ({ page }) => {
  await page.goto("/ui");
  const soon = page.locator("span.nav-item--soon");
  await expect(soon.first()).toBeVisible();
  // None of them is an anchor — they cannot navigate.
  expect(await soon.evaluateAll((els) => els.every((e) => e.tagName !== "A"))).toBe(true);
});

test("a hard navigation returns the full page, not an htmx fragment", async ({ page, chrome }) => {
  // /ui/status is fragment-or-full depending on HX-Request; a plain load (no JS,
  // no htmx header) must get the whole document.
  await page.goto("/ui/status");
  await expect(chrome.sidebar).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", /.+/);
});
