// Active SQL Exports (#833): the list-first workspace for `$sql-export` jobs.
// Runs against the sqlite server the suite boots — a real `$sql-export`
// kick-off, not a stub — so a job genuinely transitions through the states
// the card renders. Tests run in declaration order against one shared server
// (playwright.config.ts: fullyParallel: false, workers: 1); this file's own
// `afterEach` (below) restores both kinds of state it leaves on that shared
// server, so a rerun sees the same empty baseline as the very first run.
import { expect, test } from "../pages/fixtures";
import { createResource, createResources, deleteResources, waitSearchable } from "../pages/api";
import { expectDetailFieldSpacing } from "../pages/detail-spacing";

// The card's own htmx fragment polls every 5s; generous headroom for a job to
// finish without ever sleeping blindly (RNF3).
const POLL_TIMEOUT = 30_000;

// Every `ViewDefinition` a test below seeds gets its id pushed here, then
// deleted in this file's own `afterEach` (below). A `ViewDefinition` is a
// real, tenant-visible resource that `/ui/sql/view-definitions` lists with
// no filter of its own; left behind, it becomes that page's default
// selection and mounts its CodeMirror editor, which then fails
// `design-system.spec.ts`'s "every class used" sweep on whatever run
// happens to follow this one against the same shared server.
let seededViewDefinitionIds: string[] = [];

test.afterEach(async ({ request }) => {
  const ids = seededViewDefinitionIds;
  seededViewDefinitionIds = [];
  await deleteResources(request, "ViewDefinition", ids);

  // The jobs these tests start live in the per-user settings document under
  // `byTenant.<tenant>.sqlExport.jobs` (crates/ui/src/sql_export.rs); the
  // generic `/_user/settings` endpoint projects tenant-scoped keys flat for
  // the caller's own tenant (crates/rest/src/handlers/user_settings.rs), so
  // an RFC 7386 `{"sqlExport": null}` merge-patch — the same shape
  // `theme.spec.ts` uses for `theme` — deletes this tenant's whole job store
  // in one call. Left behind, "an empty list…" (the only test that asserts
  // a genuinely empty list) fails on whatever run follows this one against
  // the same reused local dev server.
  await request.patch("/_user/settings", {
    headers: { "Content-Type": "application/json" },
    data: { sqlExport: null },
  });
});

/**
 * A `$sql-export` job over a single tiny ViewDefinition finishes in well
 * under 100ms, faster than the redirect that lands on the list even renders
 * — so there is no reliable way to observe it `in-progress` there. Padding
 * the job with this many trivial subjects (a single self-search round trip
 * apiece) buys a window measured in hundreds of milliseconds, still far
 * short of the card's first 5s htmx poll, without ever waiting on a fixed
 * clock (RNF3): every assertion below still polls actual DOM state — this
 * constant only makes that state observable at all.
 */
const PADDING_SUBJECTS = 200;

test.describe.serial("Active SQL Exports", () => {
  test("an empty list shows the empty notice and the New SQL Export button", async ({
    sqlExport,
  }) => {
    await sqlExport.goto();
    await expect(sqlExport.notice).toContainText("No SQL exports yet");
    await expect(sqlExport.newButton).toBeVisible();
    await expect(sqlExport.lede).toHaveText("0 exports · 0 running");
  });

  test("a job lands in-progress and completes via the card's own htmx poll, without a reload", async ({
    page,
    request,
    sqlExport,
  }) => {
    test.setTimeout(60_000);
    // At least one real subject row, so the completion manifest carries an
    // actual download link (RF1e) instead of 200 empty outputs.
    const patientId = await createResource(request, "Patient", {
      name: [{ family: "SqlExportPaddingE2E" }],
    });
    await waitSearchable(request, "Patient", patientId);

    const prefix = `e2e_sql_export_slow_${Date.now()}`;
    const ids = await createResources(
      request,
      Array.from({ length: PADDING_SUBJECTS }, (_, i) => ({
        type: "ViewDefinition",
        body: {
          name: `${prefix}_${i}`,
          status: "active",
          resource: "Patient",
          select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
        },
      })),
    );
    seededViewDefinitionIds.push(...ids);

    await sqlExport.gotoNew();
    const checkboxes = ids
      .map((id) => `input[name="subject"][value="ViewDefinition/${id}"]`)
      .join(",");
    await expect(page.locator(checkboxes)).toHaveCount(ids.length);
    await page.locator(checkboxes).evaluateAll((inputs) => {
      inputs.forEach((input) => {
        (input as HTMLInputElement).checked = true;
      });
    });
    await sqlExport.startButton.click();

    // (c) Kick-off redirects straight to the list — no flash, the card is
    // the feedback (epic decision 7) — with an in-progress card for the job.
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(prefix);
    await expect(card).toBeVisible();
    await expect(card.locator(".tag")).toHaveText("In progress");

    // The overflow's `<details>` is server-rendered hidden — it would
    // otherwise hold nothing but the JS-only Copy job id button — but with
    // JavaScript and the Clipboard API both available (true on this loopback
    // origin), `sql-export.js` reveals it on load; the `nojs` project (no
    // script runs at all) is where it has to stay hidden (RNF1 of ticket 03).
    // Revealing the `<details>` only un-hides the summary, same as any other
    // native disclosure — its panel still needs opening to see inside.
    await expect(card.locator("details.menu")).toBeVisible();
    await card.locator("summary").click();
    await expect(card.getByRole("button", { name: "Copy job id" })).toBeVisible();
    await card.locator("summary").click();

    const progressbar = card.getByRole("progressbar");
    await expect(progressbar).toBeVisible();
    const initialProgress = await progressbar.getAttribute("aria-valuenow");
    expect(Number(initialProgress)).toBeLessThan(100);

    // (d) Without ever reloading, the card's own `hx-trigger="every 5s"`
    // fragment carries it to Complete: chip, full progress bar, and a meta
    // line naming the output files.
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await expect(progressbar).toHaveAttribute("aria-valuenow", "100");
    await expect(card).toContainText("file");

    // (e) View files leads to the completion manifest's downloads.
    await card.getByRole("link", { name: "View files" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/files\?job=/);
    await expect(page.locator(".data-table a").first()).toBeVisible();
  });

  test("New SQL Export marks a stored ViewDefinition, and Run again / Remove from list / Copy job id work", async ({
    page,
    request,
    sqlExport,
  }) => {
    const patientId = await createResource(request, "Patient", {
      name: [{ family: "SqlExportListFirstE2E" }],
    });
    const vdName = `e2e_sql_export_${Date.now()}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    // ES composites index asynchronously: the job reads its subjects through
    // search (#596), same as the pre-#833 form did.
    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Patient", patientId);

    // (b) New leads to the builder; a ViewDefinition created via the API is
    // marked and the job is started.
    await sqlExport.goto();
    await sqlExport.newButton.click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/new$/);
    await expectDetailFieldSpacing(page, "SQL Export builder");

    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.formatSelect.selectOption("csv");
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    let card = sqlExport.card(vdName);
    await expect(card).toBeVisible();
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await expect(card).toContainText("CSV");

    // (f) Run again (the overflow menu) adds a second card for the same job.
    await expect(sqlExport.card(vdName)).toHaveCount(1);
    card = sqlExport.card(vdName);
    await card.locator("summary").click();
    await card.getByRole("button", { name: "Run again" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    await expect(sqlExport.card(vdName)).toHaveCount(2);

    // The rerun lands first (most recent `startedAt`).
    const rerun = sqlExport.card(vdName).first();
    await expect(rerun.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });

    // (g) Remove from list drops it back to one card.
    await rerun.locator("summary").click();
    await rerun.getByRole("button", { name: "Remove from list" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    await expect(sqlExport.card(vdName)).toHaveCount(1);

    // (h) Copy job id is a JS-only progressive enhancement: hidden until the
    // Clipboard API is available (granted below), then writes the server's
    // job id verbatim. Located by its stable `data-copy-job-id` attribute,
    // not role+name: clicking it changes its own accessible name to
    // "Copied" as feedback, which a `getByRole(..., { name: "Copy job id" })`
    // locator would stop matching the moment that happens.
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    const remaining = sqlExport.card(vdName);
    await remaining.locator("summary").click();
    const copyButton = remaining.locator("[data-copy-job-id]");
    await expect(copyButton).toBeVisible();
    const jobId = await copyButton.getAttribute("data-copy-job-id");
    expect(jobId).toBeTruthy();
    await copyButton.click();
    await expect(copyButton).toHaveText("Copied");
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(jobId);
  });
});
