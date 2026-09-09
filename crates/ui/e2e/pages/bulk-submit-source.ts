// A stand-in Data Provider for the Import page (§7.1 of MANUAL_TESTING_MATRIX):
// the small HTTP server the manual pass runs on port 8000 to serve a Bulk
// Export manifest and its NDJSON files, shrunk to something a browser test can
// watch end to end.
//
// It exists because `$bulk-submit` is a *server-to-server* fetch: HFS reads the
// manifest and the data files itself, so `page.route()` cannot supply them —
// that only intercepts the browser's own traffic. The fixture therefore has to
// be a real socket the HFS process can reach.
//
// Two properties are deliberate, because the whole point of the test is to
// watch progress move:
//
//   * The NDJSON is served with an exact `Content-Length` and *then* trickled
//     out in timed chunks. The length is what lets the worker report a byte
//     percentage (a sizeless stream drops the manifest out of the percentage
//     sum entirely), and the trickle is what stretches the ingest over enough
//     wall-clock for the status card's 5s poll to sample it repeatedly. Pacing
//     the *source* keeps the test's timing independent of how fast the machine
//     writes to SQLite.
//   * Every Patient carries a client-assigned id and a run-unique family name,
//     so the data set stays idempotent under re-ingestion and is still findable
//     from the query builder afterwards (§7.4).
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

export interface BulkSubmitSourceOptions {
  /** Patient lines in the NDJSON file. */
  resources: number;
  /** Lines flushed per write. */
  chunkLines: number;
  /** Pause between writes, in ms. `resources / chunkLines * chunkDelayMs` is
   * roughly how long the ingest takes, whatever the machine's write speed. */
  chunkDelayMs: number;
  /** Family name stamped on every Patient — unique per run, so a search for it
   * matches this submission's data and nothing else. */
  family: string;
}

const IDENTIFIER_SYSTEM = "urn:helios:hfs:e2e:bulk-submit";

export class BulkSubmitSource {
  private server: Server | null = null;
  private origin = "";
  private readonly body: Buffer;
  /** What HFS actually requested, as `"<METHOD> <path>"`, so the test can
   * assert the fetch happened — and that it was server-to-server, not the
   * browser's (the manual pass reads the same thing out of
   * `$WORK/corpus-http.log`). */
  readonly requested: string[] = [];

  constructor(private readonly options: BulkSubmitSourceOptions) {
    const lines: string[] = [];
    for (let i = 0; i < options.resources; i += 1) {
      lines.push(
        JSON.stringify({
          resourceType: "Patient",
          id: `${options.family}-${i}`,
          identifier: [{ system: IDENTIFIER_SYSTEM, value: `${options.family}-${i}` }],
          name: [{ family: options.family, given: [`Case${i}`] }],
          gender: i % 2 === 0 ? "female" : "male",
        }),
      );
    }
    this.body = Buffer.from(`${lines.join("\n")}\n`, "utf8");
  }

  /** Binds on an ephemeral loopback port and resolves once it is accepting. */
  async start(): Promise<void> {
    const server = createServer((req, res) => {
      const path = (req.url ?? "/").split("?")[0];
      const method = req.method ?? "GET";
      this.requested.push(`${method} ${path}`);
      if (path === "/manifest.json") {
        const manifest = JSON.stringify(this.manifest(), null, 2);
        res.writeHead(200, {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(manifest),
        });
        res.end(method === "HEAD" ? undefined : manifest);
        return;
      }
      if (path === "/patients.ndjson") {
        res.writeHead(200, {
          "content-type": "application/fhir+ndjson",
          "content-length": this.body.length,
        });
        // The worker sizes every output file *before* ingesting it, with an
        // HTTP HEAD whose Content-Length it reads (`file_size` in
        // crates/rest/src/bulk_submit_fetcher.rs issues `client.head(url)`).
        // That probe must answer instantly: trickling a HEAD would stall the
        // pre-size pass for the full duration of the fixture and only *then*
        // let the real GET start, doubling the ingest and starving the very
        // counters this fixture exists to make visible.
        if (method === "HEAD") {
          res.end();
          return;
        }
        void this.trickle(res);
        return;
      }
      res.writeHead(404, { "content-type": "text/plain" }).end("not found");
    });

    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      // Port 0 asks the OS for a free ephemeral port, so the fixture can never
      // collide with a server someone else is running on this machine.
      server.listen(0, "127.0.0.1", () => resolve());
    });
    this.server = server;
    const { port } = server.address() as AddressInfo;
    this.origin = `http://127.0.0.1:${port}`;
  }

  async stop(): Promise<void> {
    const server = this.server;
    this.server = null;
    if (!server) return;
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }

  /** The URL a tester types into the Import page's **Manifest URL** field. */
  get manifestUrl(): string {
    return `${this.origin}/manifest.json`;
  }

  get ndjsonUrl(): string {
    return `${this.origin}/patients.ndjson`;
  }

  /** Bytes the NDJSON file weighs — the denominator of the byte percentage. */
  get byteLength(): number {
    return this.body.length;
  }

  private manifest(): unknown {
    return {
      transactionTime: new Date().toISOString(),
      request: this.manifestUrl,
      requiresAccessToken: false,
      output: [{ type: "Patient", url: this.ndjsonUrl }],
      error: [],
    };
  }

  /** Writes the body in timed slices, respecting backpressure. */
  private async trickle(res: NodeJS.WritableStream & { end: () => void }): Promise<void> {
    const bytesPerChunk = Math.max(
      1,
      Math.ceil(this.body.length / Math.ceil(this.options.resources / this.options.chunkLines)),
    );
    for (let at = 0; at < this.body.length; at += bytesPerChunk) {
      const flushed = res.write(this.body.subarray(at, at + bytesPerChunk));
      if (!flushed) {
        await new Promise<void>((resolve) => res.once("drain", () => resolve()));
      }
      await new Promise((resolve) => setTimeout(resolve, this.options.chunkDelayMs));
    }
    res.end();
  }
}
