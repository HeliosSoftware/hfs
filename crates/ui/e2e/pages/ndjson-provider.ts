// A throttled Data Provider for the Bulk Import specs (#968).
//
// The Abort button can only be tested against an ingest that is still running
// when the button is pressed. An ordinary static file is useless here: HFS
// swallows a few MB of NDJSON in well under a second, so the submission is
// always already `completed` by the time a browser could click anything, and
// the test would pass whether or not cancellation works.
//
// So this server *drips*: it writes one chunk per `pauseMs`, respecting
// backpressure, which makes the ingest last long enough to be interrupted and
// — more usefully — makes "did HFS stop reading?" directly observable. When
// cancellation works the recipient stops pulling and closes the socket, so
// `bytesServed` freezes partway through and `hungUp` flips. Those two are the
// server-side truth behind the UI assertions.
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

export type ProviderOptions = {
  /** How many Patient lines the NDJSON file holds. */
  lines?: number;
  /** Bytes written per tick. */
  chunkBytes?: number;
  /** Delay between ticks, in milliseconds. */
  pauseMs?: number;
  /** Prefix for the generated Patient ids, so specs can probe for them. */
  idPrefix?: string;
};

/** One download of the NDJSON file, as the provider saw it. */
export type Stream = {
  /**
   * Bytes handed to the socket — not bytes the recipient parsed.
   *
   * Once the recipient stops reading, the kernel and Node still accept about a
   * megabyte before `write()` reports backpressure, so this keeps climbing for
   * a few seconds after a cancellation and only then freezes. `hungUp` is the
   * prompt signal; this is the ceiling.
   */
  served: number;
  /** The recipient closed the connection before the file ran out. */
  hungUp: boolean;
  /** The whole file went out. */
  complete: boolean;
};

export class NdjsonProvider {
  private server?: Server;
  private port = 0;
  /**
   * Every download of the NDJSON file, in order. A retry is a second entry —
   * which is why this is a list and not a running total: a counter shared
   * across connections silently adds a re-fetch to the first attempt.
   */
  readonly streams: Stream[] = [];
  /** Every request path the recipient asked for, in order. */
  readonly requests: string[] = [];

  /** Bytes handed over on the most recent download. */
  get bytesServed(): number {
    return this.streams.at(-1)?.served ?? 0;
  }

  /** Whether the recipient walked away from the most recent download. */
  get hungUp(): boolean {
    return this.streams.at(-1)?.hungUp ?? false;
  }

  private readonly body: Buffer;
  private readonly chunkBytes: number;
  private readonly pauseMs: number;

  constructor(private readonly options: ProviderOptions = {}) {
    const lines = options.lines ?? 40_000;
    const idPrefix = options.idPrefix ?? "e2e968";
    this.chunkBytes = options.chunkBytes ?? 32 * 1024;
    this.pauseMs = options.pauseMs ?? 250;
    this.body = Buffer.from(
      Array.from(
        { length: lines },
        (_, i) =>
          `{"resourceType":"Patient","id":"${idPrefix}-${String(i).padStart(6, "0")}"}\n`,
      ).join(""),
    );
  }

  /** The id of the nth line, matching what `patients.ndjson` carries. */
  idAt(index: number): string {
    return `${this.options.idPrefix ?? "e2e968"}-${String(index).padStart(6, "0")}`;
  }

  get totalBytes(): number {
    return this.body.length;
  }

  get manifestUrl(): string {
    return `http://127.0.0.1:${this.port}/manifest.json`;
  }

  async start(): Promise<void> {
    this.server = createServer((req, res) => {
      const path = (req.url ?? "/").split("?")[0];
      this.requests.push(`${req.method} ${path}`);
      if (path === "/manifest.json") {
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            transactionTime: new Date().toISOString(),
            request: this.manifestUrl,
            requiresAccessToken: false,
            output: [
              {
                type: "Patient",
                url: `http://127.0.0.1:${this.port}/patients.ndjson`,
              },
            ],
            error: [],
          }),
        );
        return;
      }
      if (path === "/patients.ndjson") {
        // The recipient probes each file with HEAD before ingesting it, to
        // presize its progress bar (#874). It wants the Content-Length header
        // and nothing else — answer it as a header-only request, or the drip
        // below runs for a body Node is going to discard anyway.
        if (req.method === "HEAD") {
          res.writeHead(200, {
            "Content-Type": "application/fhir+ndjson",
            "Content-Length": String(this.body.length),
          });
          res.end();
          return;
        }
        // No Content-Length on the real download: this is a streamed body, and
        // the point is that the recipient may walk away from it half-read.
        res.writeHead(200, { "Content-Type": "application/fhir+ndjson" });
        void this.drip(res);
        return;
      }
      res.writeHead(404).end();
    });
    await new Promise<void>((resolve) => {
      this.server!.listen(0, "127.0.0.1", resolve);
    });
    this.port = (this.server!.address() as AddressInfo).port;
  }

  private async drip(res: import("node:http").ServerResponse): Promise<void> {
    const stream: Stream = { served: 0, hungUp: false, complete: false };
    this.streams.push(stream);

    let closed = false;
    // The recipient walking away is the event the specs care about, so record
    // it the instant it happens rather than whenever the loop below next looks
    // — by then the loop may be parked waiting for a drain that never comes.
    const giveUp = () => {
      closed = true;
      if (!stream.complete) stream.hungUp = true;
    };
    res.on("close", giveUp);
    res.on("error", giveUp);

    for (let at = 0; at < this.body.length; at += this.chunkBytes) {
      if (closed) return;
      const chunk = this.body.subarray(at, at + this.chunkBytes);
      const flushed = res.write(chunk);
      if (!flushed) {
        // The recipient is not keeping up (or has stopped reading): wait for
        // the socket to drain rather than piling the rest of the file up in
        // Node, which would let `served` run away from reality entirely.
        await new Promise<void>((resolve) => {
          const settle = () => {
            res.off("drain", settle);
            res.off("close", settle);
            res.off("error", settle);
            resolve();
          };
          res.on("drain", settle);
          res.on("close", settle);
          res.on("error", settle);
        });
        if (closed) return;
      }
      stream.served += chunk.length;
      await new Promise((resolve) => setTimeout(resolve, this.pauseMs));
    }
    stream.complete = true;
    if (!closed) res.end();
  }

  async stop(): Promise<void> {
    if (!this.server) return;
    const server = this.server;
    this.server = undefined;
    server.closeAllConnections?.();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
}
