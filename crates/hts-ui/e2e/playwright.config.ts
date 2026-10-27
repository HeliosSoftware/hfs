import { defineConfig, devices } from "@playwright/test";

// One hts instance serves the whole run. Port is overridable so the suite can
// share a server a developer already has up (reuseExistingServer, below).
const PORT = Number(process.env.HTS_E2E_PORT || 8090);
// The backend matrix runs hts on the host (against Postgres) and points the
// browser at it via HTS_E2E_BASE_URL; when that's set we target the external
// server and skip booting our own.
const externalBase = process.env.HTS_E2E_BASE_URL;
const baseURL = externalBase || `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./tests",
  // Runs after webServer readiness and before the first test. Seeds the hts
  // SQLite via POST /import with the fixture roster the specs assume
  // (ex-cs-1, ex-vs-1, ex-cm-1, plus fillers). See seed.ts for the details.
  globalSetup: "./seed.ts",
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
      testIgnore: ["**/nojs/**"],
      use: { ...devices["Desktop Chrome"] },
    },
    {
      // The design's core promise: the UI works with JavaScript disabled.
      // Every control is a real <a> or <form> first; htmx only upgrades it.
      name: "nojs",
      testMatch: "**/nojs/**/*.spec.ts",
      use: { ...devices["Desktop Chrome"], javaScriptEnabled: false },
    },
  ],
  // Boot our own hts only when no external server was handed to us.
  ...(externalBase
    ? {}
    : {
        webServer: {
          command: "node boot.mjs",
          // Readiness probe: /ui/hts is the dashboard route we own.
          url: `${baseURL}/ui/hts`,
          reuseExistingServer: !process.env.CI,
          timeout: 120_000,
          stdout: "pipe",
          stderr: "pipe",
        },
      }),
});
