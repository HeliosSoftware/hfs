// CompartmentDefinition viewer + membership tester (/ui/compartments). The rail,
// tabs, and tester are all link/GET-form based, so they work without JS.
import { expect, type Page, type Locator } from "@playwright/test";

export class CompartmentsPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<void> {
    await this.page.goto(`/ui/compartments${query}`, { waitUntil: "networkidle" });
  }
  /** Land on the tester tab (default Patient compartment) by clicking through. */
  async gotoTester(): Promise<void> {
    await this.goto();
    await this.tab(/test/i).click();
    await this.page.waitForLoadState("networkidle");
    await this.tester.waitFor();
  }

  get railItems(): Locator {
    return this.page.locator(".filter-rail__item");
  }
  /**
   * One rail item by its compartment code. There is no `data-*` attribute on
   * this rail (unlike the type rails) — the code is the item's own visible
   * text, and the five spec codes share no prefix, so a text filter is
   * unambiguous.
   */
  railItem(code: string): Locator {
    return this.railItems.filter({ hasText: code });
  }
  tab(label: RegExp | string): Locator {
    return this.page.locator("nav.tabs a.tab", { hasText: label });
  }
  /**
   * Click a rail item or tab and wait until the swapped page marks it current.
   * `networkidle` alone is not enough after a boosted click: when the page is
   * already idle it can resolve before htmx's request starts, so the next
   * step would still read the previous definition's controls.
   */
  async selectDefinition(code: string): Promise<void> {
    await this.railItem(code).click();
    await expect(this.railItem(code)).toHaveAttribute("aria-current", "true");
    await this.page.waitForLoadState("networkidle");
  }
  async openTab(label: RegExp | string): Promise<void> {
    await this.tab(label).click();
    await expect(this.tab(label)).toHaveAttribute("aria-current", "true");
    await this.page.waitForLoadState("networkidle");
  }

  // Tester form.
  get tester(): Locator {
    return this.page.locator("form.tester");
  }
  get idInput(): Locator {
    return this.tester.locator("[name=id]");
  }
  get targetInput(): Locator {
    return this.tester.locator("[name=target]");
  }
  get runButton(): Locator {
    return this.tester.locator("button[type=submit]");
  }
  get resultTitle(): Locator {
    return this.page.locator(".tester-result__title");
  }

  async runTester(id: string, target: string): Promise<void> {
    await this.idInput.fill(id);
    await this.targetInput.fill(target);
    await this.runButton.click();
    await this.page.waitForLoadState("networkidle");
  }
}
