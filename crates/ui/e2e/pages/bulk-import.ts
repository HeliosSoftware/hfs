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

  // --- Import workspace (/ui/bulk-import) -----------------------------------

  async goto(): Promise<void> {
    await this.page.goto("/ui/bulk-import", { waitUntil: "networkidle" });
  }

  /** The "New Submission" trigger — the `<summary>` of the modal disclosure. */
  get newSubmission(): Locator {
    return this.page.locator("details.addbox--modal > summary");
  }

  get createDialog(): Locator {
    return this.page.locator("details.addbox--modal [role='dialog']");
  }

  get submissionsTable(): Locator {
    return this.page.locator("section.table-card table.data-table");
  }

  /** A submissions-table row by the submission name in its first cell. */
  row(name: string): Locator {
    return this.submissionsTable.locator("tbody tr").filter({
      has: this.page.getByRole("link", { name, exact: true }),
    });
  }

  // --- Submission detail (/ui/bulk-import/{id}) ------------------------------

  get summary(): Locator {
    return this.page.locator("section.card.panel.bulk-import-section");
  }

  /** The summary card specifically — the status card shares the other classes. */
  get summaryCard(): Locator {
    return this.page.locator("section.bulk-import-summary-card");
  }

  /** The value of one summary field, addressed by its visible label. */
  summaryField(label: string): Locator {
    return this.summaryCard
      .locator(".detail__field")
      .filter({ has: this.page.locator(`span:text-is(${JSON.stringify(label)})`) })
      .locator("code, div")
      .first();
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

  /** The progress bar, present only while the recipient reports in-progress. */
  get progressBar(): Locator {
    return this.statusCard.locator("[role='progressbar']");
  }

  /** The recipient's own progress sentence, e.g.
   * "Processing 42% - 1,300 Resources written". */
  get progressText(): Locator {
    return this.statusCard.locator(".detail__field--wide > div").last();
  }

  /** The value of one status/result field, addressed by its visible label. */
  statusField(label: string): Locator {
    return this.statusCard
      .locator(".detail__field")
      .filter({ has: this.page.locator(`span:text-is(${JSON.stringify(label)})`) })
      .locator("div")
      .last();
  }

  /** The resource counter the recipient reports, or `null` when it has not
   * reported one yet. This is the number #969 is about: it is a running total
   * over every run of the manifest, so it must never walk backwards. */
  async resourcesWritten(): Promise<number | null> {
    if ((await this.progressText.count()) === 0) return null;
    const text = await this.progressText.innerText();
    const match = /([\d,]+)\s+Resources written/.exec(text);
    return match ? Number(match[1].replace(/,/g, "")) : null;
  }
}
