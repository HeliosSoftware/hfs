import { test, expect } from "../pages/fixtures";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometries,
} from "../pages/button-geometry";

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

  test("provisioning shows a spinner row until the tenant is ready", async ({ tenants }) => {
    const id = `e2e-add-${Date.now().toString(36)}`;
    await tenants.addToggle.click();
    await tenants.addForm.locator("input[name=display_name]").fill("E2E Added");
    await tenants.addForm.locator("input[name=id]").fill(id);
    await tenants.addForm.locator("button[type=submit]").click();
    // The panel closes as soon as the server accepts the job…
    await expect(tenants.addForm).toBeHidden();
    // …and the table reports the in-flight job, surviving a reload.
    const row = tenants.row(id);
    await expect(row.locator(".spinner")).toBeVisible();
    await tenants.page.reload();
    await expect(tenants.row(id).locator(".spinner")).toBeVisible();
    // Eventually the job settles into a normal row.
    await expect(tenants.row(id).locator("[hx-delete]")).toBeVisible({ timeout: 240_000 });
    await expect(tenants.row(id).locator(".spinner")).toHaveCount(0);
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

  test("a successful create clears the form and collapses the panel", async ({ tenants }) => {
    const id = `e2e-reset-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Resettable");
    await expect(tenants.row(id)).toBeVisible();
    // addTenant() collapses the panel if it is still open; after this change
    // the server-side success trigger already did that.
    await tenants.addToggle.click();
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue("");
    await expect(tenants.addForm.locator("input[name=display_name]")).toHaveValue("");
  });

  test("a failed create keeps the typed values next to the error banner", async ({ page, tenants }) => {
    const id = `e2e-dup-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "First");
    await expect(tenants.row(id)).toBeVisible();
    await tenants.addToggle.click();
    await tenants.addForm.locator("input[name=id]").fill(id);
    await tenants.addForm.locator("input[name=display_name]").fill("Second");
    await tenants.addForm.locator("button[type=submit]").click();
    await expect(page.locator("#tenant-rows .alert")).toContainText("already exists");
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue(id);
    await expect(tenants.addForm.locator("input[name=display_name]")).toHaveValue("Second");
  });

  test("typing a display name mirrors a slug into the tenant id", async ({ tenants }) => {
    await tenants.addToggle.click();
    const actionMetrics = await readButtonGeometries(
      tenants.addForm.locator(".addbox__actions .btn"),
    );
    expect(actionMetrics).toHaveLength(2);
    for (const geometry of actionMetrics) expect(geometry).toEqual(CANONICAL_BUTTON_GEOMETRY);
    await tenants.addForm.locator("input[name=display_name]").fill("Acme Health");
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue("acme-health");
    await tenants.addForm.locator("input[name=display_name]").fill("  Ünïcode & Co.  ");
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue("unicode-co");
  });

  test("editing the tenant id by hand stops the mirror", async ({ tenants }) => {
    await tenants.addToggle.click();
    await tenants.addForm.locator("input[name=display_name]").fill("Acme Health");
    await tenants.addForm.locator("input[name=id]").fill("acme");
    await tenants.addForm.locator("input[name=display_name]").fill("Acme Health Europe");
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue("acme");
  });

  test("clearing the tenant id re-arms the mirror", async ({ tenants }) => {
    await tenants.addToggle.click();
    await tenants.addForm.locator("input[name=display_name]").fill("Acme Health");
    await tenants.addForm.locator("input[name=id]").fill("acme");
    await tenants.addForm.locator("input[name=id]").fill("");
    await tenants.addForm.locator("input[name=display_name]").fill("Acme Health Europe");
    await expect(tenants.addForm.locator("input[name=id]")).toHaveValue("acme-health-europe");
  });

  test("deleting a tenant deregisters it", async ({ page, tenants }) => {
    const id = `e2e-del-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Deletable");
    const row = tenants.row(id);
    await expect(row).toBeVisible();

    page.once("dialog", (d) => d.accept()); // hx-confirm
    await row.locator("[hx-delete]").click();
    // The trash button deregisters without purging, so the tenant's data
    // still exists and every backend must keep the row visible, flagged
    // unregistered, with its purge affordance intact (#252; S3 gained
    // count_by_tenant in #330 — data-discovery is universal now).
    //
    // Poll through reloads rather than watching the one swapped fragment:
    // under a parallel suite the fragment can arrive late or carry the
    // error banner (a pool wait while another test seeds a tenant), and a
    // fresh GET is the retry the page itself would need.
    await expect
      .poll(
        async () => {
          await page.reload();
          return row
            .locator(".tag--muted")
            .isVisible()
            .catch(() => false);
        },
        { timeout: 60_000, intervals: [2_000] },
      )
      .toBe(true);
  });
});
