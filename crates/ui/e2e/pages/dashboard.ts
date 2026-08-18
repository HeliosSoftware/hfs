// Landing dashboard (/ui): stat cards, the resources-over-time chart, the
// time-window selector, and the per-type legend series selector.
import type { Page, Locator } from "@playwright/test";

export class DashboardPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<void> {
    await this.page.goto(`/ui${query}`, { waitUntil: "networkidle" });
  }

  get statCards(): Locator {
    return this.page.locator(".card.stat");
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
  windowOption(label: RegExp | string): Locator {
    return this.page.locator(".window-picker__option", { hasText: label });
  }
  get legendItems(): Locator {
    return this.page.locator(".chart-legend__item");
  }
}
