// Landing dashboard (/ui): stat cards, the resources-over-time chart, the
// time-window selector, the type picker, and the per-series legend.
import type { Page, Locator } from "@playwright/test";

export class DashboardPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<void> {
    await this.page.goto(`/ui${query}`, { waitUntil: "networkidle" });
  }

  get statCards(): Locator {
    return this.page.locator(".card.stat");
  }
  /** The "Resource types" card; its `.stat__sub` names the effective FHIR
   * version ("used for R4", #553). */
  get resourceTypesCard(): Locator {
    return this.page.locator(".card.stat", { hasText: "Resource types" });
  }
  get exportJobsCard(): Locator {
    return this.page.locator("a.card.stat", { hasText: "Export jobs" });
  }
  get importJobsCard(): Locator {
    return this.page.locator("a.card.stat", { hasText: "Import jobs" });
  }
  get chart(): Locator {
    return this.page.locator("svg.chart");
  }
  get seriesLines(): Locator {
    return this.page.locator("svg.chart polyline.series");
  }
  windowOption(label: RegExp | string): Locator {
    return this.page.locator(".window-picker__option", { hasText: label });
  }
  get legendItems(): Locator {
    return this.page.locator(".chart-legend__item");
  }
  get picker(): Locator {
    return this.page.locator(".chart-pick");
  }
  async openPicker(): Promise<void> {
    if ((await this.picker.getAttribute("open")) === null) {
      await this.picker.locator("summary").click();
    }
  }
  pickerOption(type: string): Locator {
    return this.page.locator(`[data-pick-name="${type}"]`);
  }
  get pickerFilter(): Locator {
    return this.page.locator("[data-pick-filter]");
  }
  /** "View all resources" (#599): offers every type of the active FHIR
   * version, not just the ones the tenant stores. */
  get viewAllToggle(): Locator {
    return this.page.locator(".chart-pick__option--all");
  }
  get tooltip(): Locator {
    return this.page.locator("#chart-tip");
  }
  get dataTableToggle(): Locator {
    return this.page.locator(".chart-table > summary");
  }

  /** Reloads until the chart shows at least one plotted series — outlasts the
   * 15s dashboard snapshot cache after seeding. */
  async waitForSeries(): Promise<void> {
    for (let attempt = 0; attempt < 12; attempt++) {
      if ((await this.seriesLines.count()) > 0) return;
      await this.page.waitForTimeout(2000);
      await this.page.reload({ waitUntil: "networkidle" });
    }
    throw new Error("no chart series appeared after seeding");
  }
}
