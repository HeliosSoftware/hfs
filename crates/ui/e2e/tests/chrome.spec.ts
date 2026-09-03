import { test, expect } from "../pages/fixtures";
import {
  assertLanguageRoundTrip,
  assertMenuShape,
  type MenuContext,
} from "../pages/user-menu";

// The persistent chrome: the sidebar rail (#438). There is no toggle and no
// persisted state anymore — the sidebar rests as an icon rail and expands as
// an overlay while hovered or keyboard-focused.

test("the sidebar rests as a rail and expands on hover", async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });

  // Resting: rail width, labels visually hidden (still in the a11y tree).
  await page.mouse.move(800, 400);
  await expect.poll(async () => (await chrome.sidebar.boundingBox())?.width).toBeLessThan(100);

  // Hover: expands past the rail and the labels become visible.
  await chrome.sidebar.hover();
  await expect.poll(async () => (await chrome.sidebar.boundingBox())?.width).toBeGreaterThan(250);
  await expect(chrome.navLink("/ui/resources").locator(".nav-item__label")).toBeVisible();

  // Content does not reflow: the main column starts at the rail edge and
  // stays there while the sidebar overlays it.
  const main = await page.locator(".pane").boundingBox();
  expect(main && main.x).toBeGreaterThan(60);
  expect(main && main.x).toBeLessThan(120);

  // Leave: collapses back.
  await page.mouse.move(800, 400);
  await expect.poll(async () => (await chrome.sidebar.boundingBox())?.width).toBeLessThan(100);
});

test("keyboard focus inside the sidebar also expands it", async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  await page.mouse.move(800, 400);
  await chrome.navLink("/ui/resources").focus();
  await expect.poll(async () => (await chrome.sidebar.boundingBox())?.width).toBeGreaterThan(250);
});

test("there is no expand/collapse toggle", async ({ page }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  await expect(page.locator("[data-toggle-nav]")).toHaveCount(0);
});

test("the Batch & Data section lists Import and Export", async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  await chrome.sidebar.hover();
  await expect(chrome.navLink("/ui/bulk-import")).toBeVisible();
  await expect(chrome.navLink("/ui/bulk-export")).toBeVisible();
});

test("SQL on FHIR is its own section with four navigable children", async ({
  page,
  chrome,
}) => {
  // #649: the former coming-soon placeholder inside Batch & Data became a
  // top-level section whose every child is a real route. The job-id lookup
  // form that used to round the section out to five is retired (#835): its
  // nav entry is gone and `/ui/sql/files` now just redirects to the list.
  await page.goto("/ui", { waitUntil: "networkidle" });
  await chrome.sidebar.hover();
  for (const href of [
    "/ui/sql/view-definitions",
    "/ui/sql/queries",
    "/ui/sql/views",
    "/ui/sql/export",
  ]) {
    await expect(chrome.navLink(href)).toBeVisible();
  }
  await expect(page.locator(".nav-item--soon")).toHaveCount(0);
  await expect(chrome.navLink("/ui/sql/files")).toHaveCount(0);
});

// The account menu (#725). Until #799 it had NO coverage in this project at
// all — only the `nojs` ring touched it — so a JS-enabled regression (a stray
// script closing the disclosure, a panel that never paints) went unnoticed.
// The assertions live in `pages/user-menu.ts` and are shared byte-for-byte
// with the HTS suite, because both products render one shared template.
const HFS_MENU: MenuContext = {
  langCookie: "hfs_lang",
  home: "/ui",
  favicon: "/ui/assets/logo.png",
};

test("the topbar account menu carries the identity card and the language segment", async ({
  page,
}) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  await assertMenuShape(page, expect, HFS_MENU);
});

test("the account menu switches language and persists it in the hfs_lang cookie", async ({
  page,
}) => {
  await assertLanguageRoundTrip(page, expect, HFS_MENU);
});
