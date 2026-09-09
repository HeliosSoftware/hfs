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
import { NdjsonProvider } from "../pages/ndjson-provider";

let provider: NdjsonProvider;

// ~9.6 MB dripped at ~128 KB/s. The size is not there to be ingested — the
// abort lands seconds in — but to dwarf the roughly one megabyte the socket
// buffers swallow after the recipient stops reading, so "it stopped well
// short of the end" stays a wide margin instead of a coin toss.
const LINES = 200_000;

test.beforeEach(async () => {
  provider = new NdjsonProvider({ lines: LINES, chunkBytes: 32 * 1024, pauseMs: 250 });
  await provider.start();
});

test.afterEach(async () => {
  await provider.stop();
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
    .poll(() => provider.bytesServed, { timeout: 30_000 })
    .toBeGreaterThan(300_000);

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
