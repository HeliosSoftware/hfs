import { test, expect } from "../pages/fixtures";

// The CompartmentDefinition viewer and its membership tester (/ui/compartments).
// The tester is a plain GET form that resolves against the same codegen'd table
// the REST compartment handler uses — so the four outcomes here are the API's.

test("the compartment rail and tabs render", async ({ compartments }) => {
  await compartments.goto();
  // The five spec compartments (Device/Encounter/Patient/Practitioner/RelatedPerson).
  await expect(compartments.railItems).toHaveCount(5);
  await expect(compartments.tab(/test/i)).toBeVisible();
});

test("tester: a linked type is a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Observation");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--ok/);
});

test("tester: the compartment's own type is a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Patient");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--ok/);
  await expect(compartments.resultTitle).toContainText(/member/i);
});

test("tester: an unlinked type is not a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Medication");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--danger/);
});

test("tester: the wildcard target fans out across member types", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "*");
  // Fan-out title reports a count of member types, not an ok/danger verdict.
  await expect(compartments.resultTitle).toBeVisible();
  await expect(compartments.resultTitle).not.toHaveClass(/--danger/);
});

// CRUD (#237): the stored definitions carry ids, so the definition tab offers
// Edit (editor deep-link) and Delete; New sits in the page head. The delete
// round-trip itself is exercised on the SearchParameter page — same script —
// so the seeded compartments stay intact for the other tests.
test("the definition tab offers New, Edit, and Delete", async ({ page, compartments }) => {
  await compartments.goto();
  await expect(page.locator(".page-head__actions a.btn--primary")).toHaveAttribute(
    "href",
    "/ui/editor?type=CompartmentDefinition",
  );
  await expect(page.locator(".detail__actions a.btn")).toHaveAttribute(
    "href",
    /\/ui\/editor\?type=CompartmentDefinition&id=./,
  );
  await expect(page.locator(".detail__actions [data-crud-delete]")).toBeVisible();
});
