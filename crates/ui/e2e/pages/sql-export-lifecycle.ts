// Hold only the backend status protocol, never the UI HTML: real HFS renders
// every initial page and every htmx outerHTML fragment used by these tests.
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { createServer, request as httpRequest, type Server } from "node:http";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { expect, type Browser, type BrowserContext } from "@playwright/test";
import { createResource, createSqlQueryLibrary, waitSearchable } from "./api";
import { SqlExportPage } from "./sql-export";

function localBinary(): string | undefined {
  const root = resolve(__dirname, "../../../..");
  const exe = process.platform === "win32" ? "hfs.exe" : "hfs";
  return ["release", "debug"].map((profile) => join(root, "target", profile, exe))
    .filter(existsSync).sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
}

export class SqlExportLifecycle {
  static get available(): boolean { return Boolean(localBinary()); }

  private readonly released = new Map<string, "real" | "missing">();
  private child?: ChildProcess;
  private proxy?: Server;
  private directory?: string;
  private output = "";
  private spawnError?: Error;
  private context?: BrowserContext;
  sqlExport!: SqlExportPage;
  subject!: string;
  brokenSubject!: string;

  async start(browser: Browser, options: { noClipboard?: boolean; noJS?: boolean } = {}): Promise<void> {
    try {
      const root = resolve(__dirname, "../../../..");
      const binary = localBinary();
      if (!binary) throw new Error("Build hfs with --features ui before running SQL export lifecycle tests");
      this.directory = mkdtempSync(join(existsSync("/dev/shm") ? "/dev/shm" : tmpdir(), "hfs-sql-export-"));
      let port = 0;
      this.proxy = createServer((incoming, outgoing) => {
        const status = incoming.url?.match(/^\/export\/([^/]+)\/status(?:\?|$)/);
        if (incoming.method === "GET" && status && this.released.get(status[1]) !== "real") {
          const missing = this.released.get(status[1]) === "missing";
          outgoing.writeHead(missing ? 404 : 202, { "Content-Type": "application/fhir+json", "Retry-After": "1" });
          outgoing.end(missing ? JSON.stringify({ resourceType: "OperationOutcome", issue: [{ severity: "error", code: "not-found" }] }) : "");
          return;
        }
        const upstream = httpRequest({ hostname: "::1", port, method: incoming.method, path: incoming.url,
          headers: incoming.headers }, (response) => {
          outgoing.writeHead(response.statusCode ?? 502, response.headers);
          response.pipe(outgoing);
        });
        upstream.on("error", () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
        outgoing.on("close", () => upstream.destroy());
        incoming.pipe(upstream);
      });
      await new Promise<void>((resolve, reject) => {
        this.proxy!.once("error", reject);
        this.proxy!.listen(0, "127.0.0.1", resolve);
      });
      port = (this.proxy.address() as { port: number }).port;
      const baseURL = `http://127.0.0.1:${port}`;
      // HFS's conformance client self-calls IPv4; the real listener uses IPv6
      // on the same port, so both browser requests and self-calls cross the gate.
      const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("HFS_")));
      this.child = spawn(binary, [], { cwd: this.directory, env: { ...inherited,
        HFS_SERVER_HOST: "::1", HFS_SERVER_PORT: String(port), HFS_BASE_URL: baseURL,
        HFS_STORAGE_BACKEND: "sqlite", HFS_DATABASE_URL: join(this.directory, "resources.db"),
        HFS_DATA_DIR: join(root, "data"), HFS_BULK_EXPORT_OUTPUT_DIR: join(this.directory, "outputs"),
        HFS_DEFAULT_FHIR_VERSION: "R4", HFS_AUTH_ENABLED: "false", HFS_LOG_LEVEL: "warn",
      }, stdio: ["ignore", "pipe", "pipe"] });
      const record = (chunk: Buffer) => { this.output = (this.output + chunk.toString()).slice(-16_000); };
      this.child.stdout!.on("data", record);
      this.child.stderr!.on("data", record);
      this.child.on("error", (error) => { this.spawnError = error; });
      this.context = await browser.newContext({ baseURL, javaScriptEnabled: !options.noJS,
        ...(options.noClipboard || options.noJS ? {} : { permissions: ["clipboard-read", "clipboard-write"] }) });
      await expect.poll(async () => {
        if (this.spawnError || this.child!.exitCode !== null || this.child!.signalCode !== null) {
          throw new Error(`HFS startup failed: ${this.spawnError ?? this.output}`);
        }
        try { return (await this.context.request.get("/ui", { timeout: 1_000 })).ok(); } catch { return false; }
      }, { timeout: 60_000, message: "isolated SQL export HFS becomes ready" }).toBe(true);
      if (options.noClipboard) await this.context.addInitScript(() => {
        Object.defineProperty(navigator, "clipboard", { configurable: true, value: undefined });
      });
      const api = this.context.request;
      const patient = await createResource(api, "Patient", { name: [{ family: "SqlExportLifecycle" }] });
      const canonical = "http://example.org/ViewDefinition/sql-export-lifecycle";
      const vd = await createResource(api, "ViewDefinition", { name: "sql_export_lifecycle", url: canonical,
        status: "active", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] });
      const broken = await createSqlQueryLibrary(api, "sql_export_lifecycle_broken", canonical, "SELECT no_such_column FROM v");
      await Promise.all([waitSearchable(api, "Patient", patient), waitSearchable(api, "ViewDefinition", vd), waitSearchable(api, "Library", broken)]);
      this.subject = `ViewDefinition/${vd}`;
      this.brokenSubject = `Library/${broken}`;
      this.sqlExport = new SqlExportPage(await this.context.newPage());
    } catch (error) {
      await this.stop();
      throw error;
    }
  }

  release(jobId: string, outcome: "real" | "missing" = "real"): void { this.released.set(jobId, outcome); }

  async stop(): Promise<void> {
    const { context, child, proxy, directory } = this;
    this.context = undefined;
    this.child = undefined;
    this.proxy = undefined;
    this.directory = undefined;
    try {
      if (context) await context.close();
    } finally {
      // A spawn error has no PID and never emits exit. Do not wait for it.
      if (child?.pid && child.exitCode === null && child.signalCode === null) {
        await new Promise<void>((resolve) => {
          const timer = setTimeout(() => { child.kill("SIGKILL"); }, 3_000);
          child.once("exit", () => { clearTimeout(timer); resolve(); });
          child.kill("SIGTERM");
        });
      }
      if (proxy) await new Promise<void>((resolve) => { proxy.close(() => resolve()); proxy.closeAllConnections(); });
      if (directory) rmSync(directory, { recursive: true, force: true });
    }
  }
}
