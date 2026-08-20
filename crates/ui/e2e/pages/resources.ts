// The Resources workspace (/ui/resources): the type rail with live counts, the
// filter box, "Create new", and the edit modal (Edit + History tabs). The modal
// hosts the shared Editor component and the in-modal history diff.
import type { Page, Locator } from "@playwright/test";
import { Editor } from "./editor";
import { SearchBuilder, SearchResults } from "./search-builder";

export class ResourcesPage {
  readonly modal: ResourceModal;
  readonly builder: SearchBuilder;
  readonly results: SearchResults;

  constructor(readonly page: Page) {
    this.modal = new ResourceModal(page);
    this.builder = new SearchBuilder(page);
    this.results = new SearchResults(page);
  }

  async goto(type?: string): Promise<void> {
    const q = type ? `?type=${encodeURIComponent(type)}` : "";
    await this.page.goto(`/ui/resources${q}`, { waitUntil: "networkidle" });
  }

  get railFilter(): Locator {
    return this.page.locator("#type-rail-filter");
  }
  get createButton(): Locator {
    return this.page.locator("#resource-create");
  }
  /** The type-naming text inside Create ("Create new Patient"), #605. */
  get createLabel(): Locator {
    return this.page.locator("#resource-create .resources-create__label");
  }

  // Direct-child combinator: `#type-rail-recent` (the "Recently used" group,
  // #603) nests inside `#type-rail-list` too, and its clones carry the same
  // `data-type`. `>` keeps this scoped to the full, unfiltered list.
  railItem(type: string): Locator {
    return this.page.locator(`#type-rail-list > [data-type='${type}']`);
  }
  count(type: string): Locator {
    return this.page.locator(`#type-rail-list > [data-type='${type}'] .count`);
  }

  /** Every resource type the rail offers, in DOM order. */
  async railTypes(): Promise<string[]> {
    return this.page.$$eval("#type-rail-list > a.filter-rail__item[data-type]", (els) =>
      els.map((e) => (e as HTMLElement).dataset.type!).filter(Boolean),
    );
  }

  /** Rail items currently visible (not filtered out). */
  async visibleRailTypes(): Promise<string[]> {
    return this.page.$$eval("#type-rail-list > a.filter-rail__item[data-type]", (els) =>
      els
        .filter((e) => (e as HTMLElement).offsetParent !== null)
        .map((e) => (e as HTMLElement).dataset.type!),
    );
  }

  /** The "Recently used" group (#603): hidden until at least one entry. */
  get recentGroup(): Locator {
    return this.page.locator("#type-rail-recent");
  }
  recentItem(type: string): Locator {
    return this.page.locator(`#type-rail-recent [data-type='${type}']`);
  }

  async pickType(type: string): Promise<void> {
    await this.railItem(type).click();
  }

  /** Pick a type (if given) then open the create modal. */
  async openCreate(type?: string): Promise<void> {
    if (type) await this.pickType(type);
    await this.createButton.click();
    await this.modal.waitOpen();
  }
}

export class ResourceModal {
  readonly editor: Editor;
  readonly history: ModalHistory;

  constructor(readonly page: Page) {
    this.editor = new Editor(page, page.locator("#resource-editor-body"));
    this.history = new ModalHistory(page);
  }

  get root(): Locator {
    return this.page.locator("#resource-modal");
  }
  get subject(): Locator {
    return this.page.locator("#resource-modal-subject");
  }
  get status(): Locator {
    return this.page.locator("#resource-modal-status");
  }
  get saveButton(): Locator {
    return this.page.locator("#resource-save");
  }
  get deleteButton(): Locator {
    return this.page.locator("#resource-delete");
  }

  tab(name: "edit" | "history"): Locator {
    return this.page.locator(`[data-modal-tab='${name}']`);
  }

  async waitOpen(): Promise<void> {
    await this.root.waitFor({ state: "visible" });
    // #editor-doc is a hidden input — wait for it in the DOM, not "visible".
    await this.editor.doc.waitFor({ state: "attached" });
  }

  async close(): Promise<void> {
    // The × button, not the backdrop (which sits behind the editor grid).
    await this.page.locator(".modal__x").click();
    await this.root.waitFor({ state: "hidden" });
  }

  async closeWithEscape(): Promise<void> {
    await this.page.keyboard.press("Escape");
    await this.root.waitFor({ state: "hidden" });
  }

  async save(): Promise<void> {
    await this.saveButton.click();
  }

  async showHistory(): Promise<void> {
    await this.tab("history").click();
  }

  /** The id the modal subject settled on after a save, if any. */
  async savedId(): Promise<string | undefined> {
    const subject = (await this.subject.textContent()) ?? "";
    return subject.match(/[A-Za-z]+\/(\S+)/)?.[1];
  }
}

/** The version history diff embedded in the modal's History tab. */
export class ModalHistory {
  constructor(readonly page: Page) {}

  get versions(): Locator {
    return this.page.locator("#resource-history-versions .history-version");
  }
  get fromSelect(): Locator {
    return this.page.locator("#resource-history-from");
  }
  get toSelect(): Locator {
    return this.page.locator("#resource-history-to");
  }
  get metadataCheckbox(): Locator {
    return this.page.locator("#resource-history-metadata");
  }
  get diff(): Locator {
    return this.page.locator("#resource-history-diff");
  }
}
