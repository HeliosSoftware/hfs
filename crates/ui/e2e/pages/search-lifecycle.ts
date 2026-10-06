import type { Page, Route } from "@playwright/test";
import { test, expect } from "./fixtures";
import { SearchBuilder, SearchResults } from "./search-builder";

export const searchBundle = (id = "previous") => ({
  resourceType: "Bundle", type: "searchset", total: 123,
  entry: [{ resource: { resourceType: "Patient", id, name: [{ family: id }] } }],
  link: [
    { relation: "previous", url: "https://example.invalid/page?opaque=before%2Fone" },
    { relation: "next", url: "/Patient?opaque=next%2Ftwo" },
  ],
});

/** Only holds marked search requests, leaving settings/catalog traffic real.
 * Cancellation is always produced by the application, never route.abort(). */
export async function holdSearches(page: Page) {
  const held = new Map<string, Route>();
  const failed: string[] = [];
  const requests: string[] = [];
  page.on("requestfailed", request => failed.push(request.url()));
  await page.route(/\/Patient\?.*issue1577/, route => {
    requests.push(route.request().url());
    held.set(new URL(route.request().url()).searchParams.get("_id")!, route);
  });
  return {
    held, failed, requests,
    async reply(id: string, body = searchBundle(id), status = 200) {
      await expect.poll(() => held.has(id)).toBe(true);
      await held.get(id)!.fulfill({ status, contentType: "application/fhir+json", body: JSON.stringify(body) }).catch(() => {});
    },
  };
}

export function searchLifecycleTests(path: string) {
  test.describe(`issue1577 search lifecycle ${path}`, () => {
    test("replacement by Enter aborts A and stale completion cannot clear B", async ({ page }) => {
      const pending = await holdSearches(page);
      await page.goto(path, { waitUntil: "networkidle" });
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      const builder = new SearchBuilder(page);
      const results = new SearchResults(page);
      await builder.run("Patient?_id=issue1577-A");
      await expect(builder.status).toBeVisible();
      await expect(builder.runButton).toBeEnabled();
      await builder.setUrl("Patient?_id=issue1577-B");
      await builder.url.press("Enter");
      await expect.poll(() => pending.held.has("issue1577-B")).toBe(true);
      await expect.poll(() => pending.failed.some(url => url.includes("issue1577-A"))).toBe(true);
      await pending.reply("issue1577-A");
      await expect(builder.status).toBeVisible();
      await expect(results.error).toBeHidden();
      await pending.reply("issue1577-B");
      await results.waitDone();
      await expect(results.rows).toContainText(["issue1577-B"]);
      await expect(builder.elapsed).toBeHidden();
      await expect(builder.slow).toBeHidden();
    });

    test("click replacement ignores AbortError during JSON body reading", async ({ page }) => {
      const pending = await holdSearches(page);
      await page.goto(path, { waitUntil: "networkidle" });
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      await page.evaluate(() => {
        const state = { reading: false, aborted: false, failures: 0 };
        (window as any).__searchBodyState = state;
        document.addEventListener("hfs:data-changed", event => {
          if ((event as CustomEvent).detail?.failed) state.failures++;
        });
        const original = window.fetch.bind(window);
        window.fetch = async (input, init) => {
          const response = await original(input, init);
          if (String(input).includes("issue1577-json-A")) {
            response.json = () => new Promise((_resolve, reject) => {
              state.reading = true;
              init!.signal!.addEventListener("abort", () => {
                state.aborted = true;
                reject(new DOMException("Body read cancelled", "AbortError"));
              }, { once: true });
            });
          }
          return response;
        };
      });
      const builder = new SearchBuilder(page), results = new SearchResults(page);
      await builder.run("Patient?_id=issue1577-json-A");
      await pending.reply("issue1577-json-A");
      await expect.poll(() => page.evaluate(() => (window as any).__searchBodyState.reading)).toBe(true);
      await builder.run("Patient?_id=issue1577-json-B");
      await expect.poll(() => pending.held.has("issue1577-json-B")).toBe(true);
      await expect.poll(() => page.evaluate(() => (window as any).__searchBodyState.aborted)).toBe(true);
      await expect(builder.status).toBeVisible();
      await expect(results.error).toBeHidden();
      await pending.reply("issue1577-json-B");
      await results.waitDone();
      expect(await page.evaluate(() => (window as any).__searchBodyState.failures)).toBe(0);
      await expect(results.rows).toContainText(["issue1577-json-B"]);
    });

    test("Cancel restores exact previous page and preserves candidate; Sort hydrates all controls", async ({ page }) => {
      const pending = await holdSearches(page);
      await page.goto(path, { waitUntil: "networkidle" });
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      const builder = new SearchBuilder(page), results = new SearchResults(page);
      await builder.run("Patient?_id=issue1577-prior");
      await pending.reply("issue1577-prior");
      await results.waitDone();
      const prior = await results.stableState();
      await builder.run("Patient?_id=issue1577-cancel&_sort=-birthdate");
      await expect(results.meta).toBeHidden();
      await expect(page.locator("#query-results-previous")).toBeVisible();
      await expect(results.card).toHaveClass(/is-busy/);
      await expect(builder.sort).toBeDisabled();
      await builder.setUrl("Patient?_id=issue1577-edited&_sort=-birthdate");
      await builder.cancel.focus();
      await builder.cancel.press("Enter");
      await expect.poll(() => pending.failed.some(url => url.includes("issue1577-cancel"))).toBe(true);
      await expect(builder.url).toBeFocused();
      await expect(builder.url).toHaveValue(/issue1577-edited/);
      expect(await results.stableState()).toEqual(prior);
      await builder.sort.selectOption("-_lastUpdated");
      await expect(builder.url).toHaveValue("GET /Patient?_id=issue1577-edited&_sort=-_lastUpdated");
      await expect(page.locator("#builder-controls .builder-row__value")).toHaveValue("-_lastUpdated");
      await expect(builder.sort).toHaveValue("-_lastUpdated");
      await pending.reply("issue1577-edited");
      await results.waitDone();
    });

    test("elapsed boundaries and slow notice retain the same request when continuing", async ({ page }) => {
      await page.clock.install();
      const pending = await holdSearches(page);
      await page.goto(path, { waitUntil: "networkidle" });
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      const builder = new SearchBuilder(page), results = new SearchResults(page);
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
      await builder.run("Patient?_id=issue1577-slow");
      await expect(builder.status).toBeVisible();
      await page.clock.runFor(1999);
      await expect(builder.elapsed).toBeHidden();
      await page.clock.runFor(1);
      await expect(builder.elapsed).toHaveText("2 seconds elapsed");
      await page.clock.runFor(999);
      await expect(builder.elapsed).toHaveText("2 seconds elapsed");
      await page.clock.runFor(1);
      await expect(builder.elapsed).toHaveText("3 seconds elapsed");
      await page.clock.runFor(56999);
      await expect(builder.slow).toBeHidden();
      await page.clock.runFor(1);
      await expect(builder.slow).toBeVisible();
      const slowStatus = page.locator("#query-search-slow-status");
      await expect(slowStatus).toHaveText("");
      await page.clock.runFor(1);
      await expect(slowStatus).toHaveText("This search is taking longer than expected. You can keep waiting or cancel it.");
      await builder.keepWaiting.click();
      await expect(slowStatus).toBeHidden();
      await expect(slowStatus).toHaveText("");
      await expect(builder.slow).toBeHidden();
      await expect(builder.url).toBeFocused();
      await page.clock.runFor(61000);
      await expect(builder.elapsed).toHaveText("121 seconds elapsed");
      await expect(builder.slow).toBeHidden();
      expect(pending.requests).toHaveLength(1);
      await pending.reply("issue1577-slow");
      await results.waitDone();
      await expect(builder.elapsed).toBeHidden();
      await expect(builder.cancel).toBeHidden();
      await page.clock.runFor(120000);
      await expect(builder.elapsed).toBeHidden();
      await expect(builder.slow).toBeHidden();
      await builder.run("Patient?_id=issue1577-timer-cancel");
      await expect.poll(() => pending.held.has("issue1577-timer-cancel")).toBe(true);
      await builder.cancel.click();
      await page.clock.runFor(120000);
      await expect(builder.elapsed).toBeHidden();
      await expect(builder.slow).toBeHidden();
      await builder.run("Patient?_id=issue1577-notice-cancel");
      await expect.poll(() => pending.held.has("issue1577-notice-cancel")).toBe(true);
      await page.clock.runFor(60000);
      await expect(builder.slow).toBeVisible();
      await page.evaluate(() => document.querySelector<HTMLButtonElement>("#query-search-slow-cancel")!.click());
      await page.clock.runFor(1);
      await expect(slowStatus).toBeHidden();
      await expect(slowStatus).toHaveText("");
      await expect(builder.slow).toBeHidden();
      await page.clock.resume();
    });

    test("real error cleanup and cancelling next request preserve its diagnostic", async ({ page }) => {
      const pending = await holdSearches(page);
      await page.goto(path, { waitUntil: "networkidle" });
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      const builder = new SearchBuilder(page), results = new SearchResults(page);
      await builder.run("Patient?_id=issue1577-error");
      await expect.poll(() => pending.held.has("issue1577-error")).toBe(true);
      await pending.held.get("issue1577-error")!.fulfill({ status: 501, contentType: "application/fhir+json", body: JSON.stringify({ resourceType: "OperationOutcome", issue: [{ diagnostics: "Unsupported test search" }] }) });
      await results.waitDone();
      await expect(results.error).toHaveText("Unsupported test search");
      const prior = await results.stableState();
      await builder.run("Patient?_id=issue1577-after-error");
      await expect.poll(() => pending.held.has("issue1577-after-error")).toBe(true);
      await builder.cancel.click();
      expect(await results.stableState()).toEqual(prior);
      await expect(builder.status).toBeHidden();
      await expect(builder.elapsed).toBeHidden();
    });
  });
}
