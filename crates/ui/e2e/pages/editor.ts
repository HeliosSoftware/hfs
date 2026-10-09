// The shared Guided form in the standalone editor, Resources modal,
// ViewDefinition editor, and both Library editors. Pass the host root.
import { expect, type Locator, type Page } from "@playwright/test";

export class Editor {
  constructor(
    readonly page: Page,
    readonly root: Locator,
  ) {}

  // The hidden field holding the in-flight document (guided mode).
  get doc(): Locator {
    return this.root.locator("#editor-doc");
  }
  // The JSON pane's <textarea>: the source of truth the code editor mirrors
  // into (hidden once CodeMirror is mounted, visible without the bundle).
  get source(): Locator {
    return this.root.locator("#editor-source");
  }
  // The mounted code editor of the JSON card (#1756).
  get codeEditor(): Locator {
    return this.root.locator(".code-editor--resource");
  }
  get cm(): Locator {
    return this.codeEditor.locator(".cm-content");
  }
  get form(): Locator {
    return this.root.locator("#editor-form");
  }
  get validity(): Locator {
    return this.root.locator(".editor-validity");
  }
  get formatButton(): Locator {
    return this.root.locator("[data-editor-format]");
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

  /** The JSON pane's current text (the textarea is kept in step with the
   * code editor on every change). */
  async jsonText(): Promise<string> {
    return this.source.inputValue();
  }

  /** Replaces the JSON pane's text as one input event, whichever way it is
   * shown: typed into the code editor, or filled into the textarea when the
   * bundle did not load. */
  async setJson(text: string): Promise<void> {
    if (await this.cm.isVisible().catch(() => false)) {
      await this.cm.click();
      await this.page.keyboard.press("ControlOrMeta+a");
      await this.page.keyboard.press("Delete");
      await this.page.keyboard.insertText(text);
    } else {
      await this.source.fill(text);
    }
  }

  /** Replaces the JSON with `doc`, leaving the editor as the source of truth
   * (a Save from here reads the editor's text). The guided form catches up on
   * its own after the pause; use `applyJson` to wait for that. */
  async fillRaw(doc: unknown): Promise<void> {
    await this.setJson(JSON.stringify(doc, null, 2));
  }

  /** Replaces the JSON with `doc` and waits for the guided form to have
   * re-rendered and re-validated against it. */
  async applyJson(doc: unknown): Promise<void> {
    await this.fillRaw(doc);
    await this.formCaughtUp();
  }

  /** Waits until the guided form's document is the JSON pane's document. */
  async formCaughtUp(): Promise<void> {
    await expect
      .poll(
        async () => {
          try {
            return JSON.stringify(JSON.parse(await this.source.inputValue())) ===
              JSON.stringify(JSON.parse(await this.doc.inputValue()));
          } catch {
            return false;
          }
        },
        { timeout: 10_000 },
      )
      .toBe(true);
  }

  /** Kept for specs written for the old two-mode pane: the pane is always
   * editable now, so this only waits for it. */
  async enterRaw(): Promise<void> {
    await this.source.waitFor({ state: "attached" });
  }

  /** Kept for specs written for the old two-mode pane: waits for the form to
   * have caught up with what was typed. */
  async leaveRaw(): Promise<void> {
    await this.formCaughtUp();
  }

  /** Puts the caret at UTF-16 offset `pos` of the editor's document. */
  async setCursor(pos: number): Promise<void> {
    // The row link walks the syntax tree: let it cover the whole document.
    await this.syntaxReady();
    await this.cm.evaluate((dom, offset) => {
      const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
      const view = CM.EditorView.findFromDOM(dom);
      if (!view) throw new Error("no CodeMirror view mounted on the JSON pane");
      view.dispatch({ selection: { anchor: offset } });
      view.focus();
    }, pos);
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

  // Fold controls (revealed once the code editor mounts).
  async collapseAll(): Promise<void> {
    await this.syntaxReady();
    await this.root.locator("[data-editor-fold='all']").click();
  }
  /** Waits until CodeMirror has parsed the whole document: folding works on
   * the syntax tree, so a fold command run mid-parse folds nothing. */
  async syntaxReady(): Promise<void> {
    await expect
      .poll(() =>
        this.cm.evaluate((dom) => {
          const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
          const view = CM.EditorView.findFromDOM(dom);
          return CM.syntaxTree(view.state).length === view.state.doc.length;
        }),
      )
      .toBe(true);
  }
  async expandAll(): Promise<void> {
    await this.root.locator("[data-editor-fold='none']").click();
  }
  /** Number of folded regions currently shown in the code editor. */
  foldedCount(): Promise<number> {
    return this.codeEditor.locator(".cm-foldPlaceholder").count();
  }

  // Add-node panel.
  get addPanel(): Locator {
    return this.root.locator(".editor-add").first();
  }
  /** Opens the add panel if it isn't already — the root picker auto-opens on
   *  an empty document (#547), and a blind summary click would close it. */
  async openAddPanel(): Promise<void> {
    if ((await this.addPanel.getAttribute("open")) === null) {
      await this.addPanel.locator("summary").first().click();
    }
  }
  addFilter(): Locator {
    return this.addPanel.locator(".editor-add__filter");
  }
  addItem(name: string): Locator {
    return this.addPanel.locator(`[data-add-name='${name}']`);
  }
  /** The add panel's own × close control (#1239). */
  addClose(): Locator {
    return this.addPanel.locator("[data-add-close]");
  }
  /** Add is announced without a visible success block. */
  get addStatus(): Locator {
    return this.root.locator("[data-add-status]");
  }
  addUndo(): Locator {
    return this.root.locator("[data-add-undo-note]:not([hidden]) [data-add-undo]");
  }
  /** Append to the actual array header, rather than the parent's picker. */
  collectionAdd(path: string): Locator {
    return this.rowAt(path).locator("[data-collection-add]");
  }
  /** The Elements/Extensions accordion group inside the first add panel. */
  addGroup(name: "extensions"): Locator {
    return this.addPanel.locator(`details.editor-add__group[data-add-group='${name}']`);
  }
  /** Unfolds the Extensions group if it isn't already — it stays folded by
   * default (#1239), unlike Elements. */
  async openExtensions(): Promise<void> {
    const group = this.addGroup("extensions");
    if ((await group.getAttribute("open")) === null) {
      await group.locator("summary").first().click();
    }
  }
}
