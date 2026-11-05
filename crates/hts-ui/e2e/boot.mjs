// Boots the `hts` binary for the Playwright suite. Locates the already-built
// binary (CI builds it in a prior step; locally, `cargo build -p helios-hts`),
// points it at a throwaway SQLite DB, sets `HTS_UI_ENABLED=1`, and serves
// /ui/hts. Playwright's webServer waits for the port, then tears this down.
//
// Compared to `crates/ui/e2e/boot.mjs`, this harness is intentionally simpler:
// HTS has no tenants, no auth surface, and no conformance seeding — the UI
// runs against the terminology backend in the same process (design doc §7
// degraded-state contract).

import { spawn } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findHtsBinary } from "./support/find-hts-binary.cjs";

let bin;
try {
  bin = findHtsBinary();
} catch (err) {
  console.error(err.message);
  process.exit(1);
}

const port = process.env.HTS_E2E_PORT || "8090";
const dbDir = existsSync("/dev/shm") ? "/dev/shm" : tmpdir();
const db = join(dbDir, `.hts-e2e-${port}.db`);
for (const suffix of ["", "-wal", "-shm"]) {
  try { rmSync(db + suffix, { force: true }); } catch {}
}

console.log(`[boot.mjs] spawning ${bin} on port ${port}, db=${db}`);
const child = spawn(bin, [], {
  stdio: "inherit",
  env: {
    ...process.env,
    HTS_SERVER_PORT: port,
    HTS_STORAGE_BACKEND: "sqlite",
    HTS_DATABASE_URL: db,
    HTS_LOG_LEVEL: "warn",
    HTS_UI_ENABLED: "true",
    // A tiny expansion ceiling so the value-sets spec's too-costly test
    // (ex-vs-too-costly seeded in seed.ts) reliably trips HTS's guard
    // and returns 422 with a `too-costly` OperationOutcome.
    HTS_MAX_EXPANSION_SIZE: "5",
  },
});
console.log(`[boot.mjs] child pid=${child.pid}`);

const bye = () => { try { child.kill(); } catch {} };
process.on("SIGTERM", bye);
process.on("SIGINT", bye);
process.on("exit", bye);
child.on("exit", (code, signal) => {
  console.log(`[boot.mjs] child exited code=${code} signal=${signal}`);
  process.exit(code ?? 0);
});
child.on("error", (err) => {
  console.error(`[boot.mjs] child error:`, err);
});
