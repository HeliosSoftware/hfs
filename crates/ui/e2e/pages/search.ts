// Natural-language + visual search (/ui/search). Only mounted when the NL
// translator is configured (HFS_NL_SEARCH_API_KEY set — the e2e server sets a
// placeholder). The NL pane translates text to a query; it never runs it.
import type { Page, Locator } from "@playwright/test";
import { SearchBuilder, SearchResults } from "./search-builder";

export class SearchPage {
  readonly builder: SearchBuilder;
  readonly results: SearchResults;

  constructor(readonly page: Page) {
    this.builder = new SearchBuilder(page);
    this.results = new SearchResults(page);
  }

  async goto(type?: string): Promise<void> {
    const q = type ? `?type=${encodeURIComponent(type)}` : "";
    await this.page.goto(`/ui/search${q}`, { waitUntil: "networkidle" });
  }

  get typeList(): Locator {
    return this.page.locator("#type-rail-list");
  }
  // Direct-child combinator keeps this scoped to the full, unfiltered list;
  // recently-used clones live in the sibling group above the scroller.
  railItem(type: string): Locator {
    return this.page.locator(`#type-rail-list > [data-type='${type}']`);
  }
  /** The "Recently used" group (#754/#755, server-rendered per page): hidden
   * until this page's own `rails.search.recent` has at least one entry. */
  get recentGroup(): Locator {
    return this.page.locator("#type-rail-recent");
  }
  recentItem(type: string): Locator {
    return this.page.locator(`#type-rail-recent [data-type='${type}']`);
  }
  async pickType(type: string): Promise<void> {
    await this.railItem(type).click();
  }

  modeButton(mode: "nl" | "builder"): Locator {
    return this.page.locator(`[data-mode-btn='${mode}']`);
  }
  get layout(): Locator {
    return this.page.locator(".search-page");
  }
  get nlPane(): Locator {
    return this.page.locator("#nl-pane");
  }
  get nlText(): Locator {
    return this.page.locator("#nl-text");
  }
  get nlSubmit(): Locator {
    return this.page.locator("#nl-submit");
  }
  get nlAnswer(): Locator {
    return this.page.locator("#nl-answer");
  }
  get nlExamples(): Locator {
    return this.page.locator("[data-nl-example]");
  }
  get setup(): Locator {
    return this.page.locator("#nl-setup");
  }
}
