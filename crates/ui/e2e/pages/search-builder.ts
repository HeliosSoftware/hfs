// The shared FHIR query builder (partials/search-builder.html), used on the
// Search and Resources pages: an editable GET URL and Run,
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
  get status(): Locator { return this.page.locator("#query-search-status"); }
  get cancel(): Locator { return this.page.locator("#query-search-cancel"); }
  get elapsed(): Locator { return this.page.locator("#query-search-elapsed"); }
  get slow(): Locator { return this.page.locator("#query-search-slow"); }
  get keepWaiting(): Locator { return this.page.locator("#query-search-keep-waiting"); }
  get sort(): Locator { return this.page.locator("#query-results-sort"); }

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
  /** The open typeahead listbox(es) appended to `body` by `typeahead.js`. */
  get typeaheadListboxes(): Locator {
    return this.page.locator("body > .typeahead__listbox");
  }
  get typeaheadVisibleListbox(): Locator {
    return this.page.locator("body > .typeahead__listbox:not([hidden])");
  }
  get typeaheadOptions(): Locator {
    return this.typeaheadVisibleListbox.locator(".typeahead__option");
  }
  get typeaheadOptionValues(): Locator {
    return this.typeaheadVisibleListbox.locator(".typeahead__value");
  }
  get typeaheadComboboxes(): Locator {
    return this.page.locator("#builder-sections input[role='combobox']");
  }
  get chainRows(): Locator {
    return this.page.locator("#builder-conditions .builder-row--chain");
  }
  get hasRows(): Locator {
    return this.page.locator("#builder-conditions .builder-row--has");
  }
  /** The inline "not a search parameter" message of a flagged row. */
  rowError(row: Locator): Locator {
    return row.locator(":scope > .builder-row__error");
  }
  get flaggedInputs(): Locator {
    return this.page.locator("#builder-conditions [aria-invalid='true']");
  }
  get plainText(): Locator {
    return this.page.locator("#query-plain-text");
  }
  get plainUnknown(): Locator {
    return this.page.locator("#query-plain-unknown");
  }
  drillButton(row: Locator): Locator {
    return row.locator("[data-chain-from]");
  }

  async run(query: string): Promise<void> {
    await this.setUrl(query);
    await this.page.waitForFunction(() => {
      const button = document.querySelector<HTMLButtonElement>("[data-intent='run']");
      return !!button && !button.disabled;
    });
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

  async waitDone(): Promise<void> {
    await this.page.locator("#query-search-status").waitFor({ state: "hidden" });
    await this.card.waitFor({ state: "visible" });
  }

  /** Cancel restores diagnostics as well as the previous successful page.
   * Keep this distinct from visibleState: a failed page request preserves the
   * page while deliberately changing its diagnostic. */
  async stableState() {
    return {
      ...(await this.visibleState()),
      error: (await this.error.textContent()) || "",
      errorVisible: await this.error.isVisible(),
    };
  }

  async visibleState(): Promise<{
    rows: string[];
    meta: string;
    note: string;
    prevVisible: boolean;
    prevUrl: string | null;
    nextVisible: boolean;
    nextUrl: string | null;
  }> {
    return {
      rows: await this.rows.allInnerTexts(),
      meta: await this.meta.innerText(),
      note: await this.note.innerText(),
      prevVisible: await this.prev.isVisible(),
      prevUrl: await this.prev.getAttribute("data-url"),
      nextVisible: await this.next.isVisible(),
      nextUrl: await this.next.getAttribute("data-url"),
    };
  }
}
