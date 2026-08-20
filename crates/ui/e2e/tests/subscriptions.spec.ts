import { test, expect } from "../pages/fixtures";

// The Subscriptions operator page (#580), read-only. The e2e server enables
// the engine with nothing registered, so this drives the advertised-but-empty
// state: the four cards at zero, the empty table, and the link-based sort.
// The populated table (rows, chips, streaks, ordering) is covered in Rust
// with an injected provider (crates/ui/tests/subscriptions_http.rs).
//
// CI builds hfs with the `subscriptions` feature; a local binary built with
// plain defaults compiles the engine out, so these skip on the unavailable
// state rather than failing the local run.
test.beforeEach(async ({ page }) => {
  await page.goto("/ui/subscriptions", { waitUntil: "networkidle" });
  const unavailable = await page.locator(".card.notice").count();
  test.skip(unavailable > 0, "hfs built without the subscriptions feature");
});

test("the page renders its cards and empty table when the engine is on", async ({ page }) => {

  await expect(page.locator("h1.page-head__title")).toHaveText(/subscriptions/i);
  await expect(page.locator(".card.stat")).toHaveCount(4);
  await expect(page.locator(".card.stat .stat__value").first()).toHaveText("0");
  await expect(page.locator(".data-table__empty")).toBeVisible();
});

test("the sidebar advertises the page while the engine is enabled", async ({ page, chrome }) => {
  await chrome.sidebar.hover();
  const link = chrome.navLink("/ui/subscriptions");
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute("aria-current", "page");
});

test("the sort control works as plain links", async ({ page }) => {
  await page.locator(".subs-sort summary").click();
  await page.locator('.subs-sort a[href*="sort=sent"]').click();
  await expect(page).toHaveURL(/sort=sent/);
  await expect(page.locator(".subs-sort summary")).toContainText(/most sent/i);
});
