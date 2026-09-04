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

// #836: the job-wide filters ("Narrow it down"/"Advanced") and the browser
// behavior layered over them — sql-export-form.js's CSV header visibility
// and Since custom instant, plus combobox.js's Patients/Groups pickers
// (generic, shared with Bulk Export — its own keyboard/dedupe/removal
// behavior is bulk-export.spec.ts's job; these tests only cover that it is
// wired up here). This file's own top-level `afterEach` (above) cleans up
// both kinds of resource these tests seed.
test.describe("SQL Export builder job-wide filters (#836)", () => {
  test("Patients and Groups comboboxes select via keyboard, and Tracking id travels to the detail", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const vdName = `e2e_sql_export_filters_${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    await waitSearchable(request, "ViewDefinition", vdId);

    // `$sql-export` itself validates that every patient/group reference
    // resolves to a real resource before it will even start the job
    // (crates/rest/src/handlers/sof/export.rs's `validate_patient_group_refs`
    // — a direct `storage.read()`, not a search, so no indexing delay to
    // wait out). The combobox always picks whatever the search itself
    // found, so mocking its result fragment with real ids is what makes
    // this AC end in a `complete` job at all.
    const patientAId = await createResource(request, "Patient", {
      name: [{ given: ["Ana"], family: "Rivera" }],
    });
    const patientBId = await createResource(request, "Patient", {
      name: [{ given: ["Andrés"], family: "Silva" }],
    });
    const groupId = await createResource(request, "Group", {
      type: "person",
      actual: true,
      name: "Diabetes cohort",
    });

    const patientOptions = `
      <button type="button" class="combobox__option" data-combobox-option
              data-value="Patient/${patientAId}" data-label="Ana Rivera">Ana Rivera · Patient/${patientAId}</button>
      <button type="button" class="combobox__option" data-combobox-option
              data-value="Patient/${patientBId}" data-label="Andrés Silva">Andrés Silva · Patient/${patientBId}</button>`;
    const groupOptions = `
      <button type="button" class="combobox__option" data-combobox-option
              data-value="Group/${groupId}" data-label="Diabetes cohort">Diabetes cohort</button>`;

    await page.route("**/ui/lookup/patient-options*", (route) =>
      route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
    );
    await page.route("**/ui/lookup/group-options*", (route) =>
      route.fulfill({ status: 200, contentType: "text/html", body: groupOptions }),
    );

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();

    // Choose one via keyboard, see its chip, remove it — proving the
    // combobox's own selection/removal round trip works here — then choose
    // both options (Home/End land deterministically on the first/last
    // result regardless of the listbox's current active index).
    await sqlExport.patientSearch.fill("an");
    await expect(sqlExport.patientListbox.getByRole("option")).toHaveCount(2);
    await sqlExport.patientSearch.press("Home");
    await sqlExport.patientSearch.press("Enter");
    await expect(sqlExport.selectedPatients).toHaveCount(1);
    await sqlExport.patientCombobox.getByRole("button", { name: /Remove/ }).click();
    await expect(sqlExport.selectedPatients).toHaveCount(0);

    // A distinct query string from the first one: htmx's `hx-trigger="input
    // changed delay:300ms, search"` filters out an `input` event that leaves
    // the field's value unchanged (`changed`), so repeating the exact same
    // "an" here would never re-request — the mocked route ignores `q`
    // entirely, so any new text still answers with both options.
    await sqlExport.patientSearch.fill("an-again");
    await expect(sqlExport.patientListbox.getByRole("option")).toHaveCount(2);
    await sqlExport.patientSearch.press("Home");
    await sqlExport.patientSearch.press("Enter");
    await expect(sqlExport.selectedPatients).toHaveCount(1);
    // select() keeps the listbox open so a second pick needs no re-query.
    await sqlExport.patientSearch.press("End");
    await sqlExport.patientSearch.press("Enter");
    await expect(sqlExport.selectedPatients).toHaveCount(2);

    await sqlExport.groupSearch.fill("dia");
    await expect(sqlExport.groupListbox.getByRole("option")).toHaveCount(1);
    await sqlExport.groupSearch.press("ArrowDown");
    await sqlExport.groupSearch.press("Enter");
    await expect(sqlExport.selectedGroups).toHaveCount(1);

    await sqlExport.openAdvanced();
    await sqlExport.trackingIdInput.fill("ward-census-2026-q3");

    await sqlExport.startButton.click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(vdName);
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await card.getByRole("link", { name: vdName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);

    await expect(sqlExport.detailPatients).toHaveCount(2);
    await expect(sqlExport.detailPatients).toContainText([
      `Patient/${patientAId}`,
      `Patient/${patientBId}`,
    ]);
    await expect(sqlExport.detailGroups).toHaveCount(1);
    await expect(sqlExport.detailGroups).toHaveText(`Group/${groupId}`);
    await expect(sqlExport.detailTrackingId).toHaveText("ward-census-2026-q3");
  });

  test("the CSV header switch hides for non-csv formats, and an unchecked box is recorded in the detail", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const vdName = `e2e_sql_export_header_${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    await waitSearchable(request, "ViewDefinition", vdId);

    await sqlExport.gotoNew();
    await sqlExport.openAdvanced();
    // NDJSON is the default format on a fresh load: the header switch's own
    // label starts hidden — the box itself is never disabled or unchecked.
    await expect(sqlExport.headerLabel).toBeHidden();

    await sqlExport.formatOption("csv").check();
    await expect(sqlExport.headerLabel).toBeVisible();
    await expect(sqlExport.headerCheckbox).toBeChecked();

    await sqlExport.headerCheckbox.uncheck();
    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(vdName);
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await card.getByRole("link", { name: vdName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
    await expect(sqlExport.detailFormat).toHaveText("CSV · no header row");
  });

  test("Since's custom instant enables only for Custom, blocks an invalid submit, and a preset resolves in the detail", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const vdName = `e2e_sql_export_since_${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: vdName,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);
    await waitSearchable(request, "ViewDefinition", vdId);

    await sqlExport.gotoNew();
    await expect(sqlExport.sinceCustom).toBeDisabled();

    await sqlExport.sincePreset.selectOption("custom");
    await expect(sqlExport.sinceCustom).toBeEnabled();

    await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
    await sqlExport.sinceCustom.fill("not-a-valid-instant");
    await sqlExport.startButton.click();
    // Blocked client-side: no navigation, the field is marked, and the
    // message is visible.
    await expect(page).toHaveURL(/\/ui\/sql\/export\/new$/);
    await expect(sqlExport.sinceCustom).toHaveAttribute("aria-invalid", "true");
    await expect(sqlExport.sinceCustomError).toBeVisible();
    await expect(sqlExport.sinceCustom).toBeFocused();

    await sqlExport.sincePreset.selectOption("week");
    await expect(sqlExport.sinceCustom).toBeDisabled();
    const beforeSubmit = Date.now();
    await sqlExport.startButton.click();

    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(vdName);
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await card.getByRole("link", { name: vdName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);

    const sinceText = (await sqlExport.detailSince.textContent())?.trim() ?? "";
    const sinceMs = Date.parse(sinceText);
    expect(sinceMs, `"${sinceText}" should parse as an instant`).not.toBeNaN();
    const expectedMs = beforeSubmit - 7 * 24 * 60 * 60 * 1000;
    // Generous headroom around the exact 7-day mark: only the test's own
    // wall-clock time between computing `beforeSubmit` and the server
    // resolving `since_instant("week", "")` should separate them.
    expect(Math.abs(sinceMs - expectedMs)).toBeLessThan(60_000);
  });
});

// #837: the per-SQL-Query parameter values row — expand/collapse chevron,
// collapsed chip summary, the missing-values count, the submit block on a
// missing required value, and the detail's own chips (including a Run
// again round trip). This file's own top-level `afterEach` (above) cleans
// up the Library subjects these tests seed.
test.describe("SQL Export builder parameter values row (#837)", () => {
  test("shows/folds a query's values row, blocks a folded missing required value, and Run again repeats the chips in the detail", async ({
    page,
    request,
    sqlExport,
  }) => {
    test.setTimeout(60_000);
    const stamp = Date.now();
    const canonical = `http://example.org/ViewDefinition/e2e-sql-export-params-${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: `e2e_sql_export_params_vd_${stamp}`,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);

    const wardName = `e2e_sql_export_params_ward_${stamp}`;
    const wardLibId = await createSqlQueryLibrary(request, wardName, canonical, undefined, [
      { name: "ward", use: "in", type: "string" },
    ]);
    seededLibraryIds.push(wardLibId);

    const readmitName = `e2e_sql_export_params_readmit_${stamp}`;
    const readmitLibId = await createSqlQueryLibrary(request, readmitName, canonical, undefined, [
      { name: "days", use: "in", type: "integer", defaultInteger: 30 },
      { name: "from", use: "in", type: "date" },
    ]);
    seededLibraryIds.push(readmitLibId);

    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", wardLibId);
    await waitSearchable(request, "Library", readmitLibId);

    const wardRef = `Library/${wardLibId}`;
    const readmitRef = `Library/${readmitLibId}`;

    await sqlExport.gotoNew();

    // Both values rows start hidden — nothing is checked yet.
    await expect(sqlExport.paramsRow(wardRef)).toBeHidden();
    await expect(sqlExport.paramsRow(readmitRef)).toBeHidden();

    // Marking the ward query opens its row and reveals the chevron.
    await sqlExport.subjectCheckbox(wardRef).check();
    await expect(sqlExport.paramsRow(wardRef)).toBeVisible();
    await expect(sqlExport.rowToggle(wardRef)).toBeVisible();
    await expect(sqlExport.rowToggle(wardRef)).toHaveAttribute("aria-expanded", "true");

    await sqlExport.paramField(wardRef, "ward").fill("W1");

    // Folding hides the values row and shows the chip summary; unfolding
    // reverses it.
    await sqlExport.rowToggle(wardRef).click();
    await expect(sqlExport.rowToggle(wardRef)).toHaveAttribute("aria-expanded", "false");
    await expect(sqlExport.paramsRow(wardRef)).toBeHidden();
    await expect(sqlExport.paramSummary(wardRef)).toHaveText(":ward = W1");

    await sqlExport.rowToggle(wardRef).click();
    await expect(sqlExport.paramsRow(wardRef)).toBeVisible();
    await expect(sqlExport.paramSummary(wardRef)).toBeHidden();

    // Marking the second query without filling its required `from` shows
    // the missing-values count.
    await sqlExport.subjectCheckbox(readmitRef).check();
    await expect(sqlExport.selectedCount).toHaveText("2 of 3 selected · 1 value missing");

    // Fold it — the summary names both fields, `days` from its default and
    // `from` in the alert tone — then submit while folded. A hidden
    // (`display: none`) required field is still a native constraint-
    // validation candidate in Chromium, so without the script disabling
    // native validation (`form.noValidate`) the browser would silently
    // refuse to submit with no visible feedback at all — nothing to focus
    // or scroll to. The script's own check is what gives this real
    // feedback: no navigation, the row re-opens, the field is marked and
    // focused.
    await sqlExport.rowToggle(readmitRef).click();
    await expect(sqlExport.paramSummary(readmitRef)).toContainText(":days = 30");
    await expect(sqlExport.paramSummary(readmitRef)).toContainText(":from — required");

    await sqlExport.startButton.click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/new$/);
    await expect(sqlExport.paramsRow(readmitRef)).toBeVisible();
    const fromField = sqlExport.paramField(readmitRef, "from");
    await expect(fromField).toHaveAttribute("aria-invalid", "true");
    await expect(fromField).toBeFocused();

    await fromField.fill("2026-06-01");
    await expect(sqlExport.selectedCount).toHaveText("2 of 3 selected");

    await sqlExport.startButton.click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const card = sqlExport.card(wardName);
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await card.getByRole("link", { name: wardName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);

    await expect(sqlExport.detailSubjects).toContainText(":ward = W1");
    await expect(sqlExport.detailSubjects).toContainText(":days = 30");
    await expect(sqlExport.detailSubjects).toContainText(":from = 2026-06-01");

    // Run again replays the same subjects and their parameters into a
    // brand-new job — most recent first in the list — whose own detail
    // repeats the exact same chips.
    await page.getByRole("button", { name: "Run again" }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
    const rerunCard = sqlExport.card(wardName).first();
    await expect(rerunCard.locator(".tag")).toHaveText("Complete", { timeout: POLL_TIMEOUT });
    await rerunCard.getByRole("link", { name: wardName }).click();
    await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
    await expect(sqlExport.detailSubjects).toContainText(":ward = W1");
    await expect(sqlExport.detailSubjects).toContainText(":days = 30");
    await expect(sqlExport.detailSubjects).toContainText(":from = 2026-06-01");
  });

  test("the table filter hides a checked query's values row without losing its typed value", async ({
    page,
    request,
    sqlExport,
  }) => {
    const stamp = Date.now();
    const canonical = `http://example.org/ViewDefinition/e2e-sql-export-params-filter-${stamp}`;
    const vdId = await createResource(request, "ViewDefinition", {
      name: `e2e_sql_export_params_filter_vd_${stamp}`,
      url: canonical,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    seededViewDefinitionIds.push(vdId);

    const queryName = `e2e_sql_export_params_filter_query_${stamp}`;
    const libId = await createSqlQueryLibrary(request, queryName, canonical, undefined, [
      { name: "ward", use: "in", type: "string" },
    ]);
    seededLibraryIds.push(libId);

    await waitSearchable(request, "ViewDefinition", vdId);
    await waitSearchable(request, "Library", libId);

    const reference = `Library/${libId}`;

    await sqlExport.gotoNew();
    await sqlExport.subjectCheckbox(reference).check();
    await sqlExport.paramField(reference, "ward").fill("W1");
    await expect(sqlExport.paramsRow(reference)).toBeVisible();

    // A filter text matching nothing hides the subject row and, with it,
    // its values row — the typed value stays in the DOM throughout.
    await sqlExport.subjectFilterInput.fill("no-such-subject-name");
    await expect(sqlExport.subjectRow(queryName)).toBeHidden();
    await expect(sqlExport.paramsRow(reference)).toBeHidden();

    await sqlExport.subjectFilterInput.fill("");
    await expect(sqlExport.subjectRow(queryName)).toBeVisible();
    await expect(sqlExport.paramsRow(reference)).toBeVisible();
    await expect(sqlExport.paramField(reference, "ward")).toHaveValue("W1");
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
