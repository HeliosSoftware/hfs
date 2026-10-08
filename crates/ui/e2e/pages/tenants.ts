// Tenant maintenance (/ui/tenants): the search filter (htmx), the add-tenant
// slide-over + create form, the table, and per-row delete.
import { expect, type Page, type Locator } from "@playwright/test";

export class TenantsPage {
  constructor(readonly page: Page) {}

  // The page renders the registry at once and loads resource counts in the
  // background (#1851): while they are on their way the table polls itself,
  // so "networkidle" is the wrong signal. Wait for the page itself, and for
  // the counts through their explicit marker when a test needs them.
  async goto(): Promise<void> {
    await this.page.goto("/ui/tenants");
    await this.available.or(this.unavailableNotice).first().waitFor();
  }

  // The count status line's state (data-counts-state): "pending" and
  // "refreshing" are still moving; "ready", "partial", "stale",
  // "unavailable" and "unsupported" are settled, and nothing polls then.
  get countsState(): Locator {
    return this.page.locator("#tenant-counts-status [data-counts-state]");
  }

  // Waits until the counts settle. The page's own poller does the work
  // (2, 5, 15, then 30 s), so this only watches the marker.
  async waitCountsSettled(timeout = 120_000): Promise<void> {
    await expect(this.countsState).not.toHaveAttribute(
      "data-counts-state",
      /^(pending|refreshing)$/,
      { timeout },
    );
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
    // Provisioning runs in the background (#581): the POST returns as soon as
    // the id is accepted, and the row shows a spinner until `register_tenant`
    // + the conformance seed finish (~90s measured on CI's backend; NTFS dev
    // machines have been seen to exceed 150s — #553). Wait for the row to
    // settle into its deletable, spinner-free state rather than just
    // appearing; the wait is event-driven, so fast disks pay nothing extra.
    await this.row(id).locator("[hx-delete]").waitFor({ timeout: 300_000 });
    // Finishing provisioning makes the counts recount the new tenant's
    // seeded data; let that settle too, so the table stops polling before
    // the caller acts on the row.
    await this.waitCountsSettled();
    // Collapse the slide-over so its panel stops overlaying the table below.
    // The server already closes it on acceptance (well before settlement),
    // so this is normally a no-op — kept for callers/backends where it isn't.
    if (await this.addToggle.evaluate((s) => (s.parentElement as HTMLDetailsElement).open)) {
      await this.addToggle.click();
    }
  }
}
