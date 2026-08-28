import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

// The Resources workspace beyond the edit flows: the type rail (filter + live
// counts), the modal's open/close/tab surface, the delete flow, and the promise
// that every FHIR resource type is reachable — "test all the resources".

test("the type rail lists the full resource-type set", async ({ resources }) => {
  await resources.goto("Patient");
  const types = await resources.railTypes();
  // The R4 Patient compartment enumerates the whole resource set (145 types).
  expect(types.length).toBeGreaterThan(140);
  // ViewDefinition rides the generated-enum union (#648), not the spec's
  // compartment enumeration.
  expect(types).toEqual(
    expect.arrayContaining(["Patient", "Observation", "Encounter", "Bundle", "ViewDefinition"]),
  );
});

test("the rail filter narrows the visible types", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.railFilter.fill("observ");
  await expect(resources.railItem("Observation")).toBeVisible();
  await expect(resources.railItem("Patient")).toBeHidden();
  await resources.railFilter.fill("");
  await expect(resources.railItem("Patient")).toBeVisible();
});

// Picking a type updates the URL (and back navigates) without a full reload —
// the click handler is an enhancement over the rail's real <a href> (#541).
test("picking a rail type updates the URL and back navigates", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.pickType("Observation");
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);
  await expect(resources.railItem("Observation")).toHaveAttribute("aria-current", "true");

  await page.goBack();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Patient/);
  await expect(resources.railItem("Patient")).toHaveAttribute("aria-current", "true");
  await expect(resources.builder.url).toHaveValue("GET /Patient");
  await expect(resources.createLabel).toHaveText("Create new Patient");
});

// Opening in Patient context (#605): the client takes the same path as a
// rail click on load, so results are already visible with no interaction.
test("opening Resources with no type shows Patient results without interaction", async ({
  resources,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "OpenDefault" }] });
  await waitSearchable(request, "Patient", id);

  await resources.goto();
  await expect(resources.createLabel).toHaveText("Create new Patient");
  await resources.results.waitShown();
  await expect(resources.results.rows.first()).toBeVisible();
});

test("selecting a type updates the Create label and the URL", async ({ resources, page }) => {
  await resources.goto("Patient");
  await expect(resources.createLabel).toHaveText("Create new Patient");

  await resources.pickType("Observation");
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation/);
  await expect(resources.createLabel).toHaveText("Create new Observation");
  await expect(resources.builder.url).toHaveValue("GET /Observation");
});

test("switching away and back cannot reuse a stale builder serialization", async ({
  resources,
  page,
}) => {
  await resources.goto("Patient");
  const builderMode = page.locator("[data-mode-btn='builder']");
  if (await builderMode.count()) await builderMode.click();
  await resources.builder.addButton("condition").click();
  const firstRow = resources.builder.conditionRows.first();
  await firstRow.locator(".builder-row__key").fill("name");
  await firstRow.locator(".builder-row__value").fill("BeforeSwitch");
  await expect(resources.builder.url).toHaveValue("GET /Patient?name=BeforeSwitch");

  await firstRow.locator("[data-remove-row]").click();
  await expect(resources.builder.conditionRows).toHaveCount(0);
  await expect(resources.builder.url).toHaveValue("GET /Patient");

  await resources.pickType("Observation");
  await resources.pickType("Patient");
  await expect(resources.builder.url).toHaveValue("GET /Patient");
  await expect(resources.builder.sections).toHaveAttribute("data-type", "Patient");
  await expect(page.locator("#query-plain-text")).toContainText("Patient");

  await resources.builder.addButton("condition").click();
  const patientRow = resources.builder.conditionRows.first();
  await patientRow.locator(".builder-row__key").fill("name");
  await patientRow.locator(".builder-row__value").fill("AfterSwitch");
  await expect(resources.builder.url).toHaveValue("GET /Patient?name=AfterSwitch");
  await expect(page.locator("#query-plain-text")).toContainText("AfterSwitch");
});

test("a ?url= deep link still wins over the default Patient context", async ({ resources, page }) => {
  await page.goto("/ui/resources?url=" + encodeURIComponent("/Observation?status=final"), {
    waitUntil: "networkidle",
  });
  await expect(resources.builder.url).toHaveValue("GET /Observation?status=final");
  await expect(resources.railItem("Observation")).toHaveAttribute("aria-current", "true");
  await expect(resources.createLabel).toHaveText("Create new Observation");
  await resources.results.waitShown();
});

test("invalid, wrong-case, and empty inputs fail closed without losing the typed query", async ({
  resources,
  page,
}) => {
  for (const [target, expected] of [
    ["?type=NoLongerValid", "GET /NoLongerValid"],
    ["?type=patient", "GET /patient"],
    ["?url=" + encodeURIComponent("/NoLongerValid?name=kept"), "GET /NoLongerValid?name=kept"],
    ["?type=Patient&url=" + encodeURIComponent("/NoLongerValid"), "GET /NoLongerValid"],
  ] as const) {
    await page.goto("/ui/resources" + target, { waitUntil: "networkidle" });
    await expect(resources.builder.url).toHaveValue(expected);
    await expect(resources.createButton).toBeDisabled();
    await expect(page.locator("#resource-create-reason")).toBeVisible();
    await expect(page.locator("#type-rail-list [aria-current='true']")).toHaveCount(0);
  }

  await page.goto("/ui/resources?type=", { waitUntil: "networkidle" });
  await expect(resources.builder.url).toHaveValue("GET /Patient");
  await expect(resources.createButton).toBeEnabled();
});

test("manual URL edits and popstate keep Create on the effective query type", async ({
  resources,
  page,
}) => {
  await resources.goto("Patient");
  await resources.builder.url.fill("GET /patient");
  await resources.builder.url.dispatchEvent("change");
  await expect(resources.createButton).toBeDisabled();
  await expect(page.locator("#resources")).toHaveAttribute("data-selected-type", "patient");

  await resources.builder.url.fill("GET /Observation?status=final");
  await resources.builder.url.dispatchEvent("change");
  await expect(resources.createButton).toBeEnabled();
  await expect(resources.railItem("Observation")).toHaveAttribute("aria-current", "true");

  await resources.pickType("Encounter");
  await page.goBack();
  await expect(resources.builder.url).toHaveValue("GET /Patient");
  await expect(resources.createButton).toBeEnabled();
  await expect(resources.railItem("Patient")).toHaveAttribute("aria-current", "true");
});

test("openNew refuses a disabled target even if script removes the native disabled flag", async ({
  resources,
  page,
}) => {
  await page.goto("/ui/resources?type=patient", { waitUntil: "networkidle" });
  await expect(resources.createButton).toBeDisabled();
  await resources.createButton.evaluate((button: HTMLButtonElement) => {
    button.disabled = false;
    button.click();
  });
  await expect(resources.modal.root).toBeHidden();
  await expect(page.locator("#resource-editor-body")).toBeEmpty();
});

test("a conflicting Resources bookmark uses the query URL type everywhere after reload", async ({
  resources,
  page,
  request,
}) => {
  const patientId = await createResource(request, "Patient", {
    name: [{ family: "NavAlpha" }],
  });
  await waitSearchable(request, "Patient", patientId);

  const bookmark =
    "/ui/resources?type=Observation&url=" + encodeURIComponent("/Patient?name=NavAlpha");

  const expectPatientContext = async () => {
    await expect(resources.railItem("Patient")).toHaveAttribute("aria-current", "true");
    await expect(resources.railItem("Observation")).not.toHaveAttribute("aria-current", "true");
    await expect(page.locator("#resources")).toHaveAttribute("data-selected-type", "Patient");
    await expect(resources.createLabel).toHaveText("Create new Patient");
    await expect(resources.builder.url).toHaveValue("GET /Patient?name=NavAlpha");
    await expect(page.locator("#query-plain")).toBeVisible();
    await expect(page.locator("#query-plain-text")).toContainText("Patient");
    await expect(page.locator("#query-plain-text")).toContainText("NavAlpha");
    await resources.results.waitShown();
    const resultLink = page.locator(
      `#query-results-body a.url[data-resource-type='Patient'][data-resource-id='${patientId}']`,
    );
    await expect(resultLink).toBeVisible();
    await expect(resultLink).toHaveAttribute(
      "href",
      new RegExp(`^https?://[^/]+/Patient/${patientId}$`),
    );
    await expect(resources.results.openTab).toHaveAttribute("href", "/Patient?name=NavAlpha");
  };

  const expectCreateDraft = async (type: string) => {
    await resources.openCreate();
    await expect(resources.modal.subject).toContainText(type);
    expect((await resources.modal.editor.currentDoc()).resourceType).toBe(type);
    await resources.modal.close();
  };

  await page.goto(bookmark, { waitUntil: "networkidle" });
  await expectPatientContext();
  await expectCreateDraft("Patient");

  await page.reload({ waitUntil: "networkidle" });
  await expectPatientContext();
  await expectCreateDraft("Patient");

  await resources.pickType("Observation");
  await expect(page).toHaveURL(/\/ui\/resources\?type=Observation$/);
  expect(new URL(page.url()).searchParams.has("url")).toBe(false);

  await page.reload({ waitUntil: "networkidle" });
  await expect(resources.railItem("Observation")).toHaveAttribute("aria-current", "true");
  await expect(resources.railItem("Patient")).not.toHaveAttribute("aria-current", "true");
  await expect(page.locator("#resources")).toHaveAttribute("data-selected-type", "Observation");
  await expect(resources.createLabel).toHaveText("Create new Observation");
  await expect(resources.builder.url).toHaveValue("GET /Observation");
  await expect(page.locator("#query-plain-text")).toContainText("Observation");
  await resources.results.waitShown();
  await expect(resources.results.openTab).toHaveAttribute("href", "/Observation");
  await expectCreateDraft("Observation");
});

// Long type names (#605): the button truncates instead of widening the page
// head row past the viewport. At the suite's default desktop viewport the
// page-head row has more room than the button's own max-width (320px) ever
// needs, so the cap never actually engages and no real type name reaches it
// (`.resources-create` never gets squeezed below its natural, un-clamped
// width). Narrow the viewport so the row runs out of space and the button
// (`flex: 0 1 auto`) is forced to shrink past its content width — only then
// does the label's ellipsis engage for real.
test("a long type name truncates the Create label instead of breaking the header", async ({
  resources,
  page,
}) => {
  await page.setViewportSize({ width: 480, height: 800 });
  await resources.goto("MedicinalProductContraindication");
  await expect(resources.createLabel).toHaveText("Create new MedicinalProductContraindication");
  // Ellipsis, not layout overflow: the label's box is narrower than its text,
  // and the page never gains horizontal scroll because of it.
  const labelOverflowsItsBox = await resources.createLabel.evaluate(
    (el) => el.scrollWidth > el.clientWidth,
  );
  expect(labelOverflowsItsBox).toBe(true);
  const pageScrollsSideways = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
  );
  expect(pageScrollsSideways).toBe(false);
});

test("counts render next to each type from the dashboard snapshot", async ({
  resources,
  request,
}) => {
  // Seed one so the count is unambiguous and non-empty. The dashboard
  // snapshot is cached briefly (#541), so poll a fresh page load rather than
  // waiting on a client-side hydration fetch.
  await createResource(request, "Device", {});
  await expect
    .poll(
      async () => {
        await resources.goto("Device");
        return await resources.count("Device").textContent();
      },
      { timeout: 20_000, intervals: [1_000, 2_000, 4_000] },
    )
    .not.toBe("");
});

test("every rail type is searchable, while Create opens only eligible targets", async ({ resources }) => {
  await resources.goto("Patient");
  const types = await resources.railTypes();

  for (const type of types) {
    await resources.pickType(type);
    if (await resources.createButton.isDisabled()) {
      await expect(resources.modal.root).toBeHidden();
      await expect(resources.page.locator("#resource-create-reason")).toBeVisible();
      continue;
    }
    await resources.createButton.click();
    await resources.modal.waitOpen();
    expect(
      (await resources.modal.editor.currentDoc()).resourceType,
      `Create for ${type} projected the wrong resourceType`,
    ).toBe(type);
    await resources.modal.closeWithEscape();
  }
});

test("create eligibility follows the effective FHIR version", async ({ resources }) => {
  const version = process.env.HFS_DEFAULT_FHIR_VERSION || "R4";
  const boundary = {
    R4: { accepted: "Media", rejected: "ActorDefinition" },
    R4B: { accepted: "SubscriptionTopic", rejected: "ActorDefinition" },
    R5: { accepted: "ActorDefinition", rejected: "DocumentManifest" },
    R6: { accepted: "ActorDefinition", rejected: "Media" },
  }[version];
  expect(boundary, `No Resources boundary is defined for ${version}`).toBeTruthy();

  await resources.goto(boundary!.accepted);
  await expect(resources.railItem(boundary!.accepted)).toHaveAttribute("aria-current", "true");
  await expect(resources.createButton).toBeEnabled();
  await resources.openCreate();
  expect((await resources.modal.editor.currentDoc()).resourceType).toBe(boundary!.accepted);
  await resources.modal.close();

  await resources.goto(boundary!.rejected);
  await expect(resources.builder.url).toHaveValue(`GET /${boundary!.rejected}`);
  await expect(resources.createButton).toBeDisabled();
  await expect(resources.page.locator("#resource-create-reason")).toBeVisible();
  await expect(resources.page.locator("#type-rail-list [aria-current='true']")).toHaveCount(0);
});

// "Recently used" group (#603): a per-browser convenience, populated by
// resource-filter.js from explicit rail picks and re-rendered on load.
test("the recently-used group caps at five, keeps MRU order, deduplicates, and preserves counts", async ({
  resources,
}) => {
  await resources.goto("Patient");
  await resources.pickType("Account");
  await resources.pickType("ActivityDefinition");
  await resources.pickType("AdverseEvent");
  await resources.pickType("AllergyIntolerance");
  await resources.pickType("Appointment");
  await resources.pickType("AppointmentResponse");
  await resources.pickType("AllergyIntolerance"); // re-selection: moves to front, no duplicate

  // The group is client-rendered from localStorage on load, so it only
  // reflects the picks above once the page (re)loads.
  await resources.goto("Patient");
  await expect(resources.recentGroup).toBeVisible();
  // The divider and "All Types" heading (#603 follow-up) give the general
  // list its own clearly separated section once Recently used has entries.
  await expect(resources.recentDivider).toBeVisible();
  await expect(resources.generalHeading).toBeVisible();

  const types = await resources.recentGroup
    .locator("a.filter-rail__item[data-type]")
    .evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.type!));
  expect(types).toEqual([
    "AllergyIntolerance",
    "AppointmentResponse",
    "Appointment",
    "AdverseEvent",
    "ActivityDefinition",
  ]);

  for (const type of types) {
    const listCount = await resources.count(type).textContent();
    const recentCount = await resources.recentItem(type).locator(".count").textContent();
    expect(recentCount).toBe(listCount);
  }
});

test("the type rails keep recents and the All Types heading fixed while type items scroll", async ({
  resources,
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 700 });
  await resources.goto("Account");
  for (const type of [
    "ActivityDefinition",
    "AdverseEvent",
    "AllergyIntolerance",
    "Appointment",
    "Observation",
  ]) {
    await resources.pickType(type);
  }

  for (const [name, path] of [
    ["Resources", "/ui/resources?type=Account"],
    ["Search", "/ui/search?type=Account"],
    ["Saved queries", "/ui/queries?type=Account"],
  ] as const) {
    await test.step(name, async () => {
      await page.goto(path, { waitUntil: "networkidle" });

      const recent = resources.recentGroup;
      const divider = resources.recentDivider;
      const allTypes = resources.generalHeading;
      const pinned = recent.or(divider).or(allTypes);
      const list = resources.typeList;
      const firstType = list.locator("a.filter-rail__item[data-type]").first();

      await expect(recent).toBeVisible();
      await expect(divider).toBeVisible();
      await expect(allTypes).toBeVisible();

      const pinnedTopsBefore = await pinned
        .evaluateAll((elements) => elements.map((element) => element.getBoundingClientRect().top));
      const firstTypeTopBefore = await firstType.evaluate(
        (element) => element.getBoundingClientRect().top,
      );
      const before = await list.evaluate((element) => ({
        scrollTop: element.scrollTop,
        maxScrollTop: element.scrollHeight - element.clientHeight,
      }));
      expect(before.maxScrollTop).toBeGreaterThan(0);

      await list.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
      await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(
        before.scrollTop,
      );

      const pinnedTopsAfter = await pinned
        .evaluateAll((elements) => elements.map((element) => element.getBoundingClientRect().top));
      const firstTypeTopAfter = await firstType.evaluate(
        (element) => element.getBoundingClientRect().top,
      );

      expect(pinnedTopsAfter).toHaveLength(pinnedTopsBefore.length);
      pinnedTopsAfter.forEach((top, index) => {
        expect(Math.abs(top - pinnedTopsBefore[index])).toBeLessThanOrEqual(1);
      });
      expect(firstTypeTopAfter).toBeLessThan(firstTypeTopBefore);
    });
  }
});

test("clicking a recently-used entry selects that type for real", async ({ resources, page }) => {
  await resources.goto("Patient");
  await resources.pickType("Encounter");
  await resources.goto("Patient"); // reload to populate the group

  await resources.recentItem("Encounter").click();
  await expect(page).toHaveURL(/\/ui\/resources\?type=Encounter/);
  await expect(resources.railItem("Encounter")).toHaveAttribute("aria-current", "true");
});

test("Create new does not register a recently-used entry", async ({ resources }) => {
  // Create is a <button>, not a rail `<a>`, so resource-filter.js's
  // click listener (scoped to real rail items) never matches it — even
  // though the default selection (Patient, unpicked) still names it in the
  // button's label.
  await resources.goto("Patient");
  await resources.createButton.click();
  await resources.modal.waitOpen();
  await resources.modal.closeWithEscape();

  await resources.goto("Patient");
  await expect(resources.recentGroup).toBeHidden();
  // Nothing to divide from: the divider stays hidden too, but the general
  // list still carries its own heading.
  await expect(resources.recentDivider).toBeHidden();
  await expect(resources.generalHeading).toBeVisible();
});

test("the modal closes via the X and via Escape", async ({ resources }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.close();
  await expect(resources.modal.root).toBeHidden();

  await resources.openCreate();
  await resources.modal.closeWithEscape();
  await expect(resources.modal.root).toBeHidden();
});

test("a result under a public path prefix still opens in the modal", async ({
  resources,
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", {
    name: [{ family: "PublicPrefix" }],
  });
  const queryPath = `/Patient?_id=${id}`;
  const publicUrl = `https://fhir.example.test/public/fhir/acme/Patient/${id}`;

  await page.route(`**${queryPath}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/fhir+json",
      body: JSON.stringify({
        resourceType: "Bundle",
        type: "searchset",
        total: 1,
        entry: [
          {
            fullUrl: publicUrl,
            resource: { resourceType: "Patient", id, name: [{ family: "PublicPrefix" }] },
          },
        ],
      }),
    });
  });

  await resources.goto("Patient");
  await page.locator("input.query-builder__url[name=url]").fill(queryPath.slice(1));
  await page.locator("[data-intent='run']").click();

  const resultLink = page.locator("#query-results-body a.url").first();
  await expect(resultLink).toHaveAttribute("href", publicUrl);
  await expect(resultLink).toHaveAttribute("data-resource-type", "Patient");
  await expect(resultLink).toHaveAttribute("data-resource-id", id);
  await resultLink.click();
  await resources.modal.waitOpen();
  await expect(resources.modal.subject).toContainText(id);
});

test("a created resource can be deleted from its modal", async ({ resources, page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "ToDelete" }] });
  await waitSearchable(request, "Patient", id);

  // Open it in the modal by searching for it and clicking the result row.
  await resources.goto("Patient");
  await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
  await page.locator("[data-intent='run']").click();
  await page.locator(`#query-results-body a.url`).first().click();
  await resources.modal.waitOpen();
  await expect(resources.modal.subject).toContainText(id);

  page.once("dialog", (d) => d.accept()); // confirm delete
  await resources.modal.deleteButton.click();

  await expect(resources.modal.root).toBeHidden();
  // It's gone from the API.
  const res = await request.get(`/Patient/${id}`, {
    headers: { Accept: "application/fhir+json" },
  });
  expect(res.status()).toBe(410);
});

test("the dialog stays put across tab switches and status messages", async ({
  resources,
  page,
  request,
}) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Anchored" }] });
  await waitSearchable(request, "Patient", id);
  await resources.goto("Patient");
  await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
  await page.locator("[data-intent='run']").click();
  await page.locator("#query-results-body a.url").first().click();
  await resources.modal.waitOpen();

  // The dialog occupies a fixed rectangle (#607): switching panes or a
  // status message appearing changes what is inside, never where it sits.
  const head = page.locator(".modal__head");
  const before = await head.boundingBox();
  if (!before) throw new Error("modal header has no box");

  await page.locator('[data-modal-tab="history"]').click();
  await expect(page.locator('[data-modal-pane="history"]')).toBeVisible();
  const onHistory = await head.boundingBox();
  expect(onHistory?.y).toBe(before.y);
  expect(onHistory?.height).toBe(before.height);

  await page.locator('[data-modal-tab="edit"]').click();
  await expect(page.locator('[data-modal-pane="edit"]')).toBeVisible();

  await page.click("#resource-save");
  await expect(page.locator("#resource-modal-status")).not.toBeEmpty();
  const withStatus = await head.boundingBox();
  expect(withStatus?.y).toBe(before.y);
});
