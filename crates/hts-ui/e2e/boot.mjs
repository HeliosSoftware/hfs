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
import { existsSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", ".."); // crates/hts-ui/e2e -> repo root
const exe = process.platform === "win32" ? "hts.exe" : "hts";
const candidates = [
  join(root, "target", "release", exe),
  join(root, "target", "debug", exe),
];
// Pick the most recently built binary, not a fixed profile.
const bin = candidates
  .filter(existsSync)
  .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
if (!bin) {
  console.error(
    "hts binary not found. Build it first:\n  cargo build -p helios-hts\nLooked in:\n  " +
      candidates.join("\n  "),
  );
  process.exit(1);
}

const port = process.env.HTS_E2E_PORT || "8090";
const dbDir = existsSync("/dev/shm") ? "/dev/shm" : tmpdir();
const db = join(dbDir, `.hts-e2e-${port}.db`);
for (const suffix of ["", "-wal", "-shm"]) {
  try { rmSync(db + suffix, { force: true }); } catch {}
}

const child = spawn(bin, [], {
  stdio: "inherit",
  env: {
    ...process.env,
    HTS_SERVER_PORT: port,
    HTS_STORAGE_BACKEND: "sqlite",
    HTS_DATABASE_URL: db,
    HTS_LOG_LEVEL: "warn",
    HTS_UI_ENABLED: "true",
  },
});

const bye = () => { try { child.kill(); } catch {} };
process.on("SIGTERM", bye);
process.on("SIGINT", bye);
process.on("exit", bye);
child.on("exit", (code) => process.exit(code ?? 0));
