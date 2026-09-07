// Bulk Import submission detail (/ui/bulk-import/{id}): the summary metadata
// (manifest URL included — one-shot submissions carry exactly one), the
// status fragment, and the submission log.
import type { APIRequestContext, Locator, Page } from "@playwright/test";

export class BulkImportPage {
  constructor(readonly page: Page) {}

  /** Creates a submission and returns its detail path, without navigating. */
  async seed(request: APIRequestContext, name = "e2e-bulk-import-detail"): Promise<string> {
    const response = await request.post("/ui/bulk-import", {
      form: {
        name,
        manifest_url: "https://example.test/manifest.json",
        auth: "none",
      },
      maxRedirects: 0,
    });
    const location = response.headers()["location"];
    if (!location || !location.startsWith("/ui/bulk-import/")) {
      throw new Error(
        `seeding a bulk-import submission did not redirect to detail (got ${response.status()} ${location ?? "no Location"})`,
      );
    }
    return location;
  }

  async seedAndGoto(request: APIRequestContext, name = "e2e-bulk-import-detail"): Promise<string> {
    const location = await this.seed(request, name);
    await this.page.goto(location, { waitUntil: "networkidle" });
    return location;
  }

  get summary(): Locator {
    return this.page.locator("section.card.panel.bulk-import-section");
  }

  get summaryGrid(): Locator {
    return this.summary.locator(":scope > .kv-grid");
  }

  get backLink(): Locator {
    return this.page.locator("a.back-link[href='/ui/bulk-import']");
  }

  get deleteButton(): Locator {
    return this.summary.locator("form[action$='/delete'] > button");
  }

  get logCard(): Locator {
    return this.page
      .locator("section.table-card")
      .filter({ has: this.page.getByRole("heading", { name: "Submission Log" }) });
  }

  get logEmptyState(): Locator {
    return this.logCard.locator(".empty-state");
  }

  get statusCard(): Locator {
    return this.page.locator("#bulk-status");
  }

  get statusCell(): Locator {
    return this.page.locator("#submission-status");
  }

  /** The log's entries, newest first, as the browser currently renders them. */
  async logLines(): Promise<string[]> {
    const entries = this.logCard.locator("pre.detail__code");
    if ((await entries.count()) === 0) return [];
    return (await entries.innerText())
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
  }
}
