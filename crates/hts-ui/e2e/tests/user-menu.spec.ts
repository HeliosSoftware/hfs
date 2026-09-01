import { expect, test } from "@playwright/test";
import {
  assertLanguageRoundTrip,
  assertMenuShape,
  type MenuContext,
} from "../../../ui/e2e/pages/user-menu";

// #799: HTS's topbar used to render its own chrome — a `<span
// class="topbar__avatar">K</span>` beside a standalone `<nav
// class="lang-switcher">` that no stylesheet in the workspace styled. Both are
// gone; the account menu now comes from the ONE shared template
// (`crates/ui-chrome/templates/partials/user-menu.html`) that HFS renders too,
// via `Chrome::user_menu`.
//
// So the assertions are shared as well, imported straight out of the HFS
// suite's `pages/` directory. That import crosses a crate boundary on purpose:
// a second copy of these checks here is exactly the drift #799 removed. Read
// the header of `crates/ui/e2e/pages/user-menu.ts` before touching that module
// — it is deliberately free of any *value* import of `@playwright/test`,
// because the two e2e projects have separate `node_modules` and loading a
// second Playwright instance from inside this run would abort the whole suite.
//
// HTS has no `pages/` layer of its own (see the note in `a11y.spec.ts`), so
// there is no page object to hang this on; the shared module is imported
// directly, exactly as the bare `@playwright/test` fixtures are.

const HTS_MENU: MenuContext = {
  // The one genuine per-product difference: HFS persists the choice in
  // `hfs_lang`, HTS in `hts_lang` (`crates/hts-ui/src/i18n.rs`).
  langCookie: "hts_lang",
  home: "/ui/hts",
  favicon: "/ui/hts/assets/logo.png",
};

test.describe("HTS account menu (#799)", () => {
  // Chrome parity is asserted per page, not just on Home — the layout is
  // shared, so `home.spec.ts` and `import.spec.ts` both check the topbar and
  // this follows the same convention.
  for (const route of ["/ui/hts", "/ui/hts/import"]) {
    test(`the account menu matches HFS's on ${route}`, async ({ page }) => {
      await page.goto(route);
      await assertMenuShape(page, expect, HTS_MENU);
    });
  }

  test("the language segment switches locale and persists it in hts_lang", async ({
    page,
  }) => {
    await assertLanguageRoundTrip(page, expect, HTS_MENU);
  });
});
