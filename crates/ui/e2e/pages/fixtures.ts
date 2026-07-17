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
};

export const test = base.extend<Fixtures>({
  chrome: async ({ page }, use) => use(new AppChrome(page)),
  dashboard: async ({ page }, use) => use(new DashboardPage(page)),
  resources: async ({ page }, use) => use(new ResourcesPage(page)),
  history: async ({ page }, use) => use(new HistoryPage(page)),
  compartments: async ({ page }, use) => use(new CompartmentsPage(page)),
  queries: async ({ page }, use) => use(new QueriesPage(page)),
  search: async ({ page }, use) => use(new SearchPage(page)),
  searchParameters: async ({ page }, use) => use(new SearchParametersPage(page)),
  tenants: async ({ page }, use) => use(new TenantsPage(page)),
});

export { expect };
