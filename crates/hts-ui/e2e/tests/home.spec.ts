import { expect, test } from "@playwright/test";

// Phase 2 Slice A blocker smoke: the Home page renders live cards fed by
// /health and /metadata?mode=terminology, the sidebar lists every canonical
// entry, and the language switcher lands us in Spanish via the hts_lang
// cookie. Wave 2 slices append their own specs beside this one.
//
// Formerly `dashboard.spec.ts` (renamed 2026-08-20 alongside the Fluent
// `hts-nav-dashboard` → `hts-nav-home` collapse and the module rename
// `crates/hts-ui/src/dashboard.rs` → `home.rs` for HFS parity — see
// `edson/docs/hts-ui-design.md` §7.1).

test.describe("HTS home (Phase 2 Slice A)", () => {
  test("responds at /ui/hts and renders the Home heading", async ({ page }) => {
    const response = await page.goto("/ui/hts");
    expect(response?.status(), "home route must respond 200").toBe(200);
    await expect(page.getByRole("heading", { name: "Home", exact: true })).toBeVisible();
  });

  test("cards row renders the status / backend / uptime / FHIR version tiles", async ({ page }) => {
    await page.goto("/ui/hts");
    // Scope the assertion to the Home cards region. Since the sidebar's
    // FHIR-version selector added by the 2026-08-20 visual-parity work
    // also renders "FHIR version" as its menu heading, the labels only
    // become unambiguous when we anchor at `.hts-home`.
    const cards = page.locator(".hts-home");
    for (const label of [
      "Status",
      "Backend",
      "Uptime",
      "FHIR version",
      "Loaded systems",
      "Bundled data",
    ]) {
      await expect(cards.getByText(label, { exact: true }).first()).toBeVisible();
    }
  });

  test("metrics row renders the Requests + Avg latency tiles", async ({ page }) => {
    // The two Wave-2 tiles are wired to `/metrics` in Home::fetch() and no
    // longer hardcode an em-dash. The values may still legitimately be
    // "—" the very first paint if the histogram count is zero (edge case
    // right at boot), so we only assert the labels are present — the raw
    // presence of the tiles is what proves the wiring.
    await page.goto("/ui/hts");
    for (const label of ["Requests", "Avg latency"]) {
      await expect(page.getByText(label, { exact: true })).toBeVisible();
    }
  });

  test("sidebar lists every canonical HTS UI section", async ({ page }) => {
    await page.goto("/ui/hts");
    for (const label of [
      "Home",
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
    // Spanish stub for hts-nav-home is "Inicio" (mirrors HFS `nav-home = Inicio`).
    await expect(page.getByRole("navigation").getByText("Inicio", { exact: false })).toBeVisible();
    // …and the choice is sticky via the hts_lang cookie.
    const cookies = await page.context().cookies();
    expect(cookies.find((c) => c.name === "hts_lang")?.value).toBe("es");
  });
});
