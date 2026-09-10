// The CodeMirror-mounted ViewDefinition editor (#753/#821): a thin wrapper
// around `#vd-editor .cm-content`, shared by `vd-editor-lint.spec.ts` and
// `vd-editor-completion.spec.ts` so neither file repeats the same low-level
// CodeMirror plumbing.
//
// `setCursor` places the caret at an exact character offset through
// CodeMirror's own public API (`EditorView.findFromDOM` + `view.dispatch`,
// both part of `@codemirror/view`'s documented surface, exposed on the
// vendored bundle's `window.HfsCodeMirror` namespace) rather than clicking
// coordinates or pressing arrow keys a computed number of times — the
// document text this file's callers hand `setDoc` is known exactly (a plain
// template string, never `JSON.stringify`'s own formatting), so every test
// can compute the offset it needs with a plain `text.indexOf(...)` and land
// on it deterministically, in or out of the on-screen viewport.
import type { Locator, Page } from "@playwright/test";

/** Escapes `text` for literal use inside a `RegExp` — no library dependency
 * for the one place (`VdEditor.optionByLabel`) this file needs it. */
function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The offset of the `n`th (1-based) occurrence of `needle` in `text` — for
 * a document with more than one identical substring (two columns both named
 * `"id"`, say), where `setCursorAt`'s plain `indexOf` would always land on
 * the first one. */
export function nthIndexOf(text: string, needle: string, n: number): number {
  let pos = -1;
  for (let i = 0; i < n; i++) {
    pos = text.indexOf(needle, pos + 1);
    if (pos < 0) throw new Error(`occurrence ${n} of ${JSON.stringify(needle)} not found`);
  }
  return pos;
}

export class VdEditor {
  constructor(readonly page: Page) {}

  get cmContent(): Locator {
    return this.page.locator("#vd-editor .cm-content");
  }
  get textarea(): Locator {
    return this.page.locator("textarea[name='json']");
  }
  /** The hover/cursor tooltip a lint diagnostic's range shows — one or more
   * `.cm-diagnostic` `<li>` elements, each with its message and fix buttons. */
  get lintTooltip(): Locator {
    return this.page.locator(".cm-tooltip-lint");
  }
  /** The bottom lint panel (`openLintPanel`) — same `.cm-diagnostic` markup
   * as the tooltip, one `<li>` per diagnostic in the whole document. */
  get lintPanel(): Locator {
    return this.page.locator(".cm-panel.cm-panel-lint");
  }
  /** The completion popup (`@codemirror/autocomplete`'s own tooltip). */
  get completionPopup(): Locator {
    return this.page.locator(".cm-tooltip-autocomplete");
  }
  get completionOptions(): Locator {
    return this.completionPopup.locator("ul li[role='option']");
  }
  get gutterErrorMarkers(): Locator {
    return this.page.locator(".cm-gutter-lint .cm-lint-marker-error");
  }

  /** Replaces the whole document with `text` as one input event — the same
   * technique `sql-view-definitions.spec.ts` uses (#838): a single
   * `insertText` never gives CodeMirror's `closeBrackets` extension a lone
   * "{"/"\"" keystroke to pair, so the resulting document is `text` exactly,
   * byte for byte.
   *
   * Waits for the resulting server lint round trip (`POST .../lint`, fired
   * ~400ms after the doc settles — see `vd-editor.js`'s own `CM.linter`
   * delay) before returning, so every diagnostic-driven assertion right
   * after `setDoc` sees a *completed* pass for exactly this text rather
   * than racing it — a state-based wait (NF1), not a fixed sleep. The
   * listener is armed only after the clearing `Ctrl+A`/`Delete` above, so it
   * cannot resolve on that intermediate (empty-document) pass instead of
   * the one `text` itself produces. */
  async setDoc(text: string): Promise<void> {
    await this.cmContent.click();
    await this.page.keyboard.press("ControlOrMeta+a");
    await this.page.keyboard.press("Delete");
    const linted = this.page.waitForResponse(
      (response) =>
        response.url().includes("/sql/view-definitions/lint") && response.request().method() === "POST",
    );
    await this.page.keyboard.insertText(text);
    await linted;
  }

  /** Places the caret at the UTF-16 offset `pos` into the document and
   * focuses the editor — see the module doc comment for why this, rather
   * than a click or counted arrow-key presses, is this file's one way to
   * reach an exact position. */
  async setCursor(pos: number): Promise<void> {
    await this.cmContent.evaluate((dom, offset) => {
      const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
      const view = CM.EditorView.findFromDOM(dom);
      if (!view) throw new Error("no CodeMirror view mounted on #vd-editor .cm-content");
      view.dispatch({ selection: { anchor: offset } });
      view.focus();
    }, pos);
  }

  /** Moves the caret to `marker`'s own position in the *current* document —
   * `setDoc`'s own `text` is always available to callers already, so this
   * just saves every call site its own `indexOf`. */
  async setCursorAt(text: string, marker: string): Promise<void> {
    const pos = text.indexOf(marker);
    if (pos < 0) throw new Error(`marker ${JSON.stringify(marker)} not found in document`);
    await this.setCursor(pos);
  }

  /** Moves the caret to right *after* `marker` — the common case for a
   * completion test, which almost always wants "positioned right where the
   * next keystroke belongs", not the start of some earlier, already-typed
   * text. */
  async setCursorAfter(text: string, marker: string): Promise<void> {
    const pos = text.indexOf(marker);
    if (pos < 0) throw new Error(`marker ${JSON.stringify(marker)} not found in document`);
    await this.setCursor(pos + marker.length);
  }

  /** The completion popup's own `<li>` whose visible label (`.cm-completionLabel`,
   * never its detail) is exactly `label` — `buildKeyOption`/`buildFhirpathOption`
   * (`vd-editor.js`) always set `label` to the bare candidate name, so an
   * exact match never collides with a longer one sharing a prefix (`path`
   * vs. a hypothetical `pathologyReport`). */
  optionByLabel(label: string): Locator {
    return this.completionOptions.filter({
      has: this.page.locator(".cm-completionLabel", { hasText: new RegExp(`^${escapeRegExp(label)}$`) }),
    });
  }

  /** The mounted editor's own live text (always in sync with the hidden
   * textarea `code-editor.js` writes on every change). */
  doc(): Promise<string> {
    return this.textarea.inputValue();
  }
}
