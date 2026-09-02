// View Definitions workspace (#649): a stored ViewDefinition lists in the
// rail, its JSON lands in the editor, Run previews rows through $sql-run, and
// Create New offers the starter document. Everything here is plain links and
// forms, so it also holds with JavaScript disabled (the nojs sweep loads the
// route; the flows are exercised in the chromium project only). The rail
// itself is a server-side search — name filter, `_sort=name`, 50-item pages
// with plain previous/next links (#741) — not a full-collection fetch.
import { expect, test } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

test("a stored ViewDefinition lists, edits, and previews rows", async ({ page, request }) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "ViewDefE2E" }],
  });
  const vdId = await createResource(request, "ViewDefinition", {
    name: "e2e_patients",
    status: "active",
    resource: "Patient",
    // Scoped to this spec's own patient so the 50-row preview stays
    // deterministic however populated the backing store is (#596).
    where: [{ path: "name.family = 'ViewDefE2E'" }],
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });

  // ES composites index asynchronously: the rail and the run preview both
  // read through search, so wait for the seeds to be searchable (#596).
  await waitSearchable(request, "ViewDefinition", vdId);
  await waitSearchable(request, "Patient", patientId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);
  // The rail entry, selected; the editor holds the view's JSON.
  await expect(page.locator(`#vd-rail-list [data-type='${vdId}']`)).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator("textarea[name='json']")).toContainText("e2e_patients");

  const createNew = page.locator("a[href$='?vd=new']");
  await expect(createNew).toHaveClass(/\bbtn--primary\b/);
  await expect(createNew).not.toHaveClass(/\bbtn--accent\b/);
  await expect(createNew).toHaveCSS("height", "30px");
  await expect(createNew).toHaveCSS("padding-left", "12px");

  // Run previews the output; the seeded patient's key is among the rows.
  await page.locator("a[href*='run=1']").click();
  await expect(page.locator(".data-table")).toBeVisible();
  await expect(page.locator(".data-table td", { hasText: patientId }).first()).toBeVisible();

  // Create New swaps the editor to the starter document.
  await page.goto("/ui/sql/view-definitions?vd=new");
  await expect(page.locator("textarea[name='json']")).toContainText("new_view");
});

// #838 closes #820's own test gap: until now no
// Playwright test exercised the CodeMirror 6 editor vd-editor.js mounts over
// the JSON textarea. `insertText` (a single DOM input event, not per-key
// key events) is used for the replacement doc so CodeMirror's own
// closeBrackets extension never sees a lone "{"/"\"" keystroke to pair —
// exactly the same document ends up in the textarea either way, but this
// keeps the assertion below an exact match instead of a bracket-pairing
// footgun.
test("the CodeMirror editor syncs typed keystrokes to the hidden textarea, saves them, and never traps Tab", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const vdId = await createResource(request, "ViewDefinition", starter(`zcm_${stamp}_before`));
  await waitSearchable(request, "ViewDefinition", vdId);

  await page.goto(`/ui/sql/view-definitions?vd=${vdId}`);

  const textarea = page.locator("textarea[name='json']");
  const editor = page.locator(".vd-editor .cm-content[role='textbox']");
  await expect(editor).toBeVisible();
  await expect(textarea).toBeHidden();

  // The editor exposes the textarea's own aria-label on its own
  // role="textbox" content, so the hidden textarea does not duplicate the
  // landmark (NF3).
  const ariaLabel = await textarea.getAttribute("aria-label");
  expect(ariaLabel).toBeTruthy();
  await expect(editor).toHaveAttribute("aria-label", ariaLabel!);

  // `resourceType` is what the server's own createResource fixture injects
  // for us on every other test's writes (pages/api.ts) - typed by hand here
  // since Save posts exactly what the editor shows.
  const updatedDoc = JSON.stringify({
    resourceType: "ViewDefinition",
    ...starter(`zcm_${stamp}_after`),
  });
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.insertText(updatedDoc);
  await expect(textarea).toHaveValue(updatedDoc);

  // Tab moves focus to the next form control (Save) instead of indenting.
  await page.keyboard.press("Tab");
  const save = page.locator("button[name='action'][value='save']");
  await expect(save).toBeFocused();

  // Save submits exactly what the editor showed; after the redirect, the
  // freshly rendered page's editor and its hidden textarea both show the
  // saved JSON.
  await save.click();
  await page.waitForURL(/saved=1/);
  await expect(page.locator("textarea[name='json']")).toContainText(`zcm_${stamp}_after`);
  await expect(page.locator(".vd-editor .cm-content")).toContainText(`zcm_${stamp}_after`);
});

/** A minimal savable ViewDefinition, named for the rail. */
function starter(name: string) {
  return {
    name,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  };
}

// #741: the rail is now a server-side search (name filter, `_sort=name`,
// 50-item pages) rather than a full-collection fetch filtered in memory.

test("the search box filters the rail to exactly the matching names, case-insensitively", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  // Both "patients" hits share the stamp so the filter below cannot pick up
  // an unrelated ViewDefinition left over by another spec or worker.
  const alphaId = await createResource(
    request,
    "ViewDefinition",
    starter(`zpar_${stamp}_Patients_Alpha`),
  );
  const betaId = await createResource(
    request,
    "ViewDefinition",
    starter(`zpar_${stamp}_PATIENTS_Beta`),
  );
  const gammaId = await createResource(
    request,
    "ViewDefinition",
    starter(`zpar_${stamp}_Observations_Gamma`),
  );
  await Promise.all(
    [alphaId, betaId, gammaId].map((id) => waitSearchable(request, "ViewDefinition", id)),
  );

  // Typed lowercase against stored mixed/upper case — a case-insensitive
  // substring match either side, per the SQL-on-FHIR IG's `name:contains`.
  await page.goto(`/ui/sql/view-definitions?filter=${stamp}_patients`);
  const rail = page.locator("#vd-rail-list .filter-rail__item");
  await expect(rail).toHaveCount(2);
  await expect(page.locator(`#vd-rail-list [data-type='${alphaId}']`)).toBeVisible();
  await expect(page.locator(`#vd-rail-list [data-type='${betaId}']`)).toBeVisible();
  await expect(page.locator(`#vd-rail-list [data-type='${gammaId}']`)).toHaveCount(0);
});

test("paginates the rail past 50 views, preserving the filter across pages", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const names = Array.from(
    { length: 55 },
    (_, i) => `zpage_${stamp}_${String(i + 1).padStart(2, "0")}`,
  );
  const ids = await Promise.all(
    names.map((name) => createResource(request, "ViewDefinition", starter(name))),
  );
  await Promise.all(ids.map((id) => waitSearchable(request, "ViewDefinition", id)));

  await page.goto(`/ui/sql/view-definitions?filter=zpage_${stamp}`);
  const rail = page.locator("#vd-rail-list .filter-rail__item");
  await expect(rail).toHaveCount(50);
  const pagination = page.locator("nav.pagination");
  const next = pagination.locator("a", { hasText: "Next" });
  await expect(next).toBeVisible();
  await expect(pagination.locator("a", { hasText: "Previous" })).toHaveCount(0);

  await next.click();
  await expect(page).toHaveURL(/page=2/);
  expect(new URL(page.url()).searchParams.get("filter")).toBe(`zpage_${stamp}`);
  await expect(rail).toHaveCount(5);
  await expect(pagination.locator("a", { hasText: "Previous" })).toBeVisible();
  await expect(pagination.locator("a", { hasText: "Next" })).toHaveCount(0);
});

test("a selection the filter excludes from the rail still shows its own editor", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const keepId = await createResource(request, "ViewDefinition", starter(`zsel_${stamp}_keep`));
  const otherId = await createResource(
    request,
    "ViewDefinition",
    starter(`zsel_${stamp}_exclude`),
  );
  await Promise.all([keepId, otherId].map((id) => waitSearchable(request, "ViewDefinition", id)));

  await page.goto(`/ui/sql/view-definitions?vd=${keepId}&filter=exclude`);
  // The rail only shows what the filter matches...
  await expect(page.locator(`#vd-rail-list [data-type='${otherId}']`)).toBeVisible();
  await expect(page.locator(`#vd-rail-list [data-type='${keepId}']`)).toHaveCount(0);
  // ...but the editor still holds the view the filter excluded, read
  // directly by id rather than dropped as "not found" (#741).
  await expect(page.locator("textarea[name='json']")).toContainText(`zsel_${stamp}_keep`);
});
