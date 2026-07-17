// Boots `hfs --features ui` for the Playwright suite. Locates the already-built
// binary (CI builds it in a prior step; locally, `cargo build -p helios-hfs
// --features ui`), points it at a throwaway SQLite DB, and serves /ui. Playwright's
// webServer waits for the port, then tears this down.
import { spawn } from "node:child_process";
import { existsSync, rmSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", ".."); // crates/ui/e2e -> repo root
const exe = process.platform === "win32" ? "hfs.exe" : "hfs";
const candidates = [
  join(root, "target", "release", exe),
  join(root, "target", "debug", exe),
];
// Pick the most recently built binary, not a fixed profile: a stale release
// (or debug) left over from earlier work must not shadow the fresh one.
const bin = candidates
  .filter(existsSync)
  .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
if (!bin) {
  console.error(
    "hfs binary not found. Build it first:\n  cargo build -p helios-hfs --features ui\nLooked in:\n  " +
      candidates.join("\n  "),
  );
  process.exit(1);
}

const db = join(here, ".hfs-e2e.db");
for (const suffix of ["", "-wal", "-shm"]) {
  try { rmSync(db + suffix, { force: true }); } catch {}
}

const child = spawn(bin, [], {
  stdio: "inherit",
  env: {
    ...process.env,
    HFS_SERVER_PORT: process.env.HFS_E2E_PORT || "8080",
    HFS_STORAGE_BACKEND: "sqlite",
    HFS_DATABASE_URL: db,
    // Load the vendored SearchParameter specs (so search works like production),
    // not the minimal fallback the server drops to when ./data isn't found.
    HFS_DATA_DIR: join(root, "data"),
    HFS_LOG_LEVEL: "warn",
    // Natural-language search advertises itself as configured so the search
    // area renders its working pane. The key is never used by the tests.
    HFS_NL_SEARCH_API_KEY: "e2e-placeholder-not-a-real-key",
  },
});

const bye = () => { try { child.kill(); } catch {} };
process.on("SIGTERM", bye);
process.on("SIGINT", bye);
process.on("exit", bye);
child.on("exit", (code) => process.exit(code ?? 0));
