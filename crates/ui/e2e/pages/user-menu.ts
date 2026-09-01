// The account menu (#725, unified across products by #799): one `<details
// class="menu menu--user">` in `.topbar__tools` holding the identity card, the
// EN/ES/DE language segment, and — once a login flow exists — the sign-out row.
// The markup is rendered from ONE template, `crates/ui-chrome/templates/
// partials/user-menu.html`, for both HFS (`/ui`) and HTS (`/ui/hts`), so these
// assertions are shared too: a copy per suite is exactly the drift #799 exists
// to make impossible.
//
// ─────────────────────────────────────────────────────────────────────────────
// WHY THIS FILE IMPORTS `@playwright/test` TYPE-ONLY AND TAKES `expect` AS AN
// ARGUMENT — do not "fix" either of those.
//
// `crates/ui/e2e` and `crates/hts-ui/e2e` are two independent npm projects with
// two separate `node_modules` (there is no root package.json and no npm
// workspace). This module physically lives under `crates/ui/e2e`, so a *value*
// import of `@playwright/test` here would always resolve to
// `crates/ui/e2e/node_modules/@playwright/test`. When the HTS runner — booted
// from `crates/hts-ui/e2e/node_modules` — loads a spec that imports this file,
// that second Playwright instance gets initialized inside a run owned by the
// first and the whole suite dies with:
//
//     Error: Playwright Test did not expect test() to be called here.
//
// A `import type { … }` is erased by Playwright's esbuild transform and
// resolves nothing at runtime, which is why the types below are safe. `expect`
// cannot be a type, so every assertion helper receives the *caller's* `expect`
// as a parameter instead.
//
// It also has to live under `crates/ui/e2e` specifically: `.github/workflows/
// ui-tests.yml` does a `docker cp crates/ui/e2e` into the test container, so a
// shared module parked anywhere else would simply not exist in CI.
// ─────────────────────────────────────────────────────────────────────────────
import type { Expect, Locator, Page } from "@playwright/test";

/** The per-product facts about an otherwise identical menu. The language
 * cookie name is the only genuine difference between HFS and HTS; `home` and
 * `favicon` differ only because the two UIs are mounted at different prefixes. */
export type MenuContext = {
  /** Set by the `?lang=` middleware — `crates/ui/src/i18n.rs` (HFS) and
   * `crates/hts-ui/src/i18n.rs` (HTS). */
  langCookie: "hfs_lang" | "hts_lang";
  /** The product's landing route, e.g. "/ui" or "/ui/hts". */
  home: string;
  /** The `<link rel="icon">` target, e.g. "/ui/assets/logo.png". */
  favicon: string;
};

/** The avatar disclosure in the topbar. */
export function userMenu(page: Page): Locator {
  return page.locator("details.menu.menu--user");
}

/** Opens the disclosure, idempotently.
 *
 * The `<details>` ships closed, so every interaction with the panel has to open
 * it first — and it must be opened by *clicking the summary*, never by
 * `evaluate(d => (d.open = true))`: the `nojs` project runs with
 * `javaScriptEnabled: false`, where that evaluate never executes. The click is
 * the native disclosure toggle and works in both projects. */
export async function openUserMenu(page: Page): Promise<void> {
  const menu = userMenu(page);
  if ((await menu.getAttribute("open")) === null) {
    await menu.locator("summary").click();
  }
}

/** A language option inside the menu: a real `?lang=` link, so it works with
 * JavaScript off. */
export function langLink(page: Page, lang: string): Locator {
  return page.locator(`.menu--user a[href*='lang=${lang}']`);
}

const LANGS = ["en", "es", "de"] as const;

/** Asserts the whole shape of the account menu on the page that is ALREADY
 * loaded — this helper never navigates, so callers can run it against several
 * routes (HTS checks chrome parity per page).
 *
 * Covers both halves of #799: what the menu must now contain, and the two
 * things it must no longer contain (HTS's standalone `<nav class="lang-
 * switcher">` and its hardcoded "K" avatar).
 *
 * Leaves the disclosure open; that state is page-local and dies at the next
 * navigation. */
export async function assertMenuShape(
  page: Page,
  expect: Expect,
  ctx: MenuContext,
): Promise<void> {
  const tools = page.locator(".topbar__tools");
  const menu = userMenu(page);

  // Exactly one, in the topbar, closed at rest.
  await expect(tools.locator("details.menu--user")).toHaveCount(1);
  expect(
    await menu.getAttribute("open"),
    "the account menu must ship closed",
  ).toBeNull();

  // Gone (#799): the unstyled standalone switcher and the placeholder initial.
  // `.lang-switcher` has no rule in crates/ui/assets/app.css and HFS never had
  // the element — crates/ui/tests/router_http.rs asserts its absence there.
  const summary = menu.locator("summary.topbar__avatar");
  await expect(page.locator(".lang-switcher")).toHaveCount(0);
  await expect(summary).not.toHaveText("K");

  // The other topbar control is untouched.
  await expect(tools.locator(".theme-toggle")).toHaveCount(1);

  // The avatar: labelled for screen readers, and — with no IdP photo and no
  // initials to fall back to — bottoming out at the generic user icon.
  await expect(summary).toHaveAccessibleName(/\S/);
  await expect(summary.locator("svg")).toHaveCount(1);

  // The panel is a native disclosure: hidden until the summary is clicked.
  const panel = menu.locator(".menu__panel.menu__panel--right");
  await expect(panel).toBeHidden();
  await openUserMenu(page);
  await expect(panel).toBeVisible();

  // The anonymous identity card. Neither product has a signed-in principal
  // yet (#320), so both render exactly this.
  await expect(panel.locator(".user-menu__name")).toHaveText("Anonymous user");
  await expect(panel.locator(".user-menu__hint")).toHaveText(
    "Authentication is disabled",
  );

  // The language segment: three real links, visible codes, translated
  // accessible names, and precisely one marked current.
  const options = panel.locator(".user-menu__seg a");
  await expect(options).toHaveCount(3);
  await expect(options).toHaveText(["EN", "ES", "DE"]);
  for (const lang of LANGS) {
    await expect(langLink(page, lang)).toHaveAttribute("href", `?lang=${lang}`);
  }
  await expect(panel.locator(".user-menu__seg a[aria-current='true']")).toHaveCount(1);

  // No sign-out row anywhere: it is `{% if user.can_logout %}`-gated and
  // nothing can log in yet, so nothing can log out either.
  await expect(page.locator(".user-menu__out")).toHaveCount(0);
  await expect(page.locator('a[href$="/logout"]')).toHaveCount(0);

  // The favicon the browser tab actually shows, and proof it is served.
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute("href", ctx.favicon);
  const icon = await page.request.get(ctx.favicon);
  expect(icon.status(), `${ctx.favicon} must be served`).toBe(200);
}

/** Drives the menu's language segment end to end: EN → ES → back to EN.
 *
 * Restoring English is not politeness, it is required — both configs run
 * `fullyParallel: false, workers: 1` against one shared server, and the choice
 * is sticky in a cookie, so a test that left Spanish behind would silently
 * retranslate every later spec's assertions. */
export async function assertLanguageRoundTrip(
  page: Page,
  expect: Expect,
  ctx: MenuContext,
): Promise<void> {
  await page.goto(ctx.home);

  await switchLanguage(page, "es");
  await expect(page.locator("html")).toHaveAttribute("lang", "es");

  const cookies = await page.context().cookies();
  expect(
    cookies.find((c) => c.name === ctx.langCookie)?.value,
    `the choice must persist in the ${ctx.langCookie} cookie`,
  ).toBe("es");

  // Back to English so the shared server is left as we found it.
  await switchLanguage(page, "en");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
}

/** Opens the menu, clicks a language link, and waits for the navigation it
 * starts to actually commit.
 *
 * The wait is the whole point. `click()` resolves when the click is dispatched,
 * not when the page it navigates to has loaded, so asserting straight after it
 * races a document swap: the execution context the locator resolved against is
 * torn down mid-poll. It passes on a warm server and fails on a cold one — this
 * cost one 30s timeout on the first run after a fresh build, which read as a
 * broken language switch rather than a racy assertion. */
async function switchLanguage(page: Page, lang: string): Promise<void> {
  await openUserMenu(page);
  await Promise.all([
    page.waitForURL(new RegExp(`[?&]lang=${lang}(&|$)`)),
    langLink(page, lang).click(),
  ]);
}
