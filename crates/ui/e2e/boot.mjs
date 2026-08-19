// Boots `hfs --features ui` for the Playwright suite. Locates the already-built
// binary (CI builds it in a prior step; locally, `cargo build -p helios-hfs
// --features ui`), points it at a throwaway SQLite DB, and serves /ui. Playwright's
// webServer waits for the port, then tears this down.
import { spawn } from "node:child_process";
import { generateKeyPairSync, sign } from "node:crypto";
import { existsSync, rmSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
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

// Keep the DB on tmpfs when available: startup and per-tenant conformance
// seeding write ~1.4k resources one insert (and one fsync) at a time, which
// on the container's overlayfs turns tenant creation into a minutes-long
// request. /dev/shm makes those fsyncs free without changing what is tested.
const port = process.env.HFS_E2E_PORT || "8080";
const dbDir = existsSync("/dev/shm") ? "/dev/shm" : tmpdir();
// Per-port DB: the auth legs (#320) run their own servers next to this one.
const db = join(dbDir, `.hfs-e2e-${port}.db`);
for (const suffix of ["", "-wal", "-shm"]) {
  try { rmSync(db + suffix, { force: true }); } catch {}
}

/*
 * Auth legs (#320). HFS_E2E_AUTH turns bearer authentication on, backed by a
 * throwaway RSA key served as a JWKS from a local HTTP server — the whole IdP
 * surface the server needs for validation, with no external dependency:
 *
 *  - "token":    HFS_OUTBOUND_BEARER_TOKEN carries a minted service token with
 *                the conformance read scopes, so the UI's self-fetch works.
 *  - "degraded": auth is on but no outbound token is provisioned — the
 *                self-fetch is rejected and the pages must degrade, not 404.
 *
 * The token carries no tenant claim on purpose: the claim is authoritative
 * when present, and the self-fetch scopes each call with X-Tenant-ID instead.
 */
const AUTH_MODE = process.env.HFS_E2E_AUTH || "";
const ISSUER = "https://e2e.hfs.invalid/issuer";
const AUDIENCE = "hfs-e2e";
let authEnv = {};
if (AUTH_MODE) {
  const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const kid = "hfs-e2e-key";
  const jwks = JSON.stringify({
    keys: [{ ...publicKey.export({ format: "jwk" }), kid, alg: "RS256", use: "sig" }],
  });
  const jwksPort = Number(port) + 1;
  createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(jwks);
  }).listen(jwksPort, "127.0.0.1");

  const b64url = (buf) => Buffer.from(buf).toString("base64url");
  const mint = (claims) => {
    const header = b64url(JSON.stringify({ alg: "RS256", typ: "JWT", kid }));
    const now = Math.floor(Date.now() / 1000);
    const payload = b64url(
      JSON.stringify({ iss: ISSUER, aud: AUDIENCE, iat: now, exp: now + 86400, ...claims }),
    );
    const signature = b64url(sign("sha256", Buffer.from(`${header}.${payload}`), privateKey));
    return `${header}.${payload}.${signature}`;
  };

  authEnv = {
    HFS_AUTH_ENABLED: "true",
    HFS_AUTH_JWKS_URL: `http://127.0.0.1:${jwksPort}/jwks.json`,
    HFS_AUTH_ISSUER: ISSUER,
    HFS_AUTH_AUDIENCE: AUDIENCE,
  };
  if (AUTH_MODE === "token") {
    authEnv.HFS_OUTBOUND_BEARER_TOKEN = mint({
      sub: "hfs-ui-self",
      scope: "system/SearchParameter.rs system/CompartmentDefinition.rs",
    });
  }
}

const child = spawn(bin, [], {
  stdio: "inherit",
  env: {
    ...process.env,
    HFS_SERVER_PORT: port,
    HFS_STORAGE_BACKEND: "sqlite",
    HFS_DATABASE_URL: db,
    // Load the vendored SearchParameter specs (so search works like production),
    // not the minimal fallback the server drops to when ./data isn't found.
    HFS_DATA_DIR: join(root, "data"),
    HFS_LOG_LEVEL: "warn",
    // Natural-language search advertises itself as configured so the search
    // area renders its working pane. The key is never used by the tests.
    HFS_NL_SEARCH_API_KEY: "e2e-placeholder-not-a-real-key",
    // The subscriptions engine advertises itself so the operator page (#580)
    // renders its live (empty) dashboard instead of the unavailable state.
    HFS_SUBSCRIPTIONS_ENABLED: "true",
    ...authEnv,
  },
});

const bye = () => { try { child.kill(); } catch {} };
process.on("SIGTERM", bye);
process.on("SIGINT", bye);
process.on("exit", bye);
child.on("exit", (code) => process.exit(code ?? 0));
