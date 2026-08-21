import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

// The Saved Queries workspace (/ui/queries): the shared query builder and the
// in-page results table — run a query, add builder rows, page through results,
// and see the datalist of parameters swap per type.
//
// saved-queries.js hides the whole builder form when the per-user settings
// store is unavailable (some backends don't provide one), so these skip
// themselves there — the same stance as the tenants suite.

test.describe("query builder", () => {
  test.beforeEach(async ({ queries }) => {
    await queries.goto();
    if (await queries.builder.form.isHidden().catch(() => true)) {
      test.skip(true, "no per-user settings store on this backend; the builder is hidden");
    }
  });

  test("running a query shows the results table with a total", async ({ queries, request }) => {
    await createResource(request, "Patient", { name: [{ family: "QueryA" }] });
    await createResource(request, "Patient", { name: [{ family: "QueryB" }] });

    await queries.goto();
    await queries.builder.run("Patient");
    await queries.results.waitShown();
    await expect(queries.results.rows.first()).toBeVisible();
    await expect(queries.results.meta).toContainText(/\d/);
  });

  test("the results table pages when a query spans more than one page", async ({
    queries,
    request,
  }) => {
    const devices = [];
    for (let i = 0; i < 3; i++) devices.push(await createResource(request, "Device", {}));
    // Indices refresh per resource type: wait on each device, not just the last.
    for (const id of devices) await waitSearchable(request, "Device", id);

    await queries.goto();
    await queries.builder.run("Device?_count=2");
    await queries.results.waitShown();
    await expect(queries.results.next).toBeVisible();

    const firstPage = await queries.results.rows.allInnerTexts();
    await queries.results.next.click();
    await expect
      .poll(async () => (await queries.results.rows.allInnerTexts()).join())
      .not.toBe(firstPage.join());
  });

  /// Chained search end to end (#406): the two chain directions meet on the
  /// patient in the middle — Practitioner <- Patient (generalPractitioner)
  /// <- Observation (subject) — and the combined query returns exactly it.
  test("a combined chain and _has query runs and returns the linked patient", async ({
    queries,
    request,
  }) => {
    // Unique per run: the suite may reuse a persistent dev server.
    const tag = Date.now().toString(36);
    const gp = await createResource(request, "Practitioner", {
      name: [{ family: `ChainSmith${tag}` }],
    });
    const patient = await createResource(request, "Patient", {
      name: [{ family: "ChainLinked" }],
      generalPractitioner: [{ reference: `Practitioner/${gp}` }],
    });
    const observation = await createResource(request, "Observation", {
      status: "final",
      code: { coding: [{ code: `chain-94-${tag}` }] },
      subject: { reference: `Patient/${patient}` },
    });
    // The chained query touches three indices; each refreshes on its own tick.
    await waitSearchable(request, "Practitioner", gp);
    await waitSearchable(request, "Patient", patient);
    await waitSearchable(request, "Observation", observation);

    await queries.goto();
    await queries.builder.run(
      `Patient?_has:Observation:patient:code=chain-94-${tag}&general-practitioner.name=ChainSmith${tag}`,
    );
    await queries.results.waitShown();
    await expect(queries.results.rows).toHaveCount(1);
    await expect(queries.results.rows.first()).toContainText(patient);
  });

  test("adding a condition row hydrates the builder", async ({ queries }) => {
    // The builder sections are hidden until there's a base query to parse.
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("condition").click();
    await expect(queries.builder.conditionRows).toHaveCount(1);
  });

  /* ---- chaining (#394) ------------------------------------------------- */

  test("a chained query hydrates into a forward-chain row and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.name=Smith");
    await expect(queries.builder.chainRows).toHaveCount(1);
    const row = queries.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("general-practitioner");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__value")).toHaveValue("Smith");

    // Editing the value re-serializes the same chained key.
    await row.locator(".builder-row__value").fill("Jones");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Jones",
    );
  });

  test("an explicitly typed chain keeps its :Type qualifier", async ({ queries }) => {
    await queries.builder.setUrl("Observation?subject:Patient.name:contains=ann");
    const row = queries.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("subject");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("contains");
    // The registry feeds Patient into the target-type select.
    await expect
      .poll(async () => row.locator(".builder-row__ctype").inputValue())
      .toBe("Patient");

    await row.locator(".builder-row__value").fill("bob");
    await expect(queries.builder.url).toHaveValue(
      "GET /Observation?subject:Patient.name:contains=bob",
    );
  });

  test("drilling into a reference param converts the row to a chain", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner=123");
    const row = queries.builder.conditionRows.first();
    // The affordance appears once the registry metadata loads.
    const drill = queries.builder.drillButton(row);
    await expect(drill).toBeVisible();
    await drill.click();

    await expect(queries.builder.chainRows).toHaveCount(1);
    const chain = queries.builder.chainRows.first();
    await chain.locator(".builder-row__cparam").fill("name");
    await chain.locator(".builder-row__value").fill("Smith");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Smith",
    );
    // The target-type select offers this param's registry targets.
    await expect
      .poll(async () => chain.locator(".builder-row__ctype option").count())
      .toBeGreaterThan(1);
  });

  test("a _has query hydrates into a reverse-chain row and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?_has:Observation:patient:code=1234-5");
    await expect(queries.builder.hasRows).toHaveCount(1);
    const row = queries.builder.hasRows.first();
    await expect(row.locator(".builder-row__htype")).toHaveValue("Observation");
    await expect(row.locator(".builder-row__href")).toHaveValue("patient");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("code");
    await expect(row.locator(".builder-row__value")).toHaveValue("1234-5");

    await row.locator(".builder-row__value").fill("8480-6");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=8480-6",
    );
  });

  test("the links-here button builds a _has filter from scratch", async ({ queries }) => {
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("has").click();
    const row = queries.builder.hasRows.first();
    await row.locator(".builder-row__htype").fill("Observation");
    await row.locator(".builder-row__href").fill("patient");
    await row.locator(".builder-row__cparam").fill("code");
    await row.locator(".builder-row__value").fill("1234-5");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=1234-5",
    );
  });

  test("a multi-level chain hydrates into hop segments and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.organization.name=Acme");
    await expect(queries.builder.chainRows).toHaveCount(1);
    const row = queries.builder.chainRows.first();
    const hops = row.locator(".builder-row__hopseg");
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0).locator(".builder-row__chainref")).toHaveValue(
      "general-practitioner",
    );
    await expect(hops.nth(1).locator(".builder-row__chainref")).toHaveValue("organization");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");

    await row.locator(".builder-row__value").fill("Beta");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Beta",
    );
  });

  test("drilling deeper appends a hop when the leaf is a reference", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.name=x");
    const row = queries.builder.chainRows.first();
    const leaf = row.locator(".builder-row__cparam");
    await leaf.fill("organization");
    // organization is a reference param on the target types, so the
    // drill-deeper affordance appears.
    const deeper = row.locator("[data-chain-deeper]");
    await expect(deeper).toBeVisible();
    await deeper.click();

    await expect(row.locator(".builder-row__hopseg")).toHaveCount(2);
    await leaf.fill("name");
    await row.locator(".builder-row__value").fill("Acme");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Acme",
    );
  });

  /* ---- OR values (#414) ------------------------------------------------ */

  test("comma OR values hydrate as stacked inputs and round-trip", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=Smith,Jones");
    const row = queries.builder.conditionRows.first();
    const values = row.locator(".builder-row__value");
    await expect(values).toHaveCount(2);
    await expect(values.nth(0)).toHaveValue("Smith");
    await expect(values.nth(1)).toHaveValue("Jones");

    await values.nth(1).fill("Garcia");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Smith,Garcia");
  });

  test("the + or button stacks a value; the per-value × removes it", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?name=Smith");
    const row = queries.builder.conditionRows.first();
    await row.locator("[data-add-or]").click();
    await row.locator(".builder-row__value").nth(1).fill("Jones");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Smith,Jones");

    await row.locator("[data-remove-or]").first().click();
    await expect(row.locator(".builder-row__value")).toHaveCount(1);
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Jones");
  });

  test("comparator OR values keep a prefix per alternative", async ({ queries }) => {
    await queries.builder.setUrl("Patient?birthdate=ge1980-01-01,le1990-12-31");
    const row = queries.builder.conditionRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    // The second alternative keeps its own comparator on round-trip.
    await row.locator(".builder-row__value").nth(0).fill("ge1985-01-01");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate=ge1985-01-01,le1990-12-31",
    );
  });

  /* ---- modifier panel (#415) ------------------------------------------ */

  test("the modifier select gates to the parameter type", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=x");
    const row = queries.builder.conditionRows.first();
    // string param: :contains offered, date prefixes are not
    await expect
      .poll(async () => row.locator(".builder-row__modifier option").allTextContents())
      .toContain(":contains");
    const opts = await row.locator(".builder-row__modifier option").allTextContents();
    expect(opts).not.toContain("ge");
    expect(opts).not.toContain(":in");
  });

  test("the MODIFY panel explains chips and clicking one applies it", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?name=ann");
    const row = queries.builder.conditionRows.first();
    await row.locator("[data-toggle-mods]").click();
    const panel = row.locator(".builder-row__modpanel");
    await expect(panel).toBeVisible();
    const chip = panel.locator("[data-mod-chip=contains]");
    await expect(chip).toContainText(":contains");
    await expect(chip).toContainText("anywhere");

    await chip.click();
    await expect(row.locator(".builder-row__modifier")).toHaveValue("contains");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name:contains=ann");
  });

  /* ---- results sort + typed columns (#416) ---------------------------- */

  test("typed default columns render and the sort control re-runs the query", async ({
    queries,
    request,
  }) => {
    await createResource(request, "Patient", {
      name: [{ family: "Sortable" }],
      gender: "female",
      birthDate: "1980-01-01",
    });
    await queries.goto();
    await queries.builder.run("Patient?name=Sortable");
    await queries.results.waitShown();

    // Patient renders its typed default columns without any _elements.
    const headers = queries.page.locator("#query-results-head th");
    await expect(headers).toContainText(["id", "name", "gender", "birthDate"]);

    const sort = queries.page.locator("#query-results-sort");
    await sort.selectOption("-_lastUpdated");
    await expect(queries.page.locator("#query-results-open")).toHaveAttribute(
      "href",
      "/Patient?name=Sortable&_sort=-_lastUpdated",
    );
  });

  /* ---- related data (#396) -------------------------------------------- */

  test("_include and _revinclude hydrate as structured related-data rows", async ({
    queries,
  }) => {
    await queries.builder.setUrl(
      "Patient?_include=Patient:general-practitioner&_revinclude:iterate=Observation:patient",
    );
    const rows = queries.page.locator("#builder-includes .builder-row--include");
    await expect(rows).toHaveCount(2);

    const inc = rows.nth(0);
    await expect(inc.locator(".builder-row__itype")).toHaveValue("Patient");
    await expect(inc.locator(".builder-row__iparam")).toHaveValue("general-practitioner");

    const rev = rows.nth(1);
    await expect(rev.locator(".builder-row__itype")).toHaveValue("Observation");
    await expect(rev.locator(".builder-row__iparam")).toHaveValue("patient");
    await expect(rev.locator("[data-toggle-iterate]")).toHaveAttribute("aria-pressed", "true");

    // Round-trip: toggling iterate off on the revinclude re-serializes.
    await rev.locator("[data-toggle-iterate]").click();
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_include=Patient:general-practitioner&_revinclude=Observation:patient",
    );
  });

  test("the related-data buttons build includes from scratch", async ({ queries }) => {
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("include-rev").click();
    const row = queries.page.locator("#builder-includes .builder-row--include").first();
    await row.locator(".builder-row__itype").fill("Observation");
    await row.locator(".builder-row__iparam").fill("subject");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject",
    );

    await queries.builder.addButton("include-fwd").click();
    const inc = queries.page.locator("#builder-includes .builder-row--include").nth(1);
    // The source type defaults to the base type.
    await expect(inc.locator(".builder-row__itype")).toHaveValue("Patient");
    await inc.locator(".builder-row__iparam").fill("general-practitioner");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject&_include=Patient:general-practitioner",
    );
  });

  /* ---- in plain English (#395) ---------------------------------------- */

  test("the plain-English line narrates conditions, chains, _has and includes", async ({
    queries,
  }) => {
    await queries.builder.setUrl(
      "Patient?name:contains=Smith,Jones&birthdate=ge1980-01-01" +
        "&general-practitioner.name=Ann&_has:Observation:patient:code=1234-5" +
        "&_include:iterate=Patient:general-practitioner&_count=20",
    );
    const text = queries.page.locator("#query-plain-text");
    await expect(queries.page.locator("#query-plain")).toBeVisible();
    await expect(text).toContainText("Find Patient records");
    await expect(text).toContainText("name contains “Smith” or “Jones”");
    await expect(text).toContainText("birthdate is on or after “1980-01-01”");
    await expect(text).toContainText("general-practitioner’s name is “Ann”");
    await expect(text).toContainText("related Observation whose code is “1234-5”");
    await expect(text).toContainText("Also returning the general-practitioner of each Patient (repeatedly)");
    await expect(text).toContainText("Showing 20 per page");

    // The narration follows edits made through the rows.
    const row = queries.builder.conditionRows.first();
    await row.locator(".builder-row__value").first().fill("Lopez");
    await expect(text).toContainText("name contains “Lopez” or “Jones”");
  });

  test("picking a type swaps in that type's parameter datalist", async ({ queries }) => {
    await queries.railItem("Patient").click();
    // /ui/queries/params fills #param-options for the picked type.
    await expect.poll(async () => queries.builder.paramOptions.count()).toBeGreaterThan(0);
  });

  // Picking a type updates the URL (and back navigates) without a full
  // reload — the click handler is an enhancement over the rail's real
  // <a href> (#541).
  test("picking a rail type updates the URL and back navigates", async ({ queries, page }) => {
    await queries.railItem("Observation").click();
    await expect(page).toHaveURL(/\/ui\/queries\?type=Observation/);
    await expect(queries.railItem("Observation")).toHaveAttribute("aria-current", "true");

    await page.goBack();
    await expect(page).toHaveURL(/\/ui\/queries$/);
  });

  test("a run is recorded under the Recent disclosure", async ({ queries, request }) => {
    await createResource(request, "Patient", { name: [{ family: "Recent" }] });
    await queries.goto();
    await queries.builder.run("Patient?name=Recent");
    await queries.results.waitShown();

    await queries.builder.recentToggle.click();
    await expect(queries.builder.recentPanel).toContainText(/Patient/);
  });
});
