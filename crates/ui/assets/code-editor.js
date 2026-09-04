/*
 * Shared CodeMirror 6 mount helper (#838, generalized out of
 * #753's original `vd-editor.js`).
 *
 * Every editor in this crate — the ViewDefinition JSON editor today, the SQL
 * pane editors — needs the exact same progressive-
 * enhancement contract over its `<textarea>`:
 *
 *   - the textarea stays in the DOM and stays the form's source of truth,
 *     so Save/Duplicate/Enter (plain POSTs) keep submitting what the editor
 *     shows, with or without this script;
 *   - every document change is written straight back into `textarea.value`
 *     and fires a bubbling `input` event, for parity with a native input and
 *     for any other script listening on the form;
 *   - the editor exposes the textarea's own `aria-label` on its own
 *     `role="textbox"` content, so the hidden textarea does not shadow it;
 *   - Tab is never captured for indentation — it keeps moving focus to the
 *     next form control, like it does over a plain textarea;
 *   - the wrapper and the `EditorView` are built fully in memory and the
 *     live DOM is touched only once both succeed, so a construction error
 *     never leaves a hidden textarea with no editor to show for it.
 *
 * What differs per editor — the language grammar, syntax highlighting,
 * lint, fold — is not this helper's concern: callers pass it all in via
 * `options`. This file only owns the wrapper/sync/back-out plumbing above.
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
 *                   through to the next binding when no popup is open —
 *                   without touching Tab: no command in `completionKeymap`
 *                   binds it, so it keeps moving focus to the next form
 *                   control exactly as it does today, in and out of the
 *                   popup alike). Only the ViewDefinition editor
 *                   (`vd-editor.js`) passes this; the SQL pane editors are
 *                   unaffected.
 *   - `wrapperClass` — extra class name(s) on the wrapper, alongside the
 *                   shared `code-editor` class every mount gets.
 *   - `id`        — id attribute for the wrapper element.
 */
(function () {
  "use strict";

  function toExtensionArray(value) {
    if (value == null) return [];
    return Array.isArray(value) ? value : [value];
  }

  function mount(textarea, options) {
    var CM = window.HfsCodeMirror;
    if (!CM || !textarea) return null;
    options = options || {};

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
          // `.cm-content`'s own tabindex — so Tab, pressed while a popup
          // happens to be open, reaches it as one extra, real stop before
          // whatever the next actual form control is, rather than "trapping"
          // anything: a second Tab moves on exactly as it always did. Set on
          // every update rather than once on creation: the tooltip's own
          // `<ul>` is torn down and rebuilt each time the popup closes and
          // reopens.
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
        // No indentWithTab: Tab must keep moving focus to the next form
        // control, not indent inside the editor. `completionKeymap` ahead
        // of `defaultKeymap` so a popup's Enter/Escape/arrow keys win while
        // it is open; every one of its commands returns `false` (letting
        // the keymap fall through to the next binding) when no completion
        // is active, so typing Enter for a plain newline is unaffected.
        CM.keymap.of(
          [].concat(
            CM.closeBracketsKeymap,
            options.completion ? CM.completionKeymap : [],
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
      return view;
    } catch (unavailable) {
      return null;
    }
  }

  window.HfsCodeEditor = { mount: mount };
})();
