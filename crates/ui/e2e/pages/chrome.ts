// The persistent app chrome (layouts/base.html): the sidebar rail (expands on
// hover, #438), theme buttons, and the language switcher. On every full page.
import type { Page, Locator } from "@playwright/test";

export type Theme = "light" | "dark";

export class AppChrome {
  readonly sidebar: Locator;
  readonly langSwitcher: Locator;

  constructor(readonly page: Page) {
    this.sidebar = page.locator("aside.sidebar");
    this.langSwitcher = page.locator(".lang-switcher");
  }

  /** A primary nav link by its destination path, e.g. "/ui/resources". */
  navLink(href: string): Locator {
    return this.page.locator(`a.nav-item[href='${href}']`);
  }

  /** A disabled "coming soon" nav entry by its visible label. */
  soonItem(label: string): Locator {
    return this.page.locator("span.nav-item--soon", { hasText: label });
  }

  themeButton(theme: Theme): Locator {
    return this.page.locator(`[data-set-theme='${theme}']`);
  }

  /** The current theme as stamped on <html data-theme>. */
  currentTheme(): Promise<string | null> {
    return this.page.locator("html").getAttribute("data-theme");
  }

  /** Seed the cached theme before first paint, the way a returning user has it. */
  async seedTheme(theme: Theme): Promise<void> {
    await this.page.addInitScript((t) => {
      try {
        localStorage.setItem("hfs-theme", t as string);
      } catch {}
    }, theme);
  }

  langLink(lang: string): Locator {
    return this.page.locator(`.lang-switcher a[href*='lang=${lang}']`);
  }

  /** The FHIR-version disclosure in the sidebar footer (#343): a `<details>`
   * with one `<form method=post action=/ui/version>` per compiled version, so
   * it works without JS too. */
  get versionSelector(): Locator {
    return this.page.locator(".sidebar__foot details.menu");
  }

  async openVersionSelector(): Promise<void> {
    if ((await this.versionSelector.getAttribute("open")) === null) {
      await this.versionSelector.locator("summary").click();
    }
  }

  /** Every FHIR version this binary was compiled with, in spec order — the
   * options the sidebar offers (#600: multi-version builds show more than
   * the single-version default's one entry). */
  async versionOptions(): Promise<string[]> {
    return this.page.$$eval(".sidebar__foot form input[name='version']", (els) =>
      els.map((e) => (e as HTMLInputElement).value),
    );
  }

  /** The version the sidebar currently marks as active. */
  async currentVersion(): Promise<string> {
    await this.openVersionSelector();
    const value = await this.page
      .locator(".sidebar__foot form:has(button[aria-current='true']) input[name='version']")
      .getAttribute("value");
    if (!value) throw new Error("no FHIR version is marked current in the sidebar");
    return value;
  }

  /** Submits the sidebar's `POST /ui/version` for the given version — the
   * same round trip the disclosure's own form does. */
  async selectVersion(version: string): Promise<void> {
    await this.openVersionSelector();
    // The click submits a real form (`POST /ui/version`); the server
    // redirects back. Callers that immediately assert on the response — as
    // opposed to following up with their own `.goto()` — should await
    // `page.waitForLoadState("networkidle")` themselves afterward.
    await this.page
      .locator(`.sidebar__foot form:has(input[value='${version}']) button[type='submit']`)
      .click();
    await this.page.waitForLoadState("networkidle");
  }
}
