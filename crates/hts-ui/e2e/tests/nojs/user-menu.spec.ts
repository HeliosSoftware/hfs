import { expect, test } from "@playwright/test";
import {
  assertLanguageRoundTrip,
  assertMenuShape,
  openUserMenu,
  type MenuContext,
} from "../../../../ui/e2e/pages/user-menu";

// The `nojs` project (`javaScriptEnabled: false`, see
// `crates/hts-ui/e2e/playwright.config.ts`). Its `testMatch` of
// `**/nojs/**/*.spec.ts` matched ZERO files until this one landed with #799 —
// the project was configured and then never populated, so HTS advertised the
// no-JavaScript ring without running it. This file activates it.
//
// The account menu is the right first inhabitant: it is a native `<details>`
// disclosure whose language options are real `?lang=` links, so it is meant to
// work with scripting off, and that promise is untested by the chromium ring
// (where a hypothetical JS-driven menu would pass just as well). HFS proves
// the same property in `crates/ui/e2e/tests/nojs/progressive-enhancement.spec.ts`.
//
// `openUserMenu` clicks the summary rather than setting `.open`; an
// `evaluate()`-based open would be a silent no-op here.

const HTS_MENU: MenuContext = {
  langCookie: "hts_lang",
  home: "/ui/hts",
  favicon: "/ui/hts/assets/logo.png",
};

test("the account menu renders and opens with no JavaScript", async ({ page }) => {
  await page.goto("/ui/hts");
  await assertMenuShape(page, expect, HTS_MENU);
});

test("the language options are plain links with no JavaScript", async ({ page }) => {
  await assertLanguageRoundTrip(page, expect, HTS_MENU);
});

test("the disclosure closes again when the summary is clicked twice", async ({ page }) => {
  // Native `<details>` toggling, with nothing scripted behind it.
  await page.goto("/ui/hts");
  const menu = page.locator("details.menu.menu--user");
  await openUserMenu(page);
  await expect(menu).toHaveAttribute("open", "");
  await menu.locator("summary").click();
  await expect(menu).not.toHaveAttribute("open", /.*/);
});
