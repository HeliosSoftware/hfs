// The schema-driven resource editor. The same fragment is used in two places:
// the Resources modal (root = #resource-editor-body) and the standalone
// /ui/editor page (root = #editor-body). Every control inside carries the same
// id/data-* contract, so one component drives both — pass the root locator.
import type { Locator, Page } from "@playwright/test";

export class Editor {
  constructor(
    readonly page: Page,
    readonly root: Locator,
  ) {}

  // The hidden field holding the in-flight document (guided mode).
  get doc(): Locator {
    return this.root.locator("#editor-doc");
  }
  // The raw <textarea> (raw mode source of truth).
  get source(): Locator {
    return this.root.locator("#editor-source");
  }
  get rawToggle(): Locator {
    return this.root.locator("#editor-json-edit");
  }
  get form(): Locator {
    return this.root.locator("#editor-form");
  }
  get validity(): Locator {
    return this.root.locator(".editor-validity");
  }
  get jsonView(): Locator {
    return this.root.locator("#json-view");
  }

  /** Parsed in-flight document (guided-mode hidden field). */
  async currentDoc(): Promise<Record<string, unknown>> {
    return JSON.parse(await this.doc.inputValue());
  }

  /** Number of validation issues the editor is reporting. */
  async errorCount(): Promise<number> {
    return Number((await this.form.getAttribute("data-error-count")) ?? "0");
  }

  async isValid(): Promise<boolean> {
    return (await this.errorCount()) === 0;
  }

  /** Enter raw mode and replace the JSON, staying in raw (textarea is the source
   * of truth — a Save from here reads the textarea directly). */
  async fillRaw(doc: unknown): Promise<void> {
    await this.enterRaw();
    await this.source.fill(JSON.stringify(doc, null, 2));
  }

  /** Enter raw mode, replace the JSON, then toggle back so the guided form
   * re-renders and re-validates against the typed document. */
  async applyJson(doc: unknown): Promise<void> {
    await this.fillRaw(doc);
    await this.leaveRaw();
  }

  async enterRaw(): Promise<void> {
    if (await this.source.isHidden().catch(() => true)) {
      await this.rawToggle.click();
    }
    await this.source.waitFor({ state: "visible" });
  }

  async leaveRaw(): Promise<void> {
    await this.rawToggle.click();
    await this.jsonView.waitFor({ state: "visible" });
  }

  /** The guided-form row at the exact dotted path (`gender`, `name.0`) —
   * text matching went ambiguous once add panels list profiled extensions
   * whose names embed other fields' names (genderIdentity vs gender). */
  row(field: string): Locator {
    return this.root.locator('.editor-row[data-path="' + field + '"]');
  }

  rowError(field: string): Locator {
    return this.row(field).locator(".editor-row__error");
  }

  /** The row at an exact dotted path — `name.0.family`, the validator's form. */
  rowAt(path: string): Locator {
    return this.root.locator(`.editor-row[data-path='${path}']`);
  }

  // Fold controls.
  async collapseAll(): Promise<void> {
    await this.root.locator("[data-json-fold='all']").click();
  }
  async expandAll(): Promise<void> {
    await this.root.locator("[data-json-fold='none']").click();
  }
  hiddenLineCount(): Promise<number> {
    return this.root.locator(".json-line[hidden]").count();
  }

  // Add-node panel.
  get addPanel(): Locator {
    return this.root.locator(".editor-add").first();
  }
  addFilter(): Locator {
    return this.root.locator(".editor-add__filter").first();
  }
  addItem(name: string): Locator {
    return this.root.locator(`[data-add-name='${name}']`).first();
  }
}
