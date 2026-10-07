/*
 * Shared CodeMirror 6 mount helper (#838, generalized out of
 * #753's original `vd-editor.js`).
 *
 * Every editor in this crate — the ViewDefinition JSON editor, the SQL
 * pane editors, the Library Details JSON editor (#840) — needs the exact
 * same progressive-enhancement contract over its `<textarea>`:
 *
 *   - the textarea stays in the DOM and stays the form's source of truth,
 *     so Save/Duplicate/Enter (plain POSTs) keep submitting what the editor
 *     shows, with or without this script;
 *   - every document change is written straight back into `textarea.value`
 *     and fires a bubbling `input` event, for parity with a native input and
 *     for any other script listening on the form;
 *   - the editor exposes the textarea's own `aria-label` on its own
 *     `role="textbox"` content, so the hidden textarea does not shadow it;
 *   - the editor's own scroller is reachable by Tab even when it grows tall
 *     enough to scroll internally (`contentAttributes tabindex="0"`, #840 —
 *     axe's `scrollable-region-focusable` check does not credit a bare
 *     `contenteditable` as focusable content for its scrolling ancestor);
 *   - Tab never indents. It moves focus to the next form control, like it
 *     does over a plain textarea, except that an editor mounted with
 *     `options.completion` lets Tab accept the highlighted completion while
 *     the popup is open (see `options.completion`);
 *   - the wrapper and the `EditorView` are built fully in memory and the
 *     live DOM is touched only once both succeed, so a construction error
 *     never leaves a hidden textarea with no editor to show for it.
 *
 * What differs per editor — the language grammar, syntax highlighting,
 * lint, fold — is not this helper's concern: callers pass it all in via
 * `options`, except the JSON token-color palette every JSON-editing page
 * shares (`jsonHighlight()` below, #840) — one preset rather than a copy
 * per caller. This file only owns the wrapper/sync/back-out plumbing above
 * and that shared preset.
 *
 * `window.HfsCodeEditor.mount(textarea, options)` returns the `EditorView`
 * it built, or `null` if `window.HfsCodeMirror` (the vendored bundle)
 * is not loaded, if `textarea` is missing, or if anything throws
 * during construction — a caller that gets `null` back leaves its own
 * degradation to the plain textarea, exactly like this file does for
 * itself.
 *
 * `options`:
 *   - `language`  — a single language `Extension` (e.g. `LanguageSupport`).
 *   - `highlight` — one `Extension` or an array of them, already wrapped in
 *                   `syntaxHighlighting(...)` by the caller (this helper
 *                   does not know what a HighlightStyle belongs to).
 *   - `extensions`— an array of additional extensions (e.g. a linter).
 *   - `fold`      — `true` to add a fold gutter and the fold keymap.
 *   - `completion`— (#821) an array of `@codemirror/autocomplete`
 *                   `CompletionSource` functions; when present, wires
 *                   `autocompletion({ override: completion, activateOnTyping:
 *                   true, maxRenderedOptions: 300 })` — the library's own
 *                   default (100) can silently cut off a real match — and
 *                   puts `completionKeymap` ahead of
 *                   `defaultKeymap` (so Enter/Escape/Ctrl-Space are the
 *                   popup's own — CM6's completion commands no-op and fall
 *                   through to the next binding when no popup is open).
 *                   Tab accepts the highlighted completion only while the
 *                   popup is open; with no popup it moves focus out of the
 *                   editor; it never indents. Only the ViewDefinition
 *                   editor (`vd-editor.js`) passes this; the SQL pane
 *                   editors are unaffected.
 *   - `wrapperClass` — extra class name(s) on the wrapper, alongside the
 *                   shared `code-editor` class every mount gets.
 *   - `id`        — id attribute for the wrapper element.
 *   - `format`    — (#1757) `"json"` binds Shift+Alt+F to `format(view)`
 *                   (below), ahead of `defaultKeymap`. Without the option
 *                   nothing is bound. The ViewDefinition editor and the
 *                   Library Details JSON editor pass it; the SQL editors
 *                   do not.
 *                   It also reveals the enclosing `.card`'s
 *                   `[data-editor-format]` button (rendered `hidden`) and
 *                   reports an invalid document in the card's
 *                   `[data-editor-format-status]` element, from its
 *                   `data-msg-invalid` attribute.
 *
 * `HfsCodeEditor.format(view)` rewrites only the whitespace of a JSON
 * document: `formatJson(text)` copies every string, number and literal
 * verbatim (the parser is used to validate, never to rebuild the text),
 * and `mapOffset(before, after, offset)` carries the cursor across. It
 * dispatches one transaction tagged `userEvent: "input.format"` (one undo
 * restores the old text) and returns `"formatted"`, `"unchanged"` or
 * `"invalid"` (nothing dispatched); in all three cases it then dispatches a
 * bubbling `hfs:editor-format` `CustomEvent` on `view.dom` with
 * `detail.result`.
 *
 * `window.HfsCodeEditor.jsonHighlight()` returns a `HighlightStyle`,
 * scoped to `window.HfsCodeMirror.jsonLanguage`, coloring the outer JSON's
 * five token classes (`cmt-json-key`/`-string`/`-number`/`-literal`/
 * `-punct`, `app.css`) — pass it through `syntaxHighlighting(...)` in
 * `options.highlight` the same as any other `HighlightStyle`. `null` if
 * `window.HfsCodeMirror` is not loaded.
 */
(function (root, factory) {
  "use strict";

  // Same UMD-ish shape as `vd-editor.js`: Node gets the API through
  // `module.exports` (for `code-editor.test.cjs`) and nothing else is
  // touched; a browser also gets `window.HfsCodeEditor`.
  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsCodeEditor = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  function toExtensionArray(value) {
    if (value == null) return [];
    return Array.isArray(value) ? value : [value];
  }

  function mount(textarea, options) {
    var CM = window.HfsCodeMirror;
    if (!CM || !textarea) return null;
    options = options || {};

    // The card's `[data-editor-format-status]` element, set once the Format
    // button is wired (`options.format === "json"`); the change listener
    // below clears it on every edit.
    var formatStatus = null;

    // Shift+Alt+F, only with `options.format === "json"`.
    function formatOnShortcut(view) {
      format(view);
      return true;
    }

    // Tab accepts the highlighted completion only while the popup is open.
    // Once it is open the command always reports the key as handled, even
    // when `acceptCompletion` declines during the library's short initial
    // interaction delay: focus must never jump out from under a visible
    // popup. With no popup it returns `false` and Tab moves focus on.
    function acceptCompletionOnTab(view) {
      if (CM.completionStatus(view.state) !== "active") return false;
      CM.acceptCompletion(view);
      return true;
    }

    try {
      var wrapper = document.createElement("div");
      wrapper.className = options.wrapperClass
        ? "code-editor " + options.wrapperClass
        : "code-editor";
      if (options.id) wrapper.id = options.id;

      var extensions = [];
      if (options.language) extensions.push(options.language);
      extensions = extensions.concat(toExtensionArray(options.highlight));
      extensions = extensions.concat(toExtensionArray(options.extensions));
      extensions.push(
        CM.lineNumbers(),
        CM.highlightActiveLine(),
        CM.highlightActiveLineGutter(),
        CM.drawSelection()
      );
      if (options.fold) extensions.push(CM.foldGutter());
      if (options.completion) {
        extensions.push(
          CM.autocompletion({
            override: options.completion,
            activateOnTyping: true,
            // @codemirror/autocomplete's own default (100) can silently cut
            // off a real match past the fold: a FHIRPath member chain with
            // every element of a type plus the full function catalog easily
            // clears that, and a candidate ordered near the end (e.g.
            // "where") would never render at all without scrolling ever
            // reaching it. 300 comfortably covers the largest response this
            // crate's own `/complete` sends today with room to grow.
            maxRenderedOptions: 300,
          }),
          // #821: axe's `scrollable-region-focusable` flags the popup's own
          // `<ul role="listbox">` — the library's baseTheme gives it
          // `max-height: 10em; overflow: hidden auto` once options overflow
          // it, but sets no `tabindex` of its own, and each `<li>` is a
          // plain, non-focusable node (keyboard navigation moves
          // `aria-selected` while the *editor's* content stays focused, an
          // `aria-activedescendant` pattern — unlike `.cm-scroller` below,
          // there is no focusable descendant inside this popup to credit
          // instead). A negative `tabindex` does not satisfy this specific
          // check (axe's own `focusable-element` test requires the element
          // in the real tab order, not merely script-focusable), so this
          // needs `tabindex="0"`: the tooltip mounts as a sibling inside
          // `.cm-editor` itself (no custom `EditorView.tooltips` parent is
          // configured anywhere in this crate), later in DOM order than
          // `.cm-content`'s own tabindex. Tab no longer lands on it in
          // practice: while the popup is open Tab accepts the highlighted
          // completion (`acceptCompletionOnTab`, bound in the keymap
          // below), so the attribute exists for the scan, not as a Tab
          // stop. Set on every update rather than once on creation: the
          // tooltip's own `<ul>` is torn down and rebuilt each time the
          // popup closes and reopens.
          CM.EditorView.updateListener.of(function (update) {
            if (!CM.completionStatus(update.state)) return;
            var doc = update.view.dom.ownerDocument;
            var list = doc.querySelector(".cm-tooltip-autocomplete ul[role='listbox']:not([tabindex])");
            if (list) list.setAttribute("tabindex", "0");
          })
        );
      }
      extensions.push(
        CM.bracketMatching(),
        CM.closeBrackets(),
        CM.indentOnInput(),
        CM.indentUnit.of("  "),
        CM.history(),
        CM.highlightSelectionMatches(),
        // The plain textarea this replaces soft-wraps by default; matching
        // that here avoids a new horizontal scrollbar on long lines the
        // user never had before.
        CM.EditorView.lineWrapping,
        CM.EditorView.contentAttributes.of({
          "aria-label": textarea.getAttribute("aria-label") || "",
        }),
        // `.cm-content` is `contenteditable`, genuinely reachable by Tab in
        // every real browser without this — but axe's own
        // `scrollable-region-focusable` check does not credit a bare
        // `contenteditable` as "focusable content" for its *ancestor*
        // `.cm-scroller` (`overflow: auto` in the shared chrome, #838) —
        // only an element that itself carries an explicit `tabindex`. A
        // separate `contentAttributes.of` call merges its key into the one
        // above via CodeMirror's own facet combination (#840, generalized
        // off #838's ViewDefinition-only copy of this rule).
        CM.EditorView.contentAttributes.of({ tabindex: "0" }),
        // Tab never indents (no indent command is bound). With
        // `options.completion`, Tab accepts the highlighted completion only
        // while the popup is open (`acceptCompletionOnTab`); otherwise it
        // moves focus to the next form control. `completionKeymap` ahead of
        // `defaultKeymap` so a popup's Enter/Escape/arrow keys win while it
        // is open; every one of its commands returns `false` (letting the
        // keymap fall through to the next binding) when no completion is
        // active, so typing Enter for a plain newline is unaffected.
        CM.keymap.of(
          [].concat(
            CM.closeBracketsKeymap,
            options.format === "json"
              ? [{ key: "Shift-Alt-f", preventDefault: true, run: formatOnShortcut }]
              : [],
            options.completion
              ? [{ key: "Tab", run: acceptCompletionOnTab }].concat(CM.completionKeymap)
              : [],
            CM.defaultKeymap,
            CM.historyKeymap,
            options.fold ? CM.foldKeymap : [],
            CM.searchKeymap
          )
        ),
        CM.EditorView.updateListener.of(function (update) {
          if (!update.docChanged) return;
          textarea.value = update.state.doc.toString();
          // Native-input parity for anything else listening on the form.
          textarea.dispatchEvent(new Event("input", { bubbles: true }));
          if (formatStatus) formatStatus.textContent = "";
        })
      );

      var view = new CM.EditorView({
        parent: wrapper,
        state: CM.EditorState.create({ doc: textarea.value, extensions: extensions }),
      });

      // Only now that both the wrapper and the view exist does the live DOM
      // change: insert the wrapper, then hide the textarea it replaces (CSS
      // class `code-editor__source--mounted`, app.css) from view and the
      // accessibility tree, so it does not duplicate the editor's own
      // `role="textbox"` landmark.
      textarea.parentNode.insertBefore(wrapper, textarea);
      textarea.classList.add("code-editor__source--mounted");

      // Format button (#1757): the card's `[data-editor-format]` button is
      // rendered `hidden` and only revealed here, once the editor is live.
      // The invalid-JSON message comes from `data-msg-invalid` on the status
      // element, so no visible string lives in this file.
      if (options.format === "json") {
        var card = textarea.closest(".card");
        var button = card && card.querySelector("[data-editor-format]");
        if (button) {
          formatStatus = card.querySelector("[data-editor-format-status]");
          button.hidden = false;
          button.addEventListener("click", function () {
            format(view);
            view.focus();
          });
          view.dom.addEventListener("hfs:editor-format", function (event) {
            if (!formatStatus) return;
            formatStatus.textContent =
              event.detail && event.detail.result === "invalid"
                ? formatStatus.dataset.msgInvalid || ""
                : "";
          });
        }
      }
      return view;
    } catch (unavailable) {
      return null;
    }
  }

  /* The JSON token-color preset every JSON-editing page shares (#840,
   * lifted verbatim out of the ViewDefinition editor's own copy): classes
   * only, every actual color lives in `app.css` as a CSS variable, scoped
   * to `jsonLanguage` so it only ever paints the outer JSON grammar, never
   * a language injected into one of its string values (`vd-editor.js`'s own
   * FHIRPath HighlightStyle stays separate and scoped to its own
   * language for exactly that reason). */
  function jsonHighlight() {
    var CM = window.HfsCodeMirror;
    if (!CM) return null;
    return CM.HighlightStyle.define(
      [
        { tag: CM.tags.propertyName, class: "cmt-json-key" },
        { tag: CM.tags.string, class: "cmt-json-string" },
        { tag: CM.tags.number, class: "cmt-json-number" },
        { tag: [CM.tags.bool, CM.tags.null], class: "cmt-json-literal" },
        { tag: [CM.tags.separator, CM.tags.squareBracket, CM.tags.brace], class: "cmt-json-punct" },
      ],
      { scope: CM.jsonLanguage }
    );
  }

  /* ---- JSON format (#1757) -------------------------------------------
   *
   * Pure helpers, no CodeMirror: `formatJson` and `mapOffset` are exercised
   * directly under Node by `code-editor.test.cjs`. */

  var WHITESPACE = " \t\n\r";

  function newlineAndIndent(level) {
    var out = "\n";
    for (var i = 0; i < level; i++) out += "  ";
    return out;
  }

  /* Formatted copy of `text`, or `null` when `text` is not valid JSON. Only
   * whitespace outside strings is dropped or added; every other character
   * is copied as it appears. The `JSON.parse` call below only decides
   * valid/invalid, its result is discarded. */
  function formatJson(text) {
    try {
      JSON.parse(text);
    } catch (invalid) {
      return null;
    }
    var out = "";
    var level = 0;
    var inString = false;
    for (var i = 0; i < text.length; i++) {
      var ch = text.charAt(i);
      if (inString) {
        out += ch;
        if (ch === "\\") {
          i++;
          out += text.charAt(i);
        } else if (ch === '"') {
          inString = false;
        }
        continue;
      }
      if (WHITESPACE.indexOf(ch) !== -1) continue;
      if (ch === "{" || ch === "[") {
        var close = ch === "{" ? "}" : "]";
        var next = i + 1;
        while (next < text.length && WHITESPACE.indexOf(text.charAt(next)) !== -1) next++;
        if (text.charAt(next) === close) {
          out += ch + close;
          i = next;
        } else {
          level++;
          out += ch + newlineAndIndent(level);
        }
      } else if (ch === "}" || ch === "]") {
        level--;
        out += newlineAndIndent(level) + ch;
      } else if (ch === ",") {
        out += "," + newlineAndIndent(level);
      } else if (ch === ":") {
        out += ": ";
      } else {
        if (ch === '"') inString = true;
        out += ch;
      }
    }
    return out;
  }

  /* Indexes of the significant characters of `text`: every character
   * inside a string (quotes included) and every non-whitespace character
   * outside one. */
  function significantIndexes(text) {
    var indexes = [];
    var inString = false;
    for (var i = 0; i < text.length; i++) {
      var ch = text.charAt(i);
      if (inString) {
        indexes.push(i);
        if (ch === "\\" && i + 1 < text.length) {
          i++;
          indexes.push(i);
        } else if (ch === '"') {
          inString = false;
        }
      } else if (WHITESPACE.indexOf(ch) === -1) {
        indexes.push(i);
        if (ch === '"') inString = true;
      }
    }
    return indexes;
  }

  /* The position in `after` right behind as many significant characters
   * as precede `offset` in `before`. */
  function mapOffset(before, after, offset) {
    var count = 0;
    var seen = significantIndexes(before);
    while (count < seen.length && seen[count] < offset) count++;
    if (count === 0) return 0;
    var target = significantIndexes(after);
    if (count > target.length) return after.length;
    return target[count - 1] + 1;
  }

  /* Formats the JSON document of `view` in one undoable transaction.
   * Returns "formatted", "unchanged" or "invalid"; announces the result on
   * `view.dom` as a bubbling `hfs:editor-format` event either way. */
  function format(view) {
    var before = view.state.doc.toString();
    var after = formatJson(before);
    var result;
    if (after === null) {
      result = "invalid";
    } else if (after === before) {
      result = "unchanged";
    } else {
      var head = view.state.selection.main.head;
      view.dispatch({
        changes: { from: 0, to: before.length, insert: after },
        selection: { anchor: mapOffset(before, after, head) },
        scrollIntoView: true,
        userEvent: "input.format",
      });
      result = "formatted";
    }
    view.dom.dispatchEvent(
      new CustomEvent("hfs:editor-format", { bubbles: true, detail: { result: result } })
    );
    return result;
  }

  return {
    mount: mount,
    jsonHighlight: jsonHighlight,
    format: format,
    formatJson: formatJson,
    mapOffset: mapOffset,
  };
});
