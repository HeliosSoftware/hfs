import { test, expect } from "../pages/fixtures";

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
