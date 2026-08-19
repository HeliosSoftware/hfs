import { test, expect } from "../pages/fixtures";

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
  // Import (#527) and Export (#537) are live workspaces; SQL-on-FHIR is
  // still a placeholder.
  await expect(chrome.navLink("/ui/bulk-import")).toBeVisible();
  await expect(chrome.navLink("/ui/bulk-export")).toBeVisible();
  await expect(chrome.soonItem("SQL-on-FHIR")).toBeVisible();
});
