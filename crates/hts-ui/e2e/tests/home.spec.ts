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

  test("one row of four consolidated tiles, per the V3 mockup", async ({ page }) => {
    await page.goto("/ui/hts");
    // Until 2026-08-28 this was eight tiles across three rows. Backend,
    // FHIR version, Bundled data and Avg latency no longer have tiles of
    // their own — each folds into the `.stat__sub` of the tile it
    // qualifies, which is what the approved mockup draws.
    const cards = page.locator(".hts-home");
    await expect(cards.locator("article.card.stat")).toHaveCount(4);
    for (const label of ["Status", "Uptime", "Loaded code systems", "Requests"]) {
      await expect(cards.getByText(label, { exact: true }).first()).toBeVisible();
    }
  });

  test("the folded-in values survive as tile sub-lines", async ({ page }) => {
    // The consolidation must not lose data. Every value that used to have a
    // tile still renders, one line lower. Values themselves can legitimately
    // be em-dashes at first paint (a zero-count histogram right at boot), so
    // assert on the sub-line prose that frames them.
    await page.goto("/ui/hts");
    const cards = page.locator(".hts-home");
    await expect(cards.locator(".stat__sub").first()).toContainText(/backend/i);
    await expect(cards).toContainText(/no restarts since/i);
    await expect(cards).toContainText(/bundled on disk|TerminologyCapabilities/i);
    await expect(cards).toContainText(/average · from \/metrics|Since server start/i);
  });

  test("sidebar lists every canonical HTS UI section", async ({ page }) => {
    await page.goto("/ui/hts");
    for (const label of [
      "Home",
      "Code Systems",
      "Value Sets",
      "Concept Maps",
      "Import",
      // HFS's own label, shared via the workspace Fluent catalog. This
      // entry was called "Diagnostics" until 2026-08-27. The "Operations"
      // entry that used to sit before Import went with the Operations page.
      "Capability & Conformance",
    ]) {
      await expect(page.locator("#sidebar nav").getByText(label, { exact: false })).toBeVisible();
    }
  });

  test("the topbar carries exactly HFS's three controls, and nothing more", async ({ page }) => {
    await page.goto("/ui/hts");
    const tools = page.locator(".topbar__tools");
    // HFS's topbar is the language switcher, the theme toggle and the
    // avatar — in that order. HTS is the same system to an operator, so its
    // chrome must not diverge.
    await expect(tools.locator(".lang-switcher")).toHaveCount(1);
    await expect(tools.locator(".theme-toggle")).toHaveCount(1);
    await expect(tools.locator(".topbar__avatar")).toHaveText("K");
    // A fourth control — a "dialect: en" disclosure — was removed on
    // 2026-08-28. It shipped non-functional in its first commit and HFS has
    // no counterpart. Nothing in the topbar may open a menu.
    await expect(tools.locator("details")).toHaveCount(0);
    await expect(page.locator(".dialect-chip, .dialect-chip__value")).toHaveCount(0);
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

  test("naked `/` redirects to `/ui/hts` home page", async ({ page }) => {
    // Reviewer contract: the bare root URL sends operators to the HTS UI
    // home so they never see the FHIR batch POST-only landing (bare `/`
    // would otherwise 405 on GET). The redirect lives in
    // `crates/hts/src/server.rs::create_app` inside the `ui_enabled`
    // branch, gated so a UI-off deployment keeps its 405 instead of
    // landing on a 404 at `/ui/hts`. Playwright follows the 308
    // transparently, so the assertion is: final URL under `/ui/hts` and
    // the Home heading rendered. Mirrors the redirect-follows pattern
    // used by the CS/VS/CM detail landing tests. The E2E fixture sets
    // `HTS_UI_ENABLED=true` in boot.mjs so this route is registered.
    const response = await page.goto("/");
    expect(response?.status(), "root should land at 200 after the 308").toBe(200);
    expect(page.url()).toContain("/ui/hts");
    await expect(
      page.getByRole("heading", { name: "Home", exact: true }),
    ).toBeVisible();
  });
});
