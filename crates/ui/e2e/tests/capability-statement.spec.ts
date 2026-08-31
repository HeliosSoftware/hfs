import { test, expect } from "../pages/fixtures";

test("Capability Statement links follow the active FHIR version", async ({
  capabilityStatement,
  chrome,
}) => {
  await capabilityStatement.goto("?filter=Patient");

  const version = await chrome.currentVersion();
  const configuredVersion = process.env.HFS_DEFAULT_FHIR_VERSION;
  if (configuredVersion) {
    expect(version).toBe(configuredVersion);
  }
  const expected = {
    R4: {
      summary: "4.0.1",
      patientHref: "https://hl7.org/fhir/R4/patient.html",
    },
    R4B: {
      summary: "4.3.0",
      patientHref: "https://hl7.org/fhir/R4B/patient.html",
    },
    R5: {
      summary: "5.0.0",
      patientHref: "https://hl7.org/fhir/R5/patient.html",
    },
    R6: {
      summary: "6.0.0",
      patientHref: "https://hl7.org/fhir/6.0.0-ballot4/patient.html",
    },
  }[version];
  expect(expected, `No Capability Statement expectation is defined for ${version}`).toBeTruthy();

  await expect(capabilityStatement.fhirVersionSummary).toHaveText(expected!.summary);
  await expect(capabilityStatement.resourceLink("Patient")).toHaveAttribute(
    "href",
    expected!.patientHref,
  );
});

test("typing filters resource capabilities live", async ({ capabilityStatement }) => {
  await capabilityStatement.goto();
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toBeVisible();

  await capabilityStatement.filter.fill("Patient");

  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toHaveCount(0);
});

test("the resource filter stacks inside the card at phone width", async ({
  page,
  capabilityStatement,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await capabilityStatement.goto();

  const header = page.locator(".cap-resource-card > .card-head");
  const filter = page.locator(".cap-resource-filter");
  await expect(header).toHaveCSS("flex-direction", "column");
  await expect(filter).toBeVisible();

  const headerBox = await header.boundingBox();
  const filterBox = await filter.boundingBox();
  expect(headerBox).not.toBeNull();
  expect(filterBox).not.toBeNull();
  expect(filterBox!.x).toBeGreaterThanOrEqual(headerBox!.x);
  expect(filterBox!.x + filterBox!.width).toBeLessThanOrEqual(
    headerBox!.x + headerBox!.width + 1,
  );
});
