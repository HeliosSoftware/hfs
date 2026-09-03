// Locates the already-built `hts` binary (CI builds it in a prior step;
// locally, `cargo build -p helios-hts`). Picks the most recently built one
// across the release/debug profiles rather than assuming a fixed profile.
// Shared by boot.mjs (the suite's single long-lived server) and any spec
// that needs its own dedicated `hts` process, such as bootstrap-import.spec.ts.
//
// CommonJS (.cjs) on purpose: Playwright transpiles spec imports to CJS, so
// an .mjs helper imported from a .ts spec dies with "exports is not defined
// in ES module scope". Node's ESM loader can named-import from CJS, so
// boot.mjs consumes this file too.

"use strict";

const { existsSync, statSync } = require("node:fs");
const { join } = require("node:path");

function findHtsBinary() {
  const root = join(__dirname, "..", "..", "..", ".."); // e2e/support -> repo root
  const exe = process.platform === "win32" ? "hts.exe" : "hts";
  const candidates = [
    join(root, "target", "release", exe),
    join(root, "target", "debug", exe),
  ];
  const bin = candidates
    .filter(existsSync)
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
  if (!bin) {
    throw new Error(
      "hts binary not found. Build it first:\n  cargo build -p helios-hts\nLooked in:\n  " +
        candidates.join("\n  "),
    );
  }
  return bin;
}

exports.findHtsBinary = findHtsBinary;
