import { spawn, type ChildProcess } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { findHtsBinary } from "../support/find-hts-binary.cjs";

// Issue #802: the ICD-9-CM bootstrap importer previously required a pipe
// delimiter the real CMS distribution never has, so every code silently
// failed to parse and the bootstrap ledger recorded a false "success".
//
// No other spec in this suite exercises HTS_BOOTSTRAP_DIR or the real
// bundled terminology-data zips — boot.mjs deliberately starts hts against
// an empty database and seed.ts populates it via a synthetic POST /import
// Bundle. Several specs (code-systems.spec.ts) assert on exact row and
// pagination counts from that fixed seed set, so bootstrapping ~18k real
// ICD-9-CM concepts into the *shared* server would risk destabilizing them.
//
// This spec instead runs its own dedicated hts process, on its own port,
// against a directory containing only the real bundled ICD-9-CM zip (not
// the other ~16 large terminology-data files) — real end-to-end data, but
// scoped small enough to boot quickly and not touch the shared fixtures.

const PORT = process.env.HTS_E2E_BOOTSTRAP_PORT || "8095";
const BASE_URL = `http://127.0.0.1:${PORT}`;

let child: ChildProcess | undefined;
let tmpDir: string | undefined;
let tmpDb: string | undefined;

async function waitForReady(timeoutMs = 60_000) {
  const start = Date.now();
  let lastErr: unknown;
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`${BASE_URL}/ui/hts`);
      if (res.ok) return;
      lastErr = new Error(`readiness probe responded ${res.status}`);
    } catch (err) {
      lastErr = err;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(
    `hts did not become ready at ${BASE_URL} within ${timeoutMs}ms: ${
      lastErr instanceof Error ? lastErr.message : String(lastErr)
    }`,
  );
}

test.describe("ICD-9-CM bootstrap import (issue #802)", () => {
  test.beforeAll(async () => {
    const bin = findHtsBinary();

    // A bootstrap dir containing ONLY the real ICD-9-CM zip — not the other
    // large terminology-data files, so this boots in seconds, not minutes.
    tmpDir = mkdtempSync(join(tmpdir(), "hts-icd9-bootstrap-"));
    const repoRoot = join(__dirname, "..", "..", "..", "..");
    copyFileSync(
      join(repoRoot, "crates", "hts", "terminology-data", "ICD-9-CM-v32-master-descriptions.zip"),
      join(tmpDir, "ICD-9-CM-v32-master-descriptions.zip"),
    );

    tmpDb = join(tmpDir, "hts-bootstrap-e2e.db");

    child = spawn(bin, [], {
      stdio: "inherit",
      env: {
        ...process.env,
        HTS_SERVER_PORT: PORT,
        HTS_STORAGE_BACKEND: "sqlite",
        HTS_DATABASE_URL: tmpDb,
        HTS_BOOTSTRAP_DIR: tmpDir,
        HTS_LOG_LEVEL: "warn",
        HTS_UI_ENABLED: "true",
      },
    });

    await waitForReady();
  });

  test.afterAll(() => {
    try {
      child?.kill();
    } catch {
      // already gone
    }
    if (tmpDir) {
      try {
        rmSync(tmpDir, { recursive: true, force: true });
      } catch {
        // best-effort cleanup
      }
    }
  });

  test("real bootstrap import resolves a diagnosis code via the Lookup workbench", async ({
    page,
  }) => {
    await page.goto(`${BASE_URL}/ui/hts/code-systems/icd9cm/lookup`);
    await page.getByLabel("Code", { exact: true }).fill("250.00");
    await page.getByRole("button", { name: "Run", exact: true }).click();

    const result = page.locator("#hts-workbench-result");
    // Ticket's own acceptance example.
    await expect(result).toContainText(
      "Diabetes mellitus without mention of complication, type II or unspecified type, not stated as uncontrolled",
      { timeout: 10_000 },
    );
  });

  test("real bootstrap import resolves a procedure code (DX+SG merge)", async ({ page }) => {
    await page.goto(`${BASE_URL}/ui/hts/code-systems/icd9cm/lookup`);
    // 00.01: "Therapeutic ultrasound of vessels of head and neck" — the
    // first CMS32_DESC_LONG_SG.txt line, proving the procedure file (not
    // just diagnoses) actually imported.
    await page.getByLabel("Code", { exact: true }).fill("00.01");
    await page.getByRole("button", { name: "Run", exact: true }).click();

    const result = page.locator("#hts-workbench-result");
    await expect(result).toContainText("Therapeutic ultrasound of vessels of head and neck", {
      timeout: 10_000,
    });
  });
});
