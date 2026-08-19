import { expect, test } from "@playwright/test";

// Phase 2 Slice A blocker smoke: the dashboard renders live cards fed by
// /health and /metadata?mode=terminology, the sidebar lists every canonical
// entry, and the language switcher lands us in Spanish via the hts_lang
// cookie. Wave 2 slices append their own specs beside this one.

test.describe("HTS dashboard (Phase 2 Slice A)", () => {
  test("responds at /ui/hts and renders the dashboard heading", async ({ page }) => {
    const response = await page.goto("/ui/hts");
    expect(response?.status(), "dashboard route must respond 200").toBe(200);
    await expect(page.getByRole("heading", { name: "Dashboard", exact: true })).toBeVisible();
  });

  test("cards row renders the status / backend / uptime / FHIR version tiles", async ({ page }) => {
    await page.goto("/ui/hts");
    for (const label of [
      "Status",
      "Backend",
      "Uptime",
      "FHIR version",
      "Loaded systems",
      "Bundled data",
    ]) {
      await expect(page.getByText(label, { exact: true })).toBeVisible();
    }
  });

  test("sidebar lists every canonical HTS UI section", async ({ page }) => {
    await page.goto("/ui/hts");
    for (const label of [
      "Dashboard",
      "Code Systems",
      "Value Sets",
      "Concept Maps",
      "Operations",
      "Import",
      "Diagnostics",
    ]) {
      await expect(page.locator("#sidebar nav").getByText(label, { exact: false })).toBeVisible();
    }
  });

  test("dialect chip shows the negotiated locale in the topbar", async ({ page }) => {
    await page.goto("/ui/hts");
    // The chip renders the effective BCP-47 tag in a <code> inside a
    // <details><summary>. First paint: "dialect: en".
    await expect(page.locator(".dialect-chip__value")).toContainText("en");
  });

  test("language switcher lands us in Spanish when we click ES", async ({ page }) => {
    await page.goto("/ui/hts");
    await page.getByRole("link", { name: "Spanish", exact: false }).click();
    // Spanish stub for hts-nav-dashboard is "Panel".
    await expect(page.getByRole("navigation").getByText("Panel", { exact: false })).toBeVisible();
    // …and the choice is sticky via the hts_lang cookie.
    const cookies = await page.context().cookies();
    expect(cookies.find((c) => c.name === "hts_lang")?.value).toBe("es");
  });
});
