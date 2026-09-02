// SearchParameter registry viewer (/ui/search-parameters): the filter rail
// (htmx live search), type/source facet chips, the results table with row
// selection, pagination, and the detail panel.
import type { Page, Locator } from "@playwright/test";

export class SearchParametersPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<void> {
    await this.page.goto(`/ui/search-parameters${query}`, { waitUntil: "networkidle" });
  }

  get railSearch(): Locator {
    return this.page.locator(".filter-rail__search input[name=q]");
  }
  get railList(): Locator {
    return this.page.locator("#sp-rail-list");
  }
  get railRecent(): Locator {
    return this.page.locator("#sp-rail-recent");
  }
  railItem(type: string): Locator {
    return this.page.locator(`#sp-rail-list [data-type='${type}']`);
  }
  /** The "All types" row: the one `#sp-rail-list` item with no `data-type`. */
  get allTypesLink(): Locator {
    return this.page.locator("#sp-rail-list > a.filter-rail__item:not([data-type])");
  }
  recentItem(type: string): Locator {
    return this.page.locator(`#sp-rail-recent [data-type='${type}']`);
  }

  facetChip(label: RegExp | string): Locator {
    return this.page.locator(".facets a.chip", { hasText: label });
  }
  get rows(): Locator {
    return this.page.locator("table.data-table tbody tr");
  }
  get rowLinks(): Locator {
    return this.page.locator("a.row-link");
  }
  get pagination(): Locator {
    return this.page.locator("nav.pagination");
  }
  get detailTitle(): Locator {
    return this.page.locator(".detail .detail__title");
  }
}
