// Page-object fixtures: one place that wires every page object onto Playwright's
// `test`, so specs read `test("…", async ({ resources, history }) => …)` instead
// of newing objects up. Import { test, expect } from here, not @playwright/test.
import { test as base, expect } from "@playwright/test";
import { AppChrome } from "./chrome";
import { DashboardPage } from "./dashboard";
import { ResourcesPage } from "./resources";
import { HistoryPage } from "./history";
import { CompartmentsPage } from "./compartments";
import { QueriesPage } from "./queries";
import { SearchPage } from "./search";
import { SearchParametersPage } from "./search-parameters";
import { TenantsPage } from "./tenants";
import { BulkImportPage } from "./bulk-import";
import { CapabilityStatementPage } from "./capability-statement";

type Fixtures = {
  chrome: AppChrome;
  dashboard: DashboardPage;
  resources: ResourcesPage;
  history: HistoryPage;
  compartments: CompartmentsPage;
  queries: QueriesPage;
  search: SearchPage;
  searchParameters: SearchParametersPage;
  tenants: TenantsPage;
  bulkImport: BulkImportPage;
  capabilityStatement: CapabilityStatementPage;
};

export const test = base.extend<Fixtures>({
  // The sidebar expands on hover (#438) and the mouse starts at (0,0) — over
  // the rail — so a fresh page would open with the sidebar overlaying the
  // left content edge and intercepting clicks. Park the pointer in the topbar
  // after every navigation; tests that exercise the hover do so explicitly.
  //
  // Rail state (the 754/755 epic) is server-side and per-user: the suite runs
  // every test as the same default `l2:` user (no auth), so a "last selected"
  // or "recently used" recorded by one test would otherwise leak into the
  // next one's rail. Reset the `rails` record before each test with a merge
  // patch that deletes it (`null`), same shape and endpoint `saved-queries.js`
  // and the theme toggle already use. A `501` means no settings store is
  // configured for this run (e.g. an Elasticsearch-only leg) — there is
  // nothing to reset, so it is not a failure.
  page: async ({ page }, use) => {
    const reset = await page.request.patch("/_user/settings", {
      headers: { "Content-Type": "application/json" },
      data: { rails: null },
    });
    if (!reset.ok() && reset.status() !== 501) {
      throw new Error(`resetting rails before test failed: ${reset.status()} ${await reset.text()}`);
    }

    const goto = page.goto.bind(page);
    page.goto = (async (url: string, opts?: Parameters<typeof goto>[1]) => {
      const response = await goto(url, opts);
      await page.mouse.move(700, 8);
      return response;
    }) as typeof page.goto;
    await use(page);
  },
  chrome: async ({ page }, use) => use(new AppChrome(page)),
  dashboard: async ({ page }, use) => use(new DashboardPage(page)),
  resources: async ({ page }, use) => use(new ResourcesPage(page)),
  history: async ({ page }, use) => use(new HistoryPage(page)),
  compartments: async ({ page }, use) => use(new CompartmentsPage(page)),
  queries: async ({ page }, use) => use(new QueriesPage(page)),
  search: async ({ page }, use) => use(new SearchPage(page)),
  searchParameters: async ({ page }, use) => use(new SearchParametersPage(page)),
  tenants: async ({ page }, use) => use(new TenantsPage(page)),
  bulkImport: async ({ page }, use) => use(new BulkImportPage(page)),
  capabilityStatement: async ({ page }, use) => use(new CapabilityStatementPage(page)),
});

export { expect };
