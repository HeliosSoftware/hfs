import { test, expect } from "../pages/fixtures";

// T3 (#600): the dashboard's resource-type selector against a real
// multi-version build (`cargo build -p helios-hfs --no-default-features
// --features R4,R4B,R5,R6,ui,sqlite`). Every other spec in this suite runs
// against the default R4-only binary — see crates/ui/src/search_params.rs
// (`enabled_versions()`) — where the sidebar's FHIR-version disclosure has a
// single, un-switchable option. These specs need at least a second compiled
// version to mean anything, so they skip (not fail) when they don't find one:
// the default local/PR run stays green, and CI's dedicated multi-version job
// (.github/workflows/ui-tests.yml) is what actually exercises them.
//
// Nothing here is hardcoded against a specific type name: R4-vs-R5 membership
// and counts are computed from what the running server itself already
// exposes (the Resources rail and the dashboard's own "View all resources"
// union both read `resource_type_names()` — crates/ui/src/lib.rs), so a spec
// bundle refresh that changes which types exist cannot silently desync the
// assertions from reality.
//
// No boot.mjs / playwright.config.ts change was needed: boot.mjs already
// forwards its whole environment to the spawned `hfs` process, so CI's
// dedicated job pins the seed away from the build's natural first-enabled
// version by exporting `HFS_DEFAULT_FHIR_VERSION` before `npx playwright
// test` — reproduce locally the same way (PowerShell:
// `$env:HFS_DEFAULT_FHIR_VERSION="R6"`) against a multi-version binary.

test.beforeEach(async ({ page, chrome }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  const options = await chrome.versionOptions();
  test.skip(
    options.length < 2,
    "single-version binary: nothing to switch (needs --features R4,R4B,R5,R6)",
  );
});

test("the sidebar lists every compiled FHIR version", async ({ chrome }) => {
  const options = await chrome.versionOptions();
  expect(new Set(options)).toEqual(new Set(["R4", "R4B", "R5", "R6"]));
});

test("switching version changes the dashboard's full type set (membership and counts)", async ({
  page,
  dashboard,
  resources,
  chrome,
}) => {
  // The Resources rail already exposes the complete per-version type list
  // (`resource_type_names()`), independent of the dashboard code path —
  // reading it for R4 and R5 gives ground truth to check the dashboard
  // picker against, not a value baked into the test.
  await resources.goto();
  const r4Types = new Set(await resources.railTypes());

  await chrome.selectVersion("R5");
  await resources.goto();
  const r5Types = new Set(await resources.railTypes());

  const onlyInR5 = [...r5Types].filter((t) => !r4Types.has(t));
  const onlyInR4 = [...r4Types].filter((t) => !r5Types.has(t));
  expect(onlyInR5.length, "R5 should introduce at least one type R4 doesn't have").toBeGreaterThan(0);
  expect(onlyInR4.length, "R4 should have at least one type R5 dropped").toBeGreaterThan(0);

  // The active version is still R5 (persisted by the sidebar's POST). The
  // dashboard's "View all resources" union (#599) must follow it: an R5-only
  // type is offered, an R4-only type is not.
  await dashboard.goto("?all=1");
  await dashboard.openPicker();
  await expect(dashboard.pickerOption(onlyInR5[0])).toBeVisible();
  await expect(dashboard.pickerOption(onlyInR4[0])).toHaveCount(0);

  // Counts: the picker's full-set option count matches the rail's count for
  // the same (R5) version — both read the same source, so they must agree.
  const pickerCount = await page.locator("[data-pick-name]").count();
  expect(pickerCount).toBe(r5Types.size);

  // Switching back to R4 flips the membership the other way — proves this
  // isn't a one-shot union that keeps growing.
  await chrome.selectVersion("R4");
  await dashboard.goto("?all=1");
  await dashboard.openPicker();
  await expect(dashboard.pickerOption(onlyInR4[0])).toBeVisible();
  await expect(dashboard.pickerOption(onlyInR5[0])).toHaveCount(0);
});

test("the picker follows the requested version, not the server's boot-time default", async ({
  dashboard,
  chrome,
}) => {
  // Whatever this server booted with (HFS_DEFAULT_FHIR_VERSION — CI's
  // dedicated job pins this to a version other than the one this test
  // requests, precisely to catch an "always the seed" regression), switching
  // to a *different* compiled version must change the picker's full set.
  const options = await chrome.versionOptions();
  const seed = await chrome.currentVersion();
  const requested = options.find((v) => v !== seed);
  if (!requested) test.skip(true, "no second compiled version to request instead of the seed");

  await dashboard.goto("?all=1");
  await dashboard.openPicker();
  const beforeCount = await dashboard.page.locator("[data-pick-name]").count();

  await chrome.selectVersion(requested!);
  expect(await chrome.currentVersion()).toBe(requested);

  await dashboard.goto("?all=1");
  await dashboard.openPicker();
  const afterCount = await dashboard.page.locator("[data-pick-name]").count();

  // Every FHIR version this suite compiles in has a different resource-type
  // count, so a picker that silently kept rendering the seed's set — instead
  // of the version just requested — would leave this count unchanged.
  expect(afterCount).not.toBe(beforeCount);
});
