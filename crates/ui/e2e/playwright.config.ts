import { defineConfig, devices } from "@playwright/test";

// One hfs instance serves the whole run. Port is overridable so the suite can
// share a server a developer already has up (reuseExistingServer, below).
const PORT = Number(process.env.HFS_E2E_PORT || 8080);
// The backend matrix runs hfs on the host (against Postgres/Mongo/ES/S3) and
// points the browser at it via HFS_E2E_BASE_URL; when that's set we target the
// external server and skip booting our own (see webServer, below).
const externalBase = process.env.HFS_E2E_BASE_URL;
const baseURL = externalBase || `http://127.0.0.1:${PORT}`;

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
  },
  projects: [
    {
      // The JS-enabled ring: theme behavior, axe-core a11y, no-CDN invariants.
      name: "chromium",
      testIgnore: "**/nojs/**",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      // The README's core promise: the UI works with JavaScript disabled.
      name: "nojs",
      testMatch: "**/nojs/**/*.spec.ts",
      use: { ...devices["Desktop Chrome"], javaScriptEnabled: false },
    },
  ],
  // Boot our own hfs only when no external server was handed to us. The backend
  // matrix launches hfs on the host and sets HFS_E2E_BASE_URL, so there we skip
  // this entirely and drive the already-running server.
  ...(externalBase
    ? {}
    : {
        webServer: {
          command: "node boot.mjs",
          // Readiness probe: the FHIR root does not 200, but /ui does.
          url: `${baseURL}/ui`,
          reuseExistingServer: !process.env.CI,
          timeout: 120_000,
          stdout: "pipe",
          stderr: "pipe",
        },
      }),
});
