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

  get resourceFilterVersion(): Locator {
    return this.page.locator('.cap-resource-filter input[name="version"]');
  }

  get resourceTable(): Locator {
    return this.page.locator("#cap-resource-table");
  }

  resourceRow(resourceType: string): Locator {
    return this.resourceTable.locator("tbody tr", {
      has: this.page.getByRole("link", { name: resourceType, exact: true }),
    });
  }

  resourceLink(resourceType: string): Locator {
    return this.resourceRow(resourceType).getByRole("link", {
      name: resourceType,
      exact: true,
    });
  }

  get fhirVersionSummary(): Locator {
    return this.page
      .locator(".detail__field")
      .filter({ has: this.page.getByText("FHIR version", { exact: true }) })
      .locator(":scope > div");
  }

  get rawDisclosure(): Locator {
    return this.page.locator("#capability-json-fold");
  }

  get rawSummary(): Locator {
    return this.rawDisclosure.locator(":scope > summary");
  }

  get rawBody(): Locator {
    return this.page.locator("#capability-json-body");
  }

  get rawLoadLink(): Locator {
    return this.rawBody.getByRole("link", { name: "Open plain JSON" });
  }

  get rawLoading(): Locator {
    return this.rawBody.locator("#capability-json-loading");
  }

  get collapseAllJson(): Locator {
    return this.rawDisclosure.locator('[data-capability-json-fold="all"]');
  }

  get expandAllJson(): Locator {
    return this.rawDisclosure.locator('[data-capability-json-fold="none"]');
  }

  get jsonView(): Locator {
    return this.rawBody.locator("#capability-json");
  }

  get jsonOutline(): Locator {
    return this.rawBody.locator(":scope > .capability-json-outline");
  }

  jsonNode(scope: Locator, key: string): Locator {
    return scope
      .locator("summary.capability-json-row")
      .filter({ hasText: `"${key}":` })
      .locator("..")
      .first();
  }

  nodeBody(node: Locator): Locator {
    return node.locator(":scope > [data-capability-json-body]");
  }

  pageControl(scope: Locator, direction: "previous" | "next"): Locator {
    return scope.locator(`[data-capability-json-page-direction="${direction}"]`);
  }
}
