// Active SQL Exports (#833): the list-first workspace for `$sql-export` jobs.
// Runs against the sqlite server the suite boots — a real `$sql-export`
// kick-off, not a stub — so a job genuinely transitions through the states
// the card renders. Tests run in declaration order against one shared server
// (playwright.config.ts: fullyParallel: false, workers: 1); this file's own
// `afterEach` (below) restores both kinds of state it leaves on that shared
// server, so a rerun sees the same empty baseline as the very first run.
import { expect, test } from "../pages/fixtures";
import {
  createResource,
  createResources,
  createSqlQueryLibrary,
  deleteResources,
  waitSearchable,
} from "../pages/api";

// The card's own htmx fragment polls every 5s; generous headroom for a job to
// finish without ever sleeping blindly.
const POLL_TIMEOUT = 30_000;

// Every `ViewDefinition` a test below seeds gets its id pushed here, then
// deleted in this file's own `afterEach` (below). A `ViewDefinition` is a
// real, tenant-visible resource that `/ui/sql/view-definitions` lists with
// no filter of its own; left behind, it becomes that page's default
// selection and mounts its CodeMirror editor, which then fails
// `design-system.spec.ts`'s "every class used" sweep on whatever run
// happens to follow this one against the same shared server.
let seededViewDefinitionIds: string[] = [];

// Same reasoning as `seededViewDefinitionIds` above, for the `Library`
// sql-query/sql-view subjects the builder-enhancement tests below seed: left
// behind, a `Library` becomes `/ui/sql/queries`' or `/ui/sql/views`' default
// rail selection on whatever run follows this one.
let seededLibraryIds: string[] = [];

test.afterEach(async ({ request }) => {
  const ids = seededViewDefinitionIds;
  seededViewDefinitionIds = [];
  await deleteResources(request, "ViewDefinition", ids);

  const libraryIds = seededLibraryIds;
  seededLibraryIds = [];
  await deleteResources(request, "Library", libraryIds);

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
 * clock: every assertion below still polls actual DOM state — this
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
    // actual download link instead of 200 empty outputs.
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
    // the feedback — with an in-progress card for the job.
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(prefix);
    await expect(card).toBeVisible();
    await expect(card.locator(".tag")).toHaveText("In progress");

    // The overflow's `<details>` is server-rendered hidden — it would
    // otherwise hold nothing but the JS-only Copy job id button — but with
    // JavaScript and the Clipboard API both available (true on this loopback
    // origin), `sql-export.js` reveals it on load; the `nojs` project (no
    // script runs at all) is where it has to stay hidden.
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

    // (e) View files leads to the job's own permalink (#835), listing every
    // one of this padded job's outputs and its one download pill apiece —
    // a trivial single-row `ViewDefinition` never needs a second shard.
    await card.getByRole("link", { name: "View files" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
    await expect(page.locator(".data-table tbody tr")).toHaveCount(ids.length);
    await expect(page.locator(".job-card__files a")).toHaveCount(ids.length);
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

    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.formatOption("csv").check();
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

// #834: the builder's subjects table gains a type switch, a text filter, a
// header select-all, and a live "n of m selected" count over the plain rows
// the #833 markup rendered — sql-export-form.js. This file's own top-level
// `afterEach` (above) cleans up both kinds of resource these tests seed.
test.describe("SQL Export builder subjects table (#834)", () => {
  test("marks a ViewDefinition and a SQL Query, starts as CSV, and the card summarizes both kinds", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const vdName = `e2e_sql_export_form_vd_${stamp}`;
    const canonical = `http://example.org/ViewDefinition/e2e-sql-export-form-${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);

    const queryName = `e2e_sql_export_form_query_${stamp}`;
    const libId = await createSqlQueryLibrary(request, queryName, canonical);
    seededLibraryIds.push(libId);

    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", libId);

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.subjectCheckbox(`Library/${libId}`).check();
    await sqlExport.formatOption("csv").check();
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(vdName);
    await expect(card).toBeVisible();
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await expect(card).toContainText("1 ViewDefinition");
    await expect(card).toContainText("1 SQL Query");
    await expect(card).toContainText("CSV");
  });

  test("filtering hides a checked row without unchecking it, and the hidden selection still submits", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const targetName = `e2e_sql_export_filter_target_${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: targetName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    await waitSearchable(request, "ViewDefinition", vdId);

    await sqlExport.gotoNew();
    const row = sqlExport.subjectRow(targetName);
    const checkbox = sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`);
    await checkbox.check();
    const countBefore = await sqlExport.selectedCount.textContent();

    await sqlExport.subjectFilterInput.fill(`no-such-subject-${stamp}`);
    await expect(row).toBeHidden();
    await expect(checkbox).toBeChecked();
    await expect(sqlExport.selectedCount).toHaveText(countBefore ?? "");
    await expect(sqlExport.subjectsEmptyRow).toBeVisible();

    // Submit while the row is still hidden by the filter: a hidden checked
    // box is still part of the form, and its value still reaches the job.
    await sqlExport.startButton.click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    await expect(sqlExport.card(targetName)).toBeVisible();
  });

  test("the type switch shows only the selected kind and updates aria-pressed", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const vdName = `e2e_sql_export_switch_vd_${stamp}`;
    const canonical = `http://example.org/ViewDefinition/e2e-sql-export-switch-${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);

    const queryName = `e2e_sql_export_switch_query_${stamp}`;
    const libId = await createSqlQueryLibrary(request, queryName, canonical);
    seededLibraryIds.push(libId);

    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", libId);

    await sqlExport.gotoNew();
    const vdRow = sqlExport.subjectRow(vdName);
    const queryRow = sqlExport.subjectRow(queryName);
    await expect(vdRow).toBeVisible();
    await expect(queryRow).toBeVisible();

    await sqlExport.subjectTypeButton("sql-query").click();
    await expect(sqlExport.subjectTypeButton("sql-query")).toHaveAttribute("aria-pressed", "true");
    await expect(sqlExport.subjectTypeButton("all")).toHaveAttribute("aria-pressed", "false");
    await expect(vdRow).toBeHidden();
    await expect(queryRow).toBeVisible();

    await sqlExport.subjectTypeButton("all").click();
    await expect(sqlExport.subjectTypeButton("all")).toHaveAttribute("aria-pressed", "true");
    await expect(vdRow).toBeVisible();
  });

  test("header select-all marks only the rows a filter currently shows, and the count includes hidden checked rows", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const hiddenName = `e2e_sql_export_selectall_hidden_${stamp}`;
    const visiblePrefix = `e2e_sql_export_selectall_visible_${stamp}`;
    const visibleNameA = `${visiblePrefix}_a`;
    const visibleNameB = `${visiblePrefix}_b`;
    const ids = await createResources(
      request,
      [hiddenName, visibleNameA, visibleNameB].map((name) => ({
        type: "ViewDefinition",
        body: {
          name,
          status: "active",
          resource: "Patient",
          select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
        },
      })),
    );
    seededViewDefinitionIds.push(...ids);
    await Promise.all(ids.map((id) => waitSearchable(request, "ViewDefinition", id)));

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(`ViewDefinition/${ids[0]}`).check();

    await sqlExport.subjectFilterInput.fill(visiblePrefix);
    await expect(sqlExport.subjectRow(hiddenName)).toBeHidden();
    await expect(sqlExport.subjectRow(visibleNameA)).toBeVisible();
    await expect(sqlExport.subjectRow(visibleNameB)).toBeVisible();

    await sqlExport.subjectSelectAll.check();

    await expect(sqlExport.subjectCheckbox(`ViewDefinition/${ids[1]}`)).toBeChecked();
    await expect(sqlExport.subjectCheckbox(`ViewDefinition/${ids[2]}`)).toBeChecked();
    // The hidden, already-checked row is untouched by select-all — neither
    // dropped nor double-counted.
    await expect(sqlExport.subjectCheckbox(`ViewDefinition/${ids[0]}`)).toBeChecked();
    await expect(sqlExport.selectedCount).toContainText("3 of");
    await expect(sqlExport.subjectSelectAll).toBeChecked();
  });
});

// The job's own permalink (#835), reached from the list either the card's
// title or its "View files" link — never the retired job-id lookup form.
// This file's own top-level `afterEach` (above) cleans up both kinds of
// resource these tests seed.
test.describe.serial("SQL Export job detail (#835)", () => {
  test("the card title and View files both lead to the same permalink, listing every output and its download, and it survives a reload", async ({
    page,
    request,
    sqlExport,
  }) => {
    const patientId = await createResource(request, "Patient", {
      name: [{ family: "SqlExportDetailE2E" }],
    });
    const vdName = `e2e_sql_export_detail_${Date.now()}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Patient", patientId);

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(vdName);
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });

    // The card's title leads to the job's own permalink.
    await card.getByRole("link", { name: vdName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
    const detailUrl = page.url();
    await expect(page.locator("h1.page-head__title")).toHaveText(vdName);
    // The Job card's own id field — non-empty once the kick-off succeeded.
    await expect(page.locator(".detail__field code")).not.toHaveText("");

    // The one output a single-subject job produces is named after the
    // subject itself (no collision to disambiguate, so kickoff's own
    // subject_output_names never needs to suffix it) and carries its own
    // download pill; the pill's own location is a real, fetchable file.
    const row = page.locator(".data-table tbody tr").filter({ hasText: vdName });
    await expect(row).toHaveCount(1);
    const pill = row.locator(".job-card__files a").first();
    await expect(pill).toBeVisible();
    const href = await pill.getAttribute("href");
    expect(href).toBeTruthy();
    expect((await request.get(href!)).status()).toBe(200);

    // The permalink survives a reload — it reads the notebook's own record,
    // not the server (module docs of sql_export.rs), so there is nothing
    // for the reaper or a restart to take away from it.
    await page.reload();
    await expect(page.locator("h1.page-head__title")).toHaveText(vdName);
    await expect(row).toHaveCount(1);

    // View files, from the list, leads to the exact same permalink.
    await sqlExport.goto();
    await sqlExport.card(vdName).getByRole("link", { name: "View files" }).click();
    await expect(page).toHaveURL(detailUrl);
  });

  test("a failed SQL Query names the subject in the detail's notice, and Retry adds a new card", async ({
    page,
    request,
    sqlExport,
  }) => {
    test.setTimeout(60_000);
    const stamp = Date.now();
    const canonical = `http://example.org/ViewDefinition/e2e-sql-export-failed-${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: `e2e_sql_export_failed_vd_${stamp}`,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);

    // A syntactically valid single SELECT (so kick-off itself succeeds)
    // referencing a column "v" never has: the server only validates SQL
    // shape and the dependency graph at kick-off, so this fails during the
    // job's own background execution, exactly like a real broken query
    // would (crates/rest/src/export/in_memory.rs's `run_sqlquery_job`).
    const queryName = `e2e_sql_export_failed_query_${stamp}`;
    const libId = await createSqlQueryLibrary(
      request,
      queryName,
      canonical,
      "SELECT no_such_column FROM v",
    );
    seededLibraryIds.push(libId);
    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", libId);

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(`Library/${libId}`).check();
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(queryName);
    await expect(card.locator(".tag")).toHaveText("Failed", { timeout: POLL_TIMEOUT });

    await card.getByRole("link", { name: queryName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
    const notice = page.locator(".notice--warn");
    await expect(notice).toContainText("stopped on subject");
    await expect(notice).toContainText(queryName);

    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    await expect(sqlExport.card(queryName)).toHaveCount(2);
  });
});

// #835: the job-id lookup form is retired — its nav entry is gone (see
// chrome.spec.ts) and its own URL now only redirects.
test("the sidebar carries no Files entry, and /ui/sql/files redirects to the list", async ({
  page,
  sqlExport,
}) => {
  await sqlExport.goto();
  await expect(page.locator('[href="/ui/sql/files"]')).toHaveCount(0);

  await page.goto("/ui/sql/files");
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
});
