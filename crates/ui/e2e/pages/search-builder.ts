// The shared FHIR query builder (partials/search-builder.html), used on the
// Queries, Search, and Resources pages: an editable GET URL, Run/Save intents,
// condition/include/control rows, the Recent disclosure, and the results card.
import type { Page, Locator } from "@playwright/test";

export class SearchBuilder {
  constructor(readonly page: Page) {}

  get form(): Locator {
    return this.page.locator("#saved-query-form");
  }
  get url(): Locator {
    return this.page.locator("input.query-builder__url[name=url]");
  }
  get copyButton(): Locator {
    return this.page.locator("#query-copy");
  }
  get runButton(): Locator {
    return this.page.locator("[data-intent='run']");
  }
  get saveButton(): Locator {
    return this.page.locator("[data-intent='save']");
  }
  get nameInput(): Locator {
    return this.page.locator("#saved-query-form input[name=name]");
  }
  get error(): Locator {
    return this.page.locator("#search-error");
  }
  get recentToggle(): Locator {
    return this.page.locator(".query-recent > summary");
  }
  get recentPanel(): Locator {
    return this.page.locator("#recent-searches");
  }

  get sections(): Locator {
    return this.page.locator("#builder-sections");
  }
  addButton(kind: "condition" | "include" | "control" | "has" | "include-fwd" | "include-rev"): Locator {
    return this.page.locator(`[data-add='${kind}']`);
  }
  get conditionRows(): Locator {
    return this.page.locator("#builder-conditions .builder-row");
  }
  get paramOptions(): Locator {
    return this.page.locator("#param-options option");
  }
  get chainRows(): Locator {
    return this.page.locator("#builder-conditions .builder-row--chain");
  }
  get hasRows(): Locator {
    return this.page.locator("#builder-conditions .builder-row--has");
  }
  drillButton(row: Locator): Locator {
    return row.locator("[data-chain-from]");
  }

  async run(query: string): Promise<void> {
    await this.url.fill(query);
    await this.runButton.click();
  }

  /** Set the URL and fire `change` so the builder parses it and reveals the
   * condition/include/control sections (hidden while the URL is empty). */
  async setUrl(query: string): Promise<void> {
    await this.url.fill(query);
    await this.url.dispatchEvent("change");
    // Flush the native change (dirty-value flag) now, not mid-interaction.
    await this.url.blur();
    await this.sections.waitFor({ state: "visible" });
  }
}

export class SearchResults {
  constructor(readonly page: Page) {}

  get card(): Locator {
    return this.page.locator("#query-results");
  }
  get meta(): Locator {
    return this.page.locator("#query-results-meta");
  }
  get openTab(): Locator {
    return this.page.locator("#query-results-open");
  }
  get rows(): Locator {
    return this.page.locator("#query-results-body tr");
  }
  get note(): Locator {
    return this.page.locator("#query-results-note");
  }
  get error(): Locator {
    return this.page.locator("#query-results-error");
  }
  get prev(): Locator {
    return this.page.locator("#query-results-prev");
  }
  get next(): Locator {
    return this.page.locator("#query-results-next");
  }

  async waitShown(): Promise<void> {
    await this.card.waitFor({ state: "visible" });
  }

  async visibleState(): Promise<{
    rows: string[];
    meta: string;
    openHref: string | null;
    note: string;
    prevVisible: boolean;
    prevUrl: string | null;
    nextVisible: boolean;
    nextUrl: string | null;
  }> {
    return {
      rows: await this.rows.allInnerTexts(),
      meta: await this.meta.innerText(),
      openHref: await this.openTab.getAttribute("href"),
      note: await this.note.innerText(),
      prevVisible: await this.prev.isVisible(),
      prevUrl: await this.prev.getAttribute("data-url"),
      nextVisible: await this.next.isVisible(),
      nextUrl: await this.next.getAttribute("data-url"),
    };
  }
}
