import { test, expect, acceptConfirm } from "../pages/fixtures";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometries,
} from "../pages/button-geometry";

// Tenant maintenance (/ui/tenants): the htmx add-tenant slide-over, the live
// search filter, and per-row delete (data-confirm, answered through the shared
// in-page confirmation dialog, #1667). Skips itself if this backend hasn't
// wired a tenant store.

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
    const resourcesHeader = tenants.page.locator(".data-table th.col-num");
    const resourcesCell = row.locator("td.col-num");
    await expect(resourcesHeader).toHaveCSS("text-align", "left");
    await expect(row.locator(".spinner")).toBeVisible();
    await expect(resourcesCell).toHaveCSS("text-align", "left");
    await expect(resourcesCell).toHaveCSS("font-variant-numeric", "tabular-nums");
    await tenants.page.reload();
    await expect(tenants.row(id).locator(".spinner")).toBeVisible();
    // Eventually the job settles into a normal row.
    await expect(tenants.deleteButton(id)).toBeVisible({ timeout: 240_000 });
    await expect(tenants.row(id).locator(".spinner")).toHaveCount(0);
    await expect(resourcesCell).toHaveCSS("text-align", "left");
    await expect(resourcesCell).toHaveCSS("font-variant-numeric", "tabular-nums");
    // Loading counts is its own state (#1851): a status line, never the
    // provisioning spinner. Once settled the cell carries a count state.
    await tenants.waitCountsSettled();
    await expect(tenants.countsState).toBeVisible();
    await expect(tenants.page.locator("#tenant-counts-status")).toHaveAttribute("role", "status");
    await expect(tenants.page.locator("#tenant-counts-status .busy-status")).toHaveCount(0);
    await expect(resourcesCell).toHaveAttribute("data-count-state", /.+/);
  });

  test("the search box filters the table (htmx)", async ({ page, tenants }) => {
    const id = `e2e-find-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Findable");
    await expect(tenants.row(id)).toBeVisible();

    await tenants.search.fill(id);
    await expect(tenants.row(id)).toBeVisible();
    await expect(page.locator("#tenant-rows tbody tr")).toHaveCount(1);

    // The search survives what the table does on its own and what the user
    // does to it (#1851): a rejected create reloads the rows with the same
    // term, and the cards stay global.
    await tenants.addToggle.click();
    await tenants.addForm.locator("input[name=id]").fill(id);
    await tenants.addForm.locator("button[type=submit]").click();
    await expect(tenants.addForm.locator("#tenant-add-error")).toContainText("already exists");
    await tenants.addForm.locator("[data-addbox-close]").first().click();
    await expect(tenants.search).toHaveValue(id);
    await expect(tenants.row(id)).toBeVisible();
    await expect(page.locator("#tenant-rows tbody tr")).toHaveCount(1);
    await tenants.waitCountsSettled();
    await expect(page.locator("#tenant-rows tbody tr")).toHaveCount(1);

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
    // The dialog's own submit error (#681 adenda): rendered inside the panel
    // via an out-of-band swap, not the page-level #tenant-rows banner, which
    // sits behind the modal scrim while the dialog is open.
    await expect(page.locator("#tenant-add-error")).toContainText("already exists");
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

    await tenants.deleteButton(id).click();
    await acceptConfirm(page); // asked in-page through confirm.js
    // The trash button deregisters without purging, so the tenant's data
    // still exists and every backend must keep the row visible, flagged
    // unregistered, with its purge affordance intact (#252). Data-only
    // tenants come from the background inventory (#1851): the delete
    // response shows the row from the last counts, and the counts poller
    // brings it in if they had not seen it yet — no reload needed.
    await expect(row.locator(".tag--muted")).toBeVisible({ timeout: 60_000 });
    await tenants.waitCountsSettled();
    await expect(row.locator(".tag--muted")).toBeVisible();
  });

  // Every request that swaps #tenant-rows queues on the table card (#1851).
  // htmx issues a queued request from the element that asked for it and
  // skips it if that element has left the page, so a trash button inside the
  // rows the in-flight response replaces used to lose its DELETE. Hold a
  // search in flight, confirm a delete behind it, then let the search land.
  test("a delete confirmed while the rows are loading is still sent", async ({ page, tenants }) => {
    const id = `e2e-qdel-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Queued Delete");
    await expect(tenants.row(id)).toBeVisible();

    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    let holding = true;
    await page.route(/\/ui\/tenants\/rows/, async (route) => {
      if (holding) {
        holding = false;
        await held;
      }
      await route.continue();
    });
    const search = page.waitForRequest((r) => r.url().includes("/ui/tenants/rows"));
    await tenants.search.fill(id);
    await search;

    await tenants.deleteButton(id).click();
    await acceptConfirm(page);
    await expect.poll(() => tenants.queuedRequests()).toBe(1);

    const sent = page.waitForRequest(
      (r) => r.method() === "DELETE" && new URL(r.url()).pathname === `/ui/tenants/${id}`,
      { timeout: 30_000 },
    );
    release();
    const deleted = await sent;
    expect(new URL(deleted.url()).searchParams.get("q")).toBe(id);
    // Deregistered, its seeded data kept: the row stays, flagged unregistered.
    await expect(tenants.row(id).locator(".tag--muted")).toBeVisible({ timeout: 60_000 });
    await expect(tenants.deleteButton(id)).toHaveCount(1);
    await page.unroute(/\/ui\/tenants\/rows/);
  });

  // The newest request wins (#1851): a search typed while an older one is
  // still in flight queues behind it, so the older answer can never land
  // last. And the count status line, unchanged, keeps its nodes, so a
  // screen reader is not told the same status again after every keystroke.
  test("a newer search lands after an older one still in flight", async ({ page, tenants }) => {
    const id = `e2e-order-${Date.now().toString(36)}`;
    await tenants.addTenant(id, "Ordered");
    await expect(tenants.row(id)).toBeVisible();
    await page.evaluate(() => {
      const status = document.querySelector("#tenant-counts-status [data-counts-state]");
      (status as HTMLElement & { e2eKept?: boolean }).e2eKept = true;
    });

    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    let holding = true;
    await page.route(/\/ui\/tenants\/rows/, async (route) => {
      if (holding) {
        holding = false;
        await held;
      }
      await route.continue();
    });
    const older = page.waitForRequest((r) => r.url().includes("q=zzz-no-such-tenant"));
    await tenants.search.fill("zzz-no-such-tenant");
    await older;
    await tenants.search.fill(id);
    await expect.poll(() => tenants.queuedRequests()).toBe(1);

    const newer = page.waitForResponse((r) => r.url().includes(`q=${id}`));
    release();
    await newer;
    await expect(tenants.row(id)).toBeVisible();
    await expect(page.locator("#tenant-rows tbody tr")).toHaveCount(1);
    expect(
      await page.evaluate(() => {
        const status = document.querySelector("#tenant-counts-status [data-counts-state]");
        return (status as (HTMLElement & { e2eKept?: boolean }) | null)?.e2eKept === true;
      }),
    ).toBe(true);
    await page.unroute(/\/ui\/tenants\/rows/);
  });
});
