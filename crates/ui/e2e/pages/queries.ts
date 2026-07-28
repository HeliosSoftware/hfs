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

  async goto(): Promise<void> {
    await this.page.goto("/ui/queries", { waitUntil: "networkidle" });
  }

  get railFilter(): Locator {
    return this.page.locator("#type-rail-filter");
  }
  railItem(type: string): Locator {
    return this.page.locator(`[data-rail-type='${type}']`);
  }
  count(type: string): Locator {
    return this.page.locator(`[data-count-for='${type}']`);
  }
  get savedList(): Locator {
    return this.page.locator("#saved-queries");
  }
  savedAction(action: "run" | "rename" | "delete"): Locator {
    return this.savedList.locator(`[data-action='${action}']`);
  }
}
