import { test, expect } from "../pages/fixtures";

// Tenant maintenance (/ui/tenants): the htmx add-tenant slide-over, the live
// search filter, and per-row delete (hx-confirm). Skips itself if this backend
// hasn't wired a tenant store.

test.describe("tenants", () => {
  test.beforeEach(async ({ tenants }) => {
    // Creating a tenant seeds its conformance resources (~1.4k inserts) inside
    // the request: round-trip bound on the remote-backend matrix, fsync bound
    // on filesystem SQLite (~90s measured on NTFS). Budget accordingly.
    test.setTimeout(180_000);
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

  test("deleting a tenant deregisters it, leaving a data-discovered row", async ({
    page,
    tenants,
  }) => {
    const id = `e2e-del-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Deletable");
    const row = tenants.row(id);
    await expect(row).toBeVisible();

    page.once("dialog", (d) => d.accept()); // hx-confirm
    await row.locator("[hx-delete]").click();
    // Provisioning seeded the tenant's conformance resources, and the trash
    // button deliberately never purges data: the deregistered tenant remains
    // visible as a data-discovered row, flagged unregistered.
    await expect(row.locator(".tag--muted")).toBeVisible();
  });
});
