// #953: the pre-ingest window of a bulk submission has to *say something*.
//
// Before, every status read taken before the first entry landed reported a
// bare byte percentage — a determinate-sounding sentence for a phase whose
// duration is unknown, printed under the indeterminate sweep. The handler now
// names the phase instead (`waiting for a worker`, `reading manifest`,
// `sizing N of M files`, `downloading file N of M`), and none of those strings
// may begin with `processing ` in any case, because the UI parses that prefix
// case-insensitively into a percentage and would flip the bar to a determinate
// fill — the #827 mix.
//
// Both halves are asserted here against a deliberately slow Data Provider this
// spec serves itself: every pre-ingest sample must be phase text + an
// indeterminate track with no aria-valuenow, and the ingest samples that follow
// must be the byte-percentage wording (#954) + a determinate track that
// carries one.
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { test, expect } from "../pages/fixtures";

const FILES = 8;
const MANIFEST_DELAY_MS = 6_000;
const FILE_DELAY_MS = 2_500;

const PRE_INGEST =
  /^(waiting for a worker|reading manifest|sizing \d+ of \d+ files|downloading file \d+ of \d+)$/;
const INGEST = /^Processing (\d+)% of bytes/;

/** Two Patients per file, so the ingest phase reports several percentages. */
function ndjson(index: number): string {
  return (
    JSON.stringify({
      resourceType: "Patient",
      id: `bulk-progress-${index}-a`,
      name: [{ family: `Alpha${index}` }],
      gender: "female",
    }) +
    "\n" +
    JSON.stringify({
      resourceType: "Patient",
      id: `bulk-progress-${index}-b`,
      name: [{ family: `Beta${index}` }],
      gender: "male",
    }) +
    "\n"
  );
}

/**
 * A Data Provider that answers correctly but slowly, so the pre-ingest phases
 * last long enough to be sampled: the manifest takes {@link MANIFEST_DELAY_MS},
 * and each HEAD (sizing) and GET (downloading) takes {@link FILE_DELAY_MS}.
 * Delays are timers, not sleeps, so the requests HFS issues in parallel stay
 * parallel.
 *
 * Manifests are served under any `/manifest*` path: HFS rejects a manifest URL
 * it has already ingested, so each submission needs its own.
 */
function startSlowProvider(): Promise<{ server: Server; base: string }> {
  // Resolved before the first request is served; reading server.address() from
  // the handler instead would throw on the in-flight requests HFS is still
  // making when the suite closes the listener.
  let base = "";
  const server = createServer((req, res) => {
    const url = req.url ?? "/";

    if (url.startsWith("/manifest")) {
      setTimeout(() => {
        const body = JSON.stringify({
          transactionTime: "2026-09-08T00:00:00Z",
          request: `${base}${url}`,
          requiresAccessToken: false,
          output: Array.from({ length: FILES }, (_, i) => ({
            type: "Patient",
            url: `${base}/patients-${i}.ndjson`,
            count: 2,
          })),
          deleted: [],
          error: [],
        });
        res.writeHead(200, {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        });
        res.end(body);
      }, MANIFEST_DELAY_MS);
      return;
    }

    const file = /^\/patients-(\d+)\.ndjson$/.exec(url);
    if (file) {
      const body = ndjson(Number(file[1]));
      setTimeout(() => {
        res.writeHead(200, {
          "Content-Type": "application/fhir+ndjson",
          "Content-Length": Buffer.byteLength(body),
        });
        res.end(req.method === "HEAD" ? undefined : body);
      }, FILE_DELAY_MS);
      return;
    }

    res.writeHead(404, { "Content-Length": "0" });
    res.end();
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
      resolve({ server, base });
    });
  });
}

/** One reading of the status fragment. */
type Sample = { text: string; indeterminate: boolean; valuenow: string | null };

function readFragment(html: string): Sample | null {
  // The polling card carries the hx-trigger; the terminal card does not.
  if (!/hx-trigger="every 5s"/.test(html)) return null;
  const text = /<div>([^<]*)<\/div>/.exec(html)?.[1]?.trim() ?? "";
  return {
    text,
    indeterminate: /progress-track--indeterminate/.test(html),
    valuenow: /aria-valuenow="(\d+)"/.exec(html)?.[1] ?? null,
  };
}

let provider: Server;
let providerBase: string;

test.beforeAll(async () => {
  ({ server: provider, base: providerBase } = await startSlowProvider());
});

test.afterAll(async () => {
  await new Promise((done) => provider.close(done));
});

// The provider listens on loopback, so it is only reachable by an hfs running
// on this host. A remote HFS_E2E_BASE_URL (the backend matrix) cannot fetch it.
test.beforeEach(({ baseURL }) => {
  const host = new URL(baseURL!).hostname;
  test.skip(
    host !== "127.0.0.1" && host !== "localhost" && host !== "::1",
    "the slow Data Provider is served on loopback; a remote server cannot reach it",
  );
});

test("the pre-ingest phases name themselves under an indeterminate bar", async ({ page }) => {
  test.setTimeout(240_000);
  const manifestUrl = `${providerBase}/manifest-ui-${Date.now()}.json`;

  await page.goto("/ui/bulk-import");
  await page.locator("summary.btn", { hasText: "New Submission" }).click();
  await page.locator("input[name='name']").fill(`e2e-953-${Date.now()}`);
  await page.locator("input[name='manifest_url']").fill(manifestUrl);
  // Not `button[type=submit]` alone: the dialog's Test authentication control
  // is also a submit button, distinguished only by its formaction.
  await page.locator("form[action='/ui/bulk-import'] button[type='submit'].btn--primary").click();

  await expect(page).toHaveURL(/\/ui\/bulk-import\/[^/]+$/);
  const detail = new URL(page.url()).pathname;

  // Sample the fragment the card polls, rather than waiting on htmx's 5s
  // cadence: the assertions are about that fragment's markup.
  const samples: Sample[] = [];
  const deadline = Date.now() + 200_000;
  while (Date.now() < deadline) {
    const response = await page.request.get(`${detail}/status`);
    expect(response.status()).toBe(200);
    const sample = readFragment(await response.text());
    if (!sample) break; // terminal card: the submission finished
    if (sample.text !== samples.at(-1)?.text) samples.push(sample);
    if (samples.filter((s) => INGEST.test(s.text)).length >= 2) break;
    await page.waitForTimeout(2_000);
  }

  const readings = samples
    .map((s) => `${s.text} | indeterminate=${s.indeterminate} | aria-valuenow=${s.valuenow}`)
    .join("\n");
  // Timing-sensitive, so keep what was actually sampled in the report: a
  // failure is then diagnosable without reproducing the run.
  await test
    .info()
    .attach("status-fragment-readings", { body: readings, contentType: "text/plain" });
  const preIngest = samples.filter((s) => PRE_INGEST.test(s.text));
  const ingest = samples.filter((s) => INGEST.test(s.text));

  // The regression #953 fixes: this list used to be empty, because every one
  // of these reads was a bare `0%` byte reading.
  expect(preIngest.length, `no pre-ingest phase was ever reported:\n${readings}`).toBeGreaterThan(0);
  expect(ingest.length, `the submission never reached ingest:\n${readings}`).toBeGreaterThan(0);

  // #827: a phase with no percentage must never light a determinate bar.
  for (const sample of preIngest) {
    expect(sample.indeterminate, `"${sample.text}" lit a determinate bar`).toBe(true);
    expect(sample.valuenow, `"${sample.text}" carried a percentage`).toBeNull();
  }
  // …and the converse: a percentage must never be printed under the sweep.
  for (const sample of ingest) {
    expect(sample.indeterminate, `"${sample.text}" kept the indeterminate sweep`).toBe(false);
    expect(sample.valuenow, `"${sample.text}" carried no aria-valuenow`).toBe(
      INGEST.exec(sample.text)![1],
    );
  }
  // Phases come first; the bar never falls back out of determinate.
  expect(samples.indexOf(preIngest.at(-1)!)).toBeLessThan(samples.indexOf(ingest[0]!));
});

test("X-Progress names the phase instead of reporting 0% for the whole pre-ingest", async ({
  request,
}) => {
  test.setTimeout(240_000);
  const submissionId = `e2e-953-api-${Date.now()}`;
  const submitter = { system: "http://e2e.hfs.invalid/ehr", value: "e2e-953" };
  const parameters = (extra: Record<string, unknown>[]) => ({
    resourceType: "Parameters",
    parameter: [{ name: "submitter", valueIdentifier: submitter }, ...extra],
  });

  const kickoff = await request.post("/$bulk-submit", {
    headers: { "Content-Type": "application/fhir+json" },
    data: parameters([
      { name: "submissionId", valueString: submissionId },
      { name: "manifestUrl", valueUrl: `${providerBase}/manifest-api-${Date.now()}.json` },
      { name: "fhirBaseUrl", valueUrl: `${providerBase}/fhir` },
    ]),
  });
  expect(kickoff.status(), await kickoff.text()).toBe(200);

  const statusKickoff = await request.post("/$bulk-submit-status", {
    headers: { "Content-Type": "application/fhir+json" },
    data: parameters([{ name: "submissionId", valueString: submissionId }]),
    maxRedirects: 0,
  });
  expect(statusKickoff.status(), await statusKickoff.text()).toBe(202);
  const token = statusKickoff.headers()["content-location"]?.split("/bulk-submit-status/")[1];
  expect(token, "the status kick-off returned no Content-Location token").toBeTruthy();

  const progress: string[] = [];
  const deadline = Date.now() + 200_000;
  while (Date.now() < deadline) {
    const poll = await request.get(`/bulk-submit-status/${token}`);
    if (poll.status() !== 202) break; // 200: the manifest is ready
    const header = poll.headers()["x-progress"] ?? "";
    if (header !== progress.at(-1)) progress.push(header);
    if (progress.filter((p) => INGEST.test(p)).length >= 2) break;
    await new Promise((done) => setTimeout(done, 2_000));
  }

  const seen = progress.join("\n");
  await test.info().attach("x-progress-readings", { body: seen, contentType: "text/plain" });
  expect(progress.some((p) => PRE_INGEST.test(p)), `no pre-ingest phase reported:\n${seen}`).toBe(
    true,
  );
  expect(progress.some((p) => INGEST.test(p)), `ingest never reported:\n${seen}`).toBe(true);
  // The old handler's answer for the entire pre-ingest window: a bare 0% byte
  // reading, with no phase and no resource count to qualify it.
  expect(progress, `a bare "0% of bytes" is the pre-#953 reading`).not.toContain(
    "Processing 0% of bytes",
  );
});
