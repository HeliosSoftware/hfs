import { defineConfig, devices } from "@playwright/test";

// One hfs instance serves the whole run. Port is overridable so the suite can
// share a server a developer already has up (reuseExistingServer, below).
const PORT = Number(process.env.HFS_E2E_PORT || 8080);
// The backend matrix runs hfs on the host (against Postgres/Mongo/ES/S3) and
// points the browser at it via HFS_E2E_BASE_URL; when that's set we target the
// external server and skip booting our own (see webServer, below).
const externalBase = process.env.HFS_E2E_BASE_URL;
const baseURL = externalBase || `http://127.0.0.1:${PORT}`;
// Chromium withholds secure-context APIs such as navigator.clipboard from the
// backend matrix's plain-HTTP Docker-host IP. The suite grants clipboard
// permissions in the relevant test; this flag makes that grant effective for
// the one external test origin without weakening HFS or skipping coverage.
const insecureExternalOrigin =
  externalBase && new URL(externalBase).protocol === "http:"
    ? new URL(externalBase).origin
    : undefined;

export default defineConfig({
  testDir: "./tests",
  // Deterministic against a single shared server: no cross-test DB races.
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI
    ? [["github"], ["html", { open: "never" }], ["list"]]
    : [["list"]],
  use: {
    baseURL,
    trace: "on-first-retry",
    ...(insecureExternalOrigin
      ? {
          // The legacy headless shell ignores Chromium's secure-origin
          // allowlist; the full Chromium channel honors it in headless mode.
          channel: "chromium",
          launchOptions: {
            args: [`--unsafely-treat-insecure-origin-as-secure=${insecureExternalOrigin}`],
          },
        }
      : {}),
  },
  projects: [
    {
      // The JS-enabled ring: theme behavior, axe-core a11y, no-CDN invariants.
      name: "chromium",
      testIgnore: ["**/nojs/**", "**/auth/**"],
      use: { ...devices["Desktop Chrome"] },
    },
    {
      // The README's core promise: the UI works with JavaScript disabled.
      name: "nojs",
      testMatch: "**/nojs/**/*.spec.ts",
      use: { ...devices["Desktop Chrome"], javaScriptEnabled: false },
    },
    // The Keycloak smoke (#444): an external server whose auth is backed by a
    // real IdP runs the same conformance assertions as the local auth leg —
    // the spec is identical, only who minted the token differs. Opted into
    // explicitly so ordinary external runs (the backend matrix) skip it.
    ...(externalBase && process.env.HFS_E2E_AUTH_SMOKE === "1"
      ? [
          {
            name: "auth-external",
            testMatch: "**/auth/conformance.spec.ts",
            use: { ...devices["Desktop Chrome"] },
          },
        ]
      : []),
    // The auth legs (#320) drive their own servers (see webServer below), so
    // they only exist when this run boots its servers itself.
    ...(externalBase
      ? []
      : [
          {
            // Auth enabled + outbound service token: conformance self-fetch works.
            name: "auth",
            testMatch: "**/auth/conformance.spec.ts",
            use: { ...devices["Desktop Chrome"], baseURL: `http://127.0.0.1:${PORT + 10}` },
          },
          {
            // Auth enabled, no outbound token: the pages degrade, never 404.
            name: "auth-degraded",
            testMatch: "**/auth/degraded.spec.ts",
            use: { ...devices["Desktop Chrome"], baseURL: `http://127.0.0.1:${PORT + 20}` },
          },
        ]),
  ],
  // Boot our own hfs only when no external server was handed to us. The backend
  // matrix launches hfs on the host and sets HFS_E2E_BASE_URL, so there we skip
  // this entirely and drive the already-running server.
  ...(externalBase
    ? {}
    : {
        webServer: [
          {
            command: "node boot.mjs",
            // Readiness probe: the FHIR root does not 200, but /ui does.
            url: `${baseURL}/ui`,
            reuseExistingServer: !process.env.CI,
            timeout: 120_000,
            stdout: "pipe",
            stderr: "pipe",
          },
          // The auth legs (#320): same binary, bearer auth enabled against a
          // throwaway JWKS boot.mjs serves on the port next to the server's.
          {
            command: "node boot.mjs",
            url: `http://127.0.0.1:${PORT + 10}/ui`,
            env: { HFS_E2E_PORT: String(PORT + 10), HFS_E2E_AUTH: "token" },
            reuseExistingServer: !process.env.CI,
            timeout: 120_000,
            stdout: "pipe",
            stderr: "pipe",
          },
          {
            command: "node boot.mjs",
            url: `http://127.0.0.1:${PORT + 20}/ui`,
            env: { HFS_E2E_PORT: String(PORT + 20), HFS_E2E_AUTH: "degraded" },
            reuseExistingServer: !process.env.CI,
            timeout: 120_000,
            stdout: "pipe",
            stderr: "pipe",
          },
        ],
      }),
});
