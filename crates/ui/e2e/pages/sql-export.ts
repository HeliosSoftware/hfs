// Active SQL Exports (/ui/sql/export) and its builder (/ui/sql/export/new),
// #833. Card-internal controls (status chip, overflow actions, "View files")
// are addressed by role/text straight off `SqlExportPage.card()` in specs,
// matching the rest of this Page Object Model — this class only owns what
// every spec needs to name more than once: navigation and the builder's form.
import type { Locator, Page } from "@playwright/test";

export class SqlExportPage {
  constructor(readonly page: Page) {}

  async goto(): Promise<void> {
    await this.page.goto("/ui/sql/export", { waitUntil: "networkidle" });
  }

  async gotoNew(): Promise<void> {
    await this.page.goto("/ui/sql/export/new", { waitUntil: "networkidle" });
  }

  get newButton(): Locator {
    return this.page.getByRole("link", { name: "New SQL Export" });
  }

  get notice(): Locator {
    return this.page.locator(".notice");
  }

  get lede(): Locator {
    return this.page.locator(".page-head__lede");
  }

  /** One job card on the list, matched by its name (RF19: the subjects'
   * names when the job has none of its own — the only kind the form offers
   * today). Several cards can share a name (e.g. after Run again); narrow
   * further with `.first()`/`.nth()`, which the list's most-recent-first
   * order makes deterministic. */
  card(name: string): Locator {
    return this.page.locator(".job-card").filter({ hasText: name });
  }

  // --- The builder (/ui/sql/export/new) ---

  subjectCheckbox(reference: string): Locator {
    return this.page.locator(`input[name="subject"][value="${reference}"]`);
  }

  get formatSelect(): Locator {
    return this.page.locator("#export-format");
  }

  get startButton(): Locator {
    return this.page.locator("form[action='/ui/sql/export'] button[type='submit']");
  }
}
