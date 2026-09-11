// §7 of MANUAL_TESTING_MATRIX.md ("T3 — Import the Synthea corpus from the
// Import page"), performed by the browser instead of by a person.
//
// Every other bulk-import spec seeds its subject with `bulkImport.seed()`, an
// HTTP POST straight at `/ui/bulk-import`. That is the right trade when the
// subject is a layout or a fragment swap. It is the wrong trade here: what this
// covers is the whole operator round trip — the sidebar, the New Submission
// dialog, the redirect to the detail page, the status card polling itself
// forward, the numbers it prints while it does — so it is driven exclusively
// through the page. Nothing this test *asserts on* is reached by any route but
// the browser: no `request.*` call appears anywhere in the walkthrough, on
// purpose. The one exception is teardown, which is not a step an operator
// performs and is discussed where it happens.
//
// Why the Import page can see the ingest at all: the Import page is the Data
// *Provider* half, but its recipient is this same server (`recipient_base_url`
// in crates/ui/src/bulk_import.rs returns the configured public base URL), so
// submitting from it makes HFS `$bulk-submit` to itself. The status card is
// therefore rendering the *consumer's* own progress — the byte percentage and
// the resource counter #969 is about.
//
// What only this level can prove: the counter never walks backwards. The Rust
// ring pins the invariant per backend against a stubbed job store; here it is
// read off the screen, over a real ingest, exactly as an operator reads it.
import { test, expect } from "../pages/fixtures";
import { BulkSubmitSource } from "../pages/bulk-submit-source";
import { deleteResources } from "../pages/api";

// What the sampling actually needs is two numbers, and neither of them is the
// resource count: ~40s of wall clock, so the status card's 5s poll gets ~8
// looks, and ~20 movements of the counter, so consecutive looks land on
// different values. A submission that finishes inside one poll interval leaves
// a single sample, and a single sample is no monotonicity check at all.
//
// Both are set by the *fixture's pacing* — 40 chunks, one a second — against
// the ingest batch size, so the corpus can be as small as those two allow. It
// is deliberately small: this spec shares its database with every other one
// (see the teardown at the end), and the 2000 Patients this started at broke
// twelve SQL-on-FHIR tests ~200 tests later. 400 lines at 10 per chunk buys
// exactly the same 40s and, at `HFS_BULK_SUBMIT_BATCH_SIZE=20` (boot.mjs),
// exactly the same 20 movements.
const RESOURCES = 400;
const SOURCE = { resources: RESOURCES, chunkLines: 10, chunkDelayMs: 1000 };

// Wall clock: ~40s of ingest, plus dialog work, plus the search at the end.
// Well inside this, and far enough under it that a genuine hang still fails.
test.setTimeout(240_000);

/** The `Status: …` entries the Submission Log recorded, oldest first.
 *
 * The log is the sampling instrument, not the status card. `poll_status` writes
 * one entry per *distinct* X-Progress string, so by the end the log holds every
 * value the recipient ever reported, in order, with none of them lost to a
 * screenshot landing between two polls. The card only ever shows the latest. */
function progressReports(lines: string[]): { pct: number; written: number }[] {
  return lines
    .slice()
    .reverse() // the log renders newest first
    .map((line) => /Processing (\d+)% - ([\d,]+) Resources written/.exec(line))
    .filter((match): match is RegExpExecArray => match !== null)
    .map((match) => ({ pct: Number(match[1]), written: Number(match[2].replace(/,/g, "")) }));
}

// The source listens on loopback, so it is only reachable by an hfs running on
// this host. A remote HFS_E2E_BASE_URL (the backend matrix) cannot fetch it;
// the per-PR ui-tests.yml run, where hfs and the browser share a host, keeps
// the coverage. Skipped here, before the test body starts the source at all.
test.beforeEach(({ baseURL }) => {
  const host = new URL(baseURL!).hostname;
  test.skip(
    host !== "127.0.0.1" && host !== "localhost" && host !== "::1",
    "the bulk-submit source is served on loopback; a remote server cannot fetch it",
  );
});

test("a manifest submitted from the Import page ingests, and its counters only ever climb", async ({
  page,
  chrome,
  bulkImport,
  queries,
  request,
}) => {
  // A per-run family name: the Patients this run writes are the only ones that
  // answer to it, so the search at the end is unambiguous however many times
  // the suite has run against this database.
  const family = `E2eBulkSubmit${Date.now().toString(36)}`;
  const source = new BulkSubmitSource({ ...SOURCE, family });
  await source.start();

  try {
    await test.step("§7.2 create the submission from the Import page", async () => {
      // Arrive the way an operator does: the app's front door, then the rail.
      await page.goto("/ui");
      await chrome.navLink("/ui/bulk-import").click();
      await expect(page).toHaveURL(/\/ui\/bulk-import$/);
      await expect(bulkImport.submissionsTable).toBeVisible();

      await bulkImport.newSubmission.click();
      await expect(bulkImport.createDialog).toBeVisible();
      // The dialog puts the caret where typing should start.
      await expect(page.locator("input[name='name']")).toBeFocused();

      await page.locator("input[name='name']").fill(family);
      await page.locator("input[name='manifest_url']").fill(source.manifestUrl);
      // Authentication stays on None and Advanced options stays folded: the
      // matrix's happy path is the defaults.
      await expect(page.locator("input[name='auth'][value='none']")).toBeChecked();

      await Promise.all([
        page.waitForURL(/\/ui\/bulk-import\/[^/]+$/),
        // The dialog's own action, not the auth fieldset's Test authentication,
        // which is also a type=submit inside the panel.
        page.locator(".addbox__actions button[type='submit']").click(),
      ]);
    });

    await test.step("§7.3 the summary describes what was just submitted", async () => {
      await expect(bulkImport.summaryField("Manifest URL")).toHaveText(source.manifestUrl);
      // The recipient is this server — that is what makes the ingest visible.
      await expect(bulkImport.summaryField("Data Recipient")).toContainText(
        new URL(page.url()).origin,
      );
      await expect(bulkImport.summaryField("Submission ID")).not.toBeEmpty();
      await expect(bulkImport.summaryField("Submitter")).toContainText(
        "urn:helios:hfs:bulk-submit",
      );
      await expect(bulkImport.summaryField("Status")).toHaveText("In Progress");
      await expect(bulkImport.summaryField("Authentication")).toHaveText("none");

      // Kick-off is already in the log by first paint.
      const opening = (await bulkImport.logLines()).join("\n");
      expect(opening).toContain(`Submitting manifest "${source.manifestUrl}"`);
      // Any 2xx is an acceptance; which one is the recipient's business.
      expect(opening).toMatch(/Manifest accepted by the recipient \(2\d\d\)\./);
      expect(opening).toContain("Bulk status kick-off request");
    });

    await test.step("§7.3 the status card polls itself forward while data lands", async () => {
      // Tag the document: everything below has to happen by htmx patching this
      // page, not by a reload that would refresh the numbers for free.
      await page.evaluate(() => {
        (window as unknown as { __sameDocument: boolean }).__sameDocument = true;
      });

      await expect(bulkImport.progressBar).toBeVisible({ timeout: 30_000 });
      await expect
        .poll(async () => bulkImport.resourcesWritten(), { timeout: 60_000 })
        .toBeGreaterThan(0);
      // The percentage is byte-based, which needs the file's Content-Length to
      // have reached the worker: an indeterminate bar here means the size was
      // lost on the way in.
      await expect(bulkImport.progressBar).toHaveAttribute("aria-valuenow", /^\d+$/);
      await expect(bulkImport.progressText).toHaveText(
        /Processing \d+% - [\d,]+ Resources written/,
      );

      // HFS fetched both fixture files itself, server-to-server — and sized the
      // data file with a HEAD first, which is what a byte percentage needs.
      expect(source.requested).toContain("GET /manifest.json");
      expect(source.requested).toContain("HEAD /patients.ndjson");
      expect(source.requested).toContain("GET /patients.ndjson");

      expect(
        await page.evaluate(
          () => (window as unknown as { __sameDocument?: boolean }).__sameDocument === true,
        ),
      ).toBe(true);
    });

    await test.step("§7.3 it finishes, and reports a clean result", async () => {
      await expect(bulkImport.statusField("Result")).toBeVisible({ timeout: 150_000 });
      await expect(bulkImport.statusField("Result")).toContainText("Processing finished at");
      await expect(bulkImport.statusField("Output files")).toHaveText("1");
      await expect(bulkImport.statusField("Error files")).toHaveText("0");
      // The summary's own STATUS cell rode along on the same poll.
      await expect(bulkImport.summaryField("Status")).toHaveText("Completed");
      await expect(bulkImport.progressBar).toHaveCount(0);

      const closing = await bulkImport.logLines();
      expect(closing[0]).toContain(
        "Status: got 200 OK — processing finished cleanly (1 outputs); submission completed.",
      );
    });

    await test.step("#969 every counter the recipient reported was >= the one before it", async () => {
      const reports = progressReports(await bulkImport.logLines());
      // A submission that completed inside a single poll proves nothing about
      // a sequence, so refuse to pass on one sample. Two is the floor a
      // monotonicity check is even defined at; the fixture is paced to clear
      // it by a wide margin, and measured 6 on consecutive local runs. The
      // floor stays at the semantic minimum rather than at the measurement:
      // a slower machine polls fewer times, and that is not this test's bug.
      expect(reports.length).toBeGreaterThanOrEqual(2);

      const written = reports.map((r) => r.written);
      expect(written).toEqual([...written].sort((a, b) => a - b));
      // #969's regression shape precisely: a resumed manifest restarting its
      // count, so the operator watches the number fall back toward zero.
      expect(Math.min(...written)).toBeGreaterThan(0);
      // Byte progress is a running total over the same run, so it climbs too.
      const pcts = reports.map((r) => r.pct);
      expect(pcts).toEqual([...pcts].sort((a, b) => a - b));
      // And the last thing shown never overstated what was actually written.
      expect(Math.max(...written)).toBeLessThanOrEqual(RESOURCES);
    });

    await test.step("§7.4 the imported data is searchable from the query builder", async () => {
      await queries.goto("Patient");
      await queries.builder.run(`Patient?family=${family}`);
      await queries.results.waitShown();
      // A page of real rows first: this is what an operator actually looks at,
      // and it proves the Patients were stored and indexed, not merely counted.
      await expect(queries.results.rows.first()).toBeVisible({ timeout: 60_000 });
      await expect(queries.results.rows.first()).toContainText(family);
    });

    await test.step("§7.4 and all of it landed, not just the first page", async () => {
      // A plain search returns one page, and its Bundle carries no `total`, so
      // the results meta counts the rows on screen — 20, the default page size.
      // `_summary=count` is the query that asks the server for the whole tally,
      // which is the number worth comparing against the manifest.
      await queries.builder.run(`Patient?family=${family}&_summary=count`);
      // Matched on the digits alone: Fluent wraps a placeable in bidi isolates,
      // so the rendered total is not the plain "2000 results" the .ftl reads as.
      // The window is wide because search follows the per-manifest reindex and
      // can trail the completion the status card just reported.
      await expect(queries.results.meta).toHaveText(new RegExp(`\\b${RESOURCES}\\b`), {
        timeout: 60_000,
      });
    });
  } finally {
    await source.stop();

    // Teardown, and the only thing here that is not the browser's work.
    //
    // The suite runs one server and one database for every spec file
    // (`playwright.config.ts`: `workers: 1`, `fullyParallel: false`), so
    // subjects left behind are not this spec's private mess — they are a tax
    // every later spec pays, and the SQL-on-FHIR specs pay it per row:
    // `sql-export.spec.ts` and `sql-libraries.spec.ts` run jobs over
    // `resource: "Patient"` views and poll them on a 30s budget. At the 2000
    // Patients this file first ingested they went from ~1s to ~30s, and CI
    // lost twelve of them across the two files — with this test itself green,
    // ~200 tests earlier, which is what made it worth a comment this long.
    // Cleaning up is the contract every seeding spec already keeps
    // (`deleteResources`; sql-export's own `afterEach` does the same).
    //
    // No search is needed to find them, and none is wanted: the fixture
    // assigns each Patient its id, so the ids are known without asking the
    // server, and a run that failed before ingesting anything deletes
    // nothing — `deleteResources` treats a 404 entry as already gone.
    await deleteResources(
      request,
      "Patient",
      Array.from({ length: RESOURCES }, (_, i) => `${family}-${i}`),
    );
  }
});
