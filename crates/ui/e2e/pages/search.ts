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

  async goto(): Promise<void> {
    await this.page.goto("/ui/search", { waitUntil: "networkidle" });
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
