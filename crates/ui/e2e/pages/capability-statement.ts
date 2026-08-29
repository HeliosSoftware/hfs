// Live CapabilityStatement viewer (/ui/capability-statement): system and
// resource capabilities, the progressively enhanced type filter, and raw JSON.
import type { Locator, Page } from "@playwright/test";

export class CapabilityStatementPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<void> {
    await this.page.goto(`/ui/capability-statement${query}`, { waitUntil: "networkidle" });
  }

  get filter(): Locator {
    return this.page.locator('input[name="filter"]');
  }

  get resourceTable(): Locator {
    return this.page.locator("#cap-resource-table");
  }

  resourceRow(resourceType: string): Locator {
    return this.resourceTable.locator("tbody tr", {
      has: this.page.getByRole("link", { name: resourceType, exact: true }),
    });
  }
}
