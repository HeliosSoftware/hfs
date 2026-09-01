import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";
import type { DashboardPage } from "../pages/dashboard";

// #553: the dashboard snapshot is tenant-scoped (build_index_page ->
// dashboard::snapshot(window, &tenant.id, ...) — crates/ui/src/lib.rs), but
// until this spec nothing proved it end to end. It drives the sidebar's own
// tenant switch and asserts the counts, picker, and chart follow the selected
// tenant — and that one tenant's resources never surface in another's
// dashboard.
//
// Cost note: provisioning a tenant seeds the conformance packs in the
// background (~1.4k resources, ~90s on SQLite/NTFS — see tenants.spec.ts), so
// this file creates ONE extra tenant and pays that cost once. The marker
// types are chosen to be unused by every other spec in the suite (Flag in the
// default tenant, Basic in the extra one), so membership assertions cannot
// collide with state other files leave behind on the shared server.

// Backends whose primary store has no count read path (S3) cannot feed the
// dashboard's counts or chart; the matrix sets this flag for them and this
// spec stands down entirely — every assertion here reads counts.
const noChartData = process.env.HFS_E2E_NO_CHART_DATA === "1";

// Unique per run: a re-run against a long-lived dev server then provisions a
// fresh tenant instead of colliding with the last run's — the exact Basic
// count below stays valid, and addTenant never hits the duplicate-id error.
// (The suite's own webServer boots a throwaway DB, where this is moot.)
const EXTRA_TENANT = `dash-553-${Date.now().toString(36)}`;

// The tenant choice persists server-side in the user-global settings document
// (POST /ui/tenant), so a spec that fails mid-switch would leak the extra
// tenant into every later spec. Always leave the suite on the default tenant.
test.afterEach(async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  try {
    if ((await chrome.tenantSelector.count()) > 0 && (await chrome.currentTenant()) !== "default") {
      await chrome.selectTenant("default");
    }
  } catch {
    // No picker (single-tenant phase) — nothing to restore.
  }
});

/** Reloads until the picker's option list shows the type — outlasts the 15s
 * dashboard snapshot cache after seeding, like DashboardPage.waitForSeries. */
async function untilPickerShows(dashboard: DashboardPage, type: string): Promise<void> {
  for (let attempt = 0; attempt < 12; attempt++) {
    await dashboard.openPicker();
    if ((await dashboard.pickerOption(type).count()) > 0) return;
    await dashboard.page.waitForTimeout(2000);
    await dashboard.page.reload({ waitUntil: "networkidle" });
  }
  throw new Error(`the type picker never offered ${type}`);
}

test("the dashboard's counts and chart follow the selected tenant, with no leakage either way", async ({
  request,
  dashboard,
  chrome,
  tenants,
}) => {
  test.skip(noChartData, "no count read path on this backend");
  // Tenant provisioning dominates (see the cost note); its settle-wait alone
  // may take up to 300s on a slow disk, and the afterEach tenant restore
  // spends from the same budget.
  test.setTimeout(600_000);

  // A marker only the default tenant has.
  const flagId = await createResource(request, "Flag", {
    status: "active",
    code: { text: "dashboard tenant isolation (#553)" },
    subject: { display: "e2e-553" },
  });
  await waitSearchable(request, "Flag", flagId);

  // Provision the extra tenant through the maintenance page, then give it
  // three markers of its own — a count nothing else in the suite can move.
  await tenants.goto();
  await tenants.addTenant(EXTRA_TENANT);
  for (let i = 0; i < 3; i++) {
    const id = await createResource(
      request,
      "Basic",
      { code: { text: `553 marker ${i}` } },
      EXTRA_TENANT,
    );
    await waitSearchable(request, "Basic", id, EXTRA_TENANT);
  }

  // The default tenant's dashboard: its own marker, none of the other's.
  await dashboard.goto();
  await untilPickerShows(dashboard, "Flag");
  await expect(dashboard.pickerOption("Basic")).toHaveCount(0);

  // Switch tenants through the sidebar — the same round trip a user makes.
  await chrome.selectTenant(EXTRA_TENANT);
  expect(await chrome.currentTenant()).toBe(EXTRA_TENANT);

  // The extra tenant's dashboard: its marker with its exact count, and no
  // trace of the default tenant's marker.
  await dashboard.goto();
  await untilPickerShows(dashboard, "Basic");
  await expect(dashboard.pickerOption("Basic").locator(".chart-pick__count")).toHaveText("3");
  await expect(dashboard.pickerOption("Flag")).toHaveCount(0);

  // The chart follows too: charting the marker plots a real series whose
  // legend total is the tenant's own count.
  await dashboard.goto("?types=Basic");
  await dashboard.waitForSeries();
  const legendEntry = dashboard.legendItems.filter({ hasText: "Basic" });
  await expect(legendEntry).toHaveCount(1);
  await expect(legendEntry.locator(".chart-legend__total")).toHaveText("3");

  // And back: the default tenant's dashboard is exactly as we left it —
  // switching forth and back leaks nothing in either direction.
  await chrome.selectTenant("default");
  await dashboard.goto();
  await untilPickerShows(dashboard, "Flag");
  await expect(dashboard.pickerOption("Basic")).toHaveCount(0);
});
