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

  get rawCard(): Locator {
    return this.page.locator("#capability-json-fold");
  }

  get rawHeader(): Locator {
    return this.rawCard.locator(":scope > .card-head");
  }

  get rawActions(): Locator {
    return this.rawHeader.locator("[data-capability-json-actions]");
  }

  get collapseAll(): Locator {
    return this.rawActions.locator("[data-capability-json-collapse-all]");
  }

  get expandAll(): Locator {
    return this.rawActions.locator("[data-capability-json-expand-all]");
  }

  get rawStatus(): Locator {
    return this.rawActions.locator("[data-capability-json-status]");
  }

  get rawBody(): Locator {
    return this.page.locator("#capability-json-body");
  }

  get rawLoadLink(): Locator {
    return this.rawCard.getByRole("link", { name: "Open plain JSON" });
  }

  get jsonTree(): Locator {
    return this.rawBody.locator(":scope > [data-capability-json-tree]");
  }

  get jsonOutline(): Locator {
    return this.jsonTree.locator(":scope > [data-capability-json-page]");
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
