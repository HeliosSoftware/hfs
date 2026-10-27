// #968: Abort, driven through the browser against a live ingest.
//
// The rest of the Bulk Import coverage seeds submissions over the API and
// asserts on layout. This spec instead walks the whole operator path — open
// the New Submission dialog, type a manifest URL, submit, watch the ingest
// start, press Abort — because the bug being fixed only exists while data is
// actually moving: before #968 the recipient accepted the abort, the UI said
// "Stopped", and the worker kept ingesting the manifest to the end.
//
// A static NDJSON file cannot express that: HFS finishes it faster than a
// browser can click, so the submission is already terminal and Abort is never
// even offered. `NdjsonProvider` therefore drips the file, which both keeps
// the ingest alive long enough to interrupt and turns "did the recipient stop
// reading?" into a number the test can watch (`bytesServed`).
import { test, expect } from "../pages/fixtures";
import { deleteResources } from "../pages/api";
import { NdjsonProvider } from "../pages/ndjson-provider";

// Not run in CI for now. This is the only spec in the suite that ingests in
// bulk, and it is a heavy neighbour: it holds a connection open for ~20s and
// pushes thousands of resources through the server every other spec shares.
// It stays here and runs locally — `npx playwright test bulk-import-abort` —
// because it is the only check that exercises Abort against data genuinely in
// flight, which is the one thing #968 was about.
//
// The mechanism underneath it is covered in CI without a browser, by
// `test_lease_keeper_cancels_an_aborted_submission` in
// `crates/persistence/src/core/bulk_submit_worker.rs`: the keeper is what
// re-reads the submission and trips the ingest's cancel token, and that test
// fails if the trip is removed.
test.skip(!!process.env.CI, "heavy shared-server ingest; runs locally, see the note above");

let provider: NdjsonProvider;

// ~15 MB dripped at ~128 KB/s. The size is not there to be ingested — the
// abort lands seconds in — but to dwarf the roughly one megabyte the socket
// buffers swallow after the recipient stops reading, so "it stopped well
// short of the end" stays a wide margin instead of a coin toss.
//
// Those megabytes are made of few, fat lines rather than many small ones. The
// e2e server is shared, and this spec is the only one that ingests in bulk: at
// the natural ~48 bytes a Patient it left tens of thousands of them behind for
// every later SQL-on-FHIR spec to scan, which is how it walked the browser
// suite into an OOM kill. Padding decouples the byte count from the row count.
const LINES = 30_000;
const LINE_BYTES = 512;

// Enough of the stream consumed that whole batches have certainly landed (the
// worker commits every 1000 rows), so the "partial ingest is durable" check
// below is testing cancellation semantics and not a race with the first batch.
const ABORT_AFTER_BYTES = 1_000_000;

test.beforeEach(async () => {
  provider = new NdjsonProvider({
    lines: LINES,
    lineBytes: LINE_BYTES,
    // Fresh ids per run. The sweep below leaves tombstones, and re-ingesting a
    // deleted id does not resurrect it — the server still answers `410 Gone` —
    // so a fixed id space would make the durability check below pass once and
    // then fail on every subsequent run against the same server.
    idPrefix: `e2e968-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
    chunkBytes: 32 * 1024,
    pauseMs: 250,
  });
  await provider.start();
});

test.afterEach(async ({ request }) => {
  const served = Math.max(0, ...provider.streams.map((stream) => stream.served));
  await provider.stop();

  // Leave the shared server as it was found. Nothing beyond what crossed the
  // wire can have been stored, so served bytes bound the sweep; ids that never
  // landed just come back 404, which `deleteResources` tolerates.
  const reached = Math.min(LINES, Math.ceil(served / provider.lineBytes) + 1);
  await deleteResources(
    request,
    "Patient",
    Array.from({ length: reached }, (_, i) => provider.idAt(i)),
  );

  // A sweep that quietly stopped working would look exactly like a sweep that
  // had nothing to do, and the cost of not noticing is a suite-wide timeout an
  // hour later — so make the server say the rows are gone. `410` is the answer
  // for the usual case, an id that was ingested and then deleted; `404` covers
  // a run that failed before the ingest ever reached the first line.
  const swept = await request.get(`/Patient/${provider.idAt(0)}`);
  expect([404, 410]).toContain(swept.status());
});

test("Abort stops an ingest that is under way, and the data stops arriving", async ({
  page,
  request,
  bulkImport,
}) => {
  test.slow();

  // --- The operator creates the submission through the dialog. ---
  await page.goto("/ui/bulk-import", { waitUntil: "domcontentloaded" });
  await page.locator("summary.btn", { hasText: "New Submission" }).click();

  const dialog = page.locator("details.addbox--modal[open] .addbox__panel");
  await dialog.locator("input[name='name']").fill("e2e-968-abort");
  await dialog.locator("input[name='manifest_url']").fill(provider.manifestUrl);
  await dialog.getByRole("button", { name: "Submit" }).click();

  await page.waitForURL(/\/ui\/bulk-import\/[^/]+$/);

  // --- The ingest starts: the recipient reads the manifest, sizes the file
  // with a HEAD, then opens the real download. ---
  await expect
    .poll(() => provider.requests, { timeout: 30_000 })
    .toEqual(["GET /manifest.json", "HEAD /patients.ndjson", "GET /patients.ndjson"]);

  // The status card polls itself in; while the submission is live it offers
  // Abort. That button existing is the precondition for the whole test.
  const abortButton = bulkImport.statusCard.locator("form[action$='/abort'] button");
  await expect(abortButton).toBeVisible({ timeout: 30_000 });
  await expect(bulkImport.statusCell).toHaveText("In Progress");

  // Wait until enough has genuinely crossed the wire that stopping is a real
  // interruption and not a race with the first byte.
  await expect
    .poll(() => provider.bytesServed, { timeout: 60_000 })
    .toBeGreaterThan(ABORT_AFTER_BYTES);

  // --- The operator presses Abort. ---
  await abortButton.click();

  // The UI settles into the terminal state: status flips, the buttons go, and
  // no failure banner is left behind (the recipient is this same server, so
  // the status change is delivered and acknowledged).
  await expect(bulkImport.statusCell).toHaveText("Stopped", { timeout: 30_000 });
  await expect(page.locator("#submission-error")).toBeEmpty();
  await expect(abortButton).toHaveCount(0);
  expect(await bulkImport.logLines()).toEqual(
    expect.arrayContaining([expect.stringContaining("Recipient acknowledged (200).")]),
  );

  // --- And, the point of #968: the data actually stops arriving. ---
  // The recipient drops the download it was half-way through, which the
  // provider sees as its own connection closing early. A UI that says
  // "Stopped" over a still-running ingest is exactly the bug.
  await expect.poll(() => provider.hungUp, { timeout: 30_000 }).toBe(true);

  // And it stays stopped. `served` counts bytes pushed at the socket, so it
  // coasts on for a moment past the abort on what the buffers had already
  // swallowed; once it settles it must stay put, and well short of the file.
  const settled = provider.bytesServed;
  await page.waitForTimeout(3_000);
  expect(provider.bytesServed).toBe(settled);
  expect(provider.bytesServed).toBeLessThan(provider.totalBytes / 4);

  // Stopped means stopped: the file is not quietly fetched again afterwards,
  // and no download of it ever ran to completion.
  expect(provider.streams).toEqual([{ served: settled, hungUp: true, complete: false }]);

  // The partial ingest is durable — cancelling is a stop, not a rollback —
  // while the tail of the file was never stored.
  const early = await request.get(`/Patient/${provider.idAt(0)}`);
  expect(early.status()).toBe(200);
  const last = await request.get(`/Patient/${provider.idAt(LINES - 1)}`);
  expect(last.status()).toBe(404);
});
