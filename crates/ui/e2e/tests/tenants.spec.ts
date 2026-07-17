import { test, expect } from "../pages/fixtures";

// Tenant maintenance (/ui/tenants): the htmx add-tenant slide-over, the live
// search filter, and per-row delete (hx-confirm). Skips itself if this backend
// hasn't wired a tenant store.

test.describe("tenants", () => {
  test.beforeEach(async ({ tenants }) => {
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

  test("deleting a tenant removes its row", async ({ page, tenants }) => {
    const id = `e2e-del-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Deletable");
    const row = tenants.row(id);
    await expect(row).toBeVisible();

    page.once("dialog", (d) => d.accept()); // hx-confirm
    await row.locator("[hx-delete]").click();
    await expect(row).toBeHidden();
  });
});
