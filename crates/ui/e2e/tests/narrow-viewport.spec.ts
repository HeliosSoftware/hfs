import { test, expect } from "../pages/fixtures";
import { createResource, deleteResources, seedTwoVersions } from "../pages/api";

// #1670: at phone width no page may be wider than the viewport. Each of these
// pages used to scroll sideways at 390 px — the chart tools on Home and
// Status, the results Sort control on Resources, the locate row, version rail
// and diff values on History, and the table and Add Tenant on Tenants.
test.describe("phone width (#1670)", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  let patientId = "";
  let observationId = "";

  test.beforeAll(async ({ request }) => {
    patientId = await seedTwoVersions(
      request,
      "Patient",
      {
        resourceType: "Patient",
        active: true,
        name: [{ family: "NarrowViewport", given: ["Ana"] }],
        identifier: [{ system: "urn:e2e:narrow-viewport", value: "a-long-identifier-value-1670" }],
      },
      (first) => ({ ...first, active: false, identifier: undefined }),
    );
    observationId = await createResource(request, "Observation", {
      resourceType: "Observation",
      status: "final",
      code: { text: "narrow viewport" },
      subject: { reference: `Patient/${patientId}` },
    });
  });

  test.afterAll(async ({ request }) => {
    await deleteResources(request, "Observation", [observationId].filter(Boolean));
    await deleteResources(request, "Patient", [patientId].filter(Boolean));
  });

  const pages = () => [
    "/ui",
    "/ui/status",
    "/ui/resources?type=Patient",
    "/ui/resources?type=Observation",
    `/ui/history?type=Patient&id=${patientId}`,
    "/ui/tenants",
  ];

  test("no page scrolls sideways", async ({ page }) => {
    for (const path of pages()) {
      await page.goto(path, { waitUntil: "networkidle" });
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow, `${path} is wider than the viewport`).toBeLessThanOrEqual(0);
    }
  });
});
