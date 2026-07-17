// Tenant maintenance (/ui/tenants): the search filter (htmx), the add-tenant
// slide-over + create form, the table, and per-row delete.
import type { Page, Locator } from "@playwright/test";

export class TenantsPage {
  constructor(readonly page: Page) {}

  async goto(): Promise<void> {
    await this.page.goto("/ui/tenants", { waitUntil: "networkidle" });
  }

  get available(): Locator {
    // Present only when a tenant store is wired; otherwise a notice card shows.
    return this.page.locator("section.table-card");
  }
  get unavailableNotice(): Locator {
    return this.page.locator(".card.notice");
  }
  get search(): Locator {
    return this.page.locator(".toolbar__search input[name=q]");
  }
  get addToggle(): Locator {
    return this.page.locator("details.addbox > summary");
  }
  get addForm(): Locator {
    return this.page.locator("form[hx-post='/ui/tenants']");
  }
  get rows(): Locator {
    return this.page.locator("#tenant-rows tr");
  }
  row(id: string): Locator {
    return this.page.locator("#tenant-rows tr", { hasText: id });
  }

  async addTenant(id: string, displayName?: string): Promise<void> {
    await this.addToggle.click();
    await this.addForm.locator("input[name=id]").fill(id);
    if (displayName) await this.addForm.locator("input[name=display_name]").fill(displayName);
    await this.addForm.locator("button[type=submit]").click();
    await this.row(id).waitFor();
    // Collapse the slide-over so its panel stops overlaying the table below.
    if (await this.addToggle.evaluate((s) => (s.parentElement as HTMLDetailsElement).open)) {
      await this.addToggle.click();
    }
  }
}
