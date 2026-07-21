import { test, expect } from "../pages/fixtures";

// Tenant maintenance (/ui/tenants): the htmx add-tenant slide-over, the live
// search filter, and per-row delete (hx-confirm). Skips itself if this backend
// hasn't wired a tenant store.

test.describe("tenants", () => {
  test.beforeEach(async ({ tenants }) => {
    // Creating a tenant seeds its conformance resources (~1.4k inserts) inside
    // the request: round-trip bound on the remote-backend matrix (minutes on
    // real S3), fsync bound on filesystem SQLite (~90s measured on NTFS).
    test.setTimeout(300_000);
    await tenants.goto();
    if (await tenants.unavailableNotice.isVisible().catch(() => false)) {
      test.skip(true, "no tenant store on this backend");
    }
  });

  test("adding a tenant slides it into the table", async ({ tenants }) => {
    const id = `e2e-add-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "E2E Added");
    await expect(tenants.row(id)).toBeVisible();
  });

  test("the search box filters the table (htmx)", async ({ page, tenants }) => {
    const id = `e2e-find-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Findable");
    await expect(tenants.row(id)).toBeVisible();

    await tenants.search.fill(id);
    await expect(tenants.row(id)).toBeVisible();
    await tenants.search.fill("zzz-no-such-tenant");
    await expect(tenants.row(id)).toBeHidden();
  });

  test("deleting a tenant deregisters it", async ({ page, tenants }) => {
    const id = `e2e-del-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Deletable");
    const row = tenants.row(id);
    await expect(row).toBeVisible();

    page.once("dialog", (d) => d.accept()); // hx-confirm
    await row.locator("[hx-delete]").click();
    // The trash button deregisters without purging. The invariant is "no
    // longer registered" — how that renders depends on the backend: stores
    // that discover tenants from their data (count_by_tenant) keep the row,
    // flagged unregistered; stores that cannot (S3) drop it from the list.
    await expect
      .poll(
        async () => {
          if ((await row.count()) === 0) return "gone";
          const flagged = await row
            .locator(".tag--muted")
            .isVisible()
            .catch(() => false);
          return flagged ? "unregistered" : "pending";
        },
        { timeout: 15_000 },
      )
      .not.toBe("pending");
  });
});
