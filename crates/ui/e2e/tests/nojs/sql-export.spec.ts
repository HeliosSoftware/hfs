import { test, expect } from "../../pages/fixtures";
import {
  createResource,
  createResources,
  createSqlQueryLibrary,
  deleteResources,
  waitSearchable,
} from "../../pages/api";

// This file runs in the `nojs` project (javaScriptEnabled: false): htmx,
// sql-export.js, and every `data-*` handler are inert here — the Active SQL
// Exports list (#833) has to work as plain `<a href>`/`<form>` fallbacks, and
// a card's own `hx-trigger="every 5s"` poll never fires, so it only ever
// catches up on a fresh reload.
//
// `evaluateAll` below is Playwright's own instrumentation, not a page
// `<script>`: it keeps working with JavaScript "disabled" (the flag disables
// the page's own script execution, not Playwright's), the same way
// `tests/nojs/progressive-enhancement.spec.ts` already relies on it to read
// the Bulk Export type grid's checked/disabled state in bulk.

// A `$sql-export` job over a single tiny ViewDefinition finishes in well
// under 100ms — before the redirect that lands on the list even renders —
// so there is no reliable way to observe it in-progress, let alone catch it
// with Cancel. Padding the job with this many trivial subjects (a single
// self-search round trip apiece) buys a window measured in seconds, long
// enough for a real interaction, without ever waiting on a fixed clock:
// every wait below still polls actual DOM/network state.
const PADDING_SUBJECTS = 200;

// Every padding `ViewDefinition` the test below seeds gets its id pushed
// here, then deleted in this file's own `afterEach`. Left behind, a
// `ViewDefinition` is a real, tenant-visible resource that
// `/ui/sql/view-definitions` lists with no filter of its own — it becomes
// that page's default selection on whatever run happens to follow this one
// against the same shared server (see the chromium `sql-export.spec.ts` for
// the full mechanism).
let seededViewDefinitionIds: string[] = [];

// Same reasoning as `seededViewDefinitionIds` above, for the `Library`
// sql-query subject the failed-job detail test below seeds.
let seededLibraryIds: string[] = [];

test.afterEach(async ({ request }) => {
  const ids = seededViewDefinitionIds;
  seededViewDefinitionIds = [];
  await deleteResources(request, "ViewDefinition", ids);

  const libraryIds = seededLibraryIds;
  seededLibraryIds = [];
  await deleteResources(request, "Library", libraryIds);

  // The jobs this test starts live in the per-user settings document under
  // `byTenant.<tenant>.sqlExport.jobs` (crates/ui/src/sql_export.rs); the
  // generic `/_user/settings` endpoint projects tenant-scoped keys flat for
  // the caller's own tenant, so an RFC 7386 `{"sqlExport": null}`
  // merge-patch — the same shape `theme.spec.ts` uses for `theme` — deletes
  // this tenant's whole job store in one call (the cancelled job above is
  // never removed from the list, so it would otherwise survive into
  // whatever run follows this one against the same reused local dev
  // server).
  await request.patch("/_user/settings", {
    headers: { "Content-Type": "application/json" },
    data: { sqlExport: null },
  });
});

test("SQL Export lifecycle works without JavaScript", async ({ page, request, sqlExport }) => {
  // Two padded jobs, each needing its own genuinely observable in-progress
  // window (see PADDING_SUBJECTS above).
  test.setTimeout(120_000);

  async function startPaddedExport(name: string): Promise<void> {
    const ids = await createResources(
      request,
      Array.from({ length: PADDING_SUBJECTS }, (_, i) => ({
        type: "ViewDefinition",
        body: {
          name: `${name}_${i}`,
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
    await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  }

  const cancelledName = `nojs_sql_export_cancel_${Date.now()}`;
  await startPaddedExport(cancelledName);
  let cancelledCard = sqlExport.card(cancelledName);
  await expect(cancelledCard).toBeVisible();
  await expect(cancelledCard.locator(".tag")).toHaveText("In progress");

  // The overflow would hold nothing but the JS-only Copy job id button while
  // in-progress, so the whole `<details>` starts (and, with no script to
  // reveal it, stays) hidden — never a dead "…" control.
  const overflow = cancelledCard.locator("details.menu");
  await expect(overflow).toHaveCount(1);
  await expect(overflow).toBeHidden();

  // Cancel is a plain form and works without JavaScript.
  await cancelledCard.getByRole("button", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  cancelledCard = sqlExport.card(cancelledName);
  await expect(cancelledCard.locator(".tag")).toHaveText("Cancelled");

  const completedName = `nojs_sql_export_complete_${Date.now()}`;
  await startPaddedExport(completedName);
  let completedCard = sqlExport.card(completedName);
  await expect(completedCard).toBeVisible();
  await expect(completedCard.locator(".tag")).toHaveText("In progress");

  // No htmx here: the card only catches up on a fresh GET of the list.
  await expect
    .poll(
      async () => {
        await page.reload();
        completedCard = sqlExport.card(completedName);
        return completedCard.locator(".tag").innerText();
      },
      { timeout: 30_000, intervals: [500, 1_000, 2_000] },
    )
    .toBe("Complete");

  // Remove from list lives behind the overflow's native `<details>`
  // disclosure — no JS needed to open it — and is itself a plain form.
  await completedCard.locator("summary").click();
  await completedCard.getByRole("button", { name: "Remove from list" }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  await expect(sqlExport.card(completedName)).toHaveCount(0);
});

// #834's filter/switch/select-all/count enhancement (sql-export-form.js)
// never runs in this project: the builder has to stay exactly as the server
// renders it — every subject row visible, the table's tools and the header
// select-all both `hidden` — and a subject checked by hand (a real click,
// not the `evaluateAll` instrumentation the padded scenario above needs)
// still starts the job through a plain form post.
test("the subjects table's tools stay hidden, and a subject checked by hand still starts the job", async ({
  page,
  request,
  sqlExport,
}) => {
  const name = `nojs_sql_export_manual_${Date.now()}`;
  const vdId = await createResource(request, "ViewDefinition", {
    name,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  seededViewDefinitionIds.push(vdId);
  await waitSearchable(request, "ViewDefinition", vdId);

  await sqlExport.gotoNew();
  await expect(sqlExport.subjectTypeSwitch).toBeHidden();
  await expect(sqlExport.subjectFilterInput).toBeHidden();
  await expect(sqlExport.subjectSelectAll).toBeHidden();
  await expect(sqlExport.subjectRow(name)).toBeVisible();

  await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
  await sqlExport.startButton.click();

  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  await expect(sqlExport.card(name)).toBeVisible();
});

// The job detail page (#835) is a plain server-rendered page — no htmx, no
// script — so it has to work exactly as well without JavaScript as the list
// it is reached from.
test("a completed job's detail page lists its outputs and download pills without JavaScript", async ({
  page,
  request,
  sqlExport,
}) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "NojsSqlExportDetailE2E" }],
  });
  const vdName = `nojs_sql_export_detail_${Date.now()}`;
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

  // No htmx here: the card only catches up on a fresh reload.
  let card = sqlExport.card(vdName);
  await expect
    .poll(
      async () => {
        await page.reload();
        card = sqlExport.card(vdName);
        return card.locator(".tag").innerText();
      },
      { timeout: 15_000, intervals: [250, 500, 1_000] },
    )
    .toBe("Complete");

  // The card title is a plain `<a href>` — no JS needed to reach the
  // permalink.
  await card.getByRole("link", { name: vdName }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
  const row = page.locator(".data-table tbody tr").filter({ hasText: vdName });
  await expect(row).toHaveCount(1);
  await expect(row.locator(".job-card__files a")).toHaveCount(1);
});

// #836: without sql-export-form.js, the CSV header switch never hides
// itself and the Patients/Groups comboboxes never enhance past their plain
// fallback textareas — both still have to work, and their values still have
// to reach the job and its detail page. `<details>` is a native element, so
// opening "Advanced" needs no script either.
test("the CSV header switch is visible without JavaScript, and the Patients/Groups fallback textareas submit references shown in the detail", async ({
  page,
  request,
  sqlExport,
}) => {
  const vdName = `nojs_sql_export_filters_${Date.now()}`;
  const vdId = await createResource(request, "ViewDefinition", {
    name: vdName,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  seededViewDefinitionIds.push(vdId);
  await waitSearchable(request, "ViewDefinition", vdId);

  // `$sql-export` itself validates every patient/group reference against
  // real resources before it will even start the job
  // (`validate_patient_group_refs` in crates/rest/src/handlers/sof/
  // export.rs — a direct read, not a search, so no indexing delay to wait
  // out), so the fallback textareas need real ids too for this job to
  // reach `complete` rather than `failed`.
  const patientAId = await createResource(request, "Patient", {
    name: [{ family: "NojsSqlExportFiltersA" }],
  });
  const patientBId = await createResource(request, "Patient", {
    name: [{ family: "NojsSqlExportFiltersB" }],
  });
  const groupId = await createResource(request, "Group", {
    type: "person",
    actual: true,
    name: "Nojs cohort",
  });

  await sqlExport.gotoNew();
  await sqlExport.openAdvanced();
  await expect(sqlExport.headerLabel).toBeVisible();
  await expect(sqlExport.headerCheckbox).toBeEnabled();
  await expect(sqlExport.headerCheckbox).toBeChecked();

  await sqlExport.patientFallback.fill(`Patient/${patientAId}, Patient/${patientBId}`);
  await sqlExport.groupFallback.fill(groupId);
  await sqlExport.subjectCheckbox(`ViewDefinition/${vdId}`).check();
  await sqlExport.startButton.click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);

  // No htmx here: the card only catches up on a fresh reload.
  let card = sqlExport.card(vdName);
  await expect
    .poll(
      async () => {
        await page.reload();
        card = sqlExport.card(vdName);
        return card.locator(".tag").innerText();
      },
      { timeout: 15_000, intervals: [250, 500, 1_000] },
    )
    .toBe("Complete");

  await card.getByRole("link", { name: vdName }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
  await expect(sqlExport.detailPatients).toHaveCount(2);
  await expect(sqlExport.detailPatients).toContainText([
    `Patient/${patientAId}`,
    `Patient/${patientBId}`,
  ]);
  await expect(sqlExport.detailGroups).toHaveCount(1);
  await expect(sqlExport.detailGroups).toHaveText(`Group/${groupId}`);
});

// #837: without sql-export-form.js, every values row renders visible and
// the row-toggle chevron never does — there is no fold/collapse concept at
// all, matching the design's own "no-JavaScript renders every values row
// open" rule. A missing required value is caught by the server, not the
// browser: the field's `required` attribute is itself server-rendered only
// for a subject the *previous* render already knew was checked
// (`SubjectRow::checked`), so a box checked by a real click, with no script
// to resync it, submits a plain empty value the server has to reject.
test("every values row renders open with no chevron, and a missing required value round-trips through the server", async ({
  page,
  request,
  sqlExport,
}) => {
  const stamp = Date.now();
  const canonical = `http://example.org/ViewDefinition/nojs-sql-export-params-${stamp}`;
  const vdId = await createResource(request, "ViewDefinition", {
    name: `nojs_sql_export_params_vd_${stamp}`,
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  seededViewDefinitionIds.push(vdId);

  const queryName = `nojs_sql_export_params_query_${stamp}`;
  const libId = await createSqlQueryLibrary(request, queryName, canonical, undefined, [
    { name: "ward", use: "in", type: "string" },
  ]);
  seededLibraryIds.push(libId);
  await waitSearchable(request, "ViewDefinition", vdId);
  await waitSearchable(request, "Library", libId);

  const reference = `Library/${libId}`;

  await sqlExport.gotoNew();
  await expect(sqlExport.paramsRow(reference)).toBeVisible();
  await expect(sqlExport.rowToggle(reference)).toBeHidden();

  // Checked, left empty, submitted: no client-side block possible here, so
  // the request reaches the server, which rejects it and re-renders the
  // builder directly at the form's own POST target (`/ui/sql/export`,
  // never a redirect to `/new` — this URL alone cannot distinguish a
  // rejected submission from a successful one, only the content can) — no
  // new job appears in the list.
  await sqlExport.subjectCheckbox(reference).check();
  await sqlExport.startButton.click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  await expect(sqlExport.paramField(reference, "ward")).toHaveAttribute("aria-invalid", "true");
  await expect(sqlExport.paramsRow(reference)).toContainText("This value is required.");
  await sqlExport.goto();
  await expect(sqlExport.card(queryName)).toHaveCount(0);

  // Filled in and resubmitted, the job starts and the detail shows the chip.
  await sqlExport.gotoNew();
  await sqlExport.subjectCheckbox(reference).check();
  await sqlExport.paramField(reference, "ward").fill("W1");
  await sqlExport.startButton.click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);

  let card = sqlExport.card(queryName);
  await expect
    .poll(
      async () => {
        await page.reload();
        card = sqlExport.card(queryName);
        return card.locator(".tag").innerText();
      },
      { timeout: 15_000, intervals: [250, 500, 1_000] },
    )
    .toBe("Complete");

  await card.getByRole("link", { name: queryName }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
  await expect(page.locator(".job-detail__subjects")).toContainText(":ward = W1");
});

test("Retry on a failed job's detail page works without JavaScript", async ({
  page,
  request,
  sqlExport,
}) => {
  test.setTimeout(60_000);
  const stamp = Date.now();
  const canonical = `http://example.org/ViewDefinition/nojs-sql-export-failed-${stamp}`;
  const vdId = await createResource(request, "ViewDefinition", {
    name: `nojs_sql_export_failed_vd_${stamp}`,
    url: canonical,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  seededViewDefinitionIds.push(vdId);

  // Syntactically valid (kick-off's own SQL-shape check passes) but
  // references a column "v" never has, so the job fails during its own
  // background execution — see the chromium spec's own version of this
  // fixture for the full reasoning.
  const queryName = `nojs_sql_export_failed_query_${stamp}`;
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

  let card = sqlExport.card(queryName);
  await expect
    .poll(
      async () => {
        await page.reload();
        card = sqlExport.card(queryName);
        return card.locator(".tag").innerText();
      },
      { timeout: 30_000, intervals: [500, 1_000, 2_000] },
    )
    .toBe("Failed");

  await card.getByRole("link", { name: queryName }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export\/[^/]+$/);
  await expect(page.locator(".notice--warn")).toContainText(queryName);

  // Retry is a plain form and works without JavaScript.
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page).toHaveURL(/\/ui\/sql\/export$/);
  await expect(sqlExport.card(queryName)).toHaveCount(2);
});
