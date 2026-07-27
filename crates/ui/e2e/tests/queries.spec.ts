import { test, expect } from "../pages/fixtures";
import { createResource } from "../pages/api";

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
    for (let i = 0; i < 3; i++) await createResource(request, "Device", {});

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

  test("adding a condition row hydrates the builder", async ({ queries }) => {
    // The builder sections are hidden until there's a base query to parse.
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("condition").click();
    await expect(queries.builder.conditionRows).toHaveCount(1);
  });

  test("picking a type swaps in that type's parameter datalist", async ({ queries }) => {
    await queries.railItem("Patient").click();
    // /ui/queries/params fills #param-options for the picked type.
    await expect.poll(async () => queries.builder.paramOptions.count()).toBeGreaterThan(0);
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
