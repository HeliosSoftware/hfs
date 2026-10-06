// Page-object fixtures: one place that wires every page object onto Playwright's
// `test`, so specs read `test("…", async ({ resources, history }) => …)` instead
// of newing objects up. Import { test, expect } from here, not @playwright/test.
import { test as base, expect, type Locator, type Page } from "@playwright/test";
import { AppChrome } from "./chrome";
import { DashboardPage } from "./dashboard";
import { ResourcesPage } from "./resources";
import { HistoryPage } from "./history";
import { CompartmentsPage } from "./compartments";
import { SearchPage } from "./search";
import { SearchParametersPage } from "./search-parameters";
import { TenantsPage } from "./tenants";
import { BulkImportPage } from "./bulk-import";
import { BulkExportPage } from "./bulk-export";
import { CapabilityStatementPage } from "./capability-statement";
import { SqlExportPage } from "./sql-export";

// Dialog policy (#1240, #1667). Two kinds of "are you sure?" exist:
//
// 1. The UI's own in-page confirmation (`assets/confirm.js`): one shared
//    `<dialog class="confirm-dialog">` for every delete, discard and
//    "save anyway" question. It is ordinary DOM, so specs answer it with
//    `acceptConfirm` / `dismissConfirm` below (or read `confirmDialog`),
//    never with `page.on("dialog")`.
//
// 2. The browser's native prompts. Only `beforeunload` (unsaved.js) is
//    still expected — no page may draw its own UI there. Every page gets
//    exactly one `dialog` listener, registered by the `page` fixture below:
//    - every native dialog is recorded (`{ type, message }`) for
//      `dialogsSeen` to read;
//    - a native `confirm` is a regression (#1667: nothing in the UI may call
//      `window.confirm` any more). It is dismissed, recorded separately, and
//      fails the test at fixture teardown, so it can never slip by silently;
//    - an action armed with `armDialog` is consumed once (one-shot) and
//      applied — how a spec dismisses a `beforeunload` to prove a
//      navigation was held back;
//    - otherwise `beforeunload` is accepted by default — the unsaved-changes
//      guard would otherwise abort every navigation a test performs;
//    - anything else is dismissed, but only when this is the dialog's only
//      listener: a spec's own `page.on("dialog", …)` is a second listener,
//      and this handler steps back and lets it decide instead of racing it
//      to `accept()`/`dismiss()`.
type DialogAction = "accept" | "dismiss";
type DialogRecord = { type: string; message: string };

const dialogLog = new WeakMap<Page, DialogRecord[]>();
const armedDialogAction = new WeakMap<Page, DialogAction>();
const nativeConfirms = new WeakMap<Page, string[]>();

/** Arms a one-shot action for this page's next native (`beforeunload`) dialog. */
export function armDialog(page: Page, action: DialogAction): void {
  armedDialogAction.set(page, action);
}

/** Returns every native dialog seen by this page since the last call, then clears it. */
export function dialogsSeen(page: Page): DialogRecord[] {
  const seen = dialogLog.get(page) || [];
  dialogLog.set(page, []);
  return seen;
}

/** The shared in-page confirmation dialog (`assets/confirm.js`, #1667). */
export function confirmDialog(page: Page): Locator {
  return page.locator("dialog.confirm-dialog");
}

async function answerConfirm(
  page: Page,
  button: "[data-confirm-ok]" | "[data-confirm-cancel]",
  expectedMessage?: string | RegExp,
): Promise<void> {
  const dialog = confirmDialog(page);
  await expect(dialog).toBeVisible();
  if (expectedMessage !== undefined) {
    await expect(dialog.locator(".confirm-dialog__message")).toHaveText(expectedMessage);
  }
  await dialog.locator(button).click();
  // confirm.js removes the element on close.
  await expect(dialog).toHaveCount(0);
}

/** Waits for the in-page confirmation, optionally checks its message, and confirms it. */
export async function acceptConfirm(page: Page, expectedMessage?: string | RegExp): Promise<void> {
  await answerConfirm(page, "[data-confirm-ok]", expectedMessage);
}

/** Waits for the in-page confirmation, optionally checks its message, and cancels it. */
export async function dismissConfirm(page: Page, expectedMessage?: string | RegExp): Promise<void> {
  await answerConfirm(page, "[data-confirm-cancel]", expectedMessage);
}

type Fixtures = {
  chrome: AppChrome;
  dashboard: DashboardPage;
  resources: ResourcesPage;
  history: HistoryPage;
  compartments: CompartmentsPage;
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
  //
  // Rail state (#754/#755) is server-side and per-user: on the default
  // (no-auth) projects every test runs as the same `l2:` user, so a
  // "last selected" or "recently used" recorded by one test would otherwise
  // leak into the next one's rail. Reset the `rails` record before each test
  // with a merge patch that deletes it (`null`), same shape and endpoint
  // `search-builder.js` and the theme toggle already use.
  //
  // Purely a test-isolation convenience, so this must be 100% best-effort:
  // it must never fail a test regardless of what the server does with it.
  // The `auth`/`auth-degraded` projects run against `HFS_AUTH`-enabled
  // servers and this fixture carries no bearer token, so the request comes
  // back 401/403 there; other legs can 501 (no settings store configured) or
  // anything else. Any response status is fine, and a network-level failure
  // (refused connection, timeout) is caught rather than propagated — none of
  // that is worth ever taking down a test over.
  page: async ({ page }, use) => {
    try {
      await page.request.patch("/_user/settings", {
        headers: { "Content-Type": "application/json" },
        data: { rails: null },
      });
    } catch {
      // Best-effort; see above.
    }

    const goto = page.goto.bind(page);
    page.goto = (async (url: string, opts?: Parameters<typeof goto>[1]) => {
      const response = await goto(url, opts);
      await page.mouse.move(700, 8);
      return response;
    }) as typeof page.goto;

    page.on("dialog", (dialog) => {
      const seen = dialogLog.get(page) || [];
      seen.push({ type: dialog.type(), message: dialog.message() });
      dialogLog.set(page, seen);

      if (dialog.type() === "confirm") {
        const confirms = nativeConfirms.get(page) || [];
        confirms.push(dialog.message());
        nativeConfirms.set(page, confirms);
        if (page.listenerCount("dialog") === 1) dialog.dismiss();
        return;
      }

      const armed = armedDialogAction.get(page);
      if (armed) {
        armedDialogAction.delete(page);
        if (armed === "accept") dialog.accept();
        else dialog.dismiss();
        return;
      }
      if (dialog.type() === "beforeunload") {
        dialog.accept();
        return;
      }
      if (page.listenerCount("dialog") === 1) dialog.dismiss();
    });

    await use(page);

    expect(
      nativeConfirms.get(page) || [],
      "a native window.confirm appeared; use the in-page HfsConfirm dialog (#1667)",
    ).toEqual([]);
  },
  chrome: async ({ page }, use) => use(new AppChrome(page)),
  dashboard: async ({ page }, use) => use(new DashboardPage(page)),
  resources: async ({ page }, use) => use(new ResourcesPage(page)),
  history: async ({ page }, use) => use(new HistoryPage(page)),
  compartments: async ({ page }, use) => use(new CompartmentsPage(page)),
  search: async ({ page }, use) => use(new SearchPage(page)),
  searchParameters: async ({ page }, use) => use(new SearchParametersPage(page)),
  tenants: async ({ page }, use) => use(new TenantsPage(page)),
  bulkImport: async ({ page }, use) => use(new BulkImportPage(page)),
  bulkExport: async ({ page }, use) => use(new BulkExportPage(page)),
  capabilityStatement: async ({ page }, use) => use(new CapabilityStatementPage(page)),
  sqlExport: async ({ page }, use) => use(new SqlExportPage(page)),
});

export { expect };
