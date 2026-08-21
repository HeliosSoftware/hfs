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

test("the sidebar brand is an accessible native Home link", async ({ page }) => {
  await page.goto("/ui/resources");
  const brand = page.locator("a.brand");
  const name = /^Helios FHIR Server hfs v.+ — Home$/;

  await expect(brand).toHaveAttribute("href", "/ui");
  await expect(brand).not.toHaveAttribute("aria-current", "page");
  await expect(brand).toHaveAccessibleName(name);
  await expect(brand.locator("img")).toHaveAttribute("alt", "");

  await brand.focus();
  await expect(brand).toHaveAccessibleName(name);
  await brand.press("Enter");
  await expect(page).toHaveURL(/\/ui$/);
  await expect(page.locator("h1.page-head__title")).toHaveText("Home");
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
    // Enter the rail and let it finish expanding (#438) so the link's position
    // is stable before the click — pure CSS, so it works with JS disabled too.
    await chrome.sidebar.hover();
    await expect
      .poll(async () => (await chrome.sidebar.boundingBox())?.width)
      .toBeGreaterThan(290);
    const link = chrome.navLink(href);
    await expect(link).toBeVisible();
    await link.click();
    await expect(page).toHaveURL(url);
    await expect(page.locator("h1.page-head__title, h1.page-head__title")).toBeVisible();
  });
}

test("the 'coming soon' nav entries are inert, not links", async ({ page }) => {
  await page.goto("/ui");
  const soon = page.locator("span.nav-item--soon");
  await expect(soon.first()).toBeVisible();
  // None of them is an anchor — they cannot navigate.
  expect(await soon.evaluateAll((els) => els.every((e) => e.tagName !== "A"))).toBe(true);
});

test("the Resources type rail navigates via plain links with no JavaScript", async ({
  page,
}) => {
  await page.goto("/ui/resources");
  const item = page.locator("#type-rail-list a.filter-rail__item[data-type='Observation']");
  await expect(item).toBeVisible();
  await item.click();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);
  await expect(item).toHaveAttribute("aria-current", "true");
});

test("a hard navigation returns the full page, not an htmx fragment", async ({ page, chrome }) => {
  // /ui/status is fragment-or-full depending on HX-Request; a plain load (no JS,
  // no htmx header) must get the whole document.
  await page.goto("/ui/status");
  await expect(chrome.sidebar).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", /.+/);
});
