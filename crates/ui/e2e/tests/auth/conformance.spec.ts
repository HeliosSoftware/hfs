import { test, expect } from "../../pages/fixtures";

// #320, leg 1: auth enabled AND an outbound service token provisioned
// (HFS_OUTBOUND_BEARER_TOKEN, minted by boot.mjs against the throwaway JWKS).
// The UI's self-fetch of /SearchParameter and /CompartmentDefinition carries
// that token, so the conformance pages must render real data — no carve-outs
// for the loopback call.

test("search parameters render real registry data under auth", async ({
  page,
  searchParameters,
}) => {
  await searchParameters.goto();
  // No degraded-fetch warning...
  await expect(page.locator(".notice--warn")).toHaveCount(0);
  // ...and the full spec registry came through the authenticated self-call.
  await expect(searchParameters.rows.first()).toBeVisible();
  const total = await page.locator("#sp-rail-list .filter-rail__item .count").first().innerText();
  expect(Number(total.replace(/\D/g, ""))).toBeGreaterThan(500);
});

test("compartments render the five spec definitions under auth", async ({ page }) => {
  await page.goto("/ui/compartments", { waitUntil: "networkidle" });
  await expect(page.locator(".notice--warn")).toHaveCount(0);
  for (const code of ["Patient", "Encounter", "Practitioner", "RelatedPerson", "Device"]) {
    await expect(page.locator(".filter-rail__item", { hasText: code })).toBeVisible();
  }
});

test("a direct FHIR call without a token is still rejected", async ({ request }) => {
  // The pages work because the self-call carries the service token — not
  // because auth grew a loophole. An anonymous API call stays a 401.
  const bare = await request.get("/SearchParameter?_count=1", {
    headers: { Accept: "application/fhir+json" },
  });
  expect(bare.status()).toBe(401);
});
