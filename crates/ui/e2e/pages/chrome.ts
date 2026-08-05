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
}
