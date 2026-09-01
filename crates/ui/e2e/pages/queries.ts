// Saved Queries workspace (/ui/queries): the resource picker rail with live
// counts, the shared query builder + results, and the saved-query list.
import type { Page, Locator } from "@playwright/test";
import { SearchBuilder, SearchResults } from "./search-builder";

export class QueriesPage {
  readonly builder: SearchBuilder;
  readonly results: SearchResults;

  constructor(readonly page: Page) {
    this.builder = new SearchBuilder(page);
    this.results = new SearchResults(page);
  }

  async goto(type?: string): Promise<void> {
    const q = type ? `?type=${encodeURIComponent(type)}` : "";
    await this.page.goto(`/ui/queries${q}`, { waitUntil: "networkidle" });
  }

  get railFilter(): Locator {
    return this.page.locator("#type-rail-filter");
  }
  get typeList(): Locator {
    return this.page.locator("#type-rail-list");
  }
  // Direct-child combinator keeps this scoped to the full, unfiltered list;
  // recently-used clones live in the sibling group above the scroller.
  railItem(type: string): Locator {
    return this.page.locator(`#type-rail-list > [data-type='${type}']`);
  }
  count(type: string): Locator {
    return this.page.locator(`#type-rail-list > [data-type='${type}'] .count`);
  }
  /** The "Recently used" group (#754/#755, server-rendered per page): hidden
   * until this page's own `rails.queries.recent` has at least one entry. */
  get recentGroup(): Locator {
    return this.page.locator("#type-rail-recent");
  }
  recentItem(type: string): Locator {
    return this.page.locator(`#type-rail-recent [data-type='${type}']`);
  }
  async pickType(type: string): Promise<void> {
    await this.railItem(type).click();
  }
  get savedList(): Locator {
    return this.page.locator("#saved-queries");
  }
  savedAction(action: "run" | "rename" | "delete"): Locator {
    return this.savedList.locator(`[data-action='${action}']`);
  }
}
