// Bulk Import submission detail (/ui/bulk-import/{id}): the summary metadata,
// status fragment, manifest list, and submission log.
import type { APIRequestContext, Locator, Page } from "@playwright/test";

export class BulkImportPage {
  constructor(readonly page: Page) {}

  async seedAndGoto(request: APIRequestContext, name = "e2e-bulk-import-detail"): Promise<string> {
    const response = await request.post("/ui/bulk-import", {
      form: {
        name,
        recipient_base_url: "http://127.0.0.1:9/fhir",
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

  get manifestsCard(): Locator {
    return this.page.locator("section.bulk-import-manifests-card");
  }

  get logCard(): Locator {
    return this.page
      .locator("section.table-card")
      .filter({ has: this.page.getByRole("heading", { name: "Submission Log" }) });
  }

  get manifestEmptyState(): Locator {
    return this.manifestsCard.locator(".bulk-import-manifest-empty");
  }

  get logEmptyState(): Locator {
    return this.logCard.locator(".empty-state");
  }
}
