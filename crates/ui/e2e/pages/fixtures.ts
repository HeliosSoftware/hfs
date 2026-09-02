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
import { BulkExportPage } from "./bulk-export";
import { CapabilityStatementPage } from "./capability-statement";
import { SqlExportPage } from "./sql-export";

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
  bulkExport: BulkExportPage;
  capabilityStatement: CapabilityStatementPage;
  sqlExport: SqlExportPage;
};

export const test = base.extend<Fixtures>({
  // The sidebar expands on hover (#438) and the mouse starts at (0,0) — over
  // the rail — so a fresh page would open with the sidebar overlaying the
  // left content edge and intercepting clicks. Park the pointer in the topbar
  // after every navigation; tests that exercise the hover do so explicitly.
  page: async ({ page }, use) => {
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
  bulkExport: async ({ page }, use) => use(new BulkExportPage(page)),
  capabilityStatement: async ({ page }, use) => use(new CapabilityStatementPage(page)),
  sqlExport: async ({ page }, use) => use(new SqlExportPage(page)),
});

export { expect };
